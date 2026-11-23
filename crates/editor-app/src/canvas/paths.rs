use super::*;
use crate::path_editor::NodeSet;
use document_core::{
    path_edit::{HandleSide, HandleStyle},
    VectorObject,
};
mod continuation;
mod drag;
mod nearest;
pub(super) use continuation::{Continuation, Endpoints};
pub(super) use drag::NodeDrag;

pub(super) const HANDLE_DRAG_DEAD_ZONE: f32 = 6.;

/// The active Pen node and its handles stay in artboard coordinates.
#[derive(Clone, Copy)]
pub(super) struct PenDrag {
    pub(super) anchor: Pos2,
    origin: Pos2,
    incoming: Pos2,
    outgoing: Pos2,
    split: bool,
    dragged: bool,
    endpoint: Option<EndpointPress>,
}

#[derive(Clone, Copy)]
struct EndpointPress {
    original: PathNode,
    closing: bool,
}

/// Inspect movement between the primary press and release, including a drag that
/// left and returned to its origin within one frame.
fn first_drag_delta(
    ui: &egui::Ui,
    view: View,
    origin: Pos2,
    pressed_this_frame: bool,
    threshold: f32,
) -> Option<Vec2> {
    ui.input(|input| {
        let events = &input.events;
        let begin = if pressed_this_frame {
            events
                .iter()
                .rposition(|e| {
                    matches!(
                        e,
                        egui::Event::PointerButton {
                            button: egui::PointerButton::Primary,
                            pressed: true,
                            ..
                        }
                    )
                })
                .map_or(0, |i| i + 1)
        } else {
            0
        };
        let movement = |pos| {
            let delta = view.world(pos) - origin;
            (delta.length() * view.scale >= threshold).then_some(delta)
        };
        for event in &events[begin..] {
            match event {
                egui::Event::PointerButton {
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    pos,
                    ..
                } => return movement(*pos),
                egui::Event::PointerMoved(pos) => {
                    if let Some(delta) = movement(*pos) {
                        return Some(delta);
                    }
                }
                _ => {}
            }
        }
        None
    })
}

impl PenDrag {
    pub(super) fn new(anchor: Pos2, origin: Pos2) -> Self {
        Self {
            anchor,
            origin,
            incoming: anchor,
            outgoing: anchor,
            split: false,
            dragged: false,
            endpoint: None,
        }
    }

    pub(super) fn close(first: PathNode, origin: Pos2) -> Self {
        Self::endpoint(first, origin, true)
    }

    pub(super) fn resume(node: PathNode, origin: Pos2) -> Self {
        Self::endpoint(node, origin, false)
    }

    fn endpoint(first: PathNode, origin: Pos2, closing: bool) -> Self {
        let anchor = Pos2::from(first.point);
        let incoming = Pos2::from(first.incoming);
        Self {
            anchor,
            origin,
            incoming,
            outgoing: if closing {
                anchor + (anchor - incoming)
            } else {
                first.outgoing.into()
            },
            split: false,
            dragged: false,
            endpoint: Some(EndpointPress {
                original: first,
                closing,
            }),
        }
    }

    pub(super) fn is_closing(&self) -> bool {
        self.endpoint.is_some_and(|press| press.closing)
    }

    pub(super) fn track_start(&mut self, ui: &egui::Ui, view: View, pressed_this_frame: bool) {
        if !self.dragged {
            self.dragged = first_drag_delta(
                ui,
                view,
                self.origin,
                pressed_this_frame,
                HANDLE_DRAG_DEAD_ZONE,
            )
            .is_some();
        }
    }

    pub(super) fn tangent_target(&self, pointer: Pos2) -> Pos2 {
        if self.dragged {
            self.anchor + (pointer - self.origin)
        } else {
            self.anchor
        }
    }

    pub(super) fn closing_guide(&self) -> Option<Pos2> {
        let close = self.endpoint?;
        (self.dragged && self.outgoing != Pos2::from(close.original.outgoing))
            .then_some(self.outgoing)
    }

    pub(super) fn update(
        &mut self,
        pointer: Pos2,
        snapped: Pos2,
        scale: f32,
        shift: bool,
    ) -> PathNode {
        if !self.dragged {
            return self
                .endpoint
                .map_or_else(|| PathNode::corner(self.anchor.into()), |end| end.original);
        }
        // An established drag can make a short handle or retract it near the anchor.
        let tip = if pointer.distance(self.origin) * scale < 3. {
            self.anchor
        } else {
            snapped
        };
        // Shift freezes the incoming handle where it was last displayed,
        // even if pointer movement arrives in the same frame as the modifier.
        if shift {
            self.split = true;
        } else if tip != self.outgoing {
            // Releasing Shift in place keeps the corner. A new unshifted
            // handle movement resumes the ordinary symmetric drag.
            self.split = false;
        }
        if !self.split {
            self.incoming = self.anchor - (tip - self.anchor);
        }
        self.outgoing = tip;
        if let Some(close) = self.endpoint {
            let mut first = close.original;
            let (control, target) = if close.closing {
                (&mut first.incoming, self.incoming)
            } else {
                (&mut first.outgoing, tip)
            };
            if Pos2::from(*control) != target {
                *control = target.into();
                let outgoing = Pos2::from(first.outgoing) - self.anchor;
                let tangent = self.anchor - Pos2::from(first.incoming);
                first.handle_style = if first.handle_style != HandleStyle::Corner
                    && outgoing.dot(tangent) > 0.
                    && (outgoing.normalized() - tangent.normalized()).length_sq() <= 1e-10
                {
                    HandleStyle::Smooth
                } else {
                    HandleStyle::Corner
                };
            }
            return first;
        }
        PathNode {
            handle_style: if self.split || tip == self.anchor {
                HandleStyle::Corner
            } else {
                HandleStyle::Symmetric
            },
            point: self.anchor.into(),
            incoming: self.incoming.into(),
            outgoing: tip.into(),
        }
    }
}

/// egui positions are logical screen points, independent of artboard zoom and display density.
pub(super) const CLOSE_RADIUS: f32 = 12.;

pub(super) fn finish_pen(
    nodes: &mut Vec<PathNode>,
    closed: bool,
    doc: &mut Document,
    style: &CanvasStyle,
) -> Option<u64> {
    if nodes.len() < if closed { 3 } else { 2 } {
        return None;
    }
    let id = doc
        .add_artboard_path(std::mem::take(nodes), closed, style.layer, style.fill)
        .ok()?;
    let object = doc.objects.last_mut()?;
    style.paint_new_object(object);
    Some(id)
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Part {
    Anchor,
    Incoming,
    Outgoing,
    SymmetricHandles,
}

#[derive(Clone, Copy)]
pub(super) struct NodeIndex {
    pub(super) contour: usize,
    pub(super) node: usize,
}

pub(super) fn nodes_at(
    object: &VectorObject,
    frame: u32,
    point: Pos2,
    radius: f32,
    scale: f32,
) -> Option<(NodeIndex, Part, Pos2)> {
    if object.text.is_some() {
        return None;
    }
    let mut best = None;
    let mut distance = radius;
    let world = object.local_to_world_at(frame);
    for (contour, nodes) in object.skinned_shape_at(frame).contours().iter().enumerate() {
        for (node, n) in nodes.iter().enumerate() {
            let anchor = Pos2::from(world(n.point));
            for (part, pos) in [
                (Part::Incoming, Pos2::from(world(n.incoming))),
                (Part::Outgoing, Pos2::from(world(n.outgoing))),
                (Part::Anchor, anchor),
            ] {
                if part != Part::Anchor && pos.distance(anchor) * scale <= 2. {
                    continue;
                }
                let d = pos.distance(point);
                if d <= distance {
                    best = Some((NodeIndex { contour, node }, part, pos));
                    distance = d;
                }
            }
        }
    }
    best
}

pub(super) fn move_node(
    object: &mut VectorObject,
    index: NodeIndex,
    part: Part,
    nodes: &NodeSet,
    frame: u32,
    point: Pos2,
) -> bool {
    if object.text.is_some() {
        return false;
    }
    if part == Part::Anchor {
        // The drag draft already contains this frame's sampled geometry.
        let shape = object.skin_shape_at(std::borrow::Cow::Borrowed(&object.shape), frame);
        let Some(contour) = shape.contours().get(index.contour) else {
            return false;
        };
        let Some(node) = contour.get(index.node) else {
            return false;
        };
        let delta = point - Pos2::from(object.local_to_world(node.point, frame));
        if delta == Vec2::ZERO {
            return false;
        }
        return crate::path_editor::transform_posed_nodes(object, nodes, frame, |p| {
            (Pos2::from(p) + delta).into()
        });
    }
    let Some(new) = object.unskin_local_point_at(
        index.contour,
        index.node,
        object.world_to_local(point.into(), frame),
        frame,
    ) else {
        return false;
    };
    let size = object.scaled_box_at(frame).1;
    let [sx, sy] = object.scale_at(frame).map(f32::signum);
    let shear = object.skew_at(frame).to_radians().tan() * sx * sy;
    let closed = !matches!(object.shape, Shape::Path { closed: false, .. });
    let Some(nodes) = object.shape.contours_mut().get_mut(index.contour) else {
        return false;
    };
    let has_in = closed || index.node > 0;
    let has_out = closed || index.node + 1 < nodes.len();
    let Some(node) = nodes.get_mut(index.node) else {
        return false;
    };
    match part {
        Part::Anchor => unreachable!(),
        Part::Incoming => return node.move_handle(HandleSide::Incoming, new, has_out, size, shear),
        Part::Outgoing => return node.move_handle(HandleSide::Outgoing, new, has_in, size, shear),
        Part::SymmetricHandles => {
            let anchor = Pos2::from(node.point);
            let opposite = (anchor - (Pos2::from(new) - anchor)).into();
            if (!has_out || node.outgoing == new) && (!has_in || node.incoming == opposite) {
                return false;
            }
            if has_out {
                node.outgoing = new;
            }
            if has_in {
                node.incoming = opposite;
            }
        }
    }
    true
}

fn lerp(a: Pos2, b: Pos2, t: f32) -> Pos2 {
    a + (b - a) * t
}

#[derive(Clone, Copy)]
pub(super) struct SegmentHit {
    contour: usize,
    start: usize,
    end: usize,
    t: f32,
    point: Pos2,
}

fn nearest_segment(
    shape: &Shape,
    world: impl Fn([f32; 2]) -> [f32; 2],
    point: Pos2,
    radius: f32,
) -> Option<SegmentHit> {
    let closed = !matches!(shape, Shape::Path { closed: false, .. });
    let mut best = None;
    let mut closest = radius;
    for (contour, nodes) in shape.contours().iter().enumerate() {
        for start in 0..nodes.len().saturating_sub(usize::from(!closed)) {
            let end = (start + 1) % nodes.len();
            let controls = [
                nodes[start].point,
                nodes[start].outgoing,
                nodes[end].incoming,
                nodes[end].point,
            ]
            .map(|p| Pos2::from(world(p)));
            if let Some((distance, t)) = nearest::on_cubic(controls, point, closest, radius / 64.) {
                closest = distance;
                let [a, b, c, d] = controls;
                let (ab, bc, cd) = (lerp(a, b, t), lerp(b, c, t), lerp(c, d, t));
                best = Some(SegmentHit {
                    contour,
                    start,
                    end,
                    t,
                    point: lerp(lerp(ab, bc, t), lerp(bc, cd, t), t),
                });
            }
        }
    }
    best
}

pub(crate) fn nearest_morph_point(
    shape: &Shape,
    screen: impl Fn([f32; 2]) -> [f32; 2],
    point: Pos2,
    radius: f32,
) -> Option<document_core::morph::MorphPoint> {
    nearest_segment(shape, screen, point, radius).map(|hit| document_core::morph::MorphPoint {
        segment: hit.start,
        t: hit.t,
    })
}

/// Locate the nearest visible cubic segment, then split it using de Casteljau.
/// The subdivision uses the original cubic so the authored silhouette is preserved.
pub(super) fn insert_near(
    object: &mut VectorObject,
    frame: u32,
    point: Pos2,
    radius: f32,
    allow_primitive: bool,
) -> Option<NodeIndex> {
    if object.text.is_some()
        || object.image.is_some()
        || object.is_instance()
        || object.skin.is_some()
        || !object.animated.geometry_keys.is_empty()
        || object.shape.contours().iter().map(Vec::len).sum::<usize>()
            >= document_core::MAX_PATH_NODES
    {
        return None;
    }
    let mut primitive = allow_primitive
        .then(|| document_core::path_edit::primitive_path(&object.shape))
        .flatten();
    let shape = primitive.as_ref().unwrap_or(&object.shape);
    let world = object.local_to_world_at(frame);
    let SegmentHit {
        contour,
        start: i,
        end: j,
        t,
        ..
    } = nearest_segment(shape, &world, point, radius)?;
    if allow_primitive || !(0.001..0.999).contains(&t) {
        // Primitive conversion already supplies corners and cardinal anchors.
        // An authored path already has an editable anchor at such a hit.
        let nodes = &shape.contours()[contour];
        let anchor = [i, j]
            .into_iter()
            .find(|&n| point.distance(Pos2::from(world(nodes[n].point))) <= radius);
        if let Some(node) = anchor {
            object.shape = primitive.take()?;
            return Some(NodeIndex { contour, node });
        }
    }
    // Nearby endpoint anchors were checked in screen space.
    // A tiny parameter can still be hundreds of pixels from the end of a long path.
    if t <= 0. || t >= 1. {
        return None;
    }
    let nodes = &shape.contours()[contour];
    let [a, b, c, d] = [
        nodes[i].point,
        nodes[i].outgoing,
        nodes[j].incoming,
        nodes[j].point,
    ]
    .map(Pos2::from);
    let ab = lerp(a, b, t);
    let bc = lerp(b, c, t);
    let cd = lerp(c, d, t);
    let incoming = lerp(ab, bc, t);
    let outgoing = lerp(bc, cd, t);
    let node = PathNode {
        handle_style: HandleStyle::Smooth,
        point: lerp(incoming, outgoing, t).into(),
        incoming: incoming.into(),
        outgoing: outgoing.into(),
    };
    // Extreme transforms can round a normalized anchor well outside the hit radius.
    let distance = point.distance(Pos2::from(world(node.point)));
    if !distance.is_finite() || distance > radius {
        return None;
    }
    if let Some(shape) = primitive {
        object.shape = shape;
    }
    let nodes = &mut object.shape.contours_mut()[contour];
    nodes[i].outgoing = ab.into();
    nodes[j].incoming = cd.into();
    nodes.insert(i + 1, node);
    Some(NodeIndex {
        contour,
        node: i + 1,
    })
}

#[cfg(test)]
mod compound_tests {
    use super::*;

    #[test]
    fn a_smooth_handle_drag_preserves_the_opposite_length_in_the_transformed_drawing() {
        let mut doc = Document::default();
        let nodes = vec![
            PathNode::corner([0., 0.]),
            PathNode {
                point: [0.5, 0.5],
                incoming: [0.4, 0.5],
                outgoing: [0.8, 0.5],
                handle_style: HandleStyle::Smooth,
            },
            PathNode::corner([1., 1.]),
        ];
        doc.add_shape(
            [30., 40.],
            [110., 30.],
            [255; 4],
            Shape::Path {
                nodes,
                closed: false,
            },
            1,
        );
        let o = &mut doc.objects[0];
        o.rotation = 29.;
        o.animated.skew = 32.;
        let world = o.local_to_world_at(0);
        let anchor = Pos2::from(world([0.5, 0.5]));
        let old_length = anchor.distance(world([0.4, 0.5]).into());
        let target = Pos2::from(world([0.6, 0.9]));
        assert!(move_node(
            o,
            NodeIndex {
                contour: 0,
                node: 1
            },
            Part::Outgoing,
            &NodeSet::range(0, 1, 1),
            0,
            target
        ));
        let node = o.shape.contours()[0][1];
        let incoming = Pos2::from(world(node.incoming)) - anchor;
        let outgoing = Pos2::from(world(node.outgoing)) - anchor;
        assert!((incoming.length() - old_length).abs() < 1e-4);
        assert!(incoming.normalized().dot(outgoing.normalized()) < -0.99999);
        assert!(Pos2::from(world(node.outgoing)).distance(target) < 1e-4);
    }

    #[test]
    fn a_split_that_rounds_outside_the_hit_radius_preserves_the_curve() {
        let mut doc = Document::default();
        let mut a = PathNode::corner([0., 0.]);
        a.outgoing = [0.715336, 0.13258588];
        let mut b = PathNode::corner([1., 1.]);
        b.incoming = [0.079990104, 0.52996135];
        let shape = Shape::Path {
            nodes: vec![a, b],
            closed: false,
        };
        doc.add_shape(
            [-25_216_084., -21_718_348.],
            [60_000_000.; 2],
            [255; 4],
            shape.clone(),
            1,
        );
        let o = &mut doc.objects[0];
        assert!(insert_near(o, 0, Pos2::new(0.037, 0.061), 0.125, true).is_none());
        assert_eq!(o.shape, shape);
    }

    #[test]
    fn editing_a_hole_keeps_the_outer_contour_and_its_winding() {
        let mut doc = Document::default();
        let outer = [[0., 0.], [1., 0.], [1., 1.], [0., 1.]]
            .map(PathNode::corner)
            .to_vec();
        let hole = [[0.25, 0.25], [0.25, 0.75], [0.75, 0.75], [0.75, 0.25]]
            .map(PathNode::corner)
            .to_vec();
        doc.add_shape(
            [0.; 2],
            [80.; 2],
            [255; 4],
            Shape::Compound {
                contours: vec![outer.clone(), hole],
            },
            1,
        );
        let o = &mut doc.objects[0];
        let (index, part, _) = nodes_at(o, 0, egui::pos2(20., 20.), 2., 1.).unwrap();
        assert!(move_node(
            o,
            index,
            part,
            &NodeSet::range(index.contour, index.node, index.node),
            0,
            egui::pos2(21., 20.)
        ));
        assert!(insert_near(o, 0, egui::pos2(40., 60.), 2., false).is_some());
        assert_eq!(o.shape.contours()[1].len(), 5);
        let (index, _, _) = nodes_at(o, 0, egui::pos2(40., 60.), 2., 1.).unwrap();
        assert!(crate::path_editor::delete_node_at(
            &egui::Context::default(),
            o,
            0,
            crate::timeline::KeyMode::Tracked,
            index.contour,
            index.node,
        ));
        assert_eq!(o.shape.contours()[1].len(), 4);
        assert_eq!(o.shape.contours()[0], outer);
        assert_eq!(doc.hit_test([40.; 2], 0), None);
    }
}
