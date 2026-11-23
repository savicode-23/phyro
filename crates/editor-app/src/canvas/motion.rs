use super::*;
use document_core::{path_edit::HandleSide, VectorObject};
use paths::Part;

fn shift(object: &VectorObject, frame: u32) -> Vec2 {
    let center = Vec2::from(object.unrig_point_at(object.local_to_world([0.5; 2], frame), frame));
    let first = Vec2::from(object.motion.as_ref().unwrap().nodes()[0].point);
    center - Vec2::from(object.motion_offset_at(frame)) - first
}

pub(super) fn paint(
    painter: &egui::Painter,
    view: View,
    object: &VectorObject,
    frame: u32,
    handles: bool,
) {
    let Some(path) = &object.motion else {
        return;
    };
    let shift = shift(object, frame);
    let screen = |p: [f32; 2]| {
        view.screen(
            object
                .rig_point_at((Pos2::from(p) + shift).into(), frame)
                .into(),
        )
    };
    let nodes = path.nodes();
    let color = Color32::from_rgb(236, 170, 70);
    for (a, b) in nodes
        .iter()
        .zip(nodes.iter().cycle().skip(1))
        .take(nodes.len() - usize::from(!path.closed()))
    {
        let points = [a.point, a.outgoing, b.incoming, b.point].map(screen);
        painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
            points,
            false,
            Color32::TRANSPARENT,
            (1.5, color),
        ));
    }
    if handles {
        for n in nodes {
            let point = screen(n.point);
            for c in [n.incoming, n.outgoing] {
                let handle = screen(c);
                if point.distance(handle) > 2. {
                    painter.line_segment([point, handle], (1., color));
                    painter.circle(handle, 3., Color32::WHITE, (1., color));
                }
            }
            painter.rect_filled(Rect::from_center_size(point, Vec2::splat(7.)), 0., color);
        }
    }
}

pub(super) fn hit(object: &VectorObject, frame: u32, world: Pos2, radius: f32) -> Option<Gesture> {
    let path = object.motion.as_ref()?;
    let local = Pos2::from(object.unrig_point_at(world.into(), frame)) - shift(object, frame);
    let mut best = None;
    let mut min = radius;
    for (index, node) in path.nodes().iter().enumerate() {
        for (part, p) in [
            (Part::Incoming, node.incoming),
            (Part::Outgoing, node.outgoing),
            (Part::Anchor, node.point),
        ] {
            let distance = local.distance(p.into());
            if distance <= min {
                min = distance;
                best = Some(Gesture::Guide {
                    start: world,
                    id: object.id,
                    index,
                    part,
                });
            }
        }
    }
    best
}

pub(super) fn move_handle(
    o: &mut VectorObject,
    index: usize,
    part: Part,
    point: Pos2,
    frame: u32,
) -> bool {
    let Some(path) = o.motion.as_ref() else {
        return false;
    };
    let Some(mut node) = path.nodes().get(index).copied() else {
        return false;
    };
    let local = Pos2::from(o.unrig_point_at(point.into(), frame)) - shift(o, frame);
    let delta = local - Pos2::from(node.point);
    match part {
        Part::Anchor => {
            node.incoming = (Pos2::from(node.incoming) + delta).into();
            node.outgoing = (Pos2::from(node.outgoing) + delta).into();
            node.point = local.into();
        }
        Part::Incoming | Part::Outgoing => {
            let (side, opposite) = if matches!(part, Part::Incoming) {
                (
                    HandleSide::Incoming,
                    path.closed() || index + 1 < path.nodes().len(),
                )
            } else {
                (HandleSide::Outgoing, path.closed() || index > 0)
            };
            if !node.move_handle(side, local.into(), opposite, [1.; 2], 0.) {
                return false;
            }
        }
        Part::SymmetricHandles => {
            let before = node;
            if path.closed() || index + 1 < path.nodes().len() {
                node.outgoing = local.into();
            }
            if path.closed() || index > 0 {
                node.incoming = (Pos2::from(node.point) - delta).into();
            }
            if node == before {
                return false;
            }
        }
    }
    if o.motion.as_mut().unwrap().set_node(index, node).is_err() {
        return false;
    }
    if index == 0 && part == Part::Anchor {
        for p in std::iter::once(&mut o.position).chain(o.keys.values_mut()) {
            p[0] += delta.x;
            p[1] += delta.y;
        }
    }
    true
}
