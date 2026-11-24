use super::*;

#[test]
fn blend_controls_edit_the_selected_scope_and_undo() {
    use document_core::BlendMode;

    let mut drawing = Document::default();
    let id = drawing.add_rectangle([10.; 2], [24.; 2], [160; 4]);
    drawing.objects[0].stroke_width = 4.;
    let other = drawing.add_rectangle([18.; 2], [24.; 2], [200; 4]);
    let mut w = EditorWindow::new(egui::vec2(1120., 840.));
    w.ctx.style_mut(|style| style.animation_time = 0.);
    w.editor
        .replace_project(AnimationProject::from_document(drawing.clone()), None);
    w.editor.selection = vec![id];
    for (row, mode) in [
        ("Object blend", "Overlay"),
        ("Fill blend", "Multiply"),
        ("Stroke blend", "Screen"),
    ] {
        let control = w.field_on_row(row, "Normal");
        w.click_at(control);
        w.click(mode);
    }
    let edited = &w.editor.scene().objects[0];
    assert_eq!(edited.blend_mode, BlendMode::Overlay);
    assert_eq!(edited.fill_blend_mode, BlendMode::Multiply);
    assert_eq!(edited.stroke_blend_mode, BlendMode::Screen);
    assert_eq!(w.editor.scene().objects[1], drawing.objects[1]);

    w.editor.selection = vec![id, other];
    let mixed = w.field_on_row("Fill blend", "Mixed");
    w.click_at(mixed);
    w.click("Normal");
    assert!(w
        .editor
        .scene()
        .objects
        .iter()
        .all(|o| o.fill_blend_mode.is_normal()));
    w.click_edit("Undo");
    assert_eq!(
        w.editor.scene().objects[0].fill_blend_mode,
        BlendMode::Multiply
    );

    w.click("Layers");
    let layer = w.field_on_row("Layer blend", "Normal");
    w.click_at(layer);
    w.click("Darken");
    assert_eq!(w.editor.scene().layers[0].blend_mode, BlendMode::Darken);
    w.click_edit("Undo");
    assert_eq!(w.editor.scene().layers[0].blend_mode, BlendMode::Normal);
}

#[test]
fn no_paint_buttons_restore_the_original_gradient_and_apply_to_mixed_selections() {
    use document_core::{Gradient, GradientKind};
    let mut drawing = Document::default();
    let first = drawing.add_rectangle([10.; 2], [24.; 2], [88, 209, 167, 255]);
    let second = drawing.add_rectangle([60.; 2], [18.; 2], [25, 80, 130, 0]);
    drawing.objects[0].stroke = [50, 60, 70, 42];
    drawing.objects[0].stroke_width = 4.5;
    drawing.objects[0].stroke_gradient = Some(Gradient::from_color(
        GradientKind::Linear {
            start: [0.; 2],
            end: [1.; 2],
        },
        [180, 64, 20, 255],
    ));
    let mut w = EditorWindow::new(egui::vec2(1120., 840.));
    w.ctx.style_mut(|style| style.animation_time = 0.);
    w.editor
        .replace_project(AnimationProject::from_document(drawing.clone()), None);
    w.editor.selection = vec![first];
    let remove = w.field_on_row("Stroke", "×");
    w.click_at(remove);
    let o = &w.editor.scene().objects[0];
    assert!(o.no_stroke && !o.has_stroke_at(0));
    assert_eq!(o.stroke_width, 4.5);
    assert_eq!(o.stroke, drawing.objects[0].stroke);
    assert!(w.editor.style.no_stroke);
    assert!(visible(&mut w, "No paint"));
    assert!(!visible(&mut w, "Stroke gradient"));
    w.click_at(remove);
    assert_eq!(w.editor.scene(), &drawing);
    assert!(!w.editor.style.no_stroke);

    w.editor.selection = vec![first, second];
    w.click("Fill");
    w.click("No paint");
    assert!(w.editor.scene().objects.iter().all(|o| o.no_fill));
    assert_eq!(w.editor.scene().objects[1].fill, [25, 80, 130, 0]);
    w.click_edit("Undo");
    assert_eq!(w.editor.scene(), &drawing);
}

#[test]
fn gradient_fill_types_and_stops_are_available_in_appearance_with_undo() {
    use document_core::GradientKind;
    let mut doc = Document::default();
    let color = [25, 133, 207, 199];
    let id = doc.add_rectangle([18., 30.], [62., 50.], color);
    let mut w = EditorWindow::new(egui::vec2(1120., 840.));
    w.ctx.style_mut(|style| style.animation_time = 0.);
    w.editor
        .replace_project(AnimationProject::from_document(doc), None);
    w.editor.selection = vec![id];
    w.editor.tool = Tool::Pen;

    w.click("Fill");
    w.click("Radial");
    let radial = w.editor.scene().objects[0].gradient.clone().unwrap();
    assert!(matches!(radial.kind, GradientKind::Radial { .. }));
    assert_eq!(radial.stops[0].color, color);
    assert_eq!(radial.stops[1].color, [25, 133, 207, 0]);
    assert_eq!(w.editor.style.gradient.as_deref(), Some(&radial));
    assert!(visible(&mut w, "Fill gradient"));
    w.click("Add stop");
    let expanded = w.editor.scene().objects[0].gradient.clone().unwrap();
    assert_eq!(expanded.stops.len(), 3);
    assert_eq!(expanded.stops[1].offset, 0.5);
    w.click("Fill");
    w.click("Rectangular");
    let rectangular = w.editor.scene().objects[0].gradient.as_ref().unwrap();
    assert!(matches!(rectangular.kind, GradientKind::Rectangular { .. }));
    assert_eq!(rectangular.stops, expanded.stops);
    w.click("Fill");
    w.click("Solid");
    assert!(!visible(&mut w, "Fill gradient"));
    assert!(w.editor.scene().objects[0].gradient.is_none());
    assert!(w.editor.style.gradient.is_none());
    for _ in 0..4 {
        w.click_edit("Undo");
    }
    assert!(w.editor.scene().objects[0].gradient.is_none());
    assert!(!w.editor.can_undo());
}

#[test]
fn stroke_gradient_has_its_own_stops_handles_width_and_undo() {
    use crate::canvas::PaintChannel;
    use document_core::GradientKind;
    let mut doc = Document::default();
    let id = doc.add_rectangle([23., 27.], [52., 38.], [220, 60, 20, 255]);
    doc.objects[0].stroke = [24, 102, 200, 174];
    doc.objects[0].stroke_width = 0.;
    let mut w = EditorWindow::new(egui::vec2(1120., 840.));
    w.ctx.style_mut(|style| style.animation_time = 0.);
    w.editor
        .replace_project(AnimationProject::from_document(doc.clone()), None);
    w.editor.selection = vec![id];
    w.editor.style.stroke_width = 0.;
    w.click("Fill");
    w.click("Radial");
    let before_stroke = w.editor.scene().clone();
    w.click("Stroke");
    w.click("Rectangular");
    let painted = w.editor.scene().clone();
    let border = painted.objects[0].stroke_gradient.as_ref().unwrap();
    assert!(matches!(border.kind, GradientKind::Rectangular { .. }));
    assert_eq!(border.stops[0].color, doc.objects[0].stroke);
    assert_eq!(painted.objects[0].stroke_width, 1.);
    assert_eq!(
        painted.objects[0].gradient,
        before_stroke.objects[0].gradient
    );
    assert_eq!(w.editor.style.stroke_gradient.as_deref(), Some(border));
    assert_eq!(w.editor.style.gradient_channel, PaintChannel::Stroke);
    assert!(visible(&mut w, "Stroke gradient"));
    assert!(!visible(&mut w, "Fill gradient"));
    w.click("Add stop");
    assert_eq!(
        w.editor.scene().objects[0]
            .stroke_gradient
            .as_ref()
            .unwrap()
            .stops
            .len(),
        3
    );
    w.click("Radial");
    assert_eq!(w.editor.style.gradient_channel, PaintChannel::Fill);
    assert!(visible(&mut w, "Fill gradient"));
    assert!(!visible(&mut w, "Stroke gradient"));
    w.click("Add stop");
    assert_eq!(
        w.editor.scene().objects[0]
            .gradient
            .as_ref()
            .unwrap()
            .stops
            .len(),
        3
    );
    w.click_edit("Undo");
    w.click_edit("Undo");
    assert_eq!(w.editor.scene(), &painted);
    w.click_edit("Undo");
    assert_eq!(w.editor.scene(), &before_stroke);
}

#[test]
fn changing_fill_type_on_multiple_shapes_keeps_their_own_colors_and_geometry() {
    use document_core::{Gradient, GradientKind};
    let mut doc = Document::default();
    let a = doc.add_rectangle([3., 5.], [10., 25.], [16, 24, 48, 255]);
    let b = doc.add_rectangle([30., 32.], [45., 44.], [205, 170, 16, 255]);
    let radial = Gradient::from_color(
        GradientKind::Radial {
            center: [0.24, 0.81],
            radius: 0.46,
        },
        [21, 144, 216, 155],
    );
    doc.objects[0].gradient = Some(radial.clone());
    let mut w = EditorWindow::new(egui::vec2(1120., 740.));
    w.ctx.style_mut(|style| style.animation_time = 0.);
    w.editor
        .replace_project(AnimationProject::from_document(doc.clone()), None);
    w.editor.selection = vec![a, b];

    w.click("Fill");
    w.click("Rectangular");
    let shape = &w.editor.scene().objects[0];
    let converted = shape.gradient.as_ref().unwrap();
    assert_eq!(
        converted.kind,
        GradientKind::Rectangular {
            center: [0.24, 0.81],
            radius: 0.46
        }
    );
    assert_eq!(converted.stops, radial.stops);
    let other = &w.editor.scene().objects[1];
    assert_eq!(other.gradient.as_ref().unwrap().stops[0].color, other.fill);
    w.click_edit("Undo");
    assert_eq!(w.editor.scene(), &doc);
}

#[test]
fn inspector_rotation_and_base_size_keep_the_selected_hinge_in_place() {
    let mut doc = Document::default();
    let id = doc.add_rectangle([35., 26.], [34., 16.], [62, 200, 151, 255]);
    let hinge = [28., 34.];
    assert!(doc.objects[0].set_pivot_at(0, hinge));
    let mut w = EditorWindow::new(egui::vec2(1100., 920.));
    w.ctx.style_mut(|style| style.animation_time = 0.);
    w.editor
        .replace_project(AnimationProject::from_document(doc), None);
    w.editor.selection = vec![id];
    let rotation = w.field_on_row("Rotation", "°");
    w.drag(rotation, egui::vec2(42., 0.));
    let check_hinge = |object: &document_core::VectorObject| {
        assert!(egui::Pos2::from(object.pivot_at(0)).distance(hinge.into()) < 0.001);
    };
    let turned = w.editor.scene().objects[0].clone();
    assert_ne!(turned.rotation_at(0), 0.);
    check_hinge(&turned);
    let width = w.field_on_row("W", " px");
    w.drag(width, egui::vec2(46., 0.));
    let resized = &w.editor.scene().objects[0];
    assert_ne!(resized.size, turned.size);
    check_hinge(resized);
    w.click_edit("Undo");
    assert_eq!(w.editor.scene().objects[0], turned);
}

fn dock(window: &EditorWindow) -> egui::Rect {
    egui::containers::panel::PanelState::load(&window.ctx, egui::Id::new("inspector"))
        .unwrap()
        .rect
}

fn visible(window: &mut EditorWindow, label: &str) -> bool {
    window.frame(vec![]).shapes.iter().any(|s| match &s.shape {
        egui::Shape::Text(t) if t.galley.text() == label => s
            .clip_rect
            .contains_rect(t.galley.rect.translate(t.pos.to_vec2())),
        _ => false,
    })
}

fn has_bone_overlay(window: &mut EditorWindow) -> bool {
    window.frame(vec![]).shapes.iter().any(|s| {
        matches!(&s.shape, egui::Shape::Circle(c) if c.fill == egui::Color32::from_rgb(255, 196, 100))
    })
}

fn choose_panel(w: &mut EditorWindow, label: &str) {
    if !visible(w, label) {
        w.click(w.editor.inspector_tab.label());
    }
    w.click(label);
}

#[test]
fn transparency_checkerboard_only_changes_the_artboard_backdrop() {
    let mut doc = Document::default();
    let id = doc.add_rectangle([16.; 2], [32.; 2], [220, 96, 54, 128]);
    let mut next = doc.clone();
    next.objects[0].position = [65., 20.];
    let mut project = AnimationProject::from_document(doc.clone());
    project.clips[0].cels.insert(1, next.into());
    doc.frames = 1;
    project
        .add_symbol(document_core::SymbolDefinition {
            name: "Swatch".into(),
            artwork: doc.into(),
        })
        .unwrap();
    let mut w = EditorWindow::new(egui::vec2(1100., 740.));
    w.ctx.style_mut(|style| style.animation_time = 0.);
    w.editor.replace_project(project.clone(), None);
    w.editor.timeline.onion = true;
    w.editor.selection = vec![id];
    let board = w.board();
    choose_panel(&mut w, "Library");
    w.click("Swatch");
    let folder = tempfile::tempdir().unwrap();
    let png = folder.path().join("swatch.png");
    let mut reference = None;
    for (index, enabled) in [false, true, false].into_iter().enumerate() {
        if index > 0 {
            w.click("View");
            w.click("Transparency checkerboard");
        }
        assert_eq!(w.editor.canvas.show_checkerboard(), enabled);
        assert_eq!(w.board(), board);
        assert_eq!(w.editor.selection, [id]);
        for in_library in [false, true] {
            if in_library {
                w.click("Edit artwork…");
                w.frame(vec![]);
            }
            let output = w.frame(vec![]);
            let manager = w.ctx.tex_manager();
            let textures = manager.read();
            let layers: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|s| {
                    if crate::canvas::painted_artboard(&s.shape).is_some() {
                        return Some("backdrop");
                    }
                    let egui::Shape::Mesh(mesh) = &s.shape else {
                        return None;
                    };
                    let name = textures.meta(mesh.texture_id)?.name.as_str();
                    matches!(
                        name,
                        "artboard transparency" | "onion silhouette" | "artboard artwork"
                    )
                    .then_some(name)
                })
                .collect();
            let mut expected = vec!["backdrop"];
            if enabled {
                expected.push("artboard transparency");
            }
            expected.extend(["onion silhouette", "artboard artwork"]);
            if in_library {
                expected.push("backdrop");
                if enabled {
                    expected.push("artboard transparency");
                }
                expected.push("artboard artwork");
            }
            assert_eq!(layers, expected);
            drop(textures);
            if in_library {
                w.click("Cancel");
            }
        }
        project_io::export_frame(&png, w.editor.scene(), 0).unwrap();
        let bytes = std::fs::read(&png).unwrap();
        let pixels = renderer::Pixmap::decode_png(&bytes).unwrap();
        assert_eq!(pixels.pixel(0, 0).unwrap().alpha(), 0);
        assert_eq!(pixels.pixel(24, 24).unwrap().alpha(), 128);
        assert_eq!(&bytes, reference.get_or_insert_with(|| bytes.clone()));
        assert_eq!(w.editor.project, project);
        assert!(!w.editor.can_undo());
    }
}

#[test]
fn double_clicking_an_object_in_layers_renames_it_in_one_undo_step() {
    let mut doc = Document::default();
    let id = doc.add_rectangle([10.; 2], [20.; 2], [255; 4]);
    doc.objects[0].name = "Sleeve".into();
    let mut w = EditorWindow::new(egui::vec2(1100., 740.));
    w.editor
        .replace_project(AnimationProject::from_document(doc), None);
    choose_panel(&mut w, "Layers");
    assert!(!visible(&mut w, "Undo"));
    assert!(!visible(&mut w, "Redo"));
    let key = |key| egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    let row = w.control("Sleeve");
    w.click_at(row);
    w.click_at(row);
    w.frame(vec![egui::Event::Text("Left".into())]);
    w.frame(vec![egui::Event::Text(" cuff".into())]);
    w.frame(vec![key(Key::Enter)]);
    assert_eq!(w.editor.scene().objects[0].name, "Left cuff");
    assert_eq!(w.editor.selection, [id]);
    w.click_edit("Undo");
    assert_eq!(w.editor.scene().objects[0].name, "Sleeve");
    assert!(!w.editor.can_undo());
    w.click_edit("Redo");
    let row = w.control("Left cuff");
    w.click_at(row);
    w.click_at(row);
    w.frame(vec![egui::Event::Text("temporary".into())]);
    w.frame(vec![key(Key::Escape)]);
    assert_eq!(w.editor.scene().objects[0].name, "Left cuff");
    w.click_edit("Undo");
    assert_eq!(w.editor.scene().objects[0].name, "Sleeve");
}

#[test]
fn l_locks_the_group_in_place_and_undo_restores_canvas_picking() {
    let mut doc = Document::default();
    let a = doc.add_rectangle([10.; 2], [20.; 2], [255; 4]);
    let b = doc.add_rectangle([35.; 2], [20.; 2], [255; 4]);
    commands::group(&mut doc, &[a, b]);
    let mut w = EditorWindow::new(egui::vec2(1100., 740.));
    w.editor
        .replace_project(AnimationProject::from_document(doc.clone()), None);
    let board = w.board();
    let click = board.min + board.size() * (15. / 128.);
    w.click_at(click);
    assert_eq!(w.editor.selection, [a, b]);
    pen::chord(&mut w, Key::L, egui::Modifiers::NONE);
    assert!(w.editor.scene().objects.iter().all(|o| o.locked));
    assert!(w.editor.selection.is_empty());
    w.click_at(click);
    assert!(w.editor.selection.is_empty());
    assert_eq!(
        renderer::render(w.editor.scene(), 0).unwrap().data(),
        renderer::render(&doc, 0).unwrap().data()
    );
    w.click_edit("Undo");
    assert_eq!(w.editor.scene(), &doc);
    assert!(!w.editor.can_undo());
    w.click_at(click);
    assert_eq!(w.editor.selection, [a, b]);
}

#[test]
fn layer_and_group_lock_controls_update_their_glyphs_after_each_click() {
    let mut doc = Document::default();
    doc.layers[0].name = "Ink layer".into();
    for x in [10., 35.] {
        doc.add_rectangle([x, 10.], [20.; 2], [255; 4]);
        let object = doc.objects.last_mut().unwrap();
        object.group = Some(3);
        object.group_name = Some("Ink group".into());
    }
    let mut w = EditorWindow::new(egui::vec2(1100., 740.));
    w.editor
        .replace_project(AnimationProject::from_document(doc), None);
    choose_panel(&mut w, "Layers");
    let glyph = |w: &mut EditorWindow, label: &str| {
        let row = w.control(label);
        w.frame(vec![])
            .shapes
            .into_iter()
            .find_map(|shape| {
                let egui::Shape::Rect(r) = shape.shape else {
                    return None;
                };
                (r.rect.center().x > row.x
                    && (r.rect.top() - row.y).abs() < 2.
                    && r.rect.size() == egui::vec2(9., 5.5))
                .then_some((r.rect.center(), r.fill != egui::Color32::TRANSPARENT))
            })
            .expect("visible padlock body")
    };
    for (name, layer) in [("Ink layer", true), ("Ink group", false)] {
        let (at, locked) = glyph(&mut w, name);
        assert!(!locked);
        w.click_at(at);
        assert!(glyph(&mut w, name).1);
        let scene = w.editor.scene();
        if layer {
            assert!(scene.layers[0].locked);
            assert!(glyph(&mut w, "Ink group").1);
        } else {
            assert!(scene.objects.iter().all(|o| o.locked));
        }
        w.click_at(at);
        assert!(!glyph(&mut w, name).1);
    }
}

#[test]
fn group_titles_in_layers_can_be_renamed_cleared_and_undone() {
    let mut doc = Document::default();
    let a = doc.add_rectangle([10.; 2], [20.; 2], [255; 4]);
    let b = doc.add_rectangle([35.; 2], [20.; 2], [255; 4]);
    doc.objects[0].name = "Left".into();
    doc.objects[1].name = "Right".into();
    for object in &mut doc.objects {
        object.group = Some(3);
    }
    let mut w = EditorWindow::new(egui::vec2(800., 500.));
    w.editor
        .replace_project(AnimationProject::from_document(doc), None);
    choose_panel(&mut w, "Layers");
    let open_name = |w: &mut EditorWindow, name| {
        let row = w.control(name);
        w.click_at(row);
        w.click_at(row);
    };
    let key = |key| egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };

    open_name(&mut w, "Group 3");
    w.frame(vec![egui::Event::Text("Pair".into())]);
    w.frame(vec![egui::Event::Text(" of wings".into())]);
    w.frame(vec![key(Key::Enter)]);
    assert!(visible(&mut w, "Pair of wings"));
    assert_eq!(w.editor.selection, [a, b]);
    assert_eq!(w.editor.scene().objects[0].name, "Left");
    assert_eq!(w.editor.scene().objects[1].name, "Right");
    w.click_edit("Undo");
    assert!(visible(&mut w, "Group 3"));
    assert!(!w.editor.can_undo());
    w.click_edit("Redo");

    open_name(&mut w, "Pair of wings");
    w.frame(vec![egui::Event::Text("discard".into())]);
    w.frame(vec![key(Key::Escape)]);
    assert!(visible(&mut w, "Pair of wings"));
    open_name(&mut w, "Pair of wings");
    w.frame(vec![key(Key::Backspace)]);
    w.frame(vec![key(Key::Enter)]);
    assert!(visible(&mut w, "Group 3"));
    w.click_edit("Undo");
    assert!(visible(&mut w, "Pair of wings"));
}

#[test]
fn leaving_an_inline_rename_commits_to_its_drawing_before_switching_panels_or_frames() {
    let mut doc = Document {
        frames: 12,
        ..Default::default()
    };
    for x in [10., 35.] {
        doc.add_rectangle([x, 10.], [20.; 2], [255; 4]);
        doc.objects.last_mut().unwrap().group = Some(3);
    }
    let mut project = AnimationProject::from_document(doc.clone());
    project.clips[0].cels.insert(2, doc.clone().into());
    let mut w = EditorWindow::new(egui::vec2(1100., 740.));
    w.editor.replace_project(project, None);
    choose_panel(&mut w, "Layers");
    let paste_name = |w: &mut EditorWindow, old, name: &str| {
        let row = w.control(old);
        w.click_at(row);
        w.click_at(row);
        w.frame(vec![egui::Event::Paste(name.into())]);
    };

    paste_name(&mut w, "Group 3", "Wi\nn\tgs");
    choose_panel(&mut w, "Properties");
    assert_eq!(
        w.editor.scene().objects[0].group_name.as_deref(),
        Some("Wings")
    );
    choose_panel(&mut w, "Layers");
    paste_name(&mut w, "Wings", "Feathers");
    w.click("4");
    let clip = &w.editor.project.clips[0];
    assert_eq!(w.editor.timeline.frame, 3);
    assert_eq!(
        clip.drawing_at(0).objects[0].group_name.as_deref(),
        Some("Feathers")
    );
    assert_eq!(clip.drawing_at(3).objects[0].group_name, None);
    w.click_edit("Undo");
    assert_eq!(
        w.editor.project.clips[0].drawing_at(0).objects[0]
            .group_name
            .as_deref(),
        Some("Wings")
    );

    w.click("4");
    paste_name(&mut w, "Group 3", "Old project");
    w.editor
        .replace_project(AnimationProject::from_document(doc.clone()), None);
    w.frame(vec![]);
    choose_panel(&mut w, "Properties");
    assert_eq!(w.editor.scene(), &doc);
}

#[test]
fn a_stacked_layer_rename_commits_when_the_artist_clicks_into_properties() {
    let mut doc = Document::default();
    let id = doc.add_rectangle([8.; 2], [24.; 2], [180, 100, 35, 255]);
    doc.objects[0].name = "Sleeve".into();
    let mut w = EditorWindow::new(egui::vec2(1100., 840.));
    w.ctx.style_mut(|s| s.animation_time = 0.);
    w.editor
        .replace_project(AnimationProject::from_document(doc), None);
    w.frame(vec![]);
    w.click("View");
    w.click("Panels");
    w.click("Layers and Properties together");
    let layers = w.control("Layers");
    let row = w.control("Sleeve");
    assert!(row.y > layers.y);
    w.click_at(row);
    w.click_at(row);
    w.frame(vec![egui::Event::Paste("Right sleeve".into())]);
    let properties = w.control("Properties");
    assert!(properties.y < row.y);
    w.click_at(properties);
    assert_eq!(w.editor.scene().objects[0].name, "Right sleeve");
    assert_eq!(w.editor.selection, [id]);
    assert!(!w.ctx.wants_keyboard_input());
    w.click_edit("Undo");
    assert_eq!(w.editor.scene().objects[0].name, "Sleeve");
}

#[test]
fn panels_menu_and_library_keep_the_dock_stable_and_recover_from_hiding_every_panel() {
    use crate::inspector::Tab;
    for size in [egui::vec2(800., 500.), egui::vec2(1100., 740.)] {
        let mut w = EditorWindow::new(size);
        w.ctx.style_mut(|style| style.animation_time = 0.);
        let mut doc = Document::default();
        let shape = doc.add_rectangle([20.; 2], [35.; 2], [255; 4]);
        doc.objects[0].name = "Lantern".into();
        w.editor
            .replace_project(AnimationProject::from_document(doc.clone()), None);
        let board = w.board();
        let width = dock(&w);
        choose_panel(&mut w, "Library");
        assert!(width.contains(w.control("Convert selection")));
        assert!(!visible(&mut w, "Objects"));
        assert!(!visible(&mut w, "Panels"));
        choose_panel(&mut w, "Layers");
        w.click("Lantern");
        assert_eq!(w.editor.selection, [shape]);
        w.click_at(board.min + board.size() * 0.75);
        assert!(w.editor.selection.is_empty());
        assert_eq!(width, dock(&w));
        assert_eq!(board, w.board());
        w.click("View");
        w.click("Panels");
        for label in ["Artwork library", "Rig", "Layers", "Properties"] {
            w.click(label);
        }
        assert_eq!(w.editor.visible_panels, [false; Tab::COUNT]);
        w.click("Artwork library");
        assert_eq!(w.editor.inspector_tab, Tab::Library);
        w.click_at(board.center());
        assert!(width.contains(w.control("Convert selection")));
        assert_eq!(width, dock(&w));
        assert_eq!(board, w.board());
        w.editor
            .replace_project(AnimationProject::from_document(doc), None);
        assert_eq!(w.editor.visible_panels, [false, false, false, true, false]);
        w.click("View");
        w.click("Inspector");
        let mods = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
        w.frame_with_input(egui::RawInput {
            modifiers: mods,
            events: vec![egui::Event::Key {
                key: Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: mods,
            }],
            ..Default::default()
        });
        assert!(w.editor.show_inspector);
        assert_eq!(w.editor.inspector_tab, Tab::Properties);
        assert_eq!(w.editor.visible_panels, [true, false, false, true, false]);
    }
}

#[test]
fn inspector_tabs_keep_the_chosen_width_and_limit_bone_overlays_to_the_rig() {
    for size in [egui::vec2(800., 500.), egui::vec2(1100., 740.)] {
        let mut doc = Document::default();
        let id = doc.add_rectangle([20.; 2], [35.; 2], [135, 180, 235, 255]);
        doc.objects[0].name = "Lantern".into();
        doc.layers[0].name = "Artwork".into();
        doc.add_bone("Shoulder", None, [27., 27.], [45., 35.])
            .unwrap();
        let mut w = EditorWindow::new(size);
        w.ctx.style_mut(|s| s.animation_time = 0.);
        w.editor
            .replace_project(AnimationProject::from_document(doc), None);
        w.editor.set_tool(Tool::Select);
        w.control("Tool paint");
        assert!(!visible(&mut w, "Transform"));
        let board = w.board();
        w.click_at(board.min + board.size() * 0.30);
        assert_eq!(w.editor.selection, [id]);
        w.control("Lantern");
        assert!(!has_bone_overlay(&mut w));
        for narrow in [false, true] {
            if narrow {
                let grip = dock(&w).left_center();
                w.drag(grip, egui::vec2(180., 0.));
                assert!((dock(&w).width() - 180.).abs() < 1.);
            }
            let board = w.board();
            let inspector = dock(&w);
            for tab in ["Layers", "Rig", "Properties"] {
                choose_panel(&mut w, tab);
                match tab {
                    "Layers" => {
                        w.control("Artwork");
                        w.control("Lantern");
                    }
                    "Rig" => {
                        assert!(has_bone_overlay(&mut w));
                        w.click("Show bones");
                        assert!(!has_bone_overlay(&mut w));
                        w.click("Show bones");
                    }
                    _ => {
                        w.control("Lantern");
                        if visible(&mut w, "Fill") {
                            w.click("Appearance");
                        }
                        if visible(&mut w, "X") {
                            w.click("Transform");
                        }
                        w.click("Align and Distribute");
                        let upper = w.control("Left").y;
                        let lower = w.control("Top").y;
                        assert!(lower > upper);
                        for label in ["Center X", "Right"] {
                            assert!((w.control(label).y - upper).abs() < 1.);
                        }
                        for label in ["Center Y", "Bottom"] {
                            assert!((w.control(label).y - lower).abs() < 1.);
                        }
                        w.click("Align and Distribute");
                    }
                }
                assert_eq!(w.board(), board, "board moved when opening {tab}");
                assert_eq!(dock(&w), inspector, "dock resized when opening {tab}");
                assert_eq!(has_bone_overlay(&mut w), tab == "Rig");
                assert_eq!(w.editor.selection, [id]);
            }
        }
        choose_panel(&mut w, "Rig");
        w.click("View");
        w.click("Inspector");
        assert!(!has_bone_overlay(&mut w));
        assert!(!w.editor.can_undo());
    }
}

#[test]
fn removing_a_library_symbol_then_editing_one_copy_uses_two_project_undo_steps() {
    let mut art = Document {
        width: 32,
        height: 32,
        frames: 1,
        ..Default::default()
    };
    art.add_rectangle([2., 4.], [10., 16.], [228, 40, 56, 255]);
    art.add_rectangle([18., 4.], [10., 16.], [50, 184, 107, 255]);
    let mut project = AnimationProject::from_document(Document {
        width: 128,
        height: 128,
        ..Document::default()
    });
    let symbol = project
        .add_symbol(document_core::SymbolDefinition {
            name: "Badge".into(),
            artwork: art.into(),
        })
        .unwrap();
    let first = project.place_symbol(symbol, 0, 0, 1, [6., 9.]).unwrap();
    let other = project.place_symbol(symbol, 0, 0, 1, [70., 52.]).unwrap();
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("badges.vanim");
    project_io::save_animation(&path, &project, false).unwrap();
    let mut w = EditorWindow::new(egui::vec2(1100., 740.));
    w.ctx.style_mut(|s| s.animation_time = 0.);
    w.editor.open_project(path.clone());
    let linked = w.editor.project.clone();
    let original = renderer::render(w.editor.scene(), 0).unwrap();
    choose_panel(&mut w, "Library");
    w.click("Badge");
    w.click("Remove symbol…");
    w.click("Remove and keep artwork");
    let copies = w.editor.project.clone();
    assert!(copies.symbols.is_empty());
    assert!(original.data() == renderer::render(w.editor.scene(), 0).unwrap().data());

    choose_panel(&mut w, "Properties");
    let board = w.board();
    let scene = w.editor.scene();
    let dimensions = egui::vec2(scene.width as f32, scene.height as f32);
    let point = |x, y| board.min + board.size() * egui::vec2(x, y) / dimensions;
    w.click_at(point(13., 21.));
    assert_eq!(w.editor.selection, [first]);
    w.click("Edit this copy…");
    // The modal's artboard is painted above the main scene.
    let local = w
        .frame(vec![])
        .shapes
        .into_iter()
        .rev()
        .find_map(|s| crate::canvas::painted_artboard(&s.shape))
        .unwrap();
    w.click_at(local.min + local.size() * egui::vec2(7. / 32., 12. / 32.));
    for pressed in [true, false] {
        w.frame(vec![egui::Event::Key {
            key: egui::Key::Delete,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]);
    }
    assert_eq!(
        w.editor.project, copies,
        "the draft stays local until Apply"
    );
    w.click("Apply to this copy");
    let edited = w.editor.project.clone();
    let scene = w.editor.scene();
    assert_eq!(scene.objects[0].static_artwork().unwrap().objects.len(), 1);
    assert_eq!(scene.objects[1].static_artwork().unwrap().objects.len(), 2);
    let image = renderer::render(scene, 0).unwrap();
    assert_ne!(original.pixel(13, 21).unwrap().alpha(), 0);
    assert_eq!(image.pixel(13, 21).unwrap().alpha(), 0);
    for (x, y) in [(29, 21), (77, 64), (93, 64)] {
        assert_ne!(original.pixel(x, y).unwrap().alpha(), 0);
        assert_eq!(image.pixel(x, y), original.pixel(x, y));
    }
    w.click_at(point(13., 21.));
    assert!(w.editor.selection.is_empty());
    w.click_at(point(77., 64.));
    assert_eq!(w.editor.selection, [other]);
    w.click("File");
    w.click("Save");
    assert_eq!(project_io::load_animation(&path).unwrap(), edited);
    w.click_edit("Undo");
    assert_eq!(w.editor.project, copies);
    w.click_edit("Undo");
    assert_eq!(w.editor.project, linked);
    w.click_edit("Redo");
    w.click_edit("Redo");
    assert_eq!(w.editor.project, edited);
}
