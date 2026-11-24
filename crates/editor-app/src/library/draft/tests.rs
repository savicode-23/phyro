use super::*;
use document_core::{BooleanOp, Document, Shape, SymbolDefinition};
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect};

struct Window {
    ctx: egui::Context,
    project: AnimationProject,
    draft: Draft,
    pencil_smoothing: u8,
    pencil_pressure: bool,
    finished: Option<Option<String>>,
}

#[test]
fn symbol_fill_type_applies_to_instances_and_supports_local_undo() {
    use document_core::GradientKind;
    let mut w = Window::new();
    let outer = w.project.clone();
    let original = w.drawing().clone();
    w.draft.selection = vec![original.objects[0].id];
    w.click_label("Fill");
    w.click_label("Rectangular");
    assert!(matches!(
        w.drawing().objects[0].gradient.as_ref().unwrap().kind,
        GradientKind::Rectangular { .. }
    ));
    assert_eq!(w.project, outer);
    w.key(Key::Z, Modifiers::CTRL | Modifiers::COMMAND);
    assert_eq!(w.drawing(), &original);
    w.click_board([20., 24.], PointerButton::Primary, Modifiers::NONE);
    w.click_label("Fill");
    w.click_label("Radial");
    w.click_label("Stroke");
    w.click_label("Linear");
    w.click_label("Apply to instances");
    let artwork = &w.project.symbols.values().next().unwrap().artwork;
    assert!(matches!(
        artwork.objects[0].gradient.as_ref().unwrap().kind,
        GradientKind::Radial { .. }
    ));
    assert!(matches!(
        artwork.objects[0].stroke_gradient.as_ref().unwrap().kind,
        GradientKind::Linear { .. }
    ));
}

#[test]
fn pen_undo_edits_one_anchor_in_the_symbol_before_committing_the_path() {
    let mut w = Window::new();
    let parent = w.project.clone();
    let original = w.drawing().clone();
    let cmd = Modifiers::COMMAND | Modifiers::CTRL;
    w.key(Key::B, Modifiers::NONE);
    for point in [[18., 92.], [42., 76.], [64., 100.], [84., 88.]] {
        w.click_board(point, PointerButton::Primary, Modifiers::NONE);
    }
    w.key(Key::Z, cmd);
    w.click_label("Edit");
    w.click_label("Undo");
    w.key(Key::Z, cmd | Modifiers::SHIFT);
    w.click_board([99., 72.], PointerButton::Primary, Modifiers::NONE);
    w.key(Key::Enter, Modifiers::NONE);
    let doc = w.drawing();
    assert_eq!(doc.objects.len(), original.objects.len() + 1);
    let path = doc.objects.last().unwrap();
    let nodes = &path.shape.contours()[0];
    assert_eq!(nodes.len(), 4);
    for (node, point) in nodes
        .iter()
        .zip([[18., 92.], [42., 76.], [64., 100.], [99., 72.]])
    {
        let actual = path.local_to_world(node.point, 0);
        assert!(Pos2::from(actual).distance(Pos2::from(point)) < 0.2);
    }
    w.key(Key::Z, cmd);
    assert_eq!(*w.drawing(), original);
    assert_eq!(w.project, parent);
}

#[test]
fn pencil_strokes_in_one_tablet_batch_have_separate_symbol_draft_undo_steps() {
    let mut w = Window::new();
    w.key(Key::P, Modifiers::NONE);
    let parent = w.project.clone();
    let original = w.drawing().clone();
    let board = w
        .frame(Modifiers::NONE, vec![])
        .shapes
        .iter()
        .find_map(|s| crate::canvas::painted_artboard(&s.shape))
        .unwrap();
    let a = board.center();
    let b = a + egui::vec2(18., -9.);
    let button = |pos, pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    w.frame(
        Modifiers::NONE,
        vec![
            Event::PointerMoved(a),
            button(a, true),
            Event::PointerMoved(b),
            button(b, false),
            button(b, true),
            Event::PointerMoved(a),
            button(a, false),
        ],
    );
    assert_eq!(w.drawing().objects.len(), original.objects.len() + 2);
    w.key(Key::Z, Modifiers::COMMAND | Modifiers::CTRL);
    assert_eq!(w.drawing().objects.len(), original.objects.len() + 1);
    w.key(Key::Z, Modifiers::COMMAND | Modifiers::CTRL);
    assert_eq!(*w.drawing(), original);
    assert_eq!(w.project, parent);
}

#[test]
fn node_style_shortcuts_edit_the_active_symbol_draft_in_one_step() {
    use document_core::path_edit::{primitive_path, HandleStyle};
    let mut w = Window::new();
    let outer = w.project.clone();
    let obj = &mut w.draft.temp.clips[0].drawing_at_mut(0).objects[0];
    obj.shape = primitive_path(&obj.shape).unwrap();
    w.draft.selection = vec![obj.id];
    w.draft.record();
    let before = w.drawing().clone();
    w.key(Key::N, Modifiers::NONE);
    w.key(Key::Y, Modifiers::SHIFT);
    assert_eq!(
        w.drawing().objects[0].shape.contours()[0][0].handle_style,
        HandleStyle::Symmetric
    );
    assert_eq!(w.project, outer);
    w.key(Key::Z, Modifiers::COMMAND | Modifiers::CTRL);
    assert_eq!(w.drawing(), &before);
    w.click_board([20., 24.], PointerButton::Primary, Modifiers::NONE);
    assert_eq!(w.draft.selection, [before.objects[0].id]);
    w.click_label("Smooth");
    w.click_label("Apply to instances");
    assert_eq!(
        w.project.symbols.values().next().unwrap().artwork.objects[0]
            .shape
            .contours()[0][0]
            .handle_style,
        HandleStyle::Smooth
    );
}

#[test]
fn simplify_records_smoothing_in_the_symbol_draft() {
    let mut w = Window::new();
    let parent = w.project.clone();
    let drawing = w.draft.temp.clips[0].drawing_at_mut(0);
    let id = drawing.add_shape(
        [10., 10.],
        [100., 100.],
        [255; 4],
        Shape::Path {
            nodes: [[0., 0.4], [0.5, 0.6], [1., 0.4]]
                .map(document_core::PathNode::corner)
                .to_vec(),
            closed: false,
        },
        1,
    );
    w.draft.selection = vec![id];
    w.draft.record();
    let original = w.drawing().clone();
    let command = Modifiers::COMMAND | Modifiers::CTRL;

    w.key(Key::L, command);

    let shape = &w.drawing().objects.last().unwrap().shape;
    assert_ne!(shape, &original.objects.last().unwrap().shape);
    assert_eq!(shape.contours()[0].len(), 3);
    assert_eq!(w.project, parent);
    w.key(Key::Z, command);
    assert_eq!(w.drawing(), &original);
}

#[test]
fn delete_in_nodes_mode_edits_only_selected_symbol_nodes_with_local_undo() {
    use document_core::path_edit::{primitive_path, subdivide};
    let mut w = Window::new();
    let outer = w.project.clone();
    let object = &mut w.draft.temp.clips[0].drawing_at_mut(0).objects[0];
    object.shape = primitive_path(&object.shape).unwrap();
    assert!(subdivide(&mut object.shape.contours_mut()[0], true, 3));
    let id = object.id;
    w.draft.record();
    let before = w.drawing().clone();
    let mut expected = before.clone();
    expected.objects[0].shape.contours_mut()[0].remove(1);

    w.click_board([20., 24.], PointerButton::Primary, Modifiers::NONE);
    w.key(Key::N, Modifiers::NONE);
    w.click_board([92., 16.], PointerButton::Primary, Modifiers::NONE);
    assert_eq!(crate::path_editor::selected_range(&w.ctx, id, 0), (0, 1, 1));
    for (pressed, repeat) in [(true, false), (true, true), (false, false)] {
        w.frame(
            Modifiers::NONE,
            vec![Event::Key {
                key: Key::Delete,
                physical_key: Some(Key::Delete),
                pressed,
                repeat,
                modifiers: Modifiers::NONE,
            }],
        );
    }
    assert_eq!(w.drawing(), &expected);
    assert_eq!(w.draft.selection, [id]);
    let mut minimum = expected.clone();
    minimum.objects[0].shape.contours_mut()[0].remove(1);
    w.key(Key::Delete, Modifiers::NONE);
    assert_eq!(w.drawing(), &minimum);
    // Three anchors are the minimum for a closed path.
    w.key(Key::Delete, Modifiers::NONE);
    assert_eq!(w.drawing(), &minimum);
    assert_eq!(w.draft.selection, [id]);

    let cmd = Modifiers::COMMAND | Modifiers::CTRL;
    w.key(Key::Z, cmd);
    assert_eq!(w.drawing(), &expected);
    w.key(Key::Z, cmd);
    assert_eq!(w.drawing(), &before);
    w.key(Key::Z, cmd | Modifiers::SHIFT);
    assert_eq!(w.drawing(), &expected);
    assert_eq!(w.project, outer);
    w.click_label("Apply to instances");
    assert_eq!(
        *w.project.symbols[&w.draft.symbol_id().unwrap()].artwork,
        expected
    );
}

#[test]
fn symbol_draft_places_a_pivot_outside_the_artwork_and_undoes_it_locally() {
    let mut w = Window::new();
    let parent = w.project.clone();
    let before = w.drawing().clone();
    let id = before.objects[0].id;
    w.click_board([20., 24.], PointerButton::Primary, Modifiers::NONE);
    assert_eq!(w.draft.selection, [id]);
    w.click_label("Edit pivot");
    w.click_board([100., 92.], PointerButton::Primary, Modifiers::NONE);
    let placed = w.drawing().clone();
    let point = Pos2::from(placed.objects[0].pivot_at(0));
    assert!(point.distance(Pos2::new(100., 92.)) < 0.001);
    assert_eq!(w.project, parent);
    w.key(Key::Escape, Modifiers::NONE);
    assert_eq!(w.draft.selection, [id]);
    let from = w.board_point([100., 92.]);
    let to = w.board_point([106., 80.]);
    w.frame(Modifiers::NONE, vec![Event::PointerMoved(from)]);
    for (pos, pressed) in [(from, true), (to, false)] {
        w.frame(
            Modifiers::NONE,
            vec![
                Event::PointerMoved(pos),
                Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                },
            ],
        );
    }
    let moved = Pos2::from(w.drawing().objects[0].pivot_at(0));
    assert!(moved.distance(Pos2::new(106., 80.)) < 0.001);
    assert_eq!(w.project, parent);
    let cmd = Modifiers::COMMAND | Modifiers::CTRL;
    w.key(Key::Z, cmd);
    assert_eq!(w.drawing(), &placed);
    w.key(Key::Z, cmd);
    assert_eq!(w.drawing(), &before);
    w.key(Key::Z, cmd | Modifiers::SHIFT);
    w.click_label("Apply to instances");
    assert_eq!(
        *w.project.symbols[&w.draft.symbol_id().unwrap()].artwork,
        placed
    );
}

#[test]
fn properties_can_mask_and_release_artwork_in_a_symbol_draft() {
    let mut w = Window::new();
    let original = w.drawing().clone();
    let outer = w.project.clone();
    let id = original.objects[0].id;
    let cutter = original.objects[1].id;
    w.draft.selection = vec![id, cutter];
    w.click_label("Clip and mask");
    w.click_label("Set Mask");
    let masked = w.drawing().clone();
    assert_eq!(w.draft.selection, [id]);
    assert_eq!(masked.objects.len(), original.objects.len() - 1);
    assert_eq!(
        masked.objects[0].stencil.as_ref().unwrap().alpha,
        Some([0, 255])
    );
    assert_eq!(w.project, outer);
    w.click_label("Release Mask");
    assert_eq!(w.drawing().objects, original.objects);
    w.click_label("Edit");
    w.click_label("Undo");
    assert_eq!(w.drawing(), &masked);
    w.click_label("Apply to instances");
    assert_eq!(
        *w.project.symbols[&w.draft.symbol_id().unwrap()].artwork,
        masked
    );
}

#[test]
fn eyedropper_edits_the_draft_and_keeps_master_paint_unchanged_until_apply() {
    let mut w = Window::new();
    let old = w.project.clone();
    let target = w.drawing().objects[1].id;
    w.draft.selection = vec![target];
    w.key(Key::D, Modifiers::NONE);
    assert!(w.draft.tool == Tool::Eyedropper);
    w.click_board([16., 21.], PointerButton::Primary, Modifiers::NONE);
    assert_eq!(w.drawing().objects[1].fill, [55, 94, 162, 255]);
    assert_eq!(w.project, old);
    assert_eq!(w.draft.selection, [target]);
    w.key(Key::Z, Modifiers::CTRL | Modifiers::COMMAND);
    assert_eq!(w.drawing().objects[1].fill, [228; 4]);
    w.draft.selection = vec![target];
    w.click_label("Eyedropper  D");
    w.click_board([16., 21.], PointerButton::Primary, Modifiers::SHIFT);
    assert_eq!(w.drawing().objects[1].stroke, [55, 94, 162, 255]);
    w.click_label("Apply to instances");
    assert_eq!(
        w.project.symbols.values().next().unwrap().artwork.objects[1].stroke,
        [55, 94, 162, 255]
    );
}

#[test]
fn editing_a_detached_copy_only_updates_that_placement() {
    let mut w = Window::new();
    let symbol = w.draft.symbol_id().unwrap();
    let first = w.project.clips[0].drawing_at(0).objects[0].id;
    let other = w.project.place_symbol(symbol, 0, 0, 1, [20., 12.]).unwrap();
    w.project.remove_symbol_keeping_artwork(symbol).unwrap();
    let artwork = w.project.clips[0].drawing_at(0).objects[0]
        .static_artwork()
        .unwrap()
        .clone();
    w.draft = Draft::new(
        Target::Copy {
            clip: 0,
            frame: 0,
            object: first,
        },
        "Patch".into(),
        &artwork,
        &w.project,
    );
    w.draft.selection = vec![artwork.objects[0].id];
    w.key(Key::D, Modifiers::CTRL | Modifiers::COMMAND);
    assert_eq!(w.drawing().objects.len(), artwork.objects.len() + 1);
    w.click_label("Apply to this copy");
    assert!(matches!(w.finished, Some(Some(_))));
    assert!(w.project.symbols.is_empty());
    let objects = &w.project.clips[0].drawing_at(0).objects;
    assert_eq!(
        objects
            .iter()
            .find(|o| o.id == first)
            .unwrap()
            .static_artwork()
            .unwrap()
            .objects
            .len(),
        artwork.objects.len() + 1
    );
    assert_eq!(
        objects
            .iter()
            .find(|o| o.id == other)
            .unwrap()
            .static_artwork()
            .unwrap(),
        &artwork
    );
}

#[test]
fn alignment_shortcut_reveals_the_draft_controls_and_keeps_the_outer_project() {
    let mut w = Window::new();
    let outer = w.project.clone();
    let doc = w.drawing().clone();
    let ids = vec![doc.objects[0].id, doc.objects[1].id];
    w.draft.selection = ids.clone();
    w.key(
        Key::A,
        Modifiers::CTRL | Modifiers::COMMAND | Modifiers::SHIFT,
    );
    for _ in 0..24 {
        w.frame(Modifiers::NONE, vec![]);
    }
    assert_eq!(w.draft.selection, ids);
    w.click_label("Right");
    assert_eq!(w.drawing().objects[1].position, [80., 36.]);
    w.key(Key::Z, Modifiers::CTRL | Modifiers::COMMAND);
    assert_eq!(w.drawing(), &doc);
    assert_eq!(w.project, outer);
}

#[test]
fn flip_shortcuts_edit_the_symbol_selection_locally_until_apply() {
    for (key, first, second) in [
        (Key::H, [64., 36., 76., 52.], [28., 52., 40., 68.]),
        (Key::V, [28., 52., 40., 68.], [64., 36., 76., 52.]),
    ] {
        let mut w = Window::new();
        let original = w.drawing().clone();
        let master = w.project.clone();
        let symbol = w.draft.symbol_id().unwrap();
        let ids = [original.objects[1].id, original.objects[2].id];
        w.draft.selection = ids.to_vec();
        w.key(key, Modifiers::NONE);
        let flipped = w.drawing().clone();
        assert_eq!(flipped.objects[1].bounds(0), first);
        assert_eq!(flipped.objects[2].bounds(0), second);
        assert_eq!(flipped.objects[0], original.objects[0]);
        assert_eq!(w.draft.selection, ids);
        w.key(Key::Z, Modifiers::CTRL | Modifiers::COMMAND);
        assert_eq!(w.drawing(), &original);
        assert!(!w.draft.history.can_undo());
        w.key(Key::Y, Modifiers::CTRL | Modifiers::COMMAND);
        assert_eq!(w.drawing(), &flipped);
        assert_eq!(w.project, master);
        w.click_label("Apply to instances");
        assert_eq!(*w.project.symbols[&symbol].artwork, flipped);
    }
}

#[test]
fn duplicate_in_a_symbol_draft_has_its_own_selection_and_undo() {
    let mut w = Window::new();
    let outer = w.project.clone();
    let original = w.drawing().clone();
    w.draft.selection = vec![original.objects[0].id];
    let cmd = Modifiers::CTRL | Modifiers::COMMAND;
    w.key(Key::D, cmd);
    let copied = w.drawing().clone();
    let copy = copied.objects.last().unwrap().id;
    assert_eq!(w.draft.selection, [copy]);
    assert_eq!(copied.objects.len(), original.objects.len() + 1);
    w.key(Key::Z, cmd);
    assert_eq!(w.drawing(), &original);
    assert!(!w.draft.history.can_undo());
    w.key(Key::Y, cmd);
    assert_eq!(w.drawing(), &copied);
    assert_eq!(w.project, outer);
}

impl Window {
    fn new() -> Self {
        let mut drawing = Document {
            frames: 1,
            ..Default::default()
        };
        drawing.add_rectangle([12., 16.], [80., 72.], [55, 94, 162, 255]);
        drawing.add_rectangle([28., 36.], [12., 16.], [228; 4]);
        drawing.add_rectangle([64., 52.], [12., 16.], [220; 4]);
        let mut project = AnimationProject::default();
        let id = project
            .add_symbol(SymbolDefinition {
                name: "Patch".into(),
                artwork: drawing.into(),
            })
            .unwrap();
        project.place_symbol(id, 0, 0, 1, [0.; 2]).unwrap();
        let ctx = egui::Context::default();
        ctx.style_mut(|style| style.animation_time = 0.);
        let mut window = Self {
            ctx,
            draft: Draft::symbol(id, &project),
            pencil_smoothing: crate::tool_keys::DEFAULT_PENCIL_SMOOTHING,
            pencil_pressure: false,
            project,
            finished: None,
        };
        for _ in 0..3 {
            window.frame(Modifiers::NONE, vec![]);
        }
        window
    }

    fn drawing(&self) -> &Document {
        self.draft.temp.clips[0].drawing_at(0)
    }

    fn frame(&mut self, modifiers: Modifiers, events: Vec<Event>) -> egui::FullOutput {
        self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1200., 900.))),
                modifiers,
                events,
                ..Default::default()
            },
            |ctx| {
                self.finished = self.draft.show(
                    ctx,
                    &mut self.project,
                    false,
                    &mut self.pencil_smoothing,
                    &mut self.pencil_pressure,
                );
            },
        )
    }

    fn key(&mut self, key: Key, modifiers: Modifiers) {
        for pressed in [true, false] {
            self.frame(
                modifiers,
                vec![Event::Key {
                    key,
                    physical_key: Some(key),
                    pressed,
                    repeat: false,
                    modifiers,
                }],
            );
        }
    }

    fn click(&mut self, pos: Pos2, button: PointerButton, modifiers: Modifiers) {
        self.frame(modifiers, vec![Event::PointerMoved(pos)]);
        for pressed in [true, false] {
            self.frame(
                modifiers,
                vec![Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    modifiers,
                }],
            );
        }
    }

    fn board_point(&mut self, point: [f32; 2]) -> Pos2 {
        let out = self.frame(Modifiers::NONE, vec![]);
        let board = out
            .shapes
            .iter()
            .find_map(|s| crate::canvas::painted_artboard(&s.shape))
            .unwrap();
        board.min + board.size() * (egui::Vec2::from(point) / 128.)
    }

    fn click_board(&mut self, point: [f32; 2], button: PointerButton, modifiers: Modifiers) {
        let pos = self.board_point(point);
        self.click(pos, button, modifiers);
    }

    fn control(&mut self, matches: impl Fn(&str) -> bool) -> Pos2 {
        let out = self.frame(Modifiers::NONE, vec![]);
        out.shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if matches(t.galley.text()) => {
                    let rect = t.galley.rect.translate(t.pos.to_vec2());
                    s.clip_rect.contains_rect(rect).then_some(rect.center())
                }
                _ => None,
            })
            .expect("visible draft control")
    }

    fn click_label(&mut self, label: &str) {
        let point = self.control(|text| text == label);
        self.click(point, PointerButton::Primary, Modifiers::NONE);
    }
}

#[test]
fn group_and_ungroup_shortcuts_edit_only_the_open_artwork() {
    let mut w = Window::new();
    let original = w.drawing().clone();
    let master = w.project.clone();
    let members: Vec<_> = original.objects[..2].iter().map(|o| o.id).collect();
    w.draft.selection = members.clone();
    let cmd = Modifiers::CTRL | Modifiers::COMMAND;
    w.key(Key::G, cmd);
    let grouped = w.drawing().clone();
    assert!(grouped.objects[0].group.is_some());
    assert_eq!(grouped.objects[0].group, grouped.objects[1].group);
    assert_eq!(grouped.objects[2], original.objects[2]);
    w.draft.selection = vec![members[1]];
    w.key(Key::G, cmd | Modifiers::SHIFT);
    assert_eq!(w.drawing(), &original);
    assert_eq!(w.draft.selection, members);
    w.key(Key::Z, cmd);
    assert_eq!(w.drawing(), &grouped);
    assert_eq!(w.project, master);
}

#[test]
fn paste_style_from_the_draft_clipboard_repaints_a_just_closed_path_as_one_edit() {
    let mut w = Window::new();
    let outer = w.project.clone();
    let first = &mut w.draft.temp.clips[0].drawing_at_mut(0).objects[0];
    first.stroke = [140, 80, 25, 190];
    first.stroke_width = 4.5;
    let source = first.clone();
    w.draft.selection = vec![source.id];
    let cmd = Modifiers::CTRL | Modifiers::COMMAND;
    let copied = w.frame(cmd, vec![Event::Copy]);
    let payload = copied
        .platform_output
        .commands
        .into_iter()
        .find_map(|o| match o {
            egui::OutputCommand::CopyText(text) => Some(text),
            _ => None,
        })
        .unwrap();
    w.key(Key::B, Modifiers::NONE);
    let corner = [20., 100.];
    for at in [corner, [40., 100.], [30., 122.], corner] {
        w.click_board(at, PointerButton::Primary, Modifiers::NONE);
    }
    let before = w.drawing().clone();
    let path = before.objects.last().unwrap();
    assert_eq!(w.draft.selection, [path.id]);
    w.key(Key::V, cmd | Modifiers::SHIFT);
    w.frame(Modifiers::NONE, vec![Event::Paste(payload)]);
    let result = w.drawing().objects.last().unwrap();
    assert_eq!(result.shape, path.shape);
    assert_eq!(
        (result.fill, result.stroke, result.stroke_width),
        (source.fill, source.stroke, source.stroke_width)
    );
    assert_eq!(w.draft.style.stroke_width, source.stroke_width);
    assert!(w.draft.tool == Tool::Pen);
    assert_eq!(w.draft.selection, [path.id]);
    assert_eq!(w.project, outer);
    w.key(Key::Z, cmd);
    assert_eq!(w.drawing(), &before);
}

#[test]
fn a_focused_draft_paint_field_keeps_tool_and_boolean_keys_out_of_the_canvas() {
    let mut w = Window::new();
    w.draft.set_tool(Tool::Ellipse);
    let original = w.drawing().clone();
    w.draft.selection = original.objects[..2].iter().map(|o| o.id).collect();
    let width = w.control(|text| text.ends_with(" px"));
    w.click(width, PointerButton::Primary, Modifiers::NONE);
    w.frame(Modifiers::NONE, vec![]);
    assert!(w.ctx.wants_keyboard_input());
    w.key(
        Key::Num8,
        Modifiers::COMMAND | Modifiers::CTRL | Modifiers::SHIFT,
    );
    w.key(Key::B, Modifiers::NONE);
    w.key(Key::S, Modifiers::NONE);
    assert!(w.draft.tool == Tool::Ellipse);
    assert_eq!(w.drawing(), &original);
}

#[test]
fn draft_uses_the_same_pen_and_pencil_shortcuts_as_the_canvas() {
    let mut w = Window::new();
    w.control(|label| label == "Select  S");
    w.control(|label| label == "Pen  B");
    w.control(|label| label == "Pencil  P");
    w.control(|label| label == "Text  T");
    w.key(Key::Tab, Modifiers::NONE);
    let focused = w.ctx.memory(|memory| memory.focused()).unwrap();
    assert!(egui::TextEdit::load_state(&w.ctx, focused).is_none());
    for (key, modifiers, tool) in [
        (Key::B, Modifiers::NONE, Tool::Pen),
        (Key::S, Modifiers::NONE, Tool::Select),
        (Key::F5, Modifiers::NONE, Tool::Ellipse),
        (Key::F6, Modifiers::SHIFT, Tool::Pen),
        (Key::T, Modifiers::NONE, Tool::Text),
        (Key::F8, Modifiers::NONE, Tool::Text),
        (Key::H, Modifiers::NONE, Tool::Text),
        (Key::V, Modifiers::NONE, Tool::Text),
        (Key::P, Modifiers::NONE, Tool::Pencil),
        (Key::F6, Modifiers::SHIFT, Tool::Pen),
        (Key::F6, Modifiers::NONE, Tool::Pencil),
    ] {
        w.key(key, modifiers);
        assert!(w.draft.tool == tool, "{key:?} {modifiers:?}");
    }
}

#[test]
fn point_text_in_a_symbol_draft_undoes_locally_and_applies_to_linked_instances() {
    let mut w = Window::new();
    let symbol = w.draft.symbol_id().unwrap();
    w.project.place_symbol(symbol, 0, 0, 1, [34., 8.]).unwrap();
    let outer = w.project.clone();
    let original = w.drawing().clone();
    let cmd = Modifiers::CTRL | Modifiers::COMMAND;
    w.click_label("Text  T");
    w.click_board([16., 91.], PointerButton::Primary, Modifiers::NONE);
    w.frame(Modifiers::NONE, vec![Event::Text("First".into())]);
    w.key(Key::Enter, cmd);
    let first = w.drawing().clone();
    let first_label = first.objects.last().unwrap();
    assert_eq!(first_label.text.as_ref().unwrap().content, "First");
    assert!(Pos2::from(first_label.position).distance(egui::pos2(16., 91.)) < 0.005);
    assert_eq!(w.draft.selection, [first_label.id]);
    w.key(Key::Z, cmd);
    assert_eq!(w.drawing(), &original);
    assert!(!w.draft.history.can_undo());
    w.key(Key::Y, cmd);
    assert_eq!(w.drawing(), &first);
    assert_eq!(w.project, outer);
    w.key(Key::F8, Modifiers::NONE);
    w.click_board([75., 94.], PointerButton::Primary, Modifiers::NONE);
    w.frame(Modifiers::NONE, vec![Event::Text("Second".into())]);
    w.click_label("Apply to instances");
    assert!(matches!(w.finished, Some(Some(_))));
    let master = &w.project.symbols[&symbol].artwork;
    let labels: Vec<_> = master
        .objects
        .iter()
        .filter_map(|o| o.text.as_ref().map(|t| t.content.as_str()))
        .collect();
    assert_eq!(labels, ["First", "Second"]);
    let instances = &w.project.clips[0].drawing_at(0).objects;
    assert_eq!(instances.len(), 2);
    for instance in instances {
        assert_eq!(instance.instance_at(0).unwrap().0, master.as_ref());
    }
}

#[test]
fn invalid_point_text_stays_in_the_draft_for_correction_before_apply() {
    let mut w = Window::new();
    let outer = w.project.clone();
    w.key(Key::T, Modifiers::NONE);
    w.click_board([16., 91.], PointerButton::Primary, Modifiers::NONE);
    w.frame(Modifiers::NONE, vec![Event::Text("\u{10ffff}".into())]);
    w.click_label("Apply to instances");
    assert!(w.draft.canvas.text_active());
    assert!(w.finished.is_none());
    assert_eq!(w.project, outer);
    w.key(Key::A, Modifiers::CTRL | Modifiers::COMMAND);
    w.frame(Modifiers::NONE, vec![Event::Text("Valid".into())]);
    w.click_label("Apply to instances");
    assert!(matches!(w.finished, Some(Some(_))));
    let master = &w.project.symbols[&w.draft.symbol_id().unwrap()].artwork;
    assert_eq!(
        master
            .objects
            .last()
            .unwrap()
            .text
            .as_ref()
            .unwrap()
            .content,
        "Valid"
    );
}

#[test]
fn double_click_converts_and_splits_a_symbol_edge_with_draft_undo() {
    let mut w = Window::new();
    let outer = w.project.clone();
    let original = w.drawing().clone();
    let id = original.objects[0].id;
    w.key(Key::N, Modifiers::NONE);
    w.click_board([40., 16.], PointerButton::Primary, Modifiers::NONE);
    assert_eq!(w.drawing(), &original);
    w.click_board([40., 16.], PointerButton::Primary, Modifiers::NONE);
    let split = w.drawing().clone();
    assert_eq!(split.objects[0].shape.contours()[0].len(), 5);
    assert_eq!(w.draft.selection, [id]);
    assert_eq!(crate::path_editor::selected_range(&w.ctx, id, 0), (0, 1, 1));
    assert_eq!(w.project, outer);
    let cmd = Modifiers::COMMAND | Modifiers::CTRL;
    w.key(Key::Z, cmd);
    assert_eq!(w.drawing(), &original);
    assert!(!w.draft.history.can_undo());
    w.key(Key::Z, cmd | Modifiers::SHIFT);
    w.click_label("Apply to instances");
    assert_eq!(
        *w.project.symbols[&w.draft.symbol_id().unwrap()].artwork,
        split
    );
}

#[test]
fn draft_shortcuts_cut_and_undo_locally_until_apply() {
    let mut w = Window::new();
    let outer = w.project.clone();
    let original = w.drawing().clone();
    let ids: Vec<_> = original.objects.iter().map(|o| o.id).collect();
    let mut once = original.clone();
    once.boolean(&ids[..2], 0, BooleanOp::Difference).unwrap();
    let mut twice = once.clone();
    twice
        .boolean(&[ids[0], ids[2]], 0, BooleanOp::Difference)
        .unwrap();
    let cmd = Modifiers::COMMAND | Modifiers::CTRL;
    let zoom = w.ctx.zoom_factor();
    w.click_board([20., 23.], PointerButton::Primary, Modifiers::NONE);
    w.click_board([34., 44.], PointerButton::Primary, Modifiers::SHIFT);
    w.key(Key::Minus, cmd);
    assert_eq!(w.drawing(), &once);
    assert_eq!(w.draft.selection, [ids[0]]);
    w.click_board([70., 60.], PointerButton::Primary, Modifiers::SHIFT);
    w.key(Key::Minus, cmd);
    assert_eq!(w.drawing(), &twice);
    assert_eq!(w.ctx.zoom_factor(), zoom);
    for expected in [&once, &original] {
        w.key(Key::Z, cmd);
        assert_eq!(w.drawing(), expected);
    }
    assert!(!w.draft.history.can_undo());
    for expected in [&once, &twice] {
        w.key(Key::Z, cmd | Modifiers::SHIFT);
        assert_eq!(w.drawing(), expected);
    }
    assert_eq!(w.project, outer);
    w.click_label("Apply to instances");
    assert!(matches!(w.finished, Some(Some(_))));
    assert_eq!(
        *w.project.symbols[&w.draft.symbol_id().unwrap()].artwork,
        twice
    );
    assert_eq!(w.project.symbol_usage(w.draft.symbol_id().unwrap()), 1);
}

#[test]
fn pen_keeps_its_draft_paint_and_active_contour_and_cancel_discards_both_paths() {
    let mut w = Window::new();
    let outer = w.project.clone();
    let original = w.drawing().clone();
    w.draft.selection = original.objects[..2].iter().map(|o| o.id).collect();
    w.key(Key::B, Modifiers::NONE);
    let fill = [41, 202, 79, 255];
    let stroke = [209, 48, 72, 255];
    w.draft.style.fill = fill;
    w.draft.style.stroke = stroke;
    w.draft.style.stroke_width = 3.5;
    for p in [[18., 92.], [36., 105.], [47., 90.]] {
        w.click_board(p, PointerButton::Primary, Modifiers::NONE);
    }
    w.key(
        Key::Num8,
        Modifiers::COMMAND | Modifiers::CTRL | Modifiers::SHIFT,
    );
    assert_eq!(w.drawing(), &original);
    w.click_board([18., 92.], PointerButton::Secondary, Modifiers::NONE);
    let first = w.drawing().clone();
    let path = first.objects.last().unwrap();
    assert!(w.draft.tool == Tool::Pen);
    assert!(matches!(&path.shape, Shape::Path {nodes, closed:false} if nodes.len()==3));
    assert_eq!(
        (path.fill, path.stroke, path.stroke_width),
        (fill, stroke, 3.5)
    );
    let row = w.control(|label| label == "Fill");
    let swatch = w
        .frame(Modifiers::NONE, vec![])
        .shapes
        .into_iter()
        .find_map(|s| {
            if let egui::Shape::Rect(r) = s.shape {
                let at = r.rect.center();
                if r.fill == egui::Color32::from_rgb(fill[0], fill[1], fill[2])
                    && (at.y - row.y).abs() < 4.
                    && at.x > row.x
                    && at.x < row.x + 130.
                {
                    return Some(at);
                }
            }
            None
        })
        .expect("visible fill swatch");
    w.click(swatch, PointerButton::Primary, Modifiers::NONE);
    w.click_label("Sliders");
    w.control(|label| label == "📋");
    w.click_label("Wheel");
    w.click(swatch, PointerButton::Primary, Modifiers::NONE);
    assert_eq!(w.drawing(), &first);
    w.click(swatch, PointerButton::Secondary, Modifiers::NONE);
    w.click_label("None");
    let mut repainted = first.clone();
    repainted.objects.last_mut().unwrap().no_fill = true;
    assert_eq!(w.drawing(), &repainted);
    assert!(w.draft.style.no_fill);
    assert_eq!(w.draft.style.fill, fill);
    w.key(Key::S, Modifiers::NONE);
    w.click_board([20., 23.], PointerButton::Primary, Modifiers::NONE);
    w.key(Key::B, Modifiers::NONE);
    for p in [[56., 93.], [72., 107.], [85., 89.], [56., 93.]] {
        w.click_board(p, PointerButton::Primary, Modifiers::NONE);
    }
    let path = w.drawing().objects.last().unwrap();
    assert!(matches!(&path.shape, Shape::Path {nodes, closed:true} if nodes.len()==3));
    assert!(path.no_fill && !path.no_stroke);
    assert_eq!(
        (path.fill, path.stroke, path.stroke_width),
        (fill, stroke, 3.5)
    );
    w.key(Key::Z, Modifiers::COMMAND | Modifiers::CTRL);
    assert_eq!(w.drawing(), &repainted);
    w.key(Key::Z, Modifiers::COMMAND | Modifiers::CTRL);
    assert_eq!(w.drawing(), &first);
    w.key(Key::Z, Modifiers::COMMAND | Modifiers::CTRL);
    assert_eq!(w.drawing(), &original);
    w.key(
        Key::Z,
        Modifiers::COMMAND | Modifiers::CTRL | Modifiers::SHIFT,
    );
    assert_eq!(w.drawing(), &first);
    w.click_label("Cancel");
    assert_eq!(w.finished, Some(None));
    assert_eq!(w.project, outer);
}
