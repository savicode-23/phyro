use super::*;
use document_core::Shape;

mod continuation;
mod history;

fn key(w: &mut EditorWindow, key: Key) {
    chord(w, key, egui::Modifiers::NONE);
}

pub(super) fn chord(w: &mut EditorWindow, key: Key, modifiers: egui::Modifiers) {
    for pressed in [true, false] {
        w.frame_with_input(egui::RawInput {
            modifiers,
            events: vec![egui::Event::Key {
                key,
                pressed,
                physical_key: Some(key),
                repeat: false,
                modifiers,
            }],
            ..Default::default()
        });
    }
}

#[test]
fn tool_shortcuts_respect_chords_and_focused_fields() {
    use egui::Modifiers as M;
    let mut w = EditorWindow::new(egui::vec2(800., 500.));
    w.frame(Vec::new());
    w.control("Pen");
    w.control("Select");
    w.control("Eyedropper");
    for (key, mods, expected) in [
        (Key::B, M::NONE, Tool::Pen),
        (Key::D, M::NONE, Tool::Eyedropper),
        (Key::S, M::NONE, Tool::Select),
        (Key::N, M::NONE, Tool::Node),
        (Key::R, M::NONE, Tool::Rectangle),
        (Key::E, M::NONE, Tool::Ellipse),
        (Key::F1, M::NONE, Tool::Select),
        (Key::F2, M::NONE, Tool::Node),
        (Key::F4, M::NONE, Tool::Rectangle),
        (Key::F5, M::NONE, Tool::Ellipse),
        (Key::F6, M::SHIFT, Tool::Pen),
        (Key::H, M::NONE, Tool::Pen),
        (Key::V, M::NONE, Tool::Pen),
        (Key::P, M::NONE, Tool::Pencil),
        (Key::F6, M::NONE, Tool::Pencil),
        (Key::B, M::ALT, Tool::Pencil),
    ] {
        chord(&mut w, key, mods);
        assert!(w.editor.tool == expected, "{key:?} {mods:?}");
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Shortcuts.vanim");
    w.editor.path = Some(path.clone());
    key(&mut w, Key::E);
    chord(&mut w, Key::S, M::CTRL | M::COMMAND);
    assert!(path.is_file() && w.editor.tool == Tool::Ellipse);
    for text_key in [Key::T, Key::F8] {
        key(&mut w, text_key);
        assert!(w.editor.tool == Tool::Text && w.editor.text_dialog.is_none());
        key(&mut w, Key::E);
    }
    let hold = w.field_on_row("Hold", " fr");
    w.click_at(hold);
    w.frame(Vec::new());
    assert!(w.ctx.wants_keyboard_input());
    key(&mut w, Key::B);
    key(&mut w, Key::S);
    assert!(w.editor.tool == Tool::Ellipse);
}

pub(super) fn right_click(w: &mut EditorWindow, at: egui::Pos2) {
    w.frame(vec![egui::Event::PointerMoved(at)]);
    for pressed in [true, false] {
        w.frame(vec![egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }]);
    }
}

fn swatch(w: &mut EditorWindow, label: &str, color: [u8; 4]) -> egui::Pos2 {
    let row = w.control(label);
    let color = egui::Color32::from_rgba_unmultiplied(color[0], color[1], color[2], color[3]);
    w.frame(vec![])
        .shapes
        .into_iter()
        .find_map(|s| {
            if let egui::Shape::Rect(r) = s.shape {
                let at = r.rect.center();
                if r.fill == color
                    && (at.y - row.y).abs() < 4.
                    && at.x > row.x
                    && at.x < row.x + 130.
                {
                    return Some(at);
                }
            }
            None
        })
        .expect("visible color swatch")
}

fn on_board(board: egui::Rect, x: f32, y: f32) -> egui::Pos2 {
    board.min + board.size() * egui::vec2(x / 128., y / 128.)
}

fn preview_segment(
    frame: &egui::FullOutput,
    start: egui::Pos2,
    end: egui::Pos2,
) -> [egui::Pos2; 4] {
    frame
        .shapes
        .iter()
        .find_map(|item| match &item.shape {
            egui::Shape::CubicBezier(curve)
                if start.distance(curve.points[0]) < 0.1 && end.distance(curve.points[3]) < 0.1 =>
            {
                Some(curve.points)
            }
            _ => None,
        })
        .expect("visible Pen segment")
}

#[test]
fn shift_mid_pen_drag_keeps_the_incoming_curve_and_steers_the_next_segment() {
    use document_core::path_edit::HandleStyle;
    use egui::{Event, Modifiers};

    let mut w = EditorWindow::with_artboard(egui::vec2(1100., 740.), [128; 2]);
    w.frame(vec![]);
    key(&mut w, Key::B);
    let board = w.board();
    let at = |x, y| on_board(board, x, y);
    let (a, b, c) = (at(20., 48.), at(64., 48.), at(100., 77.));
    let button = |pos, pressed| Event::PointerButton {
        pos,
        pressed,
        button: egui::PointerButton::Primary,
        modifiers: Modifiers::NONE,
    };

    w.click_at(a);
    w.frame(vec![Event::PointerMoved(b), button(b, true)]);
    let smooth = w.frame(vec![Event::PointerMoved(at(74., 48.))]);
    let incoming_curve = preview_segment(&smooth, a, b);
    assert!(incoming_curve[2].distance(at(54., 48.)) < 0.1);

    // The modifier and first motion in the new direction can arrive together.
    let release_tip = at(64., 17.);
    let split = w.frame_with_input(egui::RawInput {
        modifiers: Modifiers::SHIFT,
        events: vec![Event::PointerMoved(release_tip)],
        ..Default::default()
    });
    for tip in [at(54., 48.), release_tip] {
        assert!(
            split.shapes.iter().any(|item| matches!(
                &item.shape, egui::Shape::LineSegment { points: [from, to], .. }
                if from.distance(b) < 0.1 && to.distance(tip) < 0.1
            )),
            "both independent handles must be visible from the anchor"
        );
    }
    assert_eq!(preview_segment(&split, a, b), incoming_curve);
    let unshift = w.frame(vec![]);
    assert_eq!(preview_segment(&unshift, a, b), incoming_curve);
    w.frame(vec![button(release_tip, false)]);
    let hover = w.frame(vec![Event::PointerMoved(c)]);
    assert!(preview_segment(&hover, b, c)[1].distance(release_tip) < 0.1);
    w.drag(c, at(104., 87.) - c);
    right_click(&mut w, c);

    let object = &w.editor.scene().objects[0];
    let Shape::Path {
        nodes,
        closed: false,
    } = &object.shape
    else {
        panic!("expected a finished open path");
    };
    assert_eq!(nodes.len(), 3);
    assert_eq!(nodes[1].handle_style, HandleStyle::Corner);
    assert_eq!(nodes[2].handle_style, HandleStyle::Symmetric);
    for (point, expected) in [
        (nodes[1].incoming, at(54., 48.)),
        (nodes[1].point, b),
        (nodes[1].outgoing, release_tip),
        (nodes[2].incoming, at(96., 67.)),
    ] {
        let [x, y] = object.local_to_world(point, 0);
        assert!(at(x, y).distance(expected) < 0.1);
    }
    assert!(w.editor.tool == Tool::Pen);
    w.editor.restore(false);
    assert!(w.editor.scene().objects.is_empty());
}

#[test]
fn closing_drag_previews_the_last_segment_and_preserves_the_first_segment() {
    use egui::{vec2, Event};
    let button = |pos, pressed| Event::PointerButton {
        pos,
        pressed,
        button: egui::PointerButton::Primary,
        modifiers: egui::Modifiers::NONE,
    };
    for first_handle in [egui::Vec2::ZERO, vec2(24., -8.)] {
        let mut w = EditorWindow::with_artboard(vec2(1100., 740.), [128; 2]);
        key(&mut w, Key::B);
        let board = w.board();
        let at = |x, y| on_board(board, x, y);
        let (a, b, c) = (at(38., 42.), at(88., 46.), at(70., 90.));
        if first_handle == egui::Vec2::ZERO {
            w.click_at(a);
        } else {
            w.drag(a, first_handle);
        }
        w.click_at(b);
        w.click_at(c);
        let press = a + vec2(8., -3.);
        let hover = w.frame(vec![Event::PointerMoved(press)]);
        let opening = preview_segment(&hover, a, b);
        let closing = preview_segment(&hover, c, a);
        w.frame(vec![button(press, true)]);
        assert!(
            w.editor.scene().objects.is_empty(),
            "close committed on press"
        );
        let jitter = w.frame(vec![Event::PointerMoved(press + vec2(4., 2.))]);
        assert_eq!(preview_segment(&jitter, c, a), closing);
        let pull = vec2(16., -28.);
        let held = w.frame(vec![Event::PointerMoved(press + pull)]);
        assert_eq!(preview_segment(&held, a, b), opening);
        assert!(preview_segment(&held, c, a)[2].distance(a - pull) < 0.1);
        assert!(w.editor.scene().objects.is_empty());
        let release = vec2(27., -19.);
        w.frame(vec![button(press + release, false), Event::PointerMoved(b)]);
        let object = &w.editor.scene().objects[0];
        let Shape::Path {
            nodes,
            closed: true,
        } = &object.shape
        else {
            panic!("expected a closed path")
        };
        assert_eq!(nodes.len(), 3);
        let screen = |local| {
            let [x, y] = object.local_to_world(local, 0);
            at(x, y)
        };
        assert!(screen(nodes[0].point).distance(a) < 0.1);
        assert!(screen(nodes[0].incoming).distance(a - release) < 0.1);
        assert!(screen(nodes[0].outgoing).distance(a + first_handle) < 0.1);
        assert_eq!(w.editor.selection, [object.id]);
        assert_eq!(w.editor.tool, Tool::Pen);
        w.click_edit("Undo");
        assert!(w.editor.scene().objects.is_empty());
        assert!(!w.editor.can_undo());
        w.click_edit("Redo");
        assert_eq!(w.editor.scene().objects.len(), 1);
    }
}

#[test]
fn eyedropper_samples_composited_ink_on_and_off_the_zoomed_page_and_keeps_selection() {
    let mut doc = Document::default();
    let target = doc.add_rectangle([82., 40.], [15.; 2], [55, 66, 77, 255]);
    let initial_stroke = doc.objects[0].stroke;
    doc.add_rectangle([30., 25.], [26.; 2], [0, 0, 255, 128]);
    doc.add_rectangle([34., 32.], [26.; 2], [255, 0, 0, 128]);
    doc.add_rectangle([-15., 40.], [12.; 2], [0, 255, 0, 192]);
    let pixel = renderer::render(&doc, 0)
        .unwrap()
        .pixel(40, 40)
        .unwrap()
        .demultiply();
    let expected = [pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()];
    let mut w = EditorWindow::new(egui::vec2(800., 500.));
    w.editor
        .replace_project(AnimationProject::from_document(doc), None);
    w.editor.selection = vec![target];
    w.editor.canvas.set_checkerboard(true);
    w.frame(vec![]);
    w.click("Eyedropper");
    let original = w.board();
    w.frame(vec![egui::Event::PointerMoved(on_board(
        original, 18., 60.,
    ))]);
    w.frame(vec![egui::Event::Zoom(1.25)]);
    let board = w.board();
    assert!(board.width() > original.width());
    w.click_at(on_board(board, 40., 40.));
    assert_eq!(w.editor.style.fill, expected);
    assert_eq!(w.editor.scene().objects[0].fill, expected);
    assert_eq!(w.editor.selection, [target]);
    assert!(w.editor.tool == Tool::Eyedropper);
    let outside = on_board(board, -7., 45.);
    w.frame(vec![egui::Event::PointerMoved(outside)]);
    for pressed in [true, false] {
        w.frame_with_input(egui::RawInput {
            modifiers: egui::Modifiers::SHIFT,
            events: vec![egui::Event::PointerButton {
                pos: outside,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::SHIFT,
            }],
            ..Default::default()
        });
    }
    let obj = &w.editor.scene().objects[0];
    assert_eq!(obj.fill, expected);
    assert_eq!(obj.stroke, [0, 255, 0, 192]);
    assert!(obj.stroke_width > 0.);
    w.editor.restore(false);
    assert_eq!(w.editor.scene().objects[0].stroke, initial_stroke);
    w.click_at(on_board(board, 72., 71.));
    assert_eq!(w.editor.scene().objects[0].fill, [0; 4]);
    assert_eq!(w.editor.selection, [target]);
}

#[test]
fn eyedropper_without_selection_updates_a_previously_used_pens_remembered_color() {
    let mut doc = Document::default();
    let sampled = [56, 199, 106, 255];
    doc.add_rectangle([12., 10.], [24.; 2], sampled);
    let mut w = EditorWindow::new(egui::vec2(800., 500.));
    w.editor
        .replace_project(AnimationProject::from_document(doc), None);
    key(&mut w, Key::B);
    w.editor.style.fill = [195, 31, 88, 255];
    let board = w.board();
    key(&mut w, Key::D);
    assert!(w.editor.tool == Tool::Eyedropper);
    w.editor.style.fill = sampled;
    w.click_at(on_board(board, 20., 18.));
    assert!(w.editor.selection.is_empty());
    assert!(!w.editor.history.can_undo());
    key(&mut w, Key::B);
    for (x, y) in [(64., 62.), (92., 62.), (85., 94.)] {
        w.click_at(on_board(board, x, y));
    }
    right_click(&mut w, on_board(board, 85., 94.));
    assert_eq!(w.editor.scene().objects.len(), 2);
    assert_eq!(w.editor.scene().objects[1].fill, sampled);
}

#[test]
fn consecutive_pen_paths_keep_the_tool_and_paint_and_undo_individually() {
    let mut w = EditorWindow::with_artboard(egui::vec2(1100., 740.), [128; 2]);
    let mut empty = w.editor.project.clone();
    empty.grid.spacing = 64.;
    w.editor.replace_project(empty.clone(), None);
    let _ = w.board();
    key(&mut w, Key::B);
    let fill = [26, 183, 112, 255];
    let stroke = [51, 76, 198, 255];
    w.editor.style.fill = fill;
    w.editor.style.stroke = stroke;
    w.editor.style.stroke_width = 6.5;
    let board = w.board();
    let a = on_board(board, 32., 32.);
    let b = on_board(board, 96., 32.);
    let c = on_board(board, 96., 96.);
    for p in [a, b, c] {
        w.click_at(p);
    }
    right_click(&mut w, a); // Clicking directly on the start still finishes open.
    assert!(w.editor.tool == Tool::Pen);
    let first = w.editor.project.clone();
    let drawing = w.editor.scene();
    assert_eq!(drawing.objects.len(), 1);
    let object = &drawing.objects[0];
    assert_eq!(w.editor.selection, [object.id]);
    assert_eq!(
        (object.fill, object.stroke, object.stroke_width),
        (fill, stroke, 6.5)
    );
    let Shape::Path {
        nodes,
        closed: false,
    } = &object.shape
    else {
        panic!("expected an open path")
    };
    assert_eq!(nodes.len(), 3);
    let last = object.local_to_world(nodes.last().unwrap().point, 0);
    assert!((last[0] - 96.).abs() < 0.01 && (last[1] - 96.).abs() < 0.01);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("open.svg");
    project_io::export_svg(&path, drawing, 0, false).unwrap();
    let imported = project_io::import_svg(&path).unwrap();
    assert!(
        matches!(&imported.objects[0].shape, Shape::Path { closed:false, nodes } if nodes.len()==3)
    );
    for doc in [drawing, &imported] {
        let pix = renderer::render(doc, 0).unwrap();
        let rgba = |x, y| {
            let p = pix.pixel(x, y).unwrap();
            [p.red(), p.green(), p.blue(), p.alpha()]
        };
        assert_eq!(rgba(77, 49), fill);
        assert_eq!(rgba(62, 32), stroke);
        assert_eq!(
            rgba(62, 64),
            [0; 4],
            "fill closes implicitly, the stroke ends at the last anchor"
        );
    }

    let board = w.board();
    let a = on_board(board, 38., 52.);
    let b = on_board(board, 80., 55.);
    let c = on_board(board, 62., 94.);
    let tip = egui::vec2(9., -7.);
    w.drag(a, tip);
    w.click_at(b);
    w.click_at(c);
    w.editor.style.snap.enabled = true;
    w.editor.style.snap.anchors = false;
    w.editor.style.snap.bounds = false;
    let near = a + egui::vec2(10., -2.);
    let hover = w.frame(vec![egui::Event::PointerMoved(near)]);
    assert!(hover
        .shapes
        .iter()
        .any(|s| matches!(&s.shape, egui::Shape::Circle(r)
        if r.center.distance(a)<0.1 && r.radius==9.)));
    assert!(hover.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::CubicBezier(p)
        if p.points[0].distance(c)<0.1 && p.points[2].distance(a-tip)<0.1 && p.points[3].distance(a)<0.1)));
    w.click_at(near);
    assert!(w.editor.tool == Tool::Pen);
    let next = &w.editor.scene().objects[1];
    assert!(
        matches!(&next.shape, Shape::Path{closed:true,nodes} if nodes.len()==3 && nodes[0].point!=nodes[0].incoming)
    );
    assert_eq!(
        (next.fill, next.stroke, next.stroke_width),
        (fill, stroke, 6.5)
    );
    assert!(w.editor.style.snap.enabled);
    let second = w.editor.project.clone();

    w.editor.style.snap.enabled = false;
    w.editor.style.fill = [227, 161, 30, 215];
    w.editor.style.stroke = [0; 4];
    w.editor.style.stroke_width = 0.;
    let board = w.board();
    let a = on_board(board, 34., 60.);
    let b = on_board(board, 85., 60.);
    w.drag(a, egui::vec2(16., -42.));
    w.click_at(b);
    key(&mut w, Key::Enter);
    assert!(w.editor.tool == Tool::Pen);
    let last = &w.editor.scene().objects[2];
    assert_eq!(
        (last.fill, last.stroke, last.stroke_width),
        ([227, 161, 30, 215], [0; 4], 0.)
    );
    assert!(
        matches!(&last.shape, Shape::Path{closed:false,nodes} if nodes.len()==2 && nodes[0].outgoing!=nodes[0].point)
    );
    let mut curve = w.editor.scene().clone();
    curve.objects.retain(|o| o.id == last.id);
    let filled = renderer::render(&curve, 0).unwrap();
    assert!(filled.pixels().iter().any(|p| p.alpha() == 215));
    w.editor.restore(false);
    assert_eq!(w.editor.project, second);
    w.editor.restore(false);
    assert_eq!(w.editor.project, first);
    w.editor.restore(false);
    assert_eq!(w.editor.project, empty);
    assert!(w.editor.tool == Tool::Pen);
    assert_eq!(w.editor.style.fill, [227, 161, 30, 215]);
}

#[test]
fn copying_and_pasting_paint_while_chaining_closed_pen_paths() {
    let mut w = EditorWindow::new(egui::vec2(1100., 740.));
    let board = w.board();
    key(&mut w, Key::B);
    let draw = |w: &mut EditorWindow, x: f32| {
        for (dx, y) in [(0., 30.), (24., 40.), (6., 64.), (0., 30.)] {
            w.click_at(on_board(board, x + dx, y));
        }
    };
    w.editor.style.fill = [51, 102, 180, 210];
    w.editor.style.stroke = [101, 30, 72, 255];
    w.editor.style.stroke_width = 3.5;
    draw(&mut w, 12.);
    let copy = w
        .frame(vec![egui::Event::Copy])
        .platform_output
        .commands
        .into_iter()
        .find_map(|c| match c {
            egui::OutputCommand::CopyText(text) => Some(text),
            _ => None,
        })
        .unwrap();
    w.editor.style.fill = [230, 145, 80, 255];
    w.editor.style.stroke_width = 1.;
    draw(&mut w, 55.);
    let before = w.editor.scene().clone();
    let target = before.objects[1].id;
    w.frame_with_input(egui::RawInput {
        modifiers: egui::Modifiers::CTRL | egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        events: vec![egui::Event::Paste(copy)],
        ..Default::default()
    });
    let styled = w.editor.scene().clone();
    let paint = |o: &document_core::VectorObject| (o.fill, o.stroke, o.stroke_width);
    assert_eq!(styled.objects.len(), 2);
    assert_eq!(paint(&styled.objects[1]), paint(&before.objects[0]));
    assert_eq!(styled.objects[1].shape, before.objects[1].shape);
    assert_eq!(w.editor.selection, [target]);
    assert!(w.editor.tool == Tool::Pen);
    draw(&mut w, 89.);
    assert_eq!(
        paint(&w.editor.scene().objects[2]),
        paint(&before.objects[0])
    );
    let cmd = egui::Modifiers::CTRL | egui::Modifiers::COMMAND;
    chord(&mut w, Key::Z, cmd);
    assert_eq!(w.editor.scene(), &styled);
    chord(&mut w, Key::Z, cmd);
    assert_eq!(w.editor.scene(), &before);
}

#[test]
fn appearance_hex_and_channel_pickers_edit_the_pen_selection_without_switching_tools() {
    let mut doc = Document::default();
    let a = doc.add_rectangle([15., 15.], [30.; 2], [36, 87, 112, 80]);
    let sampled = [180, 92, 42, 255];
    doc.add_rectangle([60., 65.], [25.; 2], sampled);
    let mut w = EditorWindow::new(egui::vec2(1100., 740.));
    w.editor
        .replace_project(AnimationProject::from_document(doc), None);
    key(&mut w, Key::B);
    w.editor.selection = vec![a];
    w.editor.sync_paint();
    w.frame(vec![]);
    let board = w.board();
    w.click("245770");
    let cmd = egui::Modifiers::COMMAND | egui::Modifiers::CTRL;
    chord(&mut w, Key::A, cmd);
    w.frame(vec![egui::Event::Paste("#A051C4".into())]);
    key(&mut w, Key::Enter);
    assert_eq!(w.editor.scene().objects[0].fill, [160, 81, 196, 80]);
    let pick = |w: &mut EditorWindow, channel: &str| {
        w.frame(vec![]);
        let id = egui::Id::new((
            "sample-property-color",
            egui::LayerId::background(),
            channel,
        ));
        let at = w
            .ctx
            .read_response(id)
            .expect("property eyedropper button")
            .rect
            .center();
        w.click_at(at);
    };
    pick(&mut w, "Fill");
    key(&mut w, Key::Escape);
    assert_eq!(w.editor.selection, [a]);
    assert!(w.editor.canvas.property_pick.is_none());
    pick(&mut w, "Stroke");
    assert!(w.editor.canvas.property_pick.is_some());
    w.click_at(on_board(board, 75., 75.));
    assert_eq!(w.editor.scene().objects[0].stroke, sampled);
    assert_eq!(w.editor.scene().objects[0].fill, [160, 81, 196, 80]);
    assert!(w.editor.canvas.property_pick.is_none());
    assert!(w.editor.tool == Tool::Pen);
    assert_eq!(w.editor.selection, [a]);
    w.click_at(on_board(board, 75., 75.));
    assert!(w.editor.selection.is_empty());
    assert!(w.editor.canvas.pen_in_progress());
}

#[test]
fn pencil_strokes_use_their_own_paint_and_remain_editable_in_properties() {
    let mut w = EditorWindow::new(egui::vec2(1100., 740.));
    let board = w.board();
    key(&mut w, Key::B);
    let pen_fill = [10, 100, 220, 255];
    w.editor.style.fill = pen_fill;
    w.editor.style.stroke = [0; 4];
    w.editor.style.stroke_width = 0.;
    key(&mut w, Key::P);
    assert!(w.editor.style.no_fill);
    assert_eq!(w.editor.style.fill, pen_fill);
    assert!(w.editor.style.stroke[3] > 0 && w.editor.style.stroke_width > 0.);
    let draw = |w: &mut EditorWindow, start: f32| {
        let at = on_board(board, start, 36.);
        w.drag(at, on_board(board, start + 25., 71.) - at);
    };
    draw(&mut w, 12.);
    let stroke = &w.editor.scene().objects[0];
    assert_eq!(w.editor.selection, [stroke.id]);
    assert!(stroke.no_fill && !stroke.no_stroke);
    let original = stroke.stroke;
    w.click(&format!(
        "{:02X}{:02X}{:02X}",
        original[0], original[1], original[2]
    ));
    chord(
        &mut w,
        Key::A,
        egui::Modifiers::COMMAND | egui::Modifiers::CTRL,
    );
    w.frame(vec![egui::Event::Paste("A05A12".into())]);
    key(&mut w, Key::Enter);
    let ink = [160, 90, 18, 255];
    assert_eq!(w.editor.scene().objects[0].stroke, ink);
    assert!(w.editor.tool == Tool::Pencil);
    key(&mut w, Key::B);
    assert_eq!(w.editor.style.fill, pen_fill);
    assert!(!w.editor.style.no_fill);
    assert_eq!(w.editor.style.stroke, [0; 4]);
    assert_eq!(w.editor.style.stroke_width, 0.);
    key(&mut w, Key::P);
    draw(&mut w, 63.);
    assert_eq!(w.editor.scene().objects[1].stroke, ink);
    assert!(w.editor.scene().objects[1].no_fill);
    assert_eq!(w.editor.scene().objects[1].fill, pen_fill);
}

#[test]
fn switching_panels_with_a_fill_picker_open_returns_shortcuts_to_the_editor() {
    let fill = [230, 45, 12, 255];
    let mut doc = Document::default();
    let id = doc.add_rectangle([12.; 2], [32.; 2], fill);
    let mut w = EditorWindow::new(egui::vec2(1100., 740.));
    w.ctx.style_mut(|style| style.animation_time = 0.);
    w.editor
        .replace_project(AnimationProject::from_document(doc), None);
    w.editor.project.clips[0].set_length(10).unwrap();
    w.editor.selection = vec![id];

    let chip = swatch(&mut w, "Fill", fill);
    w.click_at(chip);
    w.control("Wheel");
    key(&mut w, Key::B);
    key(&mut w, Key::Period);
    assert_eq!(w.editor.tool, Tool::Select);
    assert_eq!(w.editor.timeline.frame, 0);

    w.click("Layers");
    assert_eq!(w.editor.inspector_tab, crate::inspector::Tab::Layers);
    key(&mut w, Key::B);
    key(&mut w, Key::Period);
    assert_eq!(w.editor.tool, Tool::Pen);
    assert_eq!(w.editor.timeline.frame, 1);
    assert!(!w.ctx.memory(|m| m.any_popup_open()));
}

#[test]
fn wheel_and_triangle_drag_repaint_the_selection_with_separate_undo_steps() {
    let original = [255, 0, 0, 127];
    let mut doc = Document::default();
    let id = doc.add_rectangle([12.; 2], [32.; 2], original);
    let mut w = EditorWindow::new(egui::vec2(1000., 700.));
    w.ctx.style_mut(|style| style.animation_time = 0.);
    w.editor
        .replace_project(AnimationProject::from_document(doc), None);
    w.editor.selection = vec![id];
    let at = swatch(&mut w, "Fill", original);
    w.click_at(at);
    w.control("Wheel");
    let [hue, white, black] = w
        .frame(vec![])
        .shapes
        .iter()
        .find_map(|clipped| {
            let egui::Shape::Mesh(mesh) = &clipped.shape else {
                return None;
            };
            let [.., hue, white, black] = mesh.vertices.as_slice() else {
                return None;
            };
            (hue.color == egui::Color32::RED
                && white.color == egui::Color32::WHITE
                && black.color == egui::Color32::BLACK)
                .then_some([hue.pos, white.pos, black.pos])
        })
        .expect("visible color wheel and triangle");
    let center = hue + (white - hue + (black - hue)) / 3.;
    let ring_top = center + (hue - center) * 1.18;
    w.drag(ring_top, (center - ring_top) * 2.);
    let cyan = [0, 255, 255, 127];
    assert_eq!(w.editor.scene().objects[0].fill, cyan);
    w.drag(center, black - center + egui::vec2(5., 5.));
    assert_eq!(w.editor.scene().objects[0].fill, [0, 0, 0, 127]);
    w.click_at(at);
    w.click_edit("Undo");
    assert_eq!(w.editor.scene().objects[0].fill, cyan);
    w.click_edit("Undo");
    assert_eq!(w.editor.scene().objects[0].fill, original);
}

#[test]
fn properties_repaint_the_finished_path_until_the_next_pen_stroke_starts() {
    let mut w = EditorWindow::new(egui::vec2(1100., 740.));
    w.ctx.style_mut(|s| s.animation_time = 0.);
    let board = w.board();
    key(&mut w, Key::B);
    w.editor.style.stroke_width = 2.;
    let triangle = [(22., 24.), (72., 24.), (72., 62.), (22., 24.)];
    for (x, y) in triangle {
        w.click_at(on_board(board, x, y));
    }
    let id = w.editor.scene().objects[0].id;
    assert_eq!(w.editor.selection, [id]);
    w.control("Appearance");
    for (label, rgba) in [
        ("Fill", w.editor.style.fill),
        ("Stroke", w.editor.style.stroke),
    ] {
        let red = rgba[0];
        let swatch = swatch(&mut w, label, rgba);
        w.click_at(swatch);
        let channel = w.control(&format!("R {red}"));
        w.drag(channel, egui::vec2(26., 0.));
        w.click_at(swatch);
        let path = &w.editor.scene().objects[0];
        let color = if label == "Fill" {
            path.fill
        } else {
            path.stroke
        };
        assert!(
            color[0] > red && color[3] == 255,
            "{label} picker must repaint the path"
        );
    }
    let rgba = w.editor.style.fill;
    let fill = swatch(&mut w, "Fill", rgba);
    right_click(&mut w, fill);
    w.click("None");
    assert!(w.editor.scene().objects[0].no_fill);
    assert_eq!(w.editor.scene().objects[0].fill, rgba);
    let width = w.field_on_row("Stroke width", " px");
    w.drag(width, egui::vec2(18., 0.));
    let edited_width = w.editor.scene().objects[0].stroke_width;
    assert!(edited_width > 2.);
    assert_eq!(w.editor.style.stroke_width, edited_width);
    let rgba = w.editor.style.stroke;
    let stroke = swatch(&mut w, "Stroke", rgba);
    right_click(&mut w, stroke);
    w.click("None");
    assert!(w.editor.scene().objects[0].no_stroke);
    assert_eq!(w.editor.scene().objects[0].stroke, rgba);
    w.click_edit("Undo");
    assert!(!w.editor.scene().objects[0].no_stroke);
    assert_eq!(w.editor.style.stroke, w.editor.scene().objects[0].stroke);
    assert_eq!(w.editor.selection, [id]);
    assert!(w.editor.tool == Tool::Pen);

    w.click_at(on_board(board, 23., 84.));
    assert!(w.editor.selection.is_empty());
    w.control("Tool paint");
    let width = w.field_on_row("Stroke width", " px");
    w.drag(width, egui::vec2(32., 0.));
    assert_eq!(w.editor.scene().objects[0].stroke_width, edited_width);
    let next_width = w.editor.style.stroke_width;
    w.click_at(on_board(board, 75., 92.));
    key(&mut w, Key::Enter);
    assert_eq!(w.editor.scene().objects[1].stroke_width, next_width);
    assert!(w.editor.scene().objects[1].no_fill);
    assert!(next_width > edited_width);
    assert!(w.editor.tool == Tool::Pen);
}

#[test]
fn cancelling_or_finishing_a_short_draft_leaves_no_stray_anchors() {
    let mut w = EditorWindow::new(egui::vec2(1100., 740.));
    let board = w.board();
    key(&mut w, Key::B);
    let a = on_board(board, 32., 32.);
    let b = on_board(board, 96., 32.);
    let c = on_board(board, 96., 96.);
    w.click_at(a);
    right_click(&mut w, c);
    assert!(w.editor.scene().objects.is_empty());
    w.click_at(a);
    w.click_at(b);
    key(&mut w, Key::Escape);
    assert!(w.editor.scene().objects.is_empty());
    w.click_at(b);
    w.click_at(c);
    right_click(&mut w, a);
    let shape = &w.editor.scene().objects[0].shape;
    assert!(matches!(shape, Shape::Path{nodes,closed:false} if nodes.len()==2));
}

#[test]
fn returning_to_pen_restores_its_paint_on_the_current_layer() {
    let mut w = EditorWindow::new(egui::vec2(1100., 740.));
    let mut doc = Document::default();
    let layer = crate::commands::add_layer(&mut doc);
    let other = doc.add_shape(
        [70., 75.],
        [22., 20.],
        [140, 95, 190, 255],
        Shape::Ellipse,
        layer,
    );
    doc.objects[0].stroke = [38, 80, 210, 220];
    doc.objects[0].stroke_width = 11.;
    w.editor
        .replace_project(AnimationProject::from_document(doc), None);
    let _ = w.board();
    key(&mut w, Key::B);
    let ink = ([94, 169, 23, 235], [235, 197, 42, 197], 3.75);
    (
        w.editor.style.fill,
        w.editor.style.stroke,
        w.editor.style.stroke_width,
    ) = ink;
    let board = w.board();
    w.click_at(on_board(board, 25., 25.));
    w.click_at(on_board(board, 52., 34.));
    right_click(&mut w, on_board(board, 74., 54.));
    key(&mut w, Key::S);
    let board = w.board();
    w.click_at(on_board(board, 81., 85.));
    assert_eq!(w.editor.selection, [other]);
    assert_eq!(w.editor.style.stroke_width, 11.);
    assert_eq!(w.editor.style.layer, layer);
    w.editor.project.grid.spacing = 5.;
    key(&mut w, Key::B);
    assert_eq!(
        (
            w.editor.style.fill,
            w.editor.style.stroke,
            w.editor.style.stroke_width
        ),
        ink
    );
    assert_eq!(w.editor.project.grid.spacing, 5.);
    let board = w.board();
    w.click_at(on_board(board, 19., 83.));
    w.click_at(on_board(board, 43., 93.));
    key(&mut w, Key::Enter);
    let drawn = w.editor.scene().objects.last().unwrap();
    assert_eq!(drawn.layer, layer);
    assert_eq!((drawn.fill, drawn.stroke, drawn.stroke_width), ink);
    assert!(w.editor.tool == Tool::Pen);
}

#[test]
fn reusing_a_recent_color_on_a_selection_changes_only_that_drawing_channel() {
    let previous = [210, 50, 160, 75];
    let unrelated = [64, 96, 28, 255];
    let mut doc = Document::default();
    let object = doc.add_rectangle([15.; 2], [20.; 2], unrelated);
    doc.objects[0].name = "Background prop".into();
    doc.objects[0].stroke_width = 17.;
    let mut w = EditorWindow::new(egui::vec2(1100., 780.));
    w.ctx.style_mut(|s| s.animation_time = 0.);
    w.editor
        .replace_project(AnimationProject::from_document(doc), None);
    crate::color_picker::recent::restore(&w.ctx, &[previous]);
    w.editor.panel_layout = super::super::panel_layout::PanelLayout::stacked();
    let board = w.board();
    w.click_at(on_board(board, 23., 23.));
    assert_eq!(w.editor.selection, [object]);
    let chip = swatch(&mut w, "Fill", unrelated);
    w.click_at(chip);
    let row = w
        .frame(vec![])
        .shapes
        .into_iter()
        .find_map(|s| match s.shape {
            egui::Shape::Text(t) if t.galley.text() == "Recent" => {
                Some(t.galley.rect.translate(t.pos.to_vec2()))
            }
            _ => None,
        })
        .unwrap();
    w.click_at(row.right_center() + egui::vec2(12., 0.));
    assert_eq!(w.editor.scene().objects[0].fill, previous);
    w.click_at(chip);
    key(&mut w, Key::B);
    assert_eq!(w.editor.style.fill, previous);
    assert_eq!(w.editor.style.stroke_width, 1.5);
    for (x, y) in [(50., 35.), (82., 35.), (70., 55.), (50., 35.)] {
        w.click_at(on_board(board, x, y));
    }
    let path = w.editor.scene().objects.last().unwrap();
    assert_eq!(path.fill, previous);
    assert_eq!(path.stroke_width, 1.5);
    key(&mut w, Key::P);
    w.editor.style.stroke_width = 3.75;
    w.editor.style.stroke = [37, 112, 70, 211];
    w.click("Background prop");
    assert_eq!(w.editor.selection, [object]);
    assert_eq!(w.editor.tool, Tool::Select);
    w.control("Appearance");
    let selected_row = w.control("Background prop");
    super::path_ops::shift_click(&mut w, selected_row);
    assert!(w.editor.selection.is_empty());
    w.control("Tool paint");
    assert_eq!(w.editor.style.stroke_width, 3.75);
    key(&mut w, Key::P);
    let tip = on_board(board, 62., 70.);
    w.drag(tip, on_board(board, 80., 89.) - tip);
    let pencil = w.editor.scene().objects.last().unwrap();
    assert!(pencil.no_fill);
    assert_eq!(pencil.stroke, [37, 112, 70, 211]);
    assert_eq!(pencil.stroke_width, 3.75);
}
