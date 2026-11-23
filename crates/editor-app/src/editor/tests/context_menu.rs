use super::*;

#[test]
fn right_click_targets_artwork_keeps_multiselections_and_reuses_edit_commands() {
    let mut doc = Document {
        width: 128,
        height: 128,
        ..Document::default()
    };
    let left = doc.add_rectangle([18., 20.], [24., 32.], [255; 4]);
    let right = doc.add_rectangle([78., 20.], [24., 32.], [180; 4]);
    doc.objects[0].name = "Lamp".into();
    let mut w = EditorWindow::new(egui::vec2(1100., 740.));
    w.ctx.style_mut(|s| s.animation_time = 0.);
    w.editor
        .replace_project(AnimationProject::from_document(doc), None);
    let board = w.board();
    let at = board.min + board.size() * egui::vec2(30. / 128., 36. / 128.);
    w.editor.selection = vec![right];
    pen::right_click(&mut w, at);
    assert_eq!(w.editor.selection, [left]);
    for label in [
        "Copy",
        "Paste style",
        "Duplicate",
        "Group",
        "Ungroup",
        "Convert to symbol",
        "Set Clip",
        "Release Clip",
    ] {
        w.control(label);
    }
    w.click("Copy");
    assert!(!w.editor.clipboard.is_empty());
    pen::right_click(&mut w, at);
    w.click("Duplicate");
    assert_eq!(w.editor.scene().objects.len(), 3);
    w.editor.restore(false);
    w.editor.selection = vec![left, right];
    pen::right_click(&mut w, at);
    w.click("Group");
    assert!(w.editor.scene().objects.iter().all(|o| o.group.is_some()));
    pen::right_click(&mut w, at);
    w.click("Ungroup");
    assert!(w.editor.scene().objects.iter().all(|o| o.group.is_none()));
    w.editor.selection.clear();
    pen::right_click(&mut w, at);
    w.click("Convert to symbol");
    assert_eq!(
        w.editor.project.symbols.values().next().unwrap().name,
        "Lamp"
    );
    assert_eq!(w.editor.scene().objects.len(), 2);
    w.editor.restore(false);
    assert!(w.editor.project.symbols.is_empty());
}

#[test]
fn conversion_shortcuts_and_path_menu_share_undoable_edits() {
    use egui::Modifiers as M;
    let mut doc = Document::default();
    let id = doc.add_rectangle([40., 30.], [34., 28.], [90, 40, 200, 255]);
    doc.objects[0].name = "Lantern".into();
    doc.objects[0].stroke = [10, 20, 30, 255];
    doc.objects[0].stroke_width = 8.;
    let mut w = EditorWindow::new(egui::vec2(1100., 740.));
    w.editor
        .replace_project(AnimationProject::from_document(doc.clone()), None);
    w.board();

    w.editor.selection = vec![id];
    let native_copy = |w: &mut EditorWindow, mods| {
        w.frame_with_input(egui::RawInput {
            events: vec![egui::Event::Copy],
            modifiers: M::COMMAND | M::CTRL | mods,
            ..Default::default()
        });
    };
    native_copy(&mut w, M::SHIFT);
    assert!(matches!(
        w.editor.scene().objects[0].shape,
        document_core::Shape::Path { .. }
    ));
    w.editor.restore(false);
    assert_eq!(w.editor.scene(), &doc);
    assert!(!w.editor.can_undo());

    for via_menu in [false, true] {
        w.editor.selection = vec![id];
        if via_menu {
            w.click("Path");
            w.click("Stroke to path");
        } else {
            native_copy(&mut w, M::ALT);
        }
        let scene = w.editor.scene();
        assert_eq!(scene.objects.len(), 2);
        let [fill, outline] = &scene.objects[..] else {
            unreachable!()
        };
        assert_eq!(fill.id, id);
        assert_ne!(outline.id, id);
        assert!(fill.no_stroke && outline.no_stroke);
        assert_eq!(outline.fill, doc.objects[0].stroke);
        assert_eq!(outline.shape.contours().len(), 2);
        assert_eq!(w.editor.selection, [fill.id, outline.id]);
        assert_eq!(
            renderer::render(scene, 0).unwrap(),
            renderer::render(&doc, 0).unwrap()
        );
        w.editor.restore(false);
        assert_eq!(w.editor.scene(), &doc);
        assert!(!w.editor.can_undo());
    }

    w.editor.selection = vec![id];
    pen::chord(&mut w, Key::Y, M::COMMAND | M::CTRL | M::SHIFT);
    assert_eq!(
        w.editor.project.symbols.values().next().unwrap().name,
        "Lantern"
    );
    assert!(w.editor.scene().objects[0].is_instance());
    w.editor.restore(false);
    assert!(w.editor.project.symbols.is_empty());
    assert_eq!(w.editor.scene(), &doc);
}

fn click_convert_in_canvas_menu(w: &mut EditorWindow) {
    let at = w.control("Convert to path");
    // Text and primitive inspectors can offer the same command. Click the
    // actual floating menu, even when a dock contains a matching label.
    assert_eq!(
        w.ctx.layer_id_at(at).map(|l| l.order),
        Some(egui::Order::Foreground)
    );
    w.click_at(at);
}

#[test]
fn right_clicking_an_unselected_circle_converts_only_that_object_and_undo_restores_it() {
    let mut doc = Document {
        width: 128,
        height: 128,
        ..Default::default()
    };
    let rect = doc.add_rectangle([10., 18.], [24., 30.], [35, 50, 60, 255]);
    let circle = doc.add_shape(
        [72., 18.],
        [30., 30.],
        [32, 164, 99, 255],
        document_core::Shape::Ellipse,
        1,
    );
    let object = &mut doc.objects[1];
    object.name = "Wheel".into();
    object.stroke_width = 3.5;
    let before = AnimationProject::from_document(doc);
    let mut after = before.clone();
    let object = &mut after.clips[0].drawing_at_mut(0).objects[1];
    object.shape = document_core::path_edit::primitive_path(&object.shape).unwrap();
    let mut w = EditorWindow::new(egui::vec2(800., 500.));
    w.ctx.style_mut(|s| s.animation_time = 0.);
    w.editor.replace_project(before.clone(), None);
    let board = w.board();
    w.editor.selection = vec![rect];
    pen::right_click(
        &mut w,
        board.min + board.size() * egui::vec2(87. / 128., 33. / 128.),
    );
    assert_eq!(w.editor.selection, [circle]);
    click_convert_in_canvas_menu(&mut w);
    assert_eq!(w.editor.project, after);
    assert_eq!(w.editor.selection, [circle]);
    assert!(w.editor.tool == Tool::Node);
    w.click_edit("Undo");
    assert_eq!(w.editor.project, before);
    assert!(!w.editor.can_undo());
    w.click_edit("Redo");
    assert_eq!(w.editor.project, after);
}

#[test]
fn context_conversion_outlines_mixed_artwork_once_and_is_disabled_on_an_existing_path() {
    let mut doc = Document {
        width: 128,
        height: 128,
        ..Default::default()
    };
    let rect = doc.add_rectangle([12., 14.], [20., 25.], [255; 4]);
    let path = doc.add_shape(
        [72., 14.],
        [25., 25.],
        [148, 60, 44, 255],
        document_core::path_edit::primitive_path(&document_core::Shape::Ellipse).unwrap(),
        1,
    );
    let text = document_core::typography::TextStyle::default()
        .insert(&mut doc, 1, [10., 55.], [255; 4])
        .unwrap();
    let before = AnimationProject::from_document(doc);
    let mut after = before.clone();
    let converted = &mut after.clips[0].drawing_at_mut(0).objects;
    converted[0].shape = document_core::path_edit::primitive_path(&converted[0].shape).unwrap();
    converted[2].text = None;
    let mut w = EditorWindow::new(egui::vec2(1100., 740.));
    w.ctx.style_mut(|s| s.animation_time = 0.);
    w.editor.replace_project(before.clone(), None);
    let board = w.board();
    w.editor.selection = vec![rect, path, text];
    pen::right_click(
        &mut w,
        board.min + board.size() * egui::vec2(22. / 128., 25. / 128.),
    );
    click_convert_in_canvas_menu(&mut w);
    assert_eq!(w.editor.selection, [rect, path, text]);
    assert_eq!(w.editor.project, after);
    w.click_edit("Undo");
    assert_eq!(w.editor.project, before);
    assert!(!w.editor.can_undo());

    w.editor.set_tool(Tool::Select);
    w.editor.selection.clear();
    pen::right_click(
        &mut w,
        board.min + board.size() * egui::vec2(84. / 128., 26. / 128.),
    );
    assert_eq!(w.editor.selection, [path]);
    click_convert_in_canvas_menu(&mut w);
    assert!(w.editor.artwork_menu_open);
    assert!(w.editor.tool == Tool::Select);
    assert_eq!(w.editor.project, before);
    assert!(!w.editor.can_undo());
}
