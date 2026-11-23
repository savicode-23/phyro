use super::*;
use crate::canvas::snapping::Subject;

pub(in crate::canvas) enum NodeDrag {
    Control {
        index: NodeIndex,
        part: Part,
        nodes: NodeSet,
        offset: Vec2,
        dragged: bool,
    },
    Pull {
        index: NodeIndex,
        anchor: Pos2,
        directions: [Option<Vec2>; 2],
        side: Option<HandleSide>,
    },
    Segment {
        hit: SegmentHit,
        dragged: bool,
    },
}

impl NodeDrag {
    pub(in crate::canvas) fn pull(
        object: &VectorObject,
        frame: u32,
        index: NodeIndex,
    ) -> Option<Self> {
        let shape = object.skinned_shape_at(frame);
        let nodes = shape.contours().get(index.contour)?;
        let node = nodes.get(index.node)?;
        let closed = !matches!(shape.as_ref(), Shape::Path { closed: false, .. });
        let world = object.local_to_world_at(frame);
        let anchor = Pos2::from(world(node.point));
        let incoming = (node.incoming == node.point && (closed || index.node > 0)).then(|| {
            let previous = (index.node + nodes.len() - 1) % nodes.len();
            (Pos2::from(world(nodes[previous].point)) - anchor).normalized()
        });
        let outgoing = (node.outgoing == node.point && (closed || index.node + 1 < nodes.len()))
            .then(|| {
                let next = (index.node + 1) % nodes.len();
                (Pos2::from(world(nodes[next].point)) - anchor).normalized()
            });
        (incoming.is_some() || outgoing.is_some()).then_some(Self::Pull {
            index,
            anchor,
            directions: [incoming, outgoing],
            side: None,
        })
    }

    pub(in crate::canvas) fn segment(
        object: &VectorObject,
        frame: u32,
        point: Pos2,
        radius: f32,
    ) -> Option<Self> {
        let shape = object.skinned_shape_at(frame);
        let hit = nearest_segment(&shape, object.local_to_world_at(frame), point, radius)?;
        (hit.t > 0. && hit.t < 1.).then_some(Self::Segment {
            hit,
            dragged: false,
        })
    }

    pub(in crate::canvas) fn track_start(
        &mut self,
        ui: &egui::Ui,
        view: View,
        start: Pos2,
        pressed_this_frame: bool,
    ) {
        if self.has_dragged() {
            return;
        }
        let threshold = if matches!(self, Self::Segment { .. }) {
            2.
        } else {
            HANDLE_DRAG_DEAD_ZONE
        };
        // Several movements can arrive in one frame. A pull uses the first drag
        // direction even if the pointer has already crossed back over the anchor.
        let Some(delta) = first_drag_delta(ui, view, start, pressed_this_frame, threshold) else {
            return;
        };
        match self {
            Self::Pull {
                directions: [incoming, outgoing],
                side,
                ..
            } => {
                *side = Some(match (*incoming, *outgoing) {
                    (Some(a), Some(b)) if b.dot(delta) > a.dot(delta) => HandleSide::Outgoing,
                    (Some(_), _) => HandleSide::Incoming,
                    _ => HandleSide::Outgoing,
                });
            }
            Self::Control { dragged, .. } | Self::Segment { dragged, .. } => *dragged = true,
        }
    }

    pub(in crate::canvas) fn has_dragged(&self) -> bool {
        match self {
            Self::Control { dragged, .. } | Self::Segment { dragged, .. } => *dragged,
            Self::Pull { side, .. } => side.is_some(),
        }
    }

    pub(in crate::canvas) fn snap_subject(
        &self,
        id: u64,
        start: Pos2,
        point: Pos2,
    ) -> (Pos2, Subject<'_>) {
        match self {
            Self::Control { nodes, offset, .. } => (point + *offset, Subject::Nodes { id, nodes }),
            Self::Pull { anchor, .. } => (*anchor + (point - start), Subject::Object(id)),
            Self::Segment { hit, .. } => (hit.point + (point - start), Subject::Object(id)),
        }
    }

    pub(in crate::canvas) fn shape_at(
        &self,
        object: &VectorObject,
        frame: u32,
        target: Pos2,
    ) -> Option<Shape> {
        if object.text.is_some() {
            return None;
        }
        match self {
            Self::Control {
                index, part, nodes, ..
            } => {
                let mut draft = object.clone();
                draft.shape = object.shape_at(frame).into_owned();
                move_node(&mut draft, *index, *part, nodes, frame, target).then_some(draft.shape)
            }
            Self::Pull { index, side, .. } => {
                let side = (*side)?;
                let local = object.unskin_local_point_at(
                    index.contour,
                    index.node,
                    object.world_to_local(target.into(), frame),
                    frame,
                )?;
                let mut shape = object.shape_at(frame).into_owned();
                let node = shape
                    .contours_mut()
                    .get_mut(index.contour)?
                    .get_mut(index.node)?;
                // A pull creates a corner, even if a collapsed node was marked symmetric.
                // The opposite control and adjacent segment must remain exactly where they were.
                if !node.move_handle(side, local, false, object.scaled_box_at(frame).1, 0.) {
                    return None;
                }
                node.handle_style = HandleStyle::Corner;
                Some(shape)
            }
            Self::Segment { hit, .. } => object.bent_segment(
                frame,
                hit.contour,
                hit.start,
                hit.t,
                (target - hit.point).into(),
            ),
        }
    }
}
