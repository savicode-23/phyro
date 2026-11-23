use super::*;
use document_core::{Gradient, GradientKind};

#[test]
fn drawing_a_gradient_on_an_unpainted_selection_uses_the_dormant_stops() {
    let mut doc = Document::default();
    let id = doc.add_rectangle([20., 20.], [60., 40.], [255; 4]);
    let object = &mut doc.objects[0];
    let original = Gradient::from_color(
        GradientKind::Linear {
            start: [0.2, 0.5],
            end: [0.8, 0.5],
        },
        [52, 118, 190, 222],
    );
    object.gradient = Some(original.clone());
    object.no_fill = true;
    object.no_stroke = true;
    let mut w = EditorWindow::new(egui::vec2(1120., 800.));
    w.editor
        .replace_project(AnimationProject::from_document(doc.clone()), None);
    let board = w.board();
    w.editor.selection = vec![id];
    pen::chord(&mut w, Key::G, egui::Modifiers::NONE);
    let screen = |x, y| board.min + board.size() * (egui::vec2(x, y) / 128.);
    // Drag from the dormant end handle. It must start a fresh gradient.
    let start = screen(68., 40.);
    w.drag(start, screen(50., 50.) - start);
    let edited = &w.editor.scene().objects[0];
    assert!(!edited.no_fill && edited.no_stroke);
    let gradient = edited.gradient.as_ref().unwrap();
    assert_eq!(gradient.stops, original.stops);
    let GradientKind::Linear { start, end } = gradient.kind else {
        unreachable!()
    };
    assert!(egui::Pos2::from(start).distance(egui::pos2(0.8, 0.5)) < 0.001);
    assert!(egui::Pos2::from(end).distance(egui::pos2(0.5, 0.75)) < 0.001);
    w.click_edit("Undo");
    assert_eq!(w.editor.scene(), &doc);
}

#[test]
fn a_horizontal_paths_radius_handle_uses_its_stroke_thickness() {
    let mut doc = Document::default();
    let id = doc.add_shape(
        [20., 30.],
        [60., 10.],
        [0; 4],
        document_core::Shape::Path {
            closed: false,
            nodes: [[0., 0.5], [1., 0.5]]
                .map(document_core::PathNode::corner)
                .into(),
        },
        1,
    );
    doc.objects[0].stroke_width = 12.;
    doc.objects[0].stroke_gradient = Some(Gradient::from_color(
        GradientKind::Radial {
            center: [0.5, 0.5],
            radius: 0.3,
        },
        [240, 160, 90, 255],
    ));
    let mut w = EditorWindow::new(egui::vec2(1120., 800.));
    w.editor
        .replace_project(AnimationProject::from_document(doc.clone()), None);
    let board = w.board();
    let screen = |x, y| board.min + board.size() * (egui::vec2(x, y) / 128.);
    w.click_at(screen(50., 35.));
    assert_eq!(w.editor.selection, [id]);
    pen::chord(&mut w, Key::G, egui::Modifiers::NONE);
    w.click("Stroke");
    let arm = screen(50., 35. + 0.3 * 12.);
    w.drag(arm, screen(50., 44.) - arm);
    let object = &w.editor.scene().objects[0];
    let GradientKind::Radial { radius, .. } = object.stroke_gradient.as_ref().unwrap().kind else {
        unreachable!()
    };
    assert!((radius - 0.75).abs() < 0.001);
    assert_eq!(object.stroke_width, 12.);
    assert!(object.gradient.is_none());
    w.click_edit("Undo");
    assert_eq!(w.editor.scene(), &doc);
}

#[test]
fn toolbar_switches_the_handles_between_independent_fill_and_stroke_gradients() {
    let mut doc = Document::default();
    let id = doc.add_rectangle([20., 30.], [60., 40.], [27, 145, 180, 255]);
    let original_fill = Gradient::from_color(
        GradientKind::Linear {
            start: [0.1, 0.2],
            end: [0.9, 0.2],
        },
        doc.objects[0].fill,
    );
    doc.objects[0].gradient = Some(original_fill.clone());
    doc.objects[0].stroke = [240, 98, 32, 255];
    doc.objects[0].stroke_width = 0.;
    let mut w = EditorWindow::new(egui::vec2(1120., 800.));
    w.editor
        .replace_project(AnimationProject::from_document(doc.clone()), None);
    let board = w.board();
    let screen = |x, y| board.min + board.size() * (egui::vec2(x, y) / 128.);
    pen::chord(&mut w, Key::G, egui::Modifiers::NONE);
    w.click("Stroke");
    let a = screen(35., 48.);
    let b = screen(65., 48.);
    w.drag(a, b - a);
    assert_eq!(w.editor.selection, [id]);
    let placed = w.editor.scene().clone();
    let o = &placed.objects[0];
    assert_eq!(o.gradient.as_ref(), Some(&original_fill));
    assert!(o.stroke_width > 0.);
    let stroke = o.stroke_gradient.as_ref().unwrap();
    assert_eq!(stroke.stops[0].color, doc.objects[0].stroke);
    w.drag(b, screen(78., 48.) - b);
    assert_ne!(
        w.editor.scene().objects[0].stroke_gradient.as_ref(),
        Some(stroke)
    );
    assert_eq!(
        w.editor.scene().objects[0].gradient.as_ref(),
        Some(&original_fill)
    );
    let resized_stroke = w.editor.scene().objects[0].stroke_gradient.clone();
    w.click("Fill");
    let fill_end = screen(74., 38.);
    w.drag(fill_end, screen(68., 42.) - fill_end);
    assert_eq!(w.editor.scene().objects[0].stroke_gradient, resized_stroke);
    assert_ne!(
        w.editor.scene().objects[0].gradient.as_ref(),
        Some(&original_fill)
    );
    w.click_edit("Undo");
    w.click_edit("Undo");
    assert_eq!(w.editor.scene(), &placed);
    w.click_edit("Undo");
    assert_eq!(w.editor.scene(), &doc);
}

#[test]
fn a_pen_gradient_stays_with_the_pen_across_tool_changes_and_new_paths() {
    let mut w = EditorWindow::new(egui::vec2(1120., 800.));
    w.editor
        .replace_project(AnimationProject::from_document(Document::default()), None);
    pen::chord(&mut w, Key::B, egui::Modifiers::NONE);
    let gradient = Gradient::from_color(
        GradientKind::Radial {
            center: [0.2, 0.6],
            radius: 0.8,
        },
        [145, 21, 114, 244],
    );
    w.editor.style.gradient = Some(std::sync::Arc::new(gradient.clone()));
    pen::chord(&mut w, Key::P, egui::Modifiers::NONE);
    assert!(w.editor.style.no_fill);
    assert_eq!(w.editor.style.gradient.as_deref(), Some(&gradient));
    pen::chord(&mut w, Key::B, egui::Modifiers::NONE);
    assert!(!w.editor.style.no_fill);
    let board = w.board();
    for shift in [0.0_f32, 40.] {
        let screen = |x: f32, y| board.min + board.size() * (egui::vec2(shift + x, y) / 128.);
        for point in [
            screen(12., 30.),
            screen(38., 30.),
            screen(25., 50.),
            screen(12., 30.),
        ] {
            w.click_at(point);
        }
        assert_eq!(
            w.editor.scene().objects.last().unwrap().gradient.as_ref(),
            Some(&gradient)
        );
    }
    assert_eq!(w.editor.scene().objects.len(), 2);
}

#[test]
fn gradient_tool_draws_a_fade_and_lengthens_it_without_moving_the_object() {
    let mut doc = Document::default();
    let ink = [105, 161, 24, 208];
    let id = doc.add_rectangle([20., 30.], [60., 40.], ink);
    let mut w = EditorWindow::new(egui::vec2(1120., 800.));
    w.editor
        .replace_project(AnimationProject::from_document(doc.clone()), None);
    let board = w.board();
    let screen = |x, y| board.min + board.size() * (egui::vec2(x, y) / 128.);
    pen::chord(&mut w, Key::G, egui::Modifiers::NONE);
    assert_eq!(w.editor.tool, Tool::Gradient);
    let a = screen(32., 50.);
    let b = screen(62., 50.);
    w.drag(a, b - a);
    assert_eq!(w.editor.selection, [id]);
    let first = w.editor.scene().clone();
    let gradient = first.objects[0].gradient.as_ref().unwrap();
    assert_eq!(gradient.stops[0].color, ink);
    assert_eq!(gradient.stops[1].color[3], 0);
    let GradientKind::Linear { start, end } = gradient.kind else {
        panic!("expected a linear fill")
    };
    assert!(egui::Pos2::from(start).distance(egui::pos2(0.2, 0.5)) < 0.001);
    assert!(egui::Pos2::from(end).distance(egui::pos2(0.7, 0.5)) < 0.001);
    w.drag(b, screen(92., 62.) - b);
    let edited = &w.editor.scene().objects[0];
    assert_eq!(edited.position, doc.objects[0].position);
    let moved = edited.gradient.as_ref().unwrap();
    let GradientKind::Linear {
        start: new_start,
        end,
    } = moved.kind
    else {
        unreachable!()
    };
    assert_eq!(new_start, start);
    assert!(egui::Pos2::from(end).distance(egui::pos2(1.2, 0.8)) < 0.001);
    assert_eq!(moved.stops, gradient.stops);
    w.click_edit("Undo");
    assert_eq!(w.editor.scene(), &first);
    w.click_edit("Undo");
    assert_eq!(w.editor.scene(), &doc);
}

#[test]
fn center_and_radius_handles_follow_a_rotated_object_in_both_center_out_modes() {
    for kind in [
        GradientKind::Radial {
            center: [0.5; 2],
            radius: 0.3,
        },
        GradientKind::Rectangular {
            center: [0.5; 2],
            radius: 0.3,
        },
    ] {
        let mut doc = Document::default();
        let id = doc.add_rectangle([30., 34.], [55., 36.], [222; 4]);
        let o = &mut doc.objects[0];
        o.rotation = 30.;
        o.gradient = Some(Gradient::from_color(kind, [22, 222, 181, 255]));
        let mut w = EditorWindow::new(egui::vec2(1120., 800.));
        w.editor
            .replace_project(AnimationProject::from_document(doc.clone()), None);
        let board = w.board();
        let screen = |x, y| {
            let [x, y] = doc.objects[0].local_to_world([x, y], 0);
            board.min + board.size() * (egui::vec2(x, y) / 128.)
        };
        w.click_at(screen(0.5, 0.5));
        assert_eq!(w.editor.selection, [id]);
        pen::chord(&mut w, Key::G, egui::Modifiers::NONE);
        let center = screen(0.5, 0.5);
        w.drag(center, screen(0.4, 0.3) - center);
        let edge = screen(0.4, 0.6);
        w.drag(edge, screen(0.4, 0.95) - edge);
        let o = &w.editor.scene().objects[0];
        let g = o.gradient.as_ref().unwrap();
        let (GradientKind::Radial { center, radius }
        | GradientKind::Rectangular { center, radius }) = g.kind
        else {
            unreachable!()
        };
        assert!(egui::Pos2::from(center).distance(egui::pos2(0.4, 0.3)) < 0.001);
        assert!((radius - 0.65).abs() < 0.001);
        assert_eq!(o.position, doc.objects[0].position);
        assert_eq!(o.rotation, 30.);
    }
}
