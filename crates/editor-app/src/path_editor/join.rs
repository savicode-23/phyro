use super::NodeSelection;
use document_core::arrange::is_object_editable;
use document_core::{path_edit, Document, PathNode, Shape, VectorObject, MAX_PATH_NODES};
use eframe::egui::{self, Ui};
use std::sync::Arc;

fn open_nodes(o: &VectorObject) -> Option<&[PathNode]> {
    match &o.shape {
        Shape::Path {
            nodes,
            closed: false,
        } if !o.is_instance()
            && o.text.is_none()
            && o.skin.is_none()
            && o.animated.geometry_keys.is_empty() =>
        {
            Some(nodes)
        }
        _ => None,
    }
}

pub(crate) fn show(ui: &mut Ui, doc: &mut Arc<Document>, ids: &mut Vec<u64>, frame: u32) {
    let index = |id: u64| {
        doc.objects
            .iter()
            .position(|o| o.id == id && is_object_editable(doc, o) && open_nodes(o).is_some())
    };
    if let [id] = ids.as_slice() {
        let Some(i) = index(*id) else {
            return;
        };
        let selected = NodeSelection::load(ui.ctx(), *id, frame);
        let node = selected.end;
        let n = open_nodes(&doc.objects[i]).unwrap().len();
        if ui
            .add_enabled(
                selected.nodes.len() == 1 && node > 0 && node < n - 1,
                egui::Button::new("Split at node"),
            )
            .on_hover_text("Split an open path at the selected interior anchor")
            .clicked()
        {
            let new = split(Arc::make_mut(doc), i, node, frame);
            ids.push(new);
        }
    } else if let [a, b] = ids.as_slice() {
        let (Some(first), Some(second)) = (index(*a), index(*b)) else {
            return;
        };
        if first == second {
            return;
        }
        let error_id = ui.id().with(("join-error", *a, *b));
        if ui
            .button("Join open paths")
            .on_hover_text("Connect nearest endpoints using the first path's paint and motion")
            .clicked()
        {
            match join(Arc::make_mut(doc), first, second, frame) {
                Ok(()) => {
                    ids.truncate(1);
                    ui.data_mut(|d| d.remove::<String>(error_id));
                }
                Err(message) => ui.data_mut(|d| d.insert_temp(error_id, message)),
            }
        }
        if let Some(error) = ui.data(|d| d.get_temp::<String>(error_id)) {
            ui.colored_label(ui.visuals().warn_fg_color, error);
        }
    }
}

fn split(doc: &mut Document, index: usize, node: usize, frame: u32) -> u64 {
    let original = &mut doc.objects[index];
    let mut copy = original.clone();
    let Shape::Path { nodes, .. } = &mut original.shape else {
        unreachable!()
    };
    let mut tail = nodes.split_off(node);
    let mut end = tail[0];
    end.outgoing = end.point;
    nodes.push(end);
    tail[0].incoming = tail[0].point;
    copy.shape = Shape::Path {
        nodes: tail,
        closed: false,
    };
    original.split_pressure_profile(&mut copy, frame);
    let id = doc.add_shape(
        copy.position,
        copy.size,
        copy.fill,
        Shape::Rectangle,
        copy.layer,
    );
    copy.id = id;
    doc.objects.pop();
    doc.objects.insert(index + 1, copy);
    id
}

fn join(doc: &mut Document, first: usize, second: usize, frame: u32) -> Result<(), String> {
    let (a, b) = (&doc.objects[first], &doc.objects[second]);
    if !a.effects.is_empty() || !b.effects.is_empty() {
        return Err("Remove the path effects before joining these curves.".into());
    }
    if a.rigid_bone != b.rigid_bone {
        return Err("Bind both paths to the same bone or detach them before joining.".into());
    }
    if a.layer != b.layer {
        return Err("Move both paths to the same layer before joining.".into());
    }
    let (mut left, right) = (
        open_nodes(a).ok_or("Select two open paths.")?.to_vec(),
        open_nodes(b).ok_or("Select two open paths.")?,
    );
    if left.len() + right.len() > MAX_PATH_NODES {
        return Err("The joined path would exceed the node limit.".into());
    }
    let to_local = |p, t| {
        if a.scaled_box_at(t) == b.scaled_box_at(t)
            && a.scale_at(t).map(f32::signum) == b.scale_at(t).map(f32::signum)
            && a.rotation_at(t) == b.rotation_at(t)
            && a.skew_at(t) == b.skew_at(t)
        {
            p
        } else {
            a.world_to_local(b.local_to_world(p, t), t)
        }
    };
    if a.last_keyframe().is_some() || b.last_keyframe().is_some() || a.has_animated_rig() {
        // A single object cannot preserve two independent transform animations.
        // Three noncollinear points completely determine the relative affine map.
        for tick in 0..doc.frames {
            for p in [[0., 0.], [1., 0.], [0., 1.]] {
                let projected = a.local_to_world(to_local(p, frame), tick);
                let target = b.local_to_world(p, tick);
                if (projected[0] - target[0]).hypot(projected[1] - target[1]) > 0.001 {
                    return Err("These paths have different animated transforms. Join paths with matching motion.".into());
                }
            }
        }
    }
    let mut right: Vec<_> = right
        .iter()
        .map(|n| PathNode {
            handle_style: n.handle_style,
            point: to_local(n.point, frame),
            incoming: to_local(n.incoming, frame),
            outgoing: to_local(n.outgoing, frame),
        })
        .collect();
    if right
        .iter()
        .flat_map(|n| [n.point, n.incoming, n.outgoing])
        .any(|p| p.iter().zip(a.size).any(|(v, s)| !(v * s).is_finite()))
    {
        return Err("These transforms exceed the drawing's coordinate range.".into());
    }
    let endpoints = |n: &[PathNode]| {
        [
            a.local_to_world(n[0].point, frame),
            a.local_to_world(n[n.len() - 1].point, frame),
        ]
    };
    let [start_a, end_a] = endpoints(&left);
    let [start_b, end_b] = endpoints(&right);
    let dist = |p: [f32; 2], q: [f32; 2]| (p[0] - q[0]).hypot(p[1] - q[1]);
    let options = [
        (dist(end_a, start_b), false, false),
        (dist(end_a, end_b), false, true),
        (dist(start_a, start_b), true, false),
        (dist(start_a, end_b), true, true),
    ];
    let (gap, reverse_a, reverse_b) = options
        .into_iter()
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .unwrap();
    if reverse_a {
        path_edit::reverse(&mut left);
    }
    if reverse_b {
        path_edit::reverse(&mut right);
    }
    let end = left.last_mut().unwrap();
    if gap < 0.0001 {
        end.outgoing = right[0].outgoing;
        left.extend_from_slice(&right[1..]);
    } else {
        end.outgoing = end.point;
        right[0].incoming = right[0].point;
        left.extend(right);
    }
    let profile = a.joined_pressure_profile(b, frame, reverse_a, reverse_b, gap);
    doc.objects[first].stroke_profile = profile;
    doc.objects[first].shape = Shape::Path {
        nodes: left,
        closed: false,
    };
    doc.objects.remove(second);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn splitting_and_joining_keeps_the_pressure_peak_at_its_original_location() {
        let mut doc = Document::default();
        let nodes = [[0., 0.], [0.3, 0.], [1., 0.]].map(PathNode::corner).into();
        let id = doc.add_shape(
            [8., 30.],
            [100., 20.],
            [0; 4],
            Shape::Path {
                nodes,
                closed: false,
            },
            1,
        );
        let o = &mut doc.objects[0];
        o.stroke_width = 20.;
        o.stroke_profile = vec![[0., 0.05], [0.2, 0.2], [0.6, 1.], [1., 0.1]];
        let sample = |d: &Document| {
            [16., 34., 55., 68., 80., 104.]
                .into_iter()
                .flat_map(|x| (31..44).map(move |y| [x, y as f32]))
                .map(|p| d.hit_test(p, 0).is_some())
                .collect::<Vec<_>>()
        };
        let original = sample(&doc);
        split(&mut doc, 0, 1, 0);
        assert_eq!(sample(&doc), original);
        join(&mut doc, 0, 1, 0).unwrap();
        assert_eq!(doc.objects[0].id, id);
        assert_eq!(sample(&doc), original);
        assert!(doc.validate().is_ok());
    }

    #[test]
    fn splitting_and_joining_a_cubic_restores_both_curves_and_object_motion() {
        let nodes = vec![
            PathNode::corner([0., 0.]),
            PathNode {
                handle_style: Default::default(),
                point: [0.5, 0.5],
                incoming: [0.1, 0.3],
                outgoing: [0.8, 0.7],
            },
            PathNode::corner([1., 1.]),
        ];
        let mut doc = Document::default();
        doc.add_shape(
            [0., 0.],
            [100., 20.],
            [0; 4],
            Shape::Path {
                nodes,
                closed: false,
            },
            1,
        );
        doc.objects[0].keys.insert(0, [8., 12.]);
        doc.objects[0].keys.insert(4, [4., 28.]);
        let original = doc.objects[0].clone();
        split(&mut doc, 0, 1, 0);
        assert_eq!(doc.objects[1].keys, original.keys);
        join(&mut doc, 0, 1, 3).unwrap();
        assert_eq!(doc.objects, [original]);
    }
    #[test]
    fn joining_independently_moving_paths_is_atomic() {
        let mut doc = Document::default();
        let line = Shape::Path {
            nodes: vec![PathNode::corner([0.; 2]), PathNode::corner([1.; 2])],
            closed: false,
        };
        for x in [0., 10.] {
            doc.add_shape([x, 0.], [10.; 2], [255; 4], line.clone(), 1);
        }
        doc.objects[1].keys.extend([(0, [10., 0.]), (4, [20., 0.])]);
        let before = doc.clone();
        assert!(join(&mut doc, 0, 1, 0).is_err());
        assert_eq!(doc, before);
    }
}
