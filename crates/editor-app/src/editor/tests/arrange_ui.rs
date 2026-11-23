use super::*;

#[test]
fn flip_shortcuts_mirror_the_selected_group_about_its_shared_center_in_one_edit() {
    use document_core::{PathNode, Shape};
    let mut drawing = Document::default();
    let block = drawing.add_rectangle([10., 10.], [14., 12.], [32, 77, 220, 255]);
    let arrow = drawing.add_shape(
        [42., 30.],
        [36., 20.],
        [237, 103, 39, 255],
        Shape::Path {
            nodes: [[0., 0.], [1., 0.], [0., 1.]]
                .map(PathNode::corner)
                .to_vec(),
            closed: true,
        },
        1,
    );
    commands::group(&mut drawing, &[block, arrow]);
    drawing.add_rectangle([99., 91.], [12., 12.], [61, 195, 112, 255]);
    let original = renderer::render(&drawing, 0).unwrap();
    for (key, label) in [(Key::H, "Flip horizontally"), (Key::V, "Flip vertically")] {
        let mut w = EditorWindow::new(egui::vec2(1100., 740.));
        w.ctx.style_mut(|style| style.animation_time = 0.);
        w.editor
            .replace_project(AnimationProject::from_document(drawing.clone()), None);
        w.board();
        w.click("Select");
        let board = w.board();
        w.click_at(board.min + board.size() * (egui::vec2(16., 16.) / 128.));
        assert_eq!(w.editor.selection, [block, arrow]);

        pen::chord(&mut w, key, egui::Modifiers::SHIFT);
        assert_eq!(w.editor.scene(), &drawing);
        for (pressed, repeat) in [(true, false), (true, true), (false, false)] {
            w.frame(vec![egui::Event::Key {
                key,
                physical_key: Some(key),
                pressed,
                repeat,
                modifiers: egui::Modifiers::NONE,
            }]);
        }
        let flipped = w.editor.scene().clone();
        assert_eq!(w.editor.selection, [block, arrow]);
        assert_eq!(flipped.objects[2], drawing.objects[2]);
        let image = renderer::render(&flipped, 0).unwrap();
        // The selected bounds are [10, 10]–[78, 50]. Pixel centers lie at x+½, y+½.
        for (x, y) in [(16, 16), (45, 32), (65, 32), (45, 45), (70, 45), (26, 25)] {
            let (rx, ry) = if key == Key::H {
                (87 - x, y)
            } else {
                (x, 59 - y)
            };
            assert_eq!(
                image.pixel(rx, ry),
                original.pixel(x, y),
                "{key:?} at ({x}, {y})"
            );
        }
        w.click_edit("Undo");
        assert_eq!(w.editor.scene(), &drawing);
        assert!(!w.editor.can_undo());
        w.click_edit("Redo");
        assert_eq!(w.editor.scene(), &flipped);
        w.click_edit("Undo");
        w.click("Object");
        let submenu = w.control("Transform");
        w.frame(vec![egui::Event::PointerMoved(submenu)]);
        w.frame(vec![]);
        w.click(label);
        assert_eq!(w.editor.scene(), &flipped);
    }
}

#[test]
fn flip_during_axis_locked_move_commits_as_one_edit() {
    let mut drawing = Document::default();
    let id = drawing.add_rectangle([20., 30.], [36., 24.], [255; 4]);
    let mut w = EditorWindow::new(egui::vec2(1100., 740.));
    w.editor
        .replace_project(AnimationProject::from_document(drawing.clone()), None);
    let board = w.board();
    let scale = board.width() / 128.;
    let start = board.min + egui::vec2(30., 39.) * scale;
    let end = start + egui::vec2(25., 14.) * scale;
    let button = |pos, pressed, modifiers| egui::Event::PointerButton {
        pos,
        pressed,
        modifiers,
        button: egui::PointerButton::Primary,
    };
    w.frame(vec![egui::Event::PointerMoved(start)]);
    w.frame(vec![button(start, true, egui::Modifiers::NONE)]);
    assert_eq!(w.editor.selection, [id]);
    w.frame(vec![egui::Event::PointerMoved(end)]);
    pen::chord(&mut w, Key::H, egui::Modifiers::NONE);
    assert_eq!(w.editor.scene(), &drawing);
    // The button event takes precedence when modifiers change in the release batch.
    w.frame(vec![button(end, false, egui::Modifiers::CTRL)]);
    let object = &w.editor.scene().objects[0];
    assert!((object.position[0] - 45.).abs() < 0.001);
    assert_eq!(object.position[1], 30.);
    assert_eq!(object.scale, [-1., 1.]);
    w.click_edit("Undo");
    assert_eq!(w.editor.scene(), &drawing);
    assert!(!w.editor.can_undo());
}

fn choose(window: &mut EditorWindow, action: &str) {
    window.click("Object");
    let submenu = window.control("Align and Distribute");
    window.frame(vec![egui::Event::PointerMoved(submenu)]);
    window.frame(Vec::new());
    window.click(action);
}

#[test]
fn object_menu_aligns_to_biggest_and_distributes_gaps_with_one_undo() {
    let mut doc = Document::default();
    let small = doc.add_rectangle([4., 16.], [8., 6.], [80, 160, 210, 255]);
    let big = doc.add_rectangle([40., 44.], [36., 40.], [55, 125, 90, 255]);
    let other = doc.add_rectangle([90., 98.], [14., 12.], [210, 115, 65, 255]);
    let ids = [other, big, small];
    for size in [egui::vec2(800., 500.), egui::vec2(1100., 740.)] {
        let mut w = EditorWindow::new(size);
        w.ctx.style_mut(|s| s.animation_time = 0.);
        w.editor
            .replace_project(AnimationProject::from_document(doc.clone()), None);
        w.frame(Vec::new());
        w.frame(Vec::new());
        let board = w.board();
        for (action, positions) in [
            ("Left", [[40., 16.], [40., 44.], [40., 98.]]),
            ("Center X", [[54., 16.], [40., 44.], [51., 98.]]),
            ("Right", [[68., 16.], [40., 44.], [62., 98.]]),
            ("Top", [[4., 44.], [40., 44.], [90., 44.]]),
            ("Center Y", [[4., 61.], [40., 44.], [90., 58.]]),
            ("Bottom", [[4., 78.], [40., 44.], [90., 72.]]),
            ("Horizontal gaps", [[4., 16.], [33., 44.], [90., 98.]]),
            ("Vertical gaps", [[4., 16.], [40., 40.], [90., 98.]]),
        ] {
            w.editor.selection = ids.to_vec();
            choose(&mut w, action);
            assert_eq!(w.editor.selection, ids);
            let result: Vec<_> = w
                .editor
                .scene()
                .objects
                .iter()
                .map(|o| o.position_at(0))
                .collect();
            assert_eq!(result, positions, "{action}, window {size:?}");
            assert_eq!(w.board(), board);
            choose(&mut w, action);
            let status = if action.ends_with("gaps") {
                "The gaps are already equal."
            } else {
                "The selection is already aligned."
            };
            assert_eq!(w.editor.status, status);
            w.click_edit("Undo");
            assert_eq!(w.editor.scene(), &doc);
            assert!(!w.editor.can_undo());
        }
    }
}

#[test]
fn object_menu_disables_alignment_and_gap_distribution_below_their_unit_counts() {
    let mut doc = Document::default();
    let a = doc.add_rectangle([2., 4.], [10., 20.], [170; 4]);
    let b = doc.add_rectangle([22., 34.], [10., 20.], [180; 4]);
    let c = doc.add_rectangle([56., 54.], [10., 20.], [190; 4]);
    commands::group(&mut doc, &[a, b]);
    for (ids, action) in [(vec![a, b], "Left"), (vec![a, b, c], "Horizontal gaps")] {
        let mut w = EditorWindow::new(egui::vec2(800., 500.));
        w.ctx.style_mut(|s| s.animation_time = 0.);
        w.editor
            .replace_project(AnimationProject::from_document(doc.clone()), None);
        w.editor.selection = ids.clone();
        w.board();
        choose(&mut w, action);
        assert_eq!(w.editor.scene(), &doc);
        assert_eq!(w.editor.selection, ids);
        assert!(!w.editor.can_undo());
    }
}

#[test]
fn inspector_switches_between_selection_bounds_and_the_artboard() {
    let mut original = Document::default();
    let small = original.add_rectangle([6., 16.], [12., 10.], [220; 4]);
    let big = original.add_rectangle([54., 27.], [45., 35.], [180; 4]);
    let mut view = EditorWindow::new(egui::vec2(1100., 1200.));
    view.ctx.style_mut(|s| s.animation_time = 0.);
    view.editor
        .replace_project(AnimationProject::from_document(original.clone()), None);
    view.editor.selection = vec![small, big];
    view.board();
    view.click("Align and Distribute");
    view.click("Biggest selected");
    view.click("Selection bounds");
    view.click("Left");
    assert_eq!(view.editor.scene().objects[0], original.objects[0]);
    assert_eq!(view.editor.scene().objects[1].position, [6., 27.]);
    view.click_edit("Undo");
    assert_eq!(view.editor.scene(), &original);

    view.editor.selection = vec![big];
    view.control("Artboard");
    view.click("Left");
    assert_eq!(view.editor.scene().objects[1].position, [0., 27.]);
    view.click_edit("Undo");
    assert_eq!(view.editor.scene(), &original);
    assert!(!view.editor.can_undo());
}

#[test]
fn object_menu_and_inspector_share_the_chosen_alignment_reference() {
    let mut original = Document::default();
    let tall = original.add_rectangle([8., 5.], [6., 85.], [220; 4]);
    let wide = original.add_rectangle([40., 97.], [78., 4.], [180; 4]);
    let square = original.add_rectangle([62., 15.], [42., 42.], [140; 4]);
    let mut view = EditorWindow::new(egui::vec2(1100., 920.));
    view.ctx.style_mut(|s| s.animation_time = 0.);
    view.editor
        .replace_project(AnimationProject::from_document(original.clone()), None);
    view.editor.selection = vec![square, wide, tall];
    view.board();

    choose(&mut view, "Left");
    assert!(view
        .editor
        .scene()
        .objects
        .iter()
        .all(|o| o.position[0] == 8.));
    view.click_edit("Undo");
    view.click("Align and Distribute");
    view.click("Biggest selected");
    view.click("Largest area");
    view.click("Align and Distribute");
    choose(&mut view, "Left");
    assert!(view
        .editor
        .scene()
        .objects
        .iter()
        .all(|o| o.position[0] == 62.));
    view.click_edit("Undo");
    view.click("Object");
    let submenu = view.control("Align and Distribute");
    view.frame(vec![egui::Event::PointerMoved(submenu)]);
    view.frame(vec![]);
    view.click("Largest area");
    view.click("Biggest selected");
    // A click on the toolbar also closes any menus left open by the picker.
    view.click("Select");
    view.click("Align and Distribute");
    view.click("Top");
    assert!(view
        .editor
        .scene()
        .objects
        .iter()
        .all(|o| o.position[1] == 97.));
    view.click_edit("Undo");
    assert_eq!(view.editor.scene(), &original);
    assert!(!view.editor.can_undo());
}
