pub(crate) mod styles;
use crate::timeline::KeyMode;
use document_core::{
    path_edit, path_effects::MAX_EFFECTS, Document, PathEffect, Property, Shape, VectorObject,
    MAX_PATH_NODES,
};
use eframe::egui::{self, Button, DragValue, Ui};
use std::borrow::Cow;
use std::sync::Arc;
mod selection;
pub(crate) mod simplify;
use selection::NodeSelection;
pub(crate) use selection::{selected_nodes, set_nodes, NodeSet};

pub(crate) fn use_canvas(ctx: &egui::Context, session: u64) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("node-canvas"), session));
}
pub fn select(ctx: &egui::Context, object: u64, frame: u32, contour: usize, node: usize) {
    let mut s = NodeSelection::load(ctx, object, frame);
    s.set_range(contour, node, node);
    s.save(ctx);
}
#[cfg(test)]
pub(crate) fn selected_range(
    ctx: &egui::Context,
    object: u64,
    frame: u32,
) -> (usize, usize, usize) {
    let nodes = selected_nodes(ctx, object, frame);
    let [single] = nodes.runs() else {
        panic!("expected a single range, got {nodes:?}")
    };
    *single
}

pub(crate) fn selected_endpoint(ctx: &egui::Context, object: u64, frame: u32) -> (usize, usize) {
    let s = NodeSelection::load(ctx, object, frame);
    (s.contour, s.end)
}

fn can_edit_topology(obj: &VectorObject, mode: KeyMode) -> bool {
    mode != KeyMode::Auto
        && obj.animated.geometry_keys.is_empty()
        && obj.skin.is_none()
        && !obj.locked
}

fn deletion_target(
    ctx: &egui::Context,
    doc: &Document,
    selection: &[u64],
    frame: u32,
    mode: KeyMode,
) -> Option<(usize, NodeSelection)> {
    let [id] = selection else { return None };
    let index = doc.objects.iter().position(|o| o.id == *id)?;
    let object = &doc.objects[index];
    if !document_core::arrange::is_object_editable(doc, object) {
        return None;
    }
    let nodes = NodeSelection::load(ctx, *id, frame);
    nodes.can_delete(object, mode).then_some((index, nodes))
}

pub(crate) fn can_delete_selected_nodes(
    ctx: &egui::Context,
    doc: &Document,
    selection: &[u64],
    frame: u32,
    mode: KeyMode,
) -> bool {
    deletion_target(ctx, doc, selection, frame, mode).is_some()
}

pub(crate) fn delete_selected_nodes(
    ctx: &egui::Context,
    doc: &mut Document,
    selection: &[u64],
    frame: u32,
    mode: KeyMode,
) -> bool {
    let Some((i, mut nodes)) = deletion_target(ctx, doc, selection, frame, mode) else {
        return false;
    };
    nodes.delete_from(&mut doc.objects[i].shape);
    nodes.save(ctx);
    true
}

fn combine_target(
    ctx: &egui::Context,
    doc: &Document,
    selection: &[u64],
    frame: u32,
    mode: KeyMode,
) -> Option<(usize, usize, NodeSet)> {
    let [id] = selection else { return None };
    let index = doc.objects.iter().position(|o| o.id == *id)?;
    let object = &doc.objects[index];
    if !document_core::arrange::is_object_editable(doc, object)
        || !object.visible_at(frame)
        || object.text.is_some()
        || object.is_instance()
        || !can_edit_topology(object, mode)
        || Property::Geometry.is_muted(object)
    {
        return None;
    }
    let nodes = selected_nodes(ctx, *id, frame);
    let &(contour, first, _) = nodes.runs().first()?;
    let &(last_contour, _, last) = nodes.runs().last()?;
    (contour == last_contour
        && nodes.valid(&object.shape)
        && path_edit::can_combine_nodes(&object.shape, contour, first, last, nodes.len()))
    .then_some((index, contour, nodes))
}

pub(crate) fn combine_button(
    ui: &mut Ui,
    doc: &Document,
    selection: &[u64],
    frame: u32,
    mode: KeyMode,
    enabled: bool,
) -> bool {
    let enabled = enabled && combine_target(ui.ctx(), doc, selection, frame, mode).is_some();
    let shortcut = crate::keymap::label(ui.ctx(), crate::keymap::Action::CombineNodes);
    ui.add_enabled(enabled, Button::new("Combine nodes"))
        .on_hover_text(format!(
            "Merge selected anchors on one contour ({shortcut})"
        ))
        .clicked()
}

pub(crate) fn combine_nodes(
    ctx: &egui::Context,
    doc: &mut Document,
    selection: &[u64],
    frame: u32,
    mode: KeyMode,
) -> bool {
    let Some((index, contour, nodes)) = combine_target(ctx, doc, selection, frame, mode) else {
        return false;
    };
    let selected: Vec<_> = nodes
        .runs()
        .iter()
        .flat_map(|&(_, start, end)| start..=end)
        .collect();
    let object = &mut doc.objects[index];
    let mut shape = object.shape.clone();
    let Some(anchor) = path_edit::combine_nodes(&mut shape, contour, &selected) else {
        return false;
    };
    if object.set_shape_at(frame, shape).is_err() {
        return false;
    }
    select(ctx, object.id, frame, contour, anchor);
    true
}

pub(crate) fn delete_node_at(
    ctx: &egui::Context,
    object: &mut VectorObject,
    frame: u32,
    mode: KeyMode,
    contour: usize,
    node: usize,
) -> bool {
    let mut selected = NodeSelection::load(ctx, object.id, frame);
    selected.set_range(contour, node, node);
    if !selected.can_delete(object, mode) {
        return false;
    }
    selected.delete_from(&mut object.shape);
    selected.save(ctx);
    true
}

pub(crate) fn can_convert_to_path(object: &VectorObject) -> bool {
    crate::commands::has_vector_paint(object)
        && (object.text.is_some() || matches!(object.shape, Shape::Rectangle | Shape::Ellipse))
}

pub(crate) fn convert_to_path(object: &mut VectorObject) -> bool {
    if !crate::commands::has_vector_paint(object) {
        return false;
    }
    // Live text already stores the shaped glyph outlines in object.shape.
    if object.text.take().is_some() {
        return true;
    }
    let Some(path) = path_edit::primitive_path(&object.shape) else {
        return false;
    };
    object.shape = path;
    true
}

pub fn show(ui: &mut Ui, doc: &mut Arc<Document>, index: usize, frame: u32, mode: KeyMode) {
    let obj = &doc.objects[index];
    if obj.text.is_some() || !crate::commands::has_vector_paint(obj) {
        return;
    }
    if !obj.shape.contours().is_empty() {
        ui.collapsing("Path effects", |ui| {
            let mut edited = doc.objects[index].effects.clone();
            effects(ui, &mut edited);
            if edited != doc.objects[index].effects {
                Arc::make_mut(doc).objects[index].effects = edited;
            }
        });
    }
    ui.collapsing("Path nodes", |ui| {
        if doc.objects[index].shape.contours().is_empty() {
            if ui
                .button("Convert to path")
                .on_hover_text("Replace the primitive with editable cubic nodes")
                .clicked()
            {
                convert_to_path(&mut Arc::make_mut(doc).objects[index]);
            }
        } else {
            let mut edited_object = doc.objects[index].clone();
            let obj = &mut edited_object;
            let keyed = Property::Geometry.last(obj).is_some() || mode == KeyMode::Auto;
            if keyed {
                let mut pose = obj.shape_at(frame).into_owned();
                std::mem::swap(&mut pose, &mut obj.shape);
                let edited = ui
                    .add_enabled_ui(mode.allows(obj, frame, Property::Geometry), |ui| {
                        edit_nodes(ui, obj, frame, mode)
                    })
                    .inner;
                std::mem::swap(&mut pose, &mut obj.shape);
                if edited && mode.prepare(obj, frame, Property::Geometry) {
                    let _ = obj.set_shape_at(frame, pose);
                }
                if obj.animated.guided_geometry.is_some() {
                    ui.weak("Use Guided morph to change how outline points match between poses.");
                } else {
                    ui.weak("Geometry keys share the same contour and node order.");
                }
            } else {
                edit_nodes(ui, obj, frame, mode);
            }
            if obj.skin.is_some() {
                ui.weak("Detach the path in Skeleton to change its node count or order.");
            }
            if edited_object != doc.objects[index] {
                Arc::make_mut(doc).objects[index] = edited_object;
            }
        }
    });
}

fn edit_nodes(ui: &mut Ui, obj: &mut VectorObject, frame: u32, mode: KeyMode) -> bool {
    let topology = can_edit_topology(obj, mode);
    let mut s = NodeSelection::load(ui.ctx(), obj.id, frame);
    let contours = obj.shape.contours();
    if s.nodes.runs().is_empty() {
        ui.weak("Drag across anchors to select nodes.");
        return false;
    }
    if !s.nodes.valid(&obj.shape) {
        let c = s.contour.min(contours.len() - 1);
        let n = s.end.min(contours[c].len() - 1);
        s.set_range(c, n, n);
    }
    if contours.len() > 1 {
        let mut contour = s.contour + 1;
        if ui
            .add(
                DragValue::new(&mut contour)
                    .range(1..=contours.len())
                    .prefix("Contour "),
            )
            .changed()
        {
            s.set_range(contour - 1, 0, 0);
        }
    }
    let nodes = &contours[s.contour];
    if s.nodes.runs().len() == 1 {
        let mut a = (s.anchor + 1).min(nodes.len());
        let mut b = (s.end + 1).min(nodes.len());
        ui.horizontal_wrapped(|ui| {
            ui.label("Nodes");
            ui.add(DragValue::new(&mut a).range(1..=nodes.len()));
            ui.label("to");
            ui.add(DragValue::new(&mut b).range(1..=nodes.len()))
                .on_hover_text("Shift-click anchors on the canvas to add them to the selection");
        });
        if (a - 1, b - 1) != (s.anchor, s.end) {
            s.set_range(s.contour, a - 1, b - 1);
        }
    } else {
        ui.label(format!("{} nodes selected", s.nodes.len()));
    }
    let posed = obj.skin_shape_at(Cow::Borrowed(&obj.shape), frame);
    let mut world = obj.local_to_world(posed.contours()[s.contour][s.anchor].point, frame);
    let original = world;
    ui.horizontal_wrapped(|ui| {
        ui.add(DragValue::new(&mut world[0]).prefix("X ").speed(0.25));
        ui.add(DragValue::new(&mut world[1]).prefix("Y ").speed(0.25));
    });
    let mut changed = false;
    if original != world && world.into_iter().all(f32::is_finite) {
        changed = transform_posed_nodes(obj, &s.nodes, frame, |p| {
            std::array::from_fn(|i| p[i] + (world[i] - original[i]))
        });
    }
    if s.nodes.len() > 1 {
        changed |= transform_range(ui, obj, frame, &s);
    }
    let single = matches!(obj.shape, Shape::Path { .. });
    let closed = !matches!(obj.shape, Shape::Path { closed: false, .. });
    let capacity = obj.shape.contours().iter().map(Vec::len).sum::<usize>() < MAX_PATH_NODES;
    ui.horizontal_wrapped(|ui| {
        if let Some(mode) = styles::buttons(ui, true, styles::common_style(&obj.shape, &s.nodes)) {
            changed |= styles::apply_to_shape(&mut obj.shape, obj.size, &s.nodes, mode);
        }
    });
    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(
                topology
                    && capacity
                    && (closed || s.end + 1 < obj.shape.contours()[s.contour].len()),
                Button::new("Subdivide"),
            )
            .on_hover_text("Insert an anchor halfway along the following segment")
            .clicked()
        {
            changed |=
                path_edit::subdivide(&mut obj.shape.contours_mut()[s.contour], closed, s.end);
            s.set_range(s.contour, s.end + 1, s.end + 1);
        }
        if ui
            .add_enabled(s.can_delete(obj, mode), Button::new("Delete nodes"))
            .clicked()
        {
            s.delete_from(&mut obj.shape);
            changed = true;
        }
        if ui.add_enabled(topology, Button::new("Reverse")).clicked() {
            let last = obj.shape.contours()[s.contour].len() - 1;
            obj.reorder_pressure_profile(frame, s.contour, last, true);
            let nodes = &mut obj.shape.contours_mut()[s.contour];
            path_edit::reverse(nodes);
            s.nodes.reverse_contour(s.contour, nodes.len());
            s.anchor = nodes.len() - 1 - s.anchor;
            s.end = nodes.len() - 1 - s.end;
            changed = true;
        }
    });
    if single {
        let can_toggle =
            closed && capacity || !closed && obj.shape.contours()[s.contour].len() >= 3;
        if ui
            .add_enabled(
                topology && can_toggle,
                Button::new(if closed { "Open at node" } else { "Close path" }),
            )
            .clicked()
        {
            if closed {
                obj.reorder_pressure_profile(frame, s.contour, s.end, false);
                path_edit::open_at(&mut obj.shape.contours_mut()[s.contour], s.end);
                s.set_range(s.contour, 0, 0);
            } else {
                let nodes = &mut obj.shape.contours_mut()[s.contour];
                path_edit::close(nodes);
                let end = s.end.min(nodes.len() - 1);
                s.set_range(s.contour, end, end);
            }
            if let Shape::Path { closed, .. } = &mut obj.shape {
                *closed = !*closed;
            }
            changed = true;
        }
    }
    s.save(ui.ctx());
    changed
}

fn transform_range(ui: &mut Ui, obj: &mut VectorObject, frame: u32, s: &NodeSelection) -> bool {
    let id = egui::Id::new((
        "path-node-transform",
        s.object,
        s.frame,
        s.contour,
        s.anchor,
        s.end,
    ));
    let mut params = ui
        .data(|d| d.get_temp::<[f32; 2]>(id))
        .unwrap_or([0., 100.]);
    ui.horizontal_wrapped(|ui| {
        ui.add(DragValue::new(&mut params[0]).suffix(" deg"));
        ui.add(
            DragValue::new(&mut params[1])
                .suffix(" %")
                .range(0.01..=10000.),
        );
    });
    ui.data_mut(|d| d.insert_temp(id, params));
    if !ui.small_button("Transform selected nodes").clicked() || params == [0., 100.] {
        return false;
    }
    let posed = obj.skin_shape_at(Cow::Borrowed(&obj.shape), frame);
    let mut sum = [0.; 2];
    let world = obj.local_to_world_at(frame);
    for &(c, a, b) in s.nodes.runs() {
        for node in &posed.contours()[c][a..=b] {
            let point = world(node.point);
            for i in 0..2 {
                sum[i] += f64::from(point[i]);
            }
        }
    }
    let center = sum.map(|total| (total / s.nodes.len() as f64) as f32);
    let (sin, cos) = params[0].to_radians().sin_cos();
    let changed = transform_posed_nodes(obj, &s.nodes, frame, |p| {
        let (x, y) = (p[0] - center[0], p[1] - center[1]);
        let k = params[1] / 100.;
        [
            center[0] + k * (x * cos - y * sin),
            center[1] + k * (x * sin + y * cos),
        ]
    });
    if changed {
        ui.data_mut(|d| d.remove::<[f32; 2]>(id));
    }
    changed
}

/// Apply one artboard-space edit to the selected anchors and their absolute handles.
/// Stage the whole edit in case an individual node's inverse skin map collapses.
pub(crate) fn transform_posed_nodes(
    obj: &mut VectorObject,
    selection: &NodeSet,
    frame: u32,
    transform: impl Fn([f32; 2]) -> [f32; 2],
) -> bool {
    let posed = obj.skin_shape_at(Cow::Borrowed(&obj.shape), frame);
    if !selection.valid(&posed) {
        return false;
    }
    let mut next = Vec::with_capacity(selection.len());
    for &(contour, a, b) in selection.runs() {
        for (index, node) in (a..=b).zip(&posed.contours()[contour][a..=b]) {
            let mut next_node = *node;
            for point in [
                &mut next_node.point,
                &mut next_node.incoming,
                &mut next_node.outgoing,
            ] {
                let world = transform(obj.local_to_world(*point, frame));
                let local = obj.world_to_local(world, frame);
                let Some(source) = obj.unskin_local_point_at(contour, index, local, frame) else {
                    return false;
                };
                if source
                    .into_iter()
                    .zip(obj.size)
                    .any(|(v, size)| !(v * size).is_finite())
                {
                    return false;
                }
                *point = source;
            }
            next.push(next_node);
        }
    }
    let mut next = next.into_iter();
    let mut changed = false;
    for &(c, a, b) in selection.runs() {
        for dest in &mut obj.shape.contours_mut()[c][a..=b] {
            let node = next.next().unwrap();
            changed |= *dest != node;
            *dest = node;
        }
    }
    changed
}

fn effects(ui: &mut Ui, effects: &mut Vec<PathEffect>) {
    let count = effects.len();
    let mut remove = None;
    let mut reorder = None;
    for (i, effect) in effects.iter_mut().enumerate() {
        ui.push_id(i, |ui| {
            ui.horizontal_wrapped(|ui| match effect {
                PathEffect::Mirror { horizontal } => {
                    ui.label("Mirror");
                    ui.selectable_value(horizontal, true, "X");
                    ui.selectable_value(horizontal, false, "Y");
                }
                PathEffect::RoundCorners { distance } => {
                    ui.label("Round corners");
                    ui.add(
                        DragValue::new(distance)
                            .range(0.0..=4096.)
                            .speed(0.1)
                            .suffix(" px"),
                    );
                }
                PathEffect::Simplify { tolerance } => {
                    ui.label("Simplify");
                    ui.add(
                        DragValue::new(tolerance)
                            .range(0.0..=4096.)
                            .speed(0.05)
                            .suffix(" px"),
                    );
                }
            });
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(i > 0, Button::new("Up").small()).clicked() {
                    reorder = Some((i, i - 1));
                }
                if ui
                    .add_enabled(i + 1 < count, Button::new("Down").small())
                    .clicked()
                {
                    reorder = Some((i, i + 1));
                }
                if ui.small_button("Remove").clicked() {
                    remove = Some(i);
                }
            });
        });
    }
    if let Some(i) = remove {
        effects.remove(i);
    } else if let Some((i, j)) = reorder {
        effects.swap(i, j);
    }
    ui.add_enabled_ui(effects.len() < MAX_EFFECTS, |ui| {
        ui.menu_button("Add effect", |ui| {
            for (label, effect) in [
                ("Round corners", PathEffect::RoundCorners { distance: 4. }),
                ("Mirror", PathEffect::Mirror { horizontal: true }),
                ("Simplify", PathEffect::Simplify { tolerance: 0.5 }),
            ] {
                if ui.button(label).clicked() {
                    effects.push(effect);
                    ui.close_menu();
                }
            }
        });
    });
}
mod join;
pub(crate) use join::show as split_or_join;

#[cfg(test)]
mod tests {
    use super::*;
    use document_core::{Document, PathNode};

    #[test]
    fn delete_nodes_checks_every_contour_before_mutating_and_selects_a_survivor() {
        let ctx = egui::Context::default();
        let mut doc = Document::default();
        let nodes = [0., 0.25, 0.5, 0.75, 1.].map(|x| PathNode::corner([x, x]));
        let id = doc.add_shape(
            [0.; 2],
            [10.; 2],
            [255; 4],
            Shape::Compound {
                contours: vec![nodes.to_vec(), nodes[..4].to_vec()],
            },
            1,
        );
        let original = doc.clone();
        let mode = KeyMode::Tracked;
        let rejected = NodeSet::Runs([(0, 3, 4), (1, 0, 1)].into());
        set_nodes(&ctx, id, 0, rejected.clone());
        assert!(!delete_selected_nodes(&ctx, &mut doc, &[id], 0, mode));
        assert_eq!(doc, original);
        assert_eq!(selected_nodes(&ctx, id, 0), rejected);

        let selected = NodeSet::Runs([(0, 1, 1), (0, 4, 4), (1, 3, 3)].into());
        set_nodes(&ctx, id, 0, selected);
        assert!(can_delete_selected_nodes(&ctx, &doc, &[id], 0, mode));
        assert!(delete_selected_nodes(&ctx, &mut doc, &[id], 0, mode));
        let contours = doc.objects[0].shape.contours();
        assert_eq!(contours[0], [nodes[0], nodes[2], nodes[3]]);
        assert_eq!(contours[1], nodes[..3]);
        assert_eq!(selected_range(&ctx, id, 0), (0, 1, 1));
    }

    #[test]
    fn range_edits_move_each_weighted_anchor_and_handle_in_posed_space() {
        let mut doc = Document::default();
        let upper = doc.add_bone("Upper", None, [10., 16.], [20., 16.]).unwrap();
        let lower = doc
            .add_bone("Lower", Some(upper), [20., 16.], [30., 16.])
            .unwrap();
        let art = doc.add_shape(
            [10., 12.],
            [20., 8.],
            [255; 4],
            Shape::Path {
                nodes: [0., 0.5, 1.]
                    .map(|x| PathNode {
                        handle_style: Default::default(),
                        point: [x, 0.5],
                        incoming: [x - 0.05, 0.25],
                        outgoing: [x + 0.05, 0.75],
                    })
                    .to_vec(),
                closed: false,
            },
            1,
        );
        doc.bind_skin_at(art, lower, 0).unwrap();
        doc.set_bone_rotation(lower, 65.).unwrap();
        let scene_points = |obj: &VectorObject| {
            obj.skinned_shape_at(0).contours()[0]
                .iter()
                .flat_map(|n| [n.point, n.incoming, n.outgoing])
                .map(|p| obj.local_to_world(p, 0))
                .collect::<Vec<_>>()
        };
        let obj = &mut doc.objects[0];
        let before = scene_points(obj);
        let edit = |[x, y]: [f32; 2]| [x + 3., y - 2.];
        assert!(transform_posed_nodes(
            obj,
            &NodeSet::range(0, 0, 2),
            0,
            edit
        ));
        for (actual, expected) in scene_points(obj)
            .into_iter()
            .zip(before.into_iter().map(edit))
        {
            assert!((actual[0] - expected[0]).hypot(actual[1] - expected[1]) < 0.001);
        }
        // The middle node collapses under equal influence at a half-turn.
        doc.set_bone_rotation(lower, 180.).unwrap();
        let obj = &mut doc.objects[0];
        let before = obj.shape.clone();
        assert!(!transform_posed_nodes(
            obj,
            &NodeSet::range(0, 0, 2),
            0,
            edit
        ));
        assert_eq!(obj.shape, before);
    }
}
