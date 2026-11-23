use crate::{Document, Property, Shape, Stencil, VectorObject, MAX_FRAMES};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, sync::Arc};
use tiny_skia_path::{Point, Transform};

/// Authored cutter shared by the members of a clipped selection.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClipSource {
    pub object: VectorObject,
    pub stack_index: usize,
    pub grouped: bool,
    /// The upper path's paint box at capture anchors one mask ramp for the whole group.
    pub mask_space: [f32; 6],
}

/// The paint box at capture maps the original cutter onto the target's current pose.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClipBinding {
    pub source: Arc<ClipSource>,
    pub paint_space: [f32; 6],
    pub original_target: u64,
    pub original_layer: u64,
}

impl PartialEq for ClipBinding {
    fn eq(&self, other: &Self) -> bool {
        self.paint_space == other.paint_space
            && self.original_layer == other.original_layer
            && self.original_target == other.original_target
            && self.same_source(other)
    }
}

fn matrix(t: Transform) -> [f32; 6] {
    [t.sx, t.ky, t.kx, t.sy, t.tx, t.ty]
}

fn transform([a, b, c, d, e, f]: [f32; 6]) -> Transform {
    Transform::from_row(a, b, c, d, e, f)
}

fn same_transform(a: Transform, b: Transform) -> bool {
    matrix(a)
        .into_iter()
        .zip(matrix(b))
        .all(|(a, b)| (a - b).abs() <= 0.0001 + 4. * f32::EPSILON * a.abs().max(b.abs()))
}

impl ClipBinding {
    fn same_source(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.source, &other.source) || self.source == other.source
    }

    fn delta(&self, current: Transform) -> Option<Transform> {
        if self.paint_space == matrix(current) {
            return Some(Transform::identity());
        }
        Some(transform(self.paint_space).invert()?.post_concat(current))
    }

    pub(super) fn validate(
        &self,
        has_stencil: bool,
        checked: &mut HashSet<*const ClipSource>,
    ) -> Result<(), String> {
        let space = transform(self.paint_space);
        if !has_stencil
            || !space.is_finite()
            || space.invert().is_none()
            || self.original_target == 0
            || self.original_layer == 0
        {
            return Err("Invalid clipping source transform.".into());
        }
        if checked.insert(Arc::as_ptr(&self.source)) {
            let o = &self.source.object;
            let space = transform(self.source.mask_space);
            if !space.is_finite()
                || space.invert().is_none()
                || !cutter(o)
                || o.id == 0
                || o.layer == 0
                || o.name.chars().count() > 80
                || o.name.chars().any(char::is_control)
            {
                return Err("Invalid stored clipping path.".into());
            }
            o.validate_art_data(MAX_FRAMES)?;
        }
        Ok(())
    }
}

impl VectorObject {
    /// Compare authored masks before compositing neighboring members into one layer.
    pub fn shares_clip_mask(&self, other: &Self, frame: u32) -> bool {
        if self.group.is_none() || self.group != other.group || self.layer != other.layer {
            return false;
        }
        let (Some(a), Some(b)) = (&self.clip_source, &other.clip_source) else {
            return false;
        };
        if !a.source.grouped
            || !a.same_source(b)
            || self.stencil.as_ref().map(|s| s.alpha) != other.stencil.as_ref().map(|s| s.alpha)
        {
            return false;
        }
        self.paint_space(frame)
            .and_then(|space| a.delta(space))
            .zip(other.paint_space(frame).and_then(|space| b.delta(space)))
            .is_some_and(|(a, b)| same_transform(a, b))
    }

    pub fn stencil_alpha_space(&self, frame: u32) -> Option<Transform> {
        let space = self.paint_space(frame)?;
        if let Some(binding) = &self.clip_source {
            Some(transform(binding.source.mask_space).post_concat(binding.delta(space)?))
        } else {
            Some(self.stencil.as_ref()?.transform(space))
        }
    }
}

fn cutter(o: &VectorObject) -> bool {
    !o.is_instance()
        && o.image.is_none()
        && o.stencil.is_none()
        && o.clip_source.is_none()
        && o.group.is_none()
        && o.group_name.is_none()
        && o.skin.is_none()
        && o.rigid_bone.is_none()
}

impl Document {
    // The selection tool may pass a single representative of a layer-local group.
    fn clip_selection(&self, ids: &[u64]) -> Vec<usize> {
        if ids.is_empty() {
            return Vec::new();
        }
        let ids: HashSet<_> = ids.iter().copied().collect();
        let layers: HashSet<_> = self
            .layers
            .iter()
            .filter(|l| l.visible && !l.locked)
            .map(|l| l.id)
            .collect();
        let groups: HashSet<_> = self
            .objects
            .iter()
            .filter(|o| ids.contains(&o.id))
            .filter_map(|o| Some((o.layer, o.group?)))
            .collect();
        self.objects
            .iter()
            .enumerate()
            .filter(|(_, o)| {
                layers.contains(&o.layer)
                    && (ids.contains(&o.id)
                        || o.group.is_some_and(|g| groups.contains(&(o.layer, g))))
            })
            .map(|(i, _)| i)
            .collect()
    }

    fn clip_operands(&self, ids: &[u64]) -> Option<(usize, Vec<usize>)> {
        if ids.len() < 2 {
            return None;
        }
        let mut selected = self.clip_selection(ids);
        let front_layer = self
            .layers
            .iter()
            .rev()
            .find(|l| selected.iter().any(|&i| self.objects[i].layer == l.id))?
            .id;
        let at = selected
            .iter()
            .rposition(|&i| self.objects[i].layer == front_layer)?;
        let source = selected.remove(at);
        if !cutter(&self.objects[source])
            || selected.is_empty()
            || selected.iter().any(|&i| self.objects[i].stencil.is_some())
        {
            return None;
        }
        let first = &self.objects[selected[0]];
        if selected.len() > 1
            && (first.group.is_none()
                || selected.iter().any(|&i| {
                    let o = &self.objects[i];
                    (o.layer, o.group) != (first.layer, first.group)
                }))
        {
            return None;
        }
        Some((source, selected))
    }

    pub fn can_set_clip(&self, ids: &[u64]) -> bool {
        self.clip_operands(ids).is_some()
    }

    pub fn can_release_clip(&self, ids: &[u64]) -> bool {
        self.clip_selection(ids)
            .iter()
            .any(|&i| self.objects[i].stencil.is_some())
    }

    pub fn set_clip(
        &mut self,
        ids: &[u64],
        frame: u32,
        alpha: Option<[u8; 2]>,
    ) -> Result<Vec<u64>, String> {
        if frame >= self.frames {
            return Err("Choose a frame in the drawing.".into());
        }
        let (source, targets) = self
            .clip_operands(ids)
            .ok_or("Select a shape or group and an unclipped path above it.")?;
        let upper = &self.objects[source];
        let outline = upper
            .outline(frame)
            .ok_or("The clipping path has no area.")?;
        let mut stencils = Vec::with_capacity(targets.len());
        for &i in &targets {
            let target = &self.objects[i];
            let space = target
                .paint_space(frame)
                .ok_or("The target has no clipping area at this frame.")?;
            let stencil = Stencil::capture_outline(outline.clone(), upper.fill_rule, space, alpha)
                .ok_or("The clipping path and target need a nonempty shape at this frame.")?;
            stencils.push((stencil, matrix(space)));
        }
        let selection = targets.iter().map(|&i| self.objects[i].id).collect();
        let shared = Arc::new(ClipSource {
            object: upper.clone(),
            stack_index: source,
            grouped: targets.len() > 1,
            mask_space: matrix(
                upper
                    .paint_space(frame)
                    .ok_or("The clipping path has no area.")?,
            ),
        });
        for (i, (stencil, paint_space)) in targets.into_iter().zip(stencils) {
            self.objects[i].stencil = Some(stencil);
            self.objects[i].clip_source = Some(ClipBinding {
                source: shared.clone(),
                paint_space,
                original_target: self.objects[i].id,
                original_layer: self.objects[i].layer,
            });
        }
        self.objects.remove(source);
        Ok(selection)
    }

    pub fn release_clip(&mut self, ids: &[u64], frame: u32) -> Result<Vec<u64>, String> {
        if frame >= self.frames {
            return Err("Choose a frame in the drawing.".into());
        }
        let targets: Vec<_> = self
            .clip_selection(ids)
            .into_iter()
            .filter(|&i| self.objects[i].stencil.is_some())
            .collect();
        if targets.is_empty() {
            return Err("Select clipped or masked artwork.".into());
        }
        let mut shared: std::collections::HashMap<_, Vec<(&ClipBinding, Transform)>> =
            Default::default();
        let mut restored = Vec::new();
        for &i in targets.iter().rev() {
            let o = &self.objects[i];
            let space = o
                .paint_space(frame)
                .ok_or("The target has no clipping area at this frame.")?;
            if let Some(b) = &o.clip_source {
                let delta = b.delta(space).ok_or("Invalid clipping transform.")?;
                if b.source.grouped {
                    let peers = shared
                        .entry((o.layer, o.group, b.source.object.id))
                        .or_default();
                    if peers.iter().any(|(other, t)| {
                        b.original_target != other.original_target
                            && b.same_source(other)
                            && same_transform(delta, *t)
                    }) {
                        continue;
                    }
                    peers.push((b, delta));
                }
                let mut source = b.source.object.clone();
                let same_layer = source.layer == o.layer
                    || o.layer != b.original_layer
                    || !self
                        .layers
                        .iter()
                        .any(|l| l.id == source.layer && l.visible && !l.locked);
                if same_layer {
                    source.layer = o.layer
                }
                if source.last_keyframe().is_some_and(|f| f >= self.frames) {
                    for p in Property::ALL {
                        for t in p.frames(&source).into_iter().filter(|t| *t >= self.frames) {
                            p.remove(&mut source, t);
                        }
                    }
                }
                if !delta.is_identity() {
                    move_source(&mut source, frame, delta)?;
                }
                if b.original_target != o.id {
                    source.id = 0;
                }
                let index = if same_layer {
                    b.source.stack_index.max(i + 1)
                } else {
                    b.source.stack_index
                };
                restored.push((index, source));
            } else {
                restored.push((i + 1, legacy_source(o, space)?));
            }
        }
        let mut selection = Vec::with_capacity(targets.len() + restored.len());
        for i in targets {
            let o = &mut self.objects[i];
            o.stencil = None;
            o.clip_source = None;
            selection.push(o.id);
        }
        restored.sort_by_key(|(i, _)| *i);
        let mut taken: HashSet<_> = self.objects.iter().map(|o| o.id).collect();
        for (offset, (index, mut source)) in restored.into_iter().enumerate() {
            if source.id == 0 || taken.contains(&source.id) {
                source.id = self.next_id
            }
            taken.insert(source.id);
            while taken.contains(&self.next_id) {
                self.next_id = self.next_id.wrapping_add(1).max(1);
            }
            selection.push(source.id);
            self.objects
                .insert(index.saturating_add(offset).min(self.objects.len()), source);
        }
        Ok(selection)
    }
}

fn legacy_source(target: &VectorObject, space: Transform) -> Result<VectorObject, String> {
    let stencil = target.stencil.as_ref().unwrap();
    let path = stencil
        .outline(space)
        .ok_or("Unable to recover the clipping silhouette.")?;
    let bounds = path
        .compute_tight_bounds()
        .and_then(|b| b.to_non_zero_rect())
        .ok_or("The clipping path has no area.")?;
    let mut contours = crate::geometry::editable_contours(&path, |p| {
        [
            (p.x - bounds.x()) / bounds.width(),
            (p.y - bounds.y()) / bounds.height(),
        ]
    })
    .ok_or("Invalid clipping silhouette.")?;
    let shape = if contours.len() == 1 {
        Shape::Path {
            nodes: contours.pop().unwrap(),
            closed: true,
        }
    } else {
        Shape::Compound { contours }
    };
    let mut temp = Document::default();
    temp.add_shape(
        [bounds.x(), bounds.y()],
        [bounds.width(), bounds.height()],
        [0, 0, 0, 255],
        shape,
        target.layer,
    );
    let mut object = temp.objects.pop().unwrap();
    object.id = 0;
    object.fill_rule = stencil.fill_rule;
    Ok(object)
}

fn move_source(o: &mut VectorObject, frame: u32, delta: Transform) -> Result<(), String> {
    if same_transform(delta, Transform::from_translate(delta.tx, delta.ty)) {
        for p in std::iter::once(&mut o.position).chain(o.keys.values_mut()) {
            p[0] += delta.tx;
            p[1] += delta.ty;
        }
    } else {
        let sample = |p: [f32; 2]| {
            let [x, y] = o.local_to_world(p, frame);
            let mut p = Point::from_xy(x, y);
            delta.map_point(&mut p);
            p
        };
        let origin = sample([0., 0.]);
        let x = sample([1., 0.]) - origin;
        let y = sample([0., 1.]) - origin;
        let center = sample([0.5, 0.5]);
        let width = x.x.hypot(x.y);
        let height = (x.x * y.y - x.y * y.x) / width;
        let skew = ((x.x * y.x + x.y * y.y) / (width * height))
            .atan()
            .to_degrees();
        if !skew.is_finite() || skew.abs() > 85. || width <= 0. || height == 0. {
            return Err("The clipped transform exceeds the editable skew range.".into());
        }
        let angle = o.rotation_at(frame);
        let turn = (x.y.atan2(x.x).to_degrees() - angle + 180.).rem_euclid(360.) - 180.;
        o.set_rotation_at(frame, angle + turn);
        o.set_scale_at(frame, [width / o.size[0], height / o.size[1]]);
        if o.animated.skew_keys.is_empty() {
            o.animated.skew = skew
        } else {
            o.animated.skew_keys.insert(frame, skew);
        }
        o.set_position_at(
            frame,
            [center.x - o.size[0] * 0.5, center.y - o.size[1] * 0.5],
        );
    }
    o.validate_art_data(MAX_FRAMES)
}
