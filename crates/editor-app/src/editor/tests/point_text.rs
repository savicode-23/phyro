use super::*;
use document_core::typography::{TextAlign, TextStyle, Typeface};
use egui::{pos2, vec2, CursorIcon, Event, Modifiers, Pos2, Rect};

fn window(doc: Document) -> EditorWindow {
    let mut w = EditorWindow::new(vec2(1100., 740.));
    w.ctx.style_mut(|style| style.animation_time = 0.);
    w.editor
        .replace_project(AnimationProject::from_document(doc), None);
    w.board();
    w
}

fn screen(board: Rect, x: f32, y: f32) -> Pos2 {
    board.min + board.size() * vec2(x / 128., y / 128.)
}

fn press(w: &mut EditorWindow, key: Key, modifiers: Modifiers, typed: Option<&str>) {
    for pressed in [true, false] {
        let mut events = vec![Event::Key {
            key,
            physical_key: Some(key),
            pressed,
            repeat: false,
            modifiers,
        }];
        if pressed {
            events.extend(typed.map(|text| Event::Text(text.into())));
        }
        w.frame_with_input(egui::RawInput {
            modifiers,
            events,
            ..Default::default()
        });
    }
}

fn command(w: &mut EditorWindow, key: Key) {
    press(w, key, Modifiers::CTRL | Modifiers::COMMAND, None);
}

#[test]
fn text_shortcuts_and_toolbar_place_multiline_editable_lettering() {
    let mut w = window(Document::default());
    let board = w.board();
    let anchor = screen(board, -8., 35.);
    for key in [Key::F8, Key::T] {
        press(&mut w, key, Modifiers::NONE, None);
        assert!(w.editor.tool == Tool::Text && w.editor.text_dialog.is_none());
        let output = w.frame(vec![Event::PointerMoved(anchor)]);
        assert_eq!(output.platform_output.cursor_icon, CursorIcon::Text);
        press(&mut w, Key::S, Modifiers::NONE, None);
        assert!(w.editor.tool == Tool::Select);
    }
    w.click("Text");
    assert!(w.editor.tool == Tool::Text);
    let fill = [42, 124, 209, 255];
    w.editor.style.fill = fill;
    w.editor.style.no_stroke = true;
    w.editor.style.cap = document_core::LineCap::Square;
    w.editor.style.join = document_core::LineJoin::Bevel;
    w.click_at(anchor);
    assert!(w.editor.text_dialog.is_none());
    assert!(
        w.editor.canvas.text_active(),
        "No active lettering session after click"
    );
    assert!(
        w.ctx.wants_keyboard_input(),
        "Canvas click did not give the lettering editor keyboard focus"
    );
    for (key, letter) in [(Key::B, "b"), (Key::S, "s"), (Key::T, "t")] {
        press(&mut w, key, Modifiers::NONE, Some(letter));
        assert!(w.editor.tool == Tool::Text);
    }
    press(&mut w, Key::Enter, Modifiers::NONE, None);
    w.frame(vec![Event::Text("Line two".into())]);
    command(&mut w, Key::Enter);
    assert_eq!(w.editor.scene().objects.len(), 1);
    let o = &w.editor.scene().objects[0];
    assert_eq!(o.fill, fill);
    assert!(!o.no_fill && o.no_stroke);
    assert_eq!(o.cap, document_core::LineCap::Square);
    assert_eq!(o.join, document_core::LineJoin::Bevel);
    assert_eq!(o.text.as_ref().unwrap().content, "bst\nLine two");
    assert_eq!(w.editor.selection, [o.id]);
    assert!(
        Pos2::from(o.position).distance(pos2(-8., 35.)) < 0.005,
        "{:?}",
        o.position
    );

    let project = w.editor.project.clone();
    let dir = tempfile::tempdir().unwrap();
    let native = dir.path().join("Lettering.vanim");
    project_io::save_animation(&native, &project, false).unwrap();
    let reopened = project_io::load_animation(&native).unwrap();
    assert_eq!(reopened, project);
    let doc = reopened.clips[0].drawing_at(0);
    let image = renderer::render(doc, 0).unwrap();
    let alpha_rows = |from, to| {
        (from..to).any(|y| (0..image.width()).any(|x| image.pixel(x, y).unwrap().alpha() > 100))
    };
    assert!(alpha_rows(35, 62) && alpha_rows(62, 95));
    let png = dir.path().join("Lettering.png");
    project_io::export_frame(&png, doc, 0).unwrap();
    assert_eq!(std::fs::read(png).unwrap(), image.encode_png().unwrap());
    command(&mut w, Key::Z);
    assert!(w.editor.scene().objects.is_empty() && !w.editor.can_undo());
    command(&mut w, Key::Y);
    assert_eq!(w.editor.project, project);
}

#[test]
fn edit_existing_lettering_by_click_keeps_style_transform_and_one_history_step() {
    let mut doc = Document::default();
    let typography = TextStyle {
        content: "OLD".into(),
        face: Typeface::HackRegular,
        size: 13.,
        letter_spacing: 1.5,
        line_height: 1.4,
        align: TextAlign::Right,
    };
    let id = typography
        .clone()
        .insert(&mut doc, 1, [32., 41.], [36, 128, 219, 235])
        .unwrap();
    let o = &mut doc.objects[0];
    o.rotation = 12.;
    o.scale = [1.17, 0.87];
    o.stroke = [48, 24, 79, 245];
    o.stroke_width = 0.7;
    // Pick a real painted glyph in the transformed source, independently of its layout box.
    let hit = (30..75)
        .flat_map(|x| (36..70).map(move |y| [x as f32 + 0.5, y as f32 + 0.5]))
        .find(|&p| doc.hit_test(p, 0) == Some(id))
        .unwrap();
    let mut w = window(doc.clone());
    let board = w.board();
    press(&mut w, Key::F8, Modifiers::NONE, None);
    w.click_at(screen(board, hit[0], hit[1]));
    command(&mut w, Key::A);
    w.frame(vec![Event::Text("Abandoned".into())]);
    press(&mut w, Key::Escape, Modifiers::NONE, None);
    assert_eq!(w.editor.scene(), &doc);
    assert!(!w.editor.can_undo());
    w.click("Text");
    w.click_at(screen(board, hit[0], hit[1]));
    command(&mut w, Key::A);
    for (key, typed) in [(Key::H, "h"), (Key::V, "v")] {
        press(&mut w, key, Modifiers::NONE, Some(typed));
    }
    command(&mut w, Key::Enter);
    let mut expected = doc.clone();
    let changed = TextStyle {
        content: "hv".into(),
        ..typography
    };
    changed.apply(&mut expected.objects[0]).unwrap();
    assert_eq!(w.editor.scene(), &expected);
    assert_eq!(w.editor.selection, [id]);
    command(&mut w, Key::Z);
    assert_eq!(w.editor.scene(), &doc);
    assert!(!w.editor.can_undo());
    command(&mut w, Key::Y);
    assert_eq!(w.editor.scene(), &expected);
    assert_eq!(w.editor.selection, [id]);
}

#[test]
fn escape_abandons_text_and_a_second_canvas_click_commits_the_first_label() {
    let mut w = window(Document::default());
    let board = w.board();
    w.click("Text");
    w.click_at(screen(board, 32., 24.));
    w.frame(vec![Event::Text("Discard this".into())]);
    press(&mut w, Key::Escape, Modifiers::NONE, None);
    assert!(w.editor.scene().objects.is_empty() && !w.editor.can_undo());
    w.click("Text");
    w.click_at(screen(board, 20., 40.));
    w.frame(vec![Event::Text("Keep".into())]);
    w.click_at(screen(board, 107., 104.));
    assert_eq!(w.editor.scene().objects.len(), 1);
    assert_eq!(
        w.editor.scene().objects[0].text.as_ref().unwrap().content,
        "Keep"
    );
    press(&mut w, Key::Escape, Modifiers::NONE, None);
    assert_eq!(w.editor.scene().objects.len(), 1);
    command(&mut w, Key::Z);
    assert!(w.editor.scene().objects.is_empty() && !w.editor.can_undo());
}

#[test]
fn text_cannot_create_or_replace_lettering_on_a_locked_active_layer() {
    let mut doc = Document::default();
    TextStyle::default()
        .insert(&mut doc, 1, [24., 24.], [255; 4])
        .unwrap();
    doc.layers[0].locked = true;
    let mut w = window(doc.clone());
    let board = w.board();
    press(&mut w, Key::T, Modifiers::NONE, None);
    for (x, y) in [(30., 38.), (96., 80.)] {
        w.click_at(screen(board, x, y));
        w.frame(vec![Event::Text("Locked".into())]);
        command(&mut w, Key::Enter);
    }
    assert_eq!(w.editor.scene(), &doc);
    assert!(!w.editor.can_undo());
}

#[test]
fn clicking_inside_existing_lettering_places_the_caret_there() {
    let mut doc = Document::default();
    TextStyle {
        content: "ABCD".into(),
        face: Typeface::HackRegular,
        size: 16.,
        ..Default::default()
    }
    .insert(&mut doc, 1, [22., 24.], [32, 52, 90, 255])
    .unwrap();
    for (rotation, scale) in [(0., [1., 1.]), (82., [1.7, 0.8])] {
        doc.objects[0].rotation = rotation;
        doc.objects[0].scale = scale;
        let after_first = doc.objects[0].local_to_world([0.25, 0.45], 0);
        let mut w = window(doc.clone());
        let board = w.board();
        w.click("Text");
        w.click_at(screen(board, after_first[0], after_first[1]));
        w.frame(vec![Event::Text("x".into())]);
        command(&mut w, Key::Enter);
        assert_eq!(
            w.editor.scene().objects[0].text.as_ref().unwrap().content,
            "AxBCD"
        );
    }
}

#[test]
fn a_second_click_and_drag_select_characters_along_the_rotated_baseline() {
    let mut doc = Document::default();
    TextStyle {
        content: "ABCD".into(),
        face: Typeface::HackRegular,
        size: 16.,
        ..Default::default()
    }
    .insert(&mut doc, 1, [44., 39.], [22, 49, 80, 255])
    .unwrap();
    doc.objects[0].rotation = 58.;
    doc.objects[0].scale = [1.38, 0.85];
    doc.objects[0].animated.skew = 11.;
    let mut w = window(doc.clone());
    w.click("Text");
    let board = w.board();
    let at = |x| {
        let [x, y] = doc.objects[0].local_to_world([x, 0.55], 0);
        screen(board, x, y)
    };
    w.click_at(at(0.98));
    w.click_at(at(0.25));
    for _ in 0..30 {
        w.frame(vec![]);
    }
    w.drag(at(0.25), at(0.75) - at(0.25));
    w.frame(vec![Event::Text("x".into())]);
    command(&mut w, Key::Enter);
    assert_eq!(
        w.editor.scene().objects[0].text.as_ref().unwrap().content,
        "AxD"
    );
}

#[test]
fn save_while_typing_writes_the_current_label_and_keeps_one_undo() {
    let mut w = window(Document::default());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Current lettering.vanim");
    w.editor.path = Some(path.clone());
    let board = w.board();
    w.click("Text");
    w.click_at(screen(board, 13., 18.));
    w.frame(vec![Event::Text("Page one".into())]);
    command(&mut w, Key::S);
    let saved = project_io::load_animation(&path).unwrap();
    assert_eq!(saved, w.editor.project);
    assert_eq!(
        saved.clips[0].drawing_at(0).objects[0]
            .text
            .as_ref()
            .unwrap()
            .content,
        "Page one"
    );
    command(&mut w, Key::Z);
    assert!(w.editor.scene().objects.is_empty() && !w.editor.can_undo());
}

#[test]
fn live_lettering_matches_exported_transformed_outlines_and_stacking() {
    let mut doc = Document::default();
    doc.add_rectangle([0.; 2], [128.; 2], [71, 205, 144, 255]);
    let mut style = TextStyle {
        content: "Old".into(),
        size: 13.,
        ..Default::default()
    };
    let id = style
        .clone()
        .insert(&mut doc, 1, [29., 35.], [45, 72, 198, 170])
        .unwrap();
    let text = &mut doc.objects[1];
    text.size[0] *= 1.38;
    text.scale = [1.2, 0.76];
    text.rotation = 21.;
    text.animated.skew = -17.;
    text.stroke = [229, 56, 88, 210];
    text.stroke_width = 0.8;
    let hit = text.local_to_world([0.2, 0.6], 0);
    doc.add_rectangle([52., 40.], [16., 29.], [255, 229, 113, 180]);
    let mut w = window(doc.clone());
    w.click("Text");
    let board = w.board();
    w.click_at(screen(board, hit[0], hit[1]));
    assert_eq!(w.editor.selection, [id]);
    let active = w.frame(vec![]);
    // An unchanged edit uses the original pixels and no opaque editing plate.
    let art_id = *w
        .ctx
        .tex_manager()
        .read()
        .allocated()
        .find(|(_, t)| t.name == "artboard artwork")
        .unwrap()
        .0;
    assert!(active
        .textures_delta
        .set
        .iter()
        .all(|(id, _)| *id != art_id));
    assert!(!active
        .shapes
        .iter()
        .any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.text() == "Old")));
    w.frame(vec![
        Event::PointerMoved(screen(board, hit[0], hit[1])),
        Event::Zoom(1.25),
    ]);
    let zoomed = w.board();
    assert!(zoomed.width() > board.width() * 1.2);
    for _ in 0..16 {
        w.frame(vec![]);
    }
    command(&mut w, Key::A);
    let typed = "Live\nlayer";
    let edited = w.frame(vec![Event::Text(typed.into())]);
    style.content = typed.into();
    let mut expected = doc.clone();
    style.apply(&mut expected.objects[1]).unwrap();
    let px = renderer::render_scaled(
        &expected,
        0,
        zoomed.width() / doc.width as f32 * w.ctx.pixels_per_point(),
    )
    .unwrap();
    let delta = edited
        .textures_delta
        .set
        .iter()
        .find(|(id, _)| *id == art_id)
        .expect("live artwork upload");
    let egui::ImageData::Color(image) = &delta.1.image else {
        panic!("artwork pixels")
    };
    assert_eq!(delta.1.pos, None);
    assert!(
        **image
            == egui::ColorImage::from_rgba_premultiplied(
                [px.width() as usize, px.height() as usize],
                px.data()
            )
    );
    assert_eq!(w.editor.scene(), &doc);
    command(&mut w, Key::Enter);
    assert_eq!(w.editor.scene(), &expected);
    command(&mut w, Key::Z);
    assert_eq!(w.editor.scene(), &doc);
}

#[test]
fn workspace_type_controls_keep_the_draft_when_choosing_a_face_and_editing_size() {
    let mut w = window(Document::default());
    let board = w.board();
    w.click("Text");
    w.click_at(screen(board, 20., 30.));
    w.frame(vec![Event::Text("Part".into())]);
    w.click("Hack Regular");
    assert!(w.editor.canvas.text_active());
    let output = w.frame(vec![]);
    let size = output
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.text().ends_with(" px") => {
                Some(t.pos + t.galley.size() * 0.5)
            }
            _ => None,
        })
        .expect("visible type size");
    w.click_at(size);
    command(&mut w, Key::A);
    w.frame(vec![Event::Text("32".into())]);
    press(&mut w, Key::Enter, Modifiers::NONE, None);
    assert!(w.editor.scene().objects.is_empty());
    assert!(w.editor.canvas.text_active());
    w.click("Center");
    w.frame(vec![Event::Text(" B".into())]);
    command(&mut w, Key::Enter);
    let saved = w.editor.scene().objects[0].text.as_ref().unwrap();
    assert_eq!(saved.content, "Part B");
    assert_eq!(saved.face, Typeface::HackRegular);
    assert_eq!(saved.align, TextAlign::Center);
    assert_eq!(saved.size, 32.);
    assert_eq!(w.board(), board);
    command(&mut w, Key::Z);
    assert!(w.editor.scene().objects.is_empty() && !w.editor.can_undo());
}

#[test]
fn clearing_existing_text_removes_only_its_label_on_commit_and_undo_restores_it() {
    let mut doc = Document::default();
    let id = TextStyle::default()
        .insert(&mut doc, 1, [16.; 2], [255; 4])
        .unwrap();
    doc.add_rectangle([90.; 2], [8.; 2], [255; 4]);
    let mut w = window(doc.clone());
    let board = w.board();
    w.click("Text");
    for commit in [false, true] {
        w.click_at(screen(board, 22., 24.));
        assert_eq!(w.editor.selection, [id]);
        command(&mut w, Key::A);
        press(&mut w, Key::Backspace, Modifiers::NONE, None);
        assert_eq!(w.editor.scene(), &doc);
        if commit {
            command(&mut w, Key::Enter);
            assert_eq!(w.editor.scene().objects.as_slice(), &doc.objects[1..]);
            assert!(w.editor.selection.is_empty());
            command(&mut w, Key::Z);
        } else {
            press(&mut w, Key::Escape, Modifiers::NONE, None);
        }
        assert_eq!(w.editor.scene(), &doc);
        assert!(!w.editor.can_undo());
    }
}

#[test]
fn suffix_typing_keeps_the_transformed_first_glyphs_in_place() {
    let mut doc = Document::default();
    let style = TextStyle {
        content: "HH".into(),
        face: Typeface::HackRegular,
        size: 15.,
        ..Default::default()
    };
    style
        .insert(&mut doc, 1, [28., 32.], [35, 95, 173, 255])
        .unwrap();
    doc.objects[0].scale = [1.52, 0.82];
    doc.objects[0].rotation = 28.;
    doc.objects[0].animated.skew = 17.;
    let o = &doc.objects[0];
    let [x, y] = o.local_to_world([0.99, 0.5], 0);
    let original = renderer::render(&doc, 0).unwrap();
    let mut w = window(doc.clone());
    w.click("Text");
    let board = w.board();
    w.click_at(screen(board, x, y));
    press(&mut w, Key::End, Modifiers::NONE, None);
    let live = w.frame(vec![Event::Text("xyz".into())]);
    let art_id = *w
        .ctx
        .tex_manager()
        .read()
        .allocated()
        .find(|(_, t)| t.name == "artboard artwork")
        .unwrap()
        .0;
    let delta = live
        .textures_delta
        .set
        .iter()
        .find(|(id, _)| *id == art_id)
        .unwrap();
    let egui::ImageData::Color(live_pixels) = &delta.1.image else {
        panic!("artwork pixels")
    };
    command(&mut w, Key::Enter);
    assert_eq!(
        w.editor.scene().objects[0].text.as_ref().unwrap().content,
        "HHxyz"
    );
    let scaled = renderer::render_scaled(
        w.editor.scene(),
        0,
        board.width() / doc.width as f32 * w.ctx.pixels_per_point(),
    )
    .unwrap();
    assert!(
        **live_pixels
            == egui::ColorImage::from_rgba_premultiplied(
                [scaled.width() as usize, scaled.height() as usize],
                scaled.data()
            )
    );
    let result = renderer::render(w.editor.scene(), 0).unwrap();
    let mut ink = 0;
    for y in 0..original.height() {
        for x in 0..original.width() {
            let [u, v] = o.world_to_local([x as f32 + 0.5, y as f32 + 0.5], 0);
            if (-0.1..0.84).contains(&u) && (-0.1..1.1).contains(&v) {
                let alpha = original.pixel(x, y).unwrap().alpha();
                ink += usize::from(alpha > 128);
                assert!(
                    alpha.abs_diff(result.pixel(x, y).unwrap().alpha()) <= 2,
                    "first glyph moved at ({x}, {y})"
                );
            }
        }
    }
    assert!(ink > 10);
}
