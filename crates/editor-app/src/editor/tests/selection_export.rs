use super::*;

#[test]
fn export_menu_keeps_the_artboard_and_selection_actions_visible_at_both_window_sizes() {
    for size in [egui::vec2(800., 500.), egui::vec2(1100., 740.)] {
        let mut w = EditorWindow::new(size);
        w.ctx.style_mut(|style| style.animation_time = 0.);
        let id = w.editor.project.clips[0].drawing_at_mut(0).add_rectangle(
            [-12., 8.],
            [8., 4.],
            [255; 4],
        );
        w.editor.finish_edit();
        let drawing = w.editor.project.clone();
        w.editor.selection = vec![id];
        let board = w.board();
        w.click("Export");
        for label in [
            "Current frame",
            "Artboard",
            "Selection",
            "Batch selected objects",
            "Animation",
            "Animated selection…",
            "Export clip…",
            "Export animation set…",
        ] {
            w.control(label);
        }
        for target in ["Artboard", "Selection", "Batch selected objects"] {
            w.click(target);
            for format in ["PNG…", "SVG…", "PNG and SVG…"] {
                w.control(format);
            }
        }
        assert_eq!(w.board(), board);
        assert_eq!(w.editor.project, drawing);
        assert_eq!(w.editor.selection, [id]);
        assert_ne!(w.editor.saved.as_ref(), Some(&drawing));
    }
}

#[test]
fn animated_selection_dialog_uses_the_timeline_frame_and_closes_on_clip_switch() {
    for size in [egui::vec2(800., 500.), egui::vec2(1100., 740.)] {
        let mut w = EditorWindow::new(size);
        w.ctx.style_mut(|style| style.animation_time = 0.);
        let id = w.editor.project.clips[0].drawing_at_mut(0).add_rectangle(
            [-12., 8.],
            [8., 4.],
            [255; 4],
        );
        let other = w.editor.project.add_clip("Other").unwrap();
        w.editor.finish_edit();
        w.editor.timeline.frame = 2;
        w.editor.selection = vec![id];
        let drawing = w.editor.project.clone();
        w.board();
        w.click("Export");
        w.click("Animated selection…");
        let frames = drawing.clips[0].frames;
        w.control(&format!(
            "Captured 1 selected object on frame 3 of {frames}."
        ));
        for label in [
            "First frame",
            "Last frame",
            "Scale",
            "Columns",
            "Export to new folder…",
        ] {
            w.control(label);
        }
        let window = w
            .ctx
            .memory(|m| m.area_rect(egui::Id::new("Animated selection")))
            .unwrap();
        assert!(
            window.center().distance(egui::Pos2::ZERO + size / 2.) < 2.,
            "{window:?}"
        );
        assert!(
            (window.bottom() - w.control("Export to new folder…").y) < 48.,
            "{window:?}"
        );
        w.click("PNG sequence");
        assert!(w.editor.exports.is_open());
        assert!(w
            .frame(Vec::new())
            .shapes
            .iter()
            .all(|s| !matches!(&s.shape, egui::Shape::Text(t) if t.galley.text() == "Columns")));
        w.click("Both");
        w.control("Columns");
        w.click("Sprite sheet");
        w.control("Columns");
        assert_eq!(w.editor.project, drawing);
        assert_eq!(w.editor.selection, [id]);
        w.editor.timeline.clip = other;
        w.frame(Vec::new());
        assert!(!w.editor.exports.is_open());
    }
}

#[test]
fn artboard_png_resolution_linked_fields_update_the_export_size_without_changing_the_document() {
    let mut w = EditorWindow::with_artboard(egui::vec2(800., 500.), [1000, 500]);
    w.ctx.style_mut(|s| s.animation_time = 0.);
    let initial = w.editor.project.clone();
    w.board();
    w.click("Export");
    w.click("Artboard");
    w.click("PNG and SVG…");
    w.control("Artboard · 1000 × 500 px");
    let field = w.field_on_row("Width", " px");
    w.click_at(field);
    w.frame(vec![
        egui::Event::Text("256".into()),
        egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        },
    ]);
    w.control("Artboard · 256 × 128 px");
    w.control("SVG keeps its size in document units.");
    w.click("100%");
    w.control("Artboard · 1000 × 500 px");
    w.click("Cancel");
    assert!(!w.editor.exports.is_open());
    assert_eq!(w.editor.project, initial);
}
