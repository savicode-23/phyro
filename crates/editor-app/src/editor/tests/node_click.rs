use super::*;
use document_core::{PathNode, Shape};
use egui::{vec2, Event, Pos2, Rect};

fn drawing(shape: Shape, stroked: bool) -> Document {
    let mut doc = Document::default();
    doc.add_shape(
        [32.; 2],
        [64.; 2],
        if stroked { [0; 4] } else { [79, 138, 85, 255] },
        shape,
        1,
    );
    if stroked {
        doc.objects[0].stroke_width = 3.;
        doc.objects[0].stroke = [33, 152, 196, 255];
    }
    doc
}

fn window(doc: &Document) -> EditorWindow {
    let mut w = EditorWindow::new(vec2(1100., 820.));
    w.ctx.style_mut(|s| s.animation_time = 0.);
    w.editor
        .replace_project(AnimationProject::from_document(doc.clone()), None);
    w.editor.set_tool(Tool::Node);
    for _ in 0..3 {
        w.frame(vec![]);
    }
    w
}

fn screen(board: Rect, world: [f32; 2]) -> Pos2 {
    board.min + board.size() * (egui::Vec2::from(world) / 128.)
}

fn open_curve() -> Shape {
    let mut a = PathNode::corner([0.05, 0.5]);
    a.outgoing = [0.28, 0.13];
    let mut b = PathNode::corner([0.95, 0.5]);
    b.incoming = [0.72, 0.87];
    Shape::Path {
        nodes: vec![a, b],
        closed: false,
    }
}

fn point_on_segment(a: &PathNode, b: &PathNode, t: f32) -> [f32; 2] {
    let u = 1. - t;
    std::array::from_fn(|i| {
        u.powi(3) * a.point[i]
            + 3. * u * u * t * a.outgoing[i]
            + 3. * u * t * t * b.incoming[i]
            + t.powi(3) * b.point[i]
    })
}

#[test]
fn segment_drag_bends_at_the_grabbed_point_on_a_transformed_path_in_one_undo() {
    let mut doc = drawing(open_curve(), true);
    let object = &mut doc.objects[0];
    object.rotation = 21.;
    object.scale = [1.2, 0.8];
    object.animated.skew = 9.;
    let object = &doc.objects[0];
    let original = &object.shape.contours()[0];
    let t = 0.36;
    let mut w = window(&doc);
    w.editor.selection = vec![object.id];
    let board = w.board();
    let screen_point = |local| screen(board, object.local_to_world(local, 0));
    let start = screen_point(point_on_segment(&original[0], &original[1], t));
    let delta = vec2(9., 22.);
    w.drag(start, delta);
    let edited = w.editor.scene().clone();
    let nodes = &edited.objects[0].shape.contours()[0];
    assert_eq!(nodes.len(), 2);
    assert_eq!(
        (nodes[0].point, nodes[1].point),
        (original[0].point, original[1].point)
    );
    assert_eq!(
        (nodes[0].incoming, nodes[1].outgoing),
        (original[0].incoming, original[1].outgoing)
    );
    assert_ne!(nodes[0].outgoing, original[0].outgoing);
    assert_ne!(nodes[1].incoming, original[1].incoming);
    let end = screen_point(point_on_segment(&nodes[0], &nodes[1], t));
    assert!(
        end.distance(start + delta) < 0.9,
        "grabbed point ended at {end:?}"
    );
    w.click_edit("Undo");
    assert_eq!(w.editor.scene(), &doc);
    assert!(!w.editor.can_undo());
    w.click_edit("Redo");
    assert_eq!(w.editor.scene(), &edited);
}

#[test]
fn ctrl_drag_from_a_corner_pulls_only_the_handle_on_the_chosen_side() {
    let original = [[0., 0.5], [0.5, 0.5], [1., 0.5]].map(PathNode::corner);
    let doc = drawing(
        Shape::Path {
            nodes: original.to_vec(),
            closed: false,
        },
        true,
    );
    let object = &doc.objects[0];
    for incoming in [true, false] {
        let mut w = window(&doc);
        w.editor.selection = vec![object.id];
        let board = w.board();
        let screen_point = |local| screen(board, object.local_to_world(local, 0));
        let anchor = screen_point(original[1].point);
        let delta = vec2(if incoming { -21. } else { 21. }, -13.);
        // The initial direction chooses the side even when the drag later crosses the node.
        let tip = if incoming {
            anchor + vec2(14., -23.)
        } else {
            anchor + delta
        };
        let modifiers = egui::Modifiers::CTRL | egui::Modifiers::COMMAND;
        let button = |pos, pressed| Event::PointerButton {
            pos,
            pressed,
            button: egui::PointerButton::Primary,
            modifiers,
        };
        for events in [
            vec![Event::PointerMoved(anchor)],
            vec![button(anchor, true)],
            vec![
                Event::PointerMoved(anchor + delta * 0.45),
                Event::PointerMoved(tip),
            ],
            vec![button(tip, false)],
        ] {
            w.frame_with_input(egui::RawInput {
                modifiers,
                events,
                ..Default::default()
            });
        }
        let nodes = &w.editor.scene().objects[0].shape.contours()[0];
        assert_eq!(nodes.len(), 3);
        assert_eq!((nodes[0], nodes[2]), (original[0], original[2]));
        let node = nodes[1];
        assert_eq!(node.point, original[1].point);
        let (moved, fixed) = if incoming {
            (node.incoming, node.outgoing)
        } else {
            (node.outgoing, node.incoming)
        };
        assert_eq!(fixed, node.point);
        assert!(
            screen_point(moved).distance(tip) < 0.75,
            "handle ended at {moved:?}"
        );
        w.click_edit("Undo");
        assert_eq!(w.editor.scene(), &doc);
        assert!(!w.editor.can_undo());
    }
}

#[test]
fn delete_key_and_edit_menu_remove_clicked_and_box_selected_nodes_with_undo() {
    use super::pen::chord;
    use egui::Modifiers as M;
    let nodes = [[0., 0.1], [0.3, 0.8], [0.5, 0.2], [0.7, 0.8], [1., 0.4]].map(PathNode::corner);
    let doc = drawing(
        Shape::Path {
            nodes: nodes.to_vec(),
            closed: false,
        },
        true,
    );
    let mut w = window(&doc);
    let object = &doc.objects[0];
    w.editor.selection = vec![object.id];
    let board = w.board();
    let at = |p| screen(board, object.local_to_world(p, 0));
    w.click_at(at(nodes[2].point));
    chord(&mut w, Key::Delete, M::NONE);
    assert_eq!(
        w.editor.scene().objects[0].shape.contours()[0],
        [nodes[0], nodes[1], nodes[3], nodes[4]]
    );
    assert_eq!(w.editor.selection, [object.id]);
    w.click_edit("Undo");
    assert_eq!(w.editor.scene(), &doc);
    assert!(!w.editor.can_undo());

    let start = at([0.18, 0.95]);
    w.drag(start, at([0.83, 0.67]) - start);
    assert_eq!(
        crate::path_editor::selected_nodes(&w.ctx, object.id, 0).runs(),
        [(0, 1, 1), (0, 3, 3)]
    );
    w.click_edit("Delete");
    assert_eq!(
        w.editor.scene().objects[0].shape.contours()[0],
        [nodes[0], nodes[2], nodes[4]]
    );
    chord(&mut w, Key::Z, M::COMMAND);
    assert_eq!(w.editor.scene(), &doc);
    assert!(!w.editor.can_undo());
}

#[test]
fn rebound_node_delete_respects_timeline_and_text_focus_and_minimum_path_size() {
    use super::pen::chord;
    use crate::keymap::{self, Action, Keymap};
    use egui::Modifiers as M;
    let nodes = [[0., 0.2], [0.45, 0.8], [1., 0.2]].map(PathNode::corner);
    let doc = drawing(
        Shape::Path {
            nodes: nodes.to_vec(),
            closed: false,
        },
        true,
    );
    let mut w = window(&doc);
    let object = &doc.objects[0];
    w.editor.selection = vec![object.id];
    let mut bindings = Keymap::default();
    bindings.set(Action::Delete, Key::F12, M::NONE).unwrap();
    keymap::install(&w.ctx, bindings.into());

    chord(&mut w, Key::K, M::NONE);
    assert!(w.editor.scene().objects[0].keys.contains_key(&0));
    chord(&mut w, Key::F12, M::NONE);
    assert_eq!(w.editor.scene(), &doc);
    let board = w.board();
    let at = screen(board, object.local_to_world(nodes[1].point, 0));
    w.click_at(at);
    let hold = w.field_on_row("Hold", " fr");
    w.click_at(hold);
    w.frame(vec![]);
    assert!(w.ctx.wants_keyboard_input());
    chord(&mut w, Key::F12, M::NONE);
    assert_eq!(w.editor.scene(), &doc);
    w.click_at(at);
    chord(&mut w, Key::Delete, M::NONE);
    assert_eq!(w.editor.scene(), &doc);
    chord(&mut w, Key::F12, M::NONE);
    let shortened = w.editor.scene().clone();
    assert_eq!(
        shortened.objects[0].shape.contours()[0],
        [nodes[0], nodes[2]]
    );
    chord(&mut w, Key::F12, M::NONE);
    assert_eq!(w.editor.scene(), &shortened);
    chord(&mut w, Key::Z, M::COMMAND);
    assert_eq!(w.editor.scene(), &doc);
}

#[test]
fn toolbar_and_node_shortcuts_edit_only_the_selected_range_and_respect_text_focus() {
    use document_core::path_edit::HandleStyle;
    use egui::Modifiers as M;
    let shape = Shape::Path {
        nodes: [[0., 0.5], [0.25, 0.1], [0.7, 0.6], [1., 0.4]]
            .map(PathNode::corner)
            .to_vec(),
        closed: false,
    };
    let doc = drawing(shape, true);
    let obj = &doc.objects[0];
    let mut w = window(&doc);
    w.size = vec2(800., 500.);
    w.editor.selection = vec![obj.id];
    let board = w.board();
    let point = |index: usize| {
        screen(
            board,
            obj.local_to_world(obj.shape.contours()[0][index].point, 0),
        )
    };
    w.click_at(point(1));
    let modifiers = M::SHIFT;
    for pressed in [true, false] {
        w.frame_with_input(egui::RawInput {
            modifiers,
            events: vec![Event::PointerButton {
                pos: point(2),
                button: egui::PointerButton::Primary,
                pressed,
                modifiers,
            }],
            ..Default::default()
        });
    }
    assert_eq!(
        crate::path_editor::selected_range(&w.ctx, obj.id, 0),
        (0, 1, 2)
    );
    assert!(w.control("Smooth").y < board.top());
    w.click("Smooth");
    let smooth = w.editor.scene().clone();
    let nodes = &smooth.objects[0].shape.contours()[0];
    for i in [1, 2] {
        assert_eq!(nodes[i].handle_style, HandleStyle::Smooth);
    }
    for i in [0, 3] {
        assert_eq!(nodes[i], obj.shape.contours()[0][i]);
    }
    super::pen::chord(&mut w, Key::Y, M::SHIFT);
    let symmetric = w.editor.scene().clone();
    assert!(symmetric.objects[0].shape.contours()[0][1..=2]
        .iter()
        .all(|n| n.handle_style == HandleStyle::Symmetric));
    super::pen::chord(&mut w, Key::C, M::SHIFT);
    let corner = &w.editor.scene().objects[0].shape.contours()[0];
    assert!(corner[1..=2]
        .iter()
        .all(|n| n.handle_style == HandleStyle::Corner));
    for node in &corner[1..=2] {
        assert_eq!(*node, PathNode::corner(node.point));
    }
    w.editor.restore(false);
    assert_eq!(w.editor.scene(), &symmetric);
    w.editor.restore(false);
    assert_eq!(w.editor.scene(), &smooth);
    w.editor.restore(false);
    assert_eq!(w.editor.scene(), &doc);
    assert!(!w.editor.can_undo());
    let hold = w.field_on_row("Hold", " fr");
    w.click_at(hold);
    w.frame(vec![]);
    assert!(w.ctx.wants_keyboard_input());
    super::pen::chord(&mut w, Key::S, M::SHIFT);
    assert_eq!(w.editor.scene(), &doc);
}

#[test]
fn node_hotkeys_follow_geometry_key_mode_and_do_not_edit_during_playback() {
    use crate::timeline::KeyMode;
    use document_core::{path_edit::HandleStyle, Property};
    let doc = drawing(open_curve(), true);
    let id = doc.objects[0].id;
    let mut w = window(&doc);
    w.editor.timeline.frame = 3;
    w.editor.timeline.key_mode = KeyMode::Auto;
    w.editor.selection = vec![id];
    w.frame(vec![]);
    super::pen::chord(&mut w, Key::S, egui::Modifiers::SHIFT);
    let keyed = w.editor.scene().clone();
    let object = &keyed.objects[0];
    assert_eq!(object.shape, doc.objects[0].shape);
    assert_eq!(Property::Geometry.frames(object), [0, 3]);
    assert_eq!(
        object.shape_at(3).contours()[0][0].handle_style,
        HandleStyle::Smooth
    );
    w.editor.timeline.key_mode = KeyMode::Manual;
    w.editor.timeline.frame = 1;
    w.frame(vec![]);
    super::pen::chord(&mut w, Key::Y, egui::Modifiers::SHIFT);
    assert_eq!(w.editor.scene(), &keyed);
    w.editor.timeline.key_mode = KeyMode::Auto;
    w.editor.timeline.toggle_clip(&w.editor.project.clips[0]);
    super::pen::chord(&mut w, Key::C, egui::Modifiers::SHIFT);
    assert_eq!(w.editor.scene(), &keyed);
}

#[test]
fn zoomed_double_click_splits_long_curves_where_the_pointer_lands() {
    // A small part of an oversized pasteboard curve crosses the center of the artboard.
    let length = 100_000.;
    for t in [64.5_f32 / 256., 0.0008] {
        let mut a = PathNode::corner([0., 0.]);
        a.outgoing = [0., 1.];
        let mut b = PathNode::corner([1., 0.]);
        b.incoming = [1., 1.];
        let expected = [t * t * (3. - 2. * t), 3. * t * (1. - t)];
        let mut doc = Document::default();
        let id = doc.add_shape(
            expected.map(|v| 64. - v * length),
            [length; 2],
            [0; 4],
            Shape::Path {
                nodes: vec![a, b],
                closed: false,
            },
            1,
        );
        doc.objects[0].stroke_width = 0.3;
        doc.objects[0].stroke = [60, 195, 140, 255];
        let mut w = window(&doc);
        let center = w.board().center();
        w.frame(vec![Event::PointerMoved(center), Event::Zoom(16.)]);
        let board = w.board();
        let hit = screen(board, doc.objects[0].local_to_world(expected, 0));
        w.click_at(hit);
        w.click_at(hit);
        let o = &w.editor.scene().objects[0];
        let path = &o.shape.contours()[0];
        assert_eq!(path.len(), 3, "missing split at parameter {t}");
        assert!(hit.distance(screen(board, o.local_to_world(path[1].point, 0))) < 0.8);
        assert_eq!(w.editor.selection, [id]);
        w.editor.restore(false);
        assert_eq!(w.editor.scene(), &doc);
        w.editor.selection = vec![id];
        let modifiers = egui::Modifiers::COMMAND | egui::Modifiers::CTRL;
        for pressed in [true, false] {
            w.frame_with_input(egui::RawInput {
                modifiers,
                events: vec![Event::PointerButton {
                    pos: hit,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers,
                }],
                ..Default::default()
            });
        }
        let o = &w.editor.scene().objects[0];
        let path = &o.shape.contours()[0];
        assert_eq!(path.len(), 3, "missing Ctrl-click split at parameter {t}");
        assert!(hit.distance(screen(board, o.local_to_world(path[1].point, 0))) < 0.8);
        w.editor.restore(false);
        assert_eq!(w.editor.scene(), &doc);
    }
}

#[test]
fn double_click_splits_an_unselected_open_curve_once_and_the_new_node_drags_at_two_zooms() {
    for factor in [1., 2.] {
        let doc = drawing(open_curve(), true);
        let id = doc.objects[0].id;
        let mut w = window(&doc);
        let fit = w.board();
        w.frame(vec![Event::PointerMoved(fit.center()), Event::Zoom(factor)]);
        let board = w.board();
        assert!((board.width() / fit.width() - factor).abs() < 0.001);
        let at = screen(board, [64.; 2]);
        w.click_at(at);
        assert_eq!(w.editor.selection, [id]);
        assert_eq!(w.editor.scene(), &doc);
        assert!(!w.editor.can_undo());
        w.click_at(at);
        let split = w.editor.scene().clone();
        let Shape::Path {
            nodes,
            closed: false,
        } = &split.objects[0].shape
        else {
            panic!("open path")
        };
        assert_eq!(nodes.len(), 3);
        assert_eq!(crate::path_editor::selected_range(&w.ctx, id, 0), (0, 1, 1));
        let p = screen(board, split.objects[0].local_to_world(nodes[1].point, 0));
        assert!(p.distance(at) < 0.3 && nodes[1].incoming != nodes[1].outgoing);
        w.drag(p, vec2(13., 17.));
        let obj = &w.editor.scene().objects[0];
        let moved = screen(
            board,
            obj.local_to_world(obj.shape.contours()[0][1].point, 0),
        );
        assert!(moved.distance(p + vec2(13., 17.)) < 0.3);
        w.editor.restore(false);
        assert_eq!(w.editor.scene(), &split);
        w.editor.restore(false);
        assert_eq!(w.editor.scene(), &doc);
        assert!(!w.editor.can_undo());
        w.editor.restore(true);
        assert_eq!(w.editor.scene(), &split);
    }
}

#[test]
fn double_click_ignores_existing_controls_drags_distant_clicks_and_other_tools() {
    let doc = drawing(open_curve(), true);
    let mut w = window(&doc);
    let board = w.board();
    let at = screen(board, [64.; 2]);
    let a = screen(board, doc.objects[0].local_to_world([0.05, 0.5], 0));
    let handle = screen(board, doc.objects[0].local_to_world([0.28, 0.13], 0));
    for pos in [a, a + vec2(2., 0.), handle, at + vec2(0., 32.)] {
        w.click_at(pos);
        w.click_at(pos);
        assert_eq!(w.editor.scene(), &doc);
    }
    w.click_at(at + vec2(30., 15.));
    w.click_at(at);
    assert_eq!(w.editor.scene(), &doc);
    w.drag(at, vec2(22., 24.));
    assert_ne!(w.editor.scene(), &doc);
    assert_eq!(w.editor.scene().objects[0].shape.contours()[0].len(), 2);
    w.editor.restore(false);
    assert_eq!(w.editor.scene(), &doc);
    w.click_at(at);
    assert_eq!(w.editor.scene(), &doc);
    w.editor.set_tool(Tool::Select);
    w.click_at(at);
    w.click_at(at);
    assert_eq!(w.editor.scene(), &doc);
    assert!(!w.editor.can_undo());
}

#[test]
fn double_click_converts_an_edge_only_and_undo_restores_the_exact_primitive() {
    for (shape, stroke, local, outside, count) in [
        (Shape::Rectangle, false, [0.5, 0.], true, 5),
        (Shape::Ellipse, true, [0.8535534, 0.1464466], false, 5),
        (Shape::Ellipse, false, [1., 0.5], false, 4),
        (Shape::Rectangle, true, [1., 0.], false, 4),
    ] {
        let mut doc = drawing(shape, stroke);
        let obj = &mut doc.objects[0];
        obj.rotation = 21.;
        obj.scale = [1.25, 0.85];
        obj.animated.skew = 12.;
        let id = obj.id;
        let mut w = window(&doc);
        let board = w.board();
        let obj = &doc.objects[0];
        let middle = screen(board, obj.local_to_world([0.5; 2], 0));
        w.click_at(middle);
        w.click_at(middle);
        assert_eq!(w.editor.scene(), &doc);
        w.editor.selection.clear();
        let edge = screen(board, obj.local_to_world(local, 0));
        let click = if outside {
            edge + (edge - middle).normalized() * 4.
        } else {
            edge
        };
        w.click_at(click);
        assert_eq!(w.editor.scene(), &doc);
        if stroke {
            assert_eq!(w.editor.selection, [id]);
        }
        if outside {
            assert!(w.editor.selection.is_empty());
        }
        w.click_at(click);
        let converted = &w.editor.scene().objects[0];
        let Shape::Path {
            nodes,
            closed: true,
        } = &converted.shape
        else {
            panic!("converted primitive: {local:?}")
        };
        assert_eq!(nodes.len(), count);
        assert_eq!(w.editor.selection, [id]);
        let (contour, first, last) = crate::path_editor::selected_range(&w.ctx, id, 0);
        assert_eq!((contour, first), (0, last));
        assert!(
            screen(board, converted.local_to_world(nodes[first].point, 0)).distance(click) < 7.
        );
        let mut expected = obj.clone();
        expected.shape = converted.shape.clone();
        assert_eq!(converted, &expected);
        w.editor.restore(false);
        assert_eq!(w.editor.scene(), &doc);
        assert!(!w.editor.can_undo());
    }
}

#[test]
fn double_click_splits_the_closing_segment_of_a_compound_hole() {
    let corners = |min, max| {
        [[min, min], [max, min], [max, max], [min, max]]
            .map(PathNode::corner)
            .to_vec()
    };
    let mut hole = corners(0.25, 0.75);
    hole.reverse();
    let doc = drawing(
        Shape::Compound {
            contours: vec![corners(0., 1.), hole],
        },
        true,
    );
    let mut w = window(&doc);
    let pos = screen(w.board(), doc.objects[0].local_to_world([0.25, 0.5], 0));
    w.click_at(pos);
    w.click_at(pos);
    let contours = w.editor.scene().objects[0].shape.contours();
    assert_eq!(contours[0], doc.objects[0].shape.contours()[0]);
    assert_eq!(contours[1].len(), 5);
    assert_eq!(
        crate::path_editor::selected_range(&w.ctx, doc.objects[0].id, 0),
        (1, 4, 4)
    );
    w.editor.restore(false);
    assert_eq!(w.editor.scene(), &doc);
}

#[test]
fn double_click_keeps_hidden_locked_keyed_and_effected_geometry_intact() {
    let plain = drawing(open_curve(), true);
    let mut hidden = plain.clone();
    hidden.layers[0].visible = false;
    let mut locked = plain.clone();
    locked.layers[0].locked = true;
    let mut keyed = plain.clone();
    keyed.objects[0]
        .animated
        .geometry_keys
        .insert(0, open_curve());
    let mut effected = plain;
    effected.objects[0]
        .effects
        .push(document_core::PathEffect::Mirror { horizontal: true });
    for doc in [hidden, locked, keyed, effected] {
        let mut w = window(&doc);
        let at = screen(w.board(), [64.; 2]);
        w.click_at(at);
        w.click_at(at);
        assert_eq!(w.editor.scene(), &doc);
        assert!(!w.editor.can_undo());
    }
}

#[test]
fn combining_shift_selected_anchors_updates_a_held_drawing_with_one_undo() {
    let nodes = [[0., 0.], [1., 0.], [1., 1.], [0., 1.]].map(PathNode::corner);
    let doc = drawing(
        Shape::Path {
            nodes: nodes.to_vec(),
            closed: true,
        },
        true,
    );
    let object = &doc.objects[0];
    let mut w = window(&doc);
    w.editor.selection = vec![object.id];
    let board = w.board();
    w.click_at(screen(board, object.local_to_world(nodes[0].point, 0)));
    let at = screen(board, object.local_to_world(nodes[2].point, 0));
    w.frame(vec![Event::PointerMoved(at)]);
    let modifiers = egui::Modifiers::SHIFT;
    for pressed in [true, false] {
        w.frame_with_input(egui::RawInput {
            modifiers,
            events: vec![Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers,
            }],
            ..Default::default()
        });
    }
    assert_eq!(
        crate::path_editor::selected_nodes(&w.ctx, object.id, 0).runs(),
        [(0, 0, 0), (0, 2, 2)]
    );
    w.click("Combine nodes");
    let clip = &w.editor.project.clips[0];
    let combined = [PathNode::corner([0.5, 0.5]), nodes[1], nodes[3]];
    for t in [0, 1, clip.frames - 1] {
        assert_eq!(clip.drawing_at(t).objects[0].shape.contours()[0], combined);
    }
    w.click_edit("Undo");
    assert_eq!(w.editor.scene(), &doc);
    assert!(!w.editor.can_undo());
}

#[test]
fn ctrl_click_still_splits_an_authored_path_on_press_and_skips_primitives() {
    for shape in [open_curve(), Shape::Rectangle] {
        let doc = drawing(shape, true);
        let mut w = window(&doc);
        w.editor.selection = vec![doc.objects[0].id];
        let is_curve = !doc.objects[0].shape.contours().is_empty();
        let at = screen(w.board(), if is_curve { [64.; 2] } else { [64., 32.] });
        w.frame(vec![Event::PointerMoved(at)]);
        let modifiers = egui::Modifiers::COMMAND | egui::Modifiers::CTRL;
        for pressed in [true, false] {
            w.frame_with_input(egui::RawInput {
                modifiers,
                events: vec![Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers,
                }],
                ..Default::default()
            });
            if is_curve {
                assert_eq!(w.editor.scene().objects[0].shape.contours()[0].len(), 3);
            } else {
                assert_eq!(w.editor.scene(), &doc);
            }
        }
        if is_curve {
            w.editor.restore(false);
            assert_eq!(w.editor.scene(), &doc);
        }
        assert!(!w.editor.can_undo());
    }
}

#[test]
fn double_click_reaches_a_thin_foreground_edge_beside_its_ink_but_never_through_a_cover() {
    for same_layer in [true, false] {
        for covered in [false, true] {
            let mut doc = Document::default();
            let background = doc.add_rectangle([2.; 2], [124.; 2], [242; 4]);
            doc.layers.push(document_core::Layer {
                id: 2,
                name: "Drawing".into(),
                ..Default::default()
            });
            let id = doc.add_shape(
                [32.; 2],
                [64.; 2],
                [0; 4],
                open_curve(),
                if same_layer { 1 } else { 2 },
            );
            doc.objects[1].stroke = [18, 65, 37, 255];
            doc.objects[1].stroke_width = 0.15;
            if !same_layer {
                doc.objects.swap(0, 1); // Layer order wins over object order.
            }
            let painted = if covered {
                let cover = doc.add_rectangle([52.; 2], [24.; 2], [255; 4]);
                doc.objects.last_mut().unwrap().layer = if same_layer { 1 } else { 2 };
                cover
            } else {
                background
            };
            let mut w = window(&doc);
            w.editor.selection = vec![id];
            let at = screen(w.board(), [64.; 2]) + vec2(0., 3.);
            w.click_at(at);
            assert_eq!(w.editor.selection, [painted]);
            assert_eq!(w.editor.scene(), &doc);
            w.click_at(at);
            if covered {
                assert_eq!(w.editor.selection, [painted]);
                assert_eq!(w.editor.scene(), &doc);
                assert!(!w.editor.can_undo());
            } else {
                assert_eq!(w.editor.selection, [id]);
                let path = w
                    .editor
                    .scene()
                    .objects
                    .iter()
                    .find(|o| o.id == id)
                    .unwrap();
                assert_eq!(path.shape.contours()[0].len(), 3);
                w.editor.restore(false);
                assert_eq!(w.editor.scene(), &doc);
                assert!(!w.editor.can_undo());
            }
        }
    }
}

mod bend_animation;

#[test]
fn editing_an_extra_node_on_a_guided_destination_preserves_the_start_pose() {
    use document_core::path_edit::HandleStyle;
    let source = open_curve();
    let mut doc = drawing(source.clone(), true);
    let object = &mut doc.objects[0];
    let destination = Shape::Path {
        closed: false,
        nodes: [[0., 0.45], [0.35, 0.2], [0.65, 0.8], [1., 0.5]]
            .map(PathNode::corner)
            .to_vec(),
    };
    object
        .set_guided_morph(0, 3, source.clone(), destination.clone(), vec![])
        .unwrap();
    let id = object.id;
    let extra = 2;
    let world = object.local_to_world(destination.contours()[0][extra].point, 3);
    let mut w = window(&doc);
    w.editor.selection = vec![id];
    w.editor.timeline.frame = 3;
    w.frame(vec![]);
    let start = screen(w.board(), world);
    w.click_at(start);
    w.click("Smooth");
    let object = &w.editor.scene().objects[0];
    assert_eq!(
        object.shape_at(3).contours()[0][extra].handle_style,
        HandleStyle::Smooth
    );
    assert_eq!(object.shape_at(0).as_ref(), &source);
    w.drag(start, vec2(-12., 11.));
    let object = &w.editor.scene().objects[0];
    assert_ne!(
        object.shape_at(3).contours()[0][extra].point,
        destination.contours()[0][extra].point
    );
    assert_eq!(object.shape_at(0).as_ref(), &source);
    w.editor.restore(false);
    w.editor.restore(false);
    assert_eq!(w.editor.scene(), &doc);
}
