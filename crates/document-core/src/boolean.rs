//! Path operations on evaluated artboard geometry. Cubics are flattened to a
//! 0.02-pixel tolerance, then snapped to a 1/1024-pixel grid for 64-bit
//! integer overlays. Eligible filled results reuse evaluated Bézier handles for
//! isolated contours and surviving spans. Cut Path orders intersections by exact rational parameters.
mod curves;
mod cut;
mod flatten;
mod planar;
mod spans;

use crate::{Document, PathNode, Shape, VectorObject};
use std::collections::HashSet;

const COMPLEXITY:&str="The paths are too complex for a boolean operation. Simplify the selection or use fewer objects.";

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BooleanOp {
    Union,
    Difference,
    Intersection,
    Exclusion,
    Division,
    CutPath,
}

type Ring = Vec<[f64; 2]>;
type Piece = (Vec<Ring>, bool);

/// Bounds of a filled outline surviving a stencil. Use the same tolerance,
/// fill rules and complexity limits as the editor's Boolean intersections.
pub(crate) fn stencil_intersection_bounds(
    outline: &tiny_skia_path::Path,
    rule: crate::FillRule,
    masks: &[(tiny_skia_path::Path, crate::FillRule)],
) -> Result<Option<[f32; 4]>, String> {
    let mut budget = 4096;
    let mut inputs = vec![flatten::contours(outline, &mut budget)?];
    let mut rules = vec![rule];
    for (mask, mask_rule) in masks {
        inputs.push(flatten::contours(mask, &mut budget)?);
        rules.push(*mask_rule);
    }
    for contour in inputs.iter_mut().flatten() {
        contour.closed = true;
    }
    let pieces = planar::combine(&inputs, &rules, BooleanOp::Intersection)?;
    let mut points = pieces.into_iter().flat_map(|(rings, _)| rings).flatten();
    let Some([x, y]) = points.next() else {
        return Ok(None);
    };
    let bounds = points.fold([x, y, x, y], |[l, t, r, b], [x, y]| {
        [l.min(x), t.min(y), r.max(x), b.max(y)]
    });
    Ok(Some(bounds.map(|v| v as f32)))
}

impl Document {
    /// Replace selected objects with editable paths in the back operand's paint
    /// slot. Transforms and path effects are baked at the supplied frame.
    /// Errors leave the drawing and the ID allocator intact.
    pub fn boolean(&mut self, ids: &[u64], frame: u32, op: BooleanOp) -> Result<Vec<u64>, String> {
        if frame >= self.frames {
            return Err("Frame outside this drawing.".into());
        }
        let selected: HashSet<_> = ids.iter().copied().collect();
        if selected.len() != ids.len() || selected.len() < 2 {
            return Err("Select at least two different objects.".into());
        }
        if matches!(op, BooleanOp::Division | BooleanOp::CutPath) && ids.len() != 2 {
            return Err("Select exactly two objects for this operation.".into());
        }
        let unavailable = "All operands must exist on visible, unlocked layers.";
        let mut operands = Vec::with_capacity(ids.len());
        for (index, object) in self.objects.iter().enumerate() {
            if selected.contains(&object.id) {
                if object.locked {
                    return Err(
                        "Unlock the selected artwork before applying a Boolean operation.".into(),
                    );
                }
                let layer = self
                    .layers
                    .iter()
                    .position(|l| l.id == object.layer && l.visible && !l.locked)
                    .ok_or(unavailable)?;
                operands.push((layer, index));
                if operands.len() == ids.len() {
                    break;
                }
            }
        }
        if operands.len() != ids.len() {
            return Err(unavailable.into());
        }
        let first = operands[0].1;
        let last = operands.last().unwrap().1;
        // Layer paint order, then stable stacking order within each layer.
        operands.sort_unstable();
        let base_index = operands[0].1;
        let objects: Vec<_> = operands
            .iter()
            .map(|&(_, index)| &self.objects[index])
            .collect();
        if objects.iter().any(|o| o.image.is_some()) {
            return Err("Boolean operations require vector artwork.".into());
        }
        if objects.iter().any(|o| o.is_instance()) {
            return Err("Edit the symbol definition to operate on its paths.".into());
        }
        if objects.iter().any(|o| o.stencil.is_some()) {
            return Err("Release the clipping path or mask before combining this geometry.".into());
        }
        let mut inputs = Vec::with_capacity(objects.len());
        let mut outlines = Vec::new();
        let mut remaining = 4096;
        let keep_outlines = objects.len() <= curves::MAX_CONTOURS;
        for obj in &objects {
            let outline = obj
                .outline(frame)
                .ok_or("A selected object has no usable outline.")?;
            let mut contours = flatten::contours(&outline, &mut remaining)?;
            let open_allowed =
                op == BooleanOp::CutPath || (op == BooleanOp::Division && !inputs.is_empty());
            if !open_allowed {
                // SVG and the canvas implicitly close open subpaths when filling.
                // A cutter for Division or Cut Path retains its open centerline.
                for contour in &mut contours {
                    if !contour.closed && contour.points.first() == contour.points.last() {
                        contour.points.pop();
                    }
                    let points = &contour.points;
                    if points.len() < 3
                        || points[2..].iter().all(|p| {
                            let (a, b) = (points[0], points[1]);
                            i128::from(b[0] - a[0]) * i128::from(p[1] - a[1])
                                == i128::from(b[1] - a[1]) * i128::from(p[0] - a[0])
                        })
                    {
                        return Err(
                            "The selected outline has no fill area. Use Cut path for a line."
                                .into(),
                        );
                    }
                    contour.closed = true;
                }
            }
            if keep_outlines {
                outlines.push(outline);
            }
            inputs.push(contours);
        }
        let base = objects[0];
        let base_id = base.id;
        let mut replacements = if op == BooleanOp::CutPath {
            planar::check_arrangement_budget(&inputs)?;
            cut::objects(base, frame, &inputs[0], &inputs[1], &outlines[0])?
        } else {
            let rules: Vec<_> = objects.iter().map(|o| o.fill_rule).collect();
            let pieces = planar::combine(&inputs, &rules, op)?;
            let originals = curves::originals(&outlines, &inputs, &pieces);
            let spans = spans::CurveSpans::collect(&outlines, &inputs, &pieces);
            pieces
                .into_iter()
                .map(|(rings, closed)| {
                    path_object(base, frame, rings, closed, |ring| {
                        originals
                            .iter()
                            .find_map(|source| source.nodes_for(ring))
                            .or_else(|| spans.as_ref().and_then(|s| s.nodes_for(ring)))
                    })
                })
                .collect::<Result<Vec<_>, _>>()?
        };
        if replacements.len() > 1 {
            let mut used: HashSet<_> = self.objects.iter().map(|o| o.id).collect();
            for obj in replacements.iter_mut().skip(1) {
                obj.id = self.next_id;
                used.insert(obj.id);
                while used.contains(&self.next_id) {
                    self.next_id = self.next_id.wrapping_add(1).max(1);
                }
            }
        }
        let result = replacements.iter().map(|o| o.id).collect();
        if last - first + 1 == operands.len() {
            self.objects.splice(first..=last, replacements);
        } else {
            // Remove the cutters in the affected span. Vec moves the untouched tail
            // as one block. The back operand can lie after a cutter in storage order.
            let removed_before_base = operands
                .iter()
                .filter(|&&(_, index)| index < base_index)
                .count();
            self.objects
                .extract_if(first..=last, |o| {
                    o.id != base_id && selected.contains(&o.id)
                })
                .for_each(drop);
            let slot = base_index - removed_before_base;
            self.objects.splice(slot..=slot, replacements);
        }
        Ok(result)
    }
}

fn path_object(
    base: &VectorObject,
    frame: u32,
    rings: Vec<Ring>,
    closed: bool,
    mut recover: impl FnMut(&Ring) -> Option<Vec<PathNode>>,
) -> Result<VectorObject, String> {
    let mut low = [f64::INFINITY; 2];
    let mut high = [f64::NEG_INFINITY; 2];
    for point in rings.iter().flatten() {
        for a in 0..2 {
            low[a] = low[a].min(point[a]);
            high[a] = high[a].max(point[a]);
        }
    }
    let position = low.map(|v| v as f32);
    // Open horizontal and vertical cuts have no extent on one axis.
    let size = std::array::from_fn(|a| {
        if high[a] == low[a] {
            1.
        } else {
            (high[a] - low[a]) as f32
        }
    });
    let local = |p: [f64; 2]| {
        std::array::from_fn(|axis| {
            ((p[axis] - f64::from(position[axis])) / f64::from(size[axis])) as f32
        })
    };
    let mut contours = Vec::with_capacity(rings.len());
    for ring in rings {
        if let Some(mut nodes) = recover(&ring) {
            for n in &mut nodes {
                n.point = local(n.point.map(f64::from));
                n.incoming = local(n.incoming.map(f64::from));
                n.outgoing = local(n.outgoing.map(f64::from));
            }
            contours.push(nodes);
            continue;
        }
        let mut nodes = Vec::with_capacity(ring.len());
        for p in ring {
            let node = PathNode::corner(local(p));
            if nodes.last() != Some(&node) {
                nodes.push(node);
            }
        }
        if closed && nodes.last() == nodes.first() {
            nodes.pop();
        }
        if nodes.len() < if closed { 3 } else { 2 } {
            return Err("The result is smaller than the path coordinate precision.".into());
        }
        contours.push(nodes);
    }
    let mut object = base.clone();
    // The replacement contours already contain the evaluated artboard pose.
    object.skin = None;
    object.rigid_bone = None;
    object.rig_offset = Default::default();
    object.rig_pose = Default::default();
    object.shape = if contours.len() == 1 {
        Shape::Path {
            nodes: contours.pop().unwrap(),
            closed,
        }
    } else {
        Shape::Compound { contours }
    };
    if !object.shape.valid_nodes(size) {
        return Err(COMPLEXITY.into());
    }
    object.position = position;
    object.size = size;
    object.rotation = 0.;
    object.scale = [1.; 2];
    object.opacity = base.opacity_at(frame);
    object.fill = base.fill_at(frame);
    object.stroke = base.stroke_at(frame);
    object.stroke_width = base.stroke_width_at(frame);
    object.stroke_profile.clear();
    object.animated = crate::AnimatedProperties {
        visible: base.visible_at(frame),
        ..Default::default()
    };
    object.motion = None;
    object.keys.clear();
    object.rotation_keys.clear();
    object.scale_keys.clear();
    object.opacity_keys.clear();
    object.effects.clear();
    object.text = None;
    object.fill_rule = crate::FillRule::NonZero;
    if base.pivot.is_some() {
        // The result has a new local bounds box. Keep the artist's world-space hinge.
        object.pivot = None;
        object.set_pivot_at(0, base.pivot_at(frame));
    }
    Ok(object)
}

#[cfg(test)]
mod tests;
