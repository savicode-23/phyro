use super::*;
use crate::timeline::Timeline;

mod group_stretch;
mod input_modifiers;
mod marquee;
mod patch;
mod resize_modifiers;
mod split;
mod zoom_input;

struct CanvasSession {
    ctx: egui::Context,
    canvas: Canvas,
    doc: Document,
    selected: Vec<u64>,
    tool: Tool,
    timeline: Timeline,
    layer: u64,
    modifiers: egui::Modifiers,
    edit_motion: bool,
    snap: SnapSettings,
}

#[test]
fn drag_texture_shares_the_staged_drawing_snapshot() {
    let mut s = CanvasSession::new(Tool::Rectangle);
    let mut draft = s.doc.clone();
    draft.add_rectangle([5.; 2], [20.; 2], [255; 4]);
    let snapshot = std::sync::Arc::new(draft);
    let _ = s.ctx.run(Default::default(), |ctx| {
        s.canvas
            .artwork_texture(ctx, &s.doc, 0, 1., Some(&snapshot))
            .unwrap();
    });
    assert!(std::sync::Arc::ptr_eq(
        &snapshot,
        &s.canvas.texture.as_ref().unwrap().document
    ));
    assert!(s.doc.objects.is_empty());
}

impl CanvasSession {
    fn new(tool: Tool) -> Self {
        Self {
            ctx: egui::Context::default(),
            canvas: Canvas::default(),
            doc: Document::default(),
            selected: Vec::new(),
            tool,
            timeline: Timeline::default(),
            layer: 1,
            modifiers: egui::Modifiers::NONE,
            edit_motion: false,
            snap: Default::default(),
        }
    }

    fn input(&mut self, event: egui::Event) -> bool {
        self.paint(egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.0))),
            events: vec![event],
            modifiers: self.modifiers,
            ..Default::default()
        })
        .0
    }

    fn paint(&mut self, input: egui::RawInput) -> (bool, egui::FullOutput) {
        let mut changed = false;
        let ctx = self.ctx.clone();
        let mut authored = std::sync::Arc::new(std::mem::take(&mut self.doc));
        let output = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                changed |= self
                    .canvas
                    .show(
                        ui,
                        &mut authored,
                        self.timeline.frame,
                        self.tool,
                        &mut self.selected,
                        &CanvasStyle {
                            edit_motion: self.edit_motion,
                            key_mode: self.timeline.key_mode,
                            fill: [255; 4],
                            gradient: None,
                            stroke_gradient: None,
                            no_fill: false,
                            no_stroke: false,
                            gradient_channel: crate::canvas::PaintChannel::Fill,
                            cap: Default::default(),
                            join: Default::default(),
                            stroke: [0; 4],
                            stroke_width: 0.0,
                            layer: self.layer,
                            snap: self.snap,
                        },
                        !self.timeline.is_playing(),
                    )
                    .unwrap();
            });
        });
        self.doc = std::sync::Arc::unwrap_or_clone(authored);
        (changed, output)
    }

    fn button(&mut self, pos: Pos2, pressed: bool) -> bool {
        self.input(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: self.modifiers,
        })
    }

    fn click(&mut self, pos: Pos2) -> bool {
        self.input(egui::Event::PointerMoved(pos));
        let press = self.button(pos, true);
        self.button(pos, false) || press
    }

    fn drag(&mut self, from: Pos2, to: Pos2) -> bool {
        let before = self.doc.clone();
        self.input(egui::Event::PointerMoved(from));
        assert!(!self.button(from, true));
        assert!(!self.input(egui::Event::PointerMoved(to)));
        assert_eq!(self.doc, before, "preview must leave authored data intact");
        self.button(to, false)
    }
}

#[test]
fn wheel_zoom_tracks_the_pointer_and_refines_the_texture_once_after_scrolling() {
    let mut session = CanvasSession::new(Tool::Select);
    session.doc.add_rectangle([20.; 2], [30.; 2], [255; 4]);
    let pointer = egui::pos2(131., 137.);
    let paint = |s: &mut CanvasSession, tick: usize, events| {
        s.paint(egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.))),
            time: Some(tick as f64 / 60.),
            events,
            ..Default::default()
        })
        .1
    };
    paint(&mut session, 0, vec![egui::Event::PointerMoved(pointer)]);
    let first = session.canvas.texture.as_ref().unwrap();
    let initial_scale = first.scale;
    let id = first.handle.id();
    let uploaded = |out: &egui::FullOutput| {
        out.textures_delta
            .set
            .iter()
            .any(|(texture, _)| *texture == id)
    };
    let offset = pointer - egui::pos2(200., 200.);
    for tick in 1..=16 {
        let old_zoom = session.canvas.zoom_steps;
        let out = paint(
            &mut session,
            tick,
            vec![egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0., 7.),
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(session.canvas.zoom_steps > old_zoom);
        let factor = session.canvas.zoom_steps.exp2();
        assert!(((offset - session.canvas.pan) / factor - offset).length() < 0.001);
        assert!(
            !uploaded(&out),
            "Redundant raster upload during wheel input"
        );
    }
    session.doc.objects[0].fill = [45, 60, 90, 255];
    assert!(
        uploaded(&paint(&mut session, 17, vec![])),
        "Edits must remain visible during zoom"
    );
    assert_eq!(
        session.canvas.texture.as_ref().unwrap().scale,
        initial_scale
    );
    assert!(
        !uploaded(&paint(&mut session, 30, vec![])),
        "Starting a refinement must leave the input frame free"
    );
    assert!(session.canvas.refinement.is_some());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !uploaded(&paint(&mut session, 31, vec![])) {
        assert!(
            std::time::Instant::now() < deadline,
            "Settled view must sharpen"
        );
        std::thread::yield_now();
    }
    let target = initial_scale * session.canvas.zoom_steps.exp2();
    assert!((session.canvas.texture.as_ref().unwrap().scale - target).abs() < 0.0001);
    assert!(!uploaded(&paint(&mut session, 32, vec![])));
}

#[test]
fn a_completed_zoom_cannot_replace_new_artwork() {
    let mut session = CanvasSession::new(Tool::Select);
    session.doc.add_rectangle([10.; 2], [30.; 2], [255; 4]);
    let ctx = session.ctx.clone();
    let canvas = &mut session.canvas;
    let draw = &mut session.doc;
    let _ = ctx.run(
        egui::RawInput {
            time: Some(2.),
            ..Default::default()
        },
        |_| {},
    );
    canvas.frame_texture(&ctx, draw, 0, 1.).unwrap();
    canvas.sharpen_at = Some(1.);
    canvas.frame_texture(&ctx, draw, 0, 2.).unwrap();
    assert!(canvas.refinement.is_some());
    draw.objects[0].fill = [10, 80, 255, 220];
    canvas.frame_texture(&ctx, draw, 0, 2.).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while canvas.refinement.is_some() {
        canvas.frame_texture(&ctx, draw, 0, 2.).unwrap();
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    let handle = &canvas.texture.as_ref().unwrap().handle;
    let out = ctx.run(egui::RawInput::default(), |_| {});
    let (_, update) = out
        .textures_delta
        .set
        .iter()
        .rev()
        .find(|(id, _)| *id == handle.id())
        .unwrap();
    let pixels = renderer::render_scaled(draw, 0, 2.).unwrap();
    let image = egui::ColorImage::from_rgba_premultiplied(handle.size(), pixels.data());
    let egui::ImageData::Color(actual) = &update.image else {
        panic!("expected artwork texture")
    };
    assert_eq!(actual.pixels, image.pixels);
}

#[test]
fn dragging_one_anchor_moves_the_selected_node_range_with_its_handles() {
    let mut session = CanvasSession::new(Tool::Pen);
    let [a, b, c] = [
        egui::pos2(110., 180.),
        egui::pos2(185., 105.),
        egui::pos2(260., 180.),
    ];
    for p in [a, b, c, a] {
        session.click(p);
    }
    session.tool = Tool::Node;
    session.click(a);
    session.modifiers = egui::Modifiers::SHIFT;
    session.click(b);
    session.modifiers = egui::Modifiers::NONE;
    let before = session.doc.objects[0].shape.contours()[0].clone();
    assert!(session.drag(a, a + egui::vec2(18., 24.)));
    let after = &session.doc.objects[0].shape.contours()[0];
    let delta = Pos2::from(after[0].point) - Pos2::from(before[0].point);
    assert!(delta.length_sq() > 0.);
    for i in [0, 1] {
        assert!(
            ((Pos2::from(after[i].point) - Pos2::from(before[i].point)) - delta).length() < 0.001
        );
        assert!(
            ((Pos2::from(after[i].outgoing) - Pos2::from(before[i].outgoing)) - delta).length()
                < 0.001
        );
    }
    assert_eq!(before[2], after[2]);
}

#[test]
fn shift_drag_pulls_handles_from_a_straight_corner() {
    let mut s = CanvasSession::new(Tool::Pen);
    let a = egui::pos2(110., 180.);
    let b = egui::pos2(185., 105.);
    let c = egui::pos2(260., 180.);
    for p in [a, b, c] {
        assert!(!s.click(p));
    }
    assert!(s.click(a));
    let before = renderer::render(&s.doc, 0).unwrap();
    let corner = |s: &CanvasSession| {
        let Shape::Path { nodes, .. } = &s.doc.objects[0].shape else {
            unreachable!()
        };
        nodes[1]
    };
    let old = corner(&s);
    assert_eq!(old.point, old.incoming);
    assert_eq!(old.point, old.outgoing);
    s.tool = Tool::Node;
    s.modifiers = egui::Modifiers::SHIFT;
    let tip = b + egui::vec2(34., 4.);
    assert!(s.drag(b, tip));
    let curved = corner(&s);
    assert_eq!(curved.point, old.point);
    assert_ne!(curved.outgoing, old.outgoing);
    let anchor = Pos2::from(old.point);
    assert!(
        ((Pos2::from(curved.incoming) - anchor) + (Pos2::from(curved.outgoing) - anchor)).length()
            < 0.00001
    );
    assert_ne!(renderer::render(&s.doc, 0).unwrap().data(), before.data());
    s.modifiers = egui::Modifiers::NONE;
    assert!(s.drag(tip, tip + egui::vec2(12., -16.)));
    let adjusted = corner(&s);
    assert_eq!(adjusted.incoming, curved.incoming);
    assert_ne!(adjusted.outgoing, curved.outgoing);
}

#[test]
fn live_drag_previews_restore_exact_pixels_at_the_display_density() {
    for dpi in [1., 2.] {
        for (tool, cancel) in [
            (Tool::Select, true),
            (Tool::Select, false),
            (Tool::Node, true),
            (Tool::Node, false),
        ] {
            let mut s = CanvasSession::new(tool);
            let nodes = [[0., 0.], [1., 0.], [1., 1.], [0., 1.]]
                .map(PathNode::corner)
                .to_vec();
            let id = s.doc.add_shape(
                [40.; 2],
                [40.; 2],
                [180, 40, 110, 180],
                Shape::Path {
                    nodes,
                    closed: true,
                },
                1,
            );
            s.selected.push(id);
            let original = s.doc.clone();
            let paint = |s: &mut CanvasSession, events| {
                let mut input = egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.))),
                    events,
                    ..Default::default()
                };
                input
                    .viewports
                    .get_mut(&egui::ViewportId::ROOT)
                    .unwrap()
                    .native_pixels_per_point = Some(dpi);
                s.paint(input)
            };
            paint(&mut s, vec![]);
            let full_scale = s.canvas.texture.as_ref().unwrap().scale;
            let texture_id = s.canvas.texture.as_ref().unwrap().handle.id();
            let logical_scale = full_scale / dpi;
            let start = if tool == Tool::Node {
                egui::pos2(200., 200.) + Vec2::splat((40. - 64.) * logical_scale)
            } else {
                egui::pos2(200., 200.)
            };
            let end = start + egui::vec2(13.3, 7.7);
            let button = |pos, pressed| egui::Event::PointerButton {
                pos,
                pressed,
                button: egui::PointerButton::Primary,
                modifiers: Default::default(),
            };
            paint(
                &mut s,
                vec![egui::Event::PointerMoved(start), button(start, true)],
            );
            // An edit immediately following a wheel gesture must also sharpen on release.
            s.canvas.sharpen_at = Some(1000.);
            let (_, preview) = paint(&mut s, vec![egui::Event::PointerMoved(end)]);
            assert_eq!(s.doc, original);
            assert_eq!(s.canvas.texture.as_ref().unwrap().scale, logical_scale);
            let finish = if cancel {
                egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                }
            } else {
                // Some window systems deliver a last movement with the release.
                button(end + egui::vec2(3., -2.), false)
            };
            let (committed, mut final_output) = paint(&mut s, vec![finish]);
            assert_eq!(committed, !cancel);
            assert_eq!(s.doc == original, cancel);
            let cached = s.canvas.texture.as_ref().unwrap();
            assert_eq!(cached.document.as_ref(), &s.doc);
            if dpi > 1. {
                assert_eq!(cached.scale, logical_scale);
                assert!(s.canvas.refinement.is_some());
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                while s.canvas.texture.as_ref().unwrap().scale != full_scale {
                    final_output = paint(&mut s, vec![]).1;
                    assert!(std::time::Instant::now() < deadline);
                    std::thread::yield_now();
                }
            }
            let update = final_output
                .textures_delta
                .set
                .iter()
                .chain(&preview.textures_delta.set)
                .find(|(id, _)| *id == texture_id)
                .unwrap();
            let egui::ImageData::Color(actual) = &update.1.image else {
                panic!("Artwork texture should be RGBA")
            };
            let pixels = renderer::render_scaled(&s.doc, 0, full_scale).unwrap();
            let expected = egui::ColorImage::from_rgba_premultiplied(
                [pixels.width() as usize, pixels.height() as usize],
                pixels.data(),
            );
            assert!(*actual.as_ref() == expected);
        }
    }
}

#[test]
fn drag_keyframe_reopen_and_export() {
    let mut session = CanvasSession::new(Tool::Rectangle);
    assert!(session.drag(egui::pos2(200.0, 180.0), egui::pos2(120.0, 140.0)));
    assert_eq!(session.doc.objects.len(), 1);
    let rect = &session.doc.objects[0];
    let base = rect.position;
    assert_eq!(session.selected, [rect.id]);
    assert!(rect.size[0] > rect.size[1] && rect.size[1] > 0.0);
    assert!(session
        .timeline
        .key_selected_track(&mut session.doc, session.selected.first().copied()));

    session.tool = Tool::Select;
    session.selected.clear();
    session.timeline.frame = 4;
    assert!(session.drag(egui::pos2(150.0, 155.0), egui::pos2(250.0, 155.0)));
    let rect = &session.doc.objects[0];
    assert_eq!(session.selected, [rect.id]);
    assert_eq!(rect.position, base);
    assert_eq!(rect.position_at(0), base);
    assert!(rect.position_at(2)[0] > base[0]);
    assert!(rect.position_at(2)[0] < rect.position_at(4)[0]);

    session.timeline.toggle(&session.doc);
    let before = session.doc.clone();
    assert!(!session.drag(egui::pos2(250.0, 155.0), egui::pos2(300.0, 155.0)));
    assert_eq!(session.doc, before);

    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("gesture.vanim");
    let sequence = temp.path().join("gesture");
    project_io::save_project(&project, &session.doc).unwrap();
    let reopened = project_io::load_project(&project).unwrap();
    assert_eq!(reopened, session.doc);
    project_io::export_sequence(&sequence, &reopened).unwrap();
    for frame in 0..5 {
        let image =
            renderer::Pixmap::load_png(sequence.join(format!("frame_{frame:04}.png"))).unwrap();
        let [x, y] = reopened.objects[0].position_at(frame);
        assert_eq!(
            image.pixel(x as u32 + 2, y as u32 + 2).unwrap().alpha(),
            255
        );
        assert_eq!(image.pixel(0, 0).unwrap().alpha(), 0);
    }
}

#[test]
fn pen_creates_curve_and_node_tool_preserves_shape_on_split() {
    let mut s = CanvasSession::new(Tool::Pen);
    let a = egui::pos2(100., 100.);
    let b = egui::pos2(260., 100.);
    let c = egui::pos2(180., 260.);
    assert!(!s.click(a));
    assert!(!s.drag(b, b + egui::vec2(22., 18.)));
    assert!(!s.click(c));
    assert!(s.doc.objects.is_empty());
    assert!(s.click(a));
    let Shape::Path { nodes, closed } = &s.doc.objects[0].shape else {
        panic!("pen did not create path")
    };
    assert!(*closed && nodes.len() == 3);
    assert_ne!(nodes[1].outgoing, nodes[1].point);
    assert_eq!(
        nodes[0].handle_style,
        document_core::path_edit::HandleStyle::Corner
    );
    assert_eq!(
        nodes[1].handle_style,
        document_core::path_edit::HandleStyle::Symmetric
    );
    s.tool = Tool::Node;
    let new_c = c + egui::vec2(10., 0.);
    assert!(s.drag(c, new_c));
    let old = renderer::render(&s.doc, 0).unwrap();
    s.modifiers = egui::Modifiers::COMMAND;
    assert!(s.click(new_c + (a - new_c) * 0.5));
    let new = renderer::render(&s.doc, 0).unwrap();
    for (p, q) in old.data().iter().zip(new.data()) {
        assert!(p.abs_diff(*q) <= 1);
    }
    let Shape::Path { nodes, .. } = &s.doc.objects[0].shape else {
        unreachable!()
    };
    assert_eq!(nodes.len(), 4);
    s.modifiers = egui::Modifiers::ALT;
    let inserted = new_c + (a - new_c) * 0.5;
    s.timeline.key_mode = crate::timeline::KeyMode::Auto;
    assert!(!s.click(inserted));
    assert_eq!(s.doc.objects[0].shape.contours()[0].len(), 4);
    s.timeline.key_mode = crate::timeline::KeyMode::Tracked;
    assert!(s.click(inserted));
    let Shape::Path { nodes, .. } = &s.doc.objects[0].shape else {
        unreachable!()
    };
    assert_eq!(nodes.len(), 3);
    assert_eq!(
        crate::path_editor::selected_range(&s.ctx, s.selected[0], 0),
        (0, 2, 2)
    );
}

#[test]
fn pen_handles_follow_shift_during_motion_and_on_the_final_mouse_event() {
    use document_core::path_edit::HandleStyle;
    use egui::{pos2, vec2, Event, Modifiers};

    let mut s = CanvasSession::new(Tool::Pen);
    let a = pos2(100., 120.);
    let b = pos2(200., 200.);
    let c = pos2(250., 260.);
    let right = vec2(20., 0.);
    let down = vec2(0., 20.);

    s.input(Event::PointerMoved(a));
    s.modifiers = Modifiers::SHIFT;
    s.button(a, true);
    s.input(Event::PointerMoved(a + right));
    s.button(a + right, false);
    let first = &s.canvas.pen[0];
    assert_eq!(first.incoming, first.point);
    assert_ne!(first.outgoing, first.point);
    assert_eq!(first.handle_style, HandleStyle::Corner);

    s.input(Event::PointerMoved(b));
    s.button(b, true);
    s.input(Event::PointerMoved(b + right));
    s.modifiers = Modifiers::NONE;
    s.input(Event::PointerMoved(b + down));
    s.button(b + down, false);
    let middle = &s.canvas.pen[1];
    assert_eq!(middle.handle_style, HandleStyle::Symmetric);
    assert!(middle.incoming[1] < middle.point[1] && middle.point[1] < middle.outgoing[1]);

    s.input(Event::PointerMoved(c));
    s.button(c, true);
    s.input(Event::PointerMoved(c + right));
    // Mouse-up can carry Shift even when the modifier was released before repaint.
    s.input(Event::PointerButton {
        pos: c + vec2(1., 0.),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::SHIFT,
    });
    let last = &s.canvas.pen[2];
    assert!(last.incoming[0] < last.point[0]);
    assert_eq!(last.outgoing, last.point);
    assert_eq!(last.handle_style, HandleStyle::Corner);
}

#[test]
fn enter_during_endpoint_pickup_keeps_the_continuation_attached_until_release() {
    use egui::{pos2, Event};

    let mut s = CanvasSession::new(Tool::Pen);
    let id = s
        .doc
        .add_artboard_path(
            [[25., 50.], [50., 35.], [75., 50.]]
                .map(PathNode::corner)
                .to_vec(),
            false,
            s.layer,
            [255; 4],
        )
        .unwrap();
    s.selected = vec![id];
    s.input(Event::PointerMoved(pos2(160., 160.)));
    let view = s.canvas.context_surface.as_ref().unwrap().1;
    let end = view.screen(pos2(75., 50.));
    let enter = |pressed| Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    s.input(Event::PointerMoved(end));
    assert!(!s.button(end, true));
    assert!(!s.input(enter(true)));
    assert!(!s.input(enter(false)));
    assert!(!s.button(end, false));
    assert!(!s.click(view.screen(pos2(100., 75.))));
    assert!(s.input(enter(true)));
    assert_eq!(s.doc.objects.len(), 1);
    let object = &s.doc.objects[0];
    assert_eq!(object.id, id);
    assert_eq!(object.shape.contours()[0].len(), 4);
}

#[test]
fn coalesced_pen_drag_can_retract_the_closing_handle_before_mouse_up() {
    use egui::{pos2, vec2, Event};

    let mut s = CanvasSession::new(Tool::Pen);
    let start = pos2(130., 110.);
    s.drag(start, start + vec2(20., 4.));
    s.click(pos2(240., 160.));
    s.click(pos2(180., 245.));
    let first = s.canvas.pen[0];
    assert_ne!(first.incoming, first.point);
    let button = |pressed| Event::PointerButton {
        pos: start,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    let (committed, _) = s.paint(egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.))),
        events: vec![
            Event::PointerMoved(start),
            button(true),
            Event::PointerMoved(start + vec2(0., 18.)),
            Event::PointerMoved(start),
            button(false),
            Event::PointerMoved(pos2(410., 410.)),
        ],
        ..Default::default()
    });
    assert!(committed);
    let object = &s.doc.objects[0];
    let node = object.shape.contours()[0][0];
    assert_eq!(node.incoming, node.point);
    assert!(
        Pos2::from(object.local_to_world(node.outgoing, 0)).distance(first.outgoing.into()) < 0.001
    );
}

#[test]
fn closing_pen_drag_snaps_the_tangent_and_shift_keeps_it_until_release() {
    use egui::{pos2, vec2, Event, Modifiers};

    let mut s = CanvasSession::new(Tool::Pen);
    s.input(Event::PointerMoved(pos2(150., 150.)));
    let view = s.canvas.context_surface.as_ref().unwrap().1;
    let anchor = pos2(33., 41.);
    for point in [anchor, pos2(90., 56.), pos2(81., 89.)] {
        s.click(view.screen(point));
    }
    s.snap = SnapSettings {
        enabled: true,
        grid_settings: document_core::GridSettings {
            spacing: 16.,
            ..Default::default()
        },
        ..Default::default()
    };
    let press = view.screen(anchor) + vec2(8., 0.);
    let grid = pos2(48., 32.);
    let drag = press + (grid - anchor) * view.scale + vec2(1., 1.);
    s.input(Event::PointerMoved(press));
    assert!(!s.button(press, true));
    s.input(Event::PointerMoved(drag));
    s.modifiers = Modifiers::SHIFT;
    let end = drag + vec2(24., 29.);
    s.input(Event::PointerMoved(end));
    assert!(s.button(end, false));
    let obj = &s.doc.objects[0];
    let node = obj.shape.contours()[0][0];
    let world = |p| Pos2::from(obj.local_to_world(p, 0));
    assert!(world(node.point).distance(anchor) < 0.001);
    assert!(world(node.incoming).distance(anchor - (grid - anchor)) < 0.001);
}

#[test]
fn escape_during_a_closing_pen_drag_discards_the_draft_before_mouse_up() {
    use egui::{pos2, vec2, Event};

    let mut s = CanvasSession::new(Tool::Pen);
    let start = pos2(130., 110.);
    for point in [start, pos2(240., 160.), pos2(180., 245.)] {
        s.click(point);
    }
    s.input(Event::PointerMoved(start));
    assert!(!s.button(start, true));
    let end = start + vec2(18., -14.);
    s.input(Event::PointerMoved(end));
    s.input(Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    assert!(!s.button(end, false));
    assert!(s.doc.objects.is_empty() && s.canvas.pen.is_empty());
    assert!(s.selected.is_empty());
    s.click(start);
    assert_eq!(s.canvas.pen.len(), 1);
}

#[test]
fn pen_closes_within_the_same_screen_radius_at_different_zooms_and_densities() {
    for (zoom, dpi) in [(-1., 1.), (1., 2.)] {
        let mut s = CanvasSession::new(Tool::Pen);
        s.canvas.zoom_steps = zoom;
        let a = egui::pos2(175., 180.);
        let b = egui::pos2(228., 175.);
        let c = egui::pos2(223., 235.);
        let frame = |s: &mut CanvasSession, events| {
            let mut input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.))),
                events,
                ..Default::default()
            };
            input
                .viewports
                .get_mut(&egui::ViewportId::ROOT)
                .unwrap()
                .native_pixels_per_point = Some(dpi);
            s.paint(input)
        };
        let click = |s: &mut CanvasSession, at| {
            frame(s, vec![egui::Event::PointerMoved(at)]);
            for pressed in [true, false] {
                frame(
                    s,
                    vec![egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    }],
                );
            }
        };
        frame(&mut s, vec![]);
        click(&mut s, a);
        click(&mut s, b);
        let near = a + egui::vec2(11., 0.);
        let cue = |s: &mut CanvasSession, p| {
            frame(s, vec![egui::Event::PointerMoved(p)])
                .1
                .shapes
                .iter()
                .any(|item| {
                    matches!(&item.shape, egui::Shape::Circle(circle)
                if circle.radius == 9. && circle.center.distance(a) < 0.1)
                })
        };
        assert!(!cue(&mut s, near), "two authored nodes cannot close");
        click(&mut s, c);
        let far = a + egui::vec2(14., 0.);
        assert!(!cue(&mut s, far));
        click(&mut s, far);
        assert_eq!(s.canvas.pen.len(), 4);
        assert!(cue(&mut s, near));
        click(&mut s, near);
        assert!(s.canvas.pen.is_empty());
        assert!(
            matches!(&s.doc.objects[0].shape, Shape::Path {nodes, closed:true} if nodes.len()==4)
        );
    }
}

#[test]
fn shift_removes_only_the_picked_group_and_preserves_the_remaining_order() {
    use crate::commands;
    let canvas = Canvas::default();
    let mut doc = Document::default();
    let a = doc.add_rectangle([10.; 2], [9.; 2], [255; 4]);
    let b = doc.add_rectangle([30.; 2], [8.; 2], [255; 4]);
    for o in &mut doc.objects {
        o.group = Some(2);
    }
    doc.objects.swap(0, 1);
    let cutter = doc.add_rectangle([48., 36.], [8.; 2], [255; 4]);
    let mut other = 0;
    for (visible, locked) in [(true, false), (false, false), (true, true)] {
        let layer = commands::add_layer(&mut doc);
        // Locked and hidden shapes cover the click. The editable group with
        // the same group number on another layer is separately selected.
        let point = if visible && !locked {
            [80.; 2]
        } else {
            [10.; 2]
        };
        let id = doc.add_shape(point, [9.; 2], [255; 4], Shape::Rectangle, layer);
        doc.objects.last_mut().unwrap().group = Some(2);
        let layer = doc.layers.iter_mut().find(|l| l.id == layer).unwrap();
        layer.visible = visible;
        layer.locked = locked;
        if visible && !locked {
            other = id;
        }
    }
    let mut selected = vec![other, b, cutter, a];
    let pick = |ids: &mut Vec<u64>| canvas.select(&doc, 0, egui::pos2(14., 14.), ids, true, 1.);
    assert!(pick(&mut selected).is_none());
    assert_eq!(selected, [other, cutter]);
    assert!(matches!(pick(&mut selected), Some(Gesture::Move { .. })));
    assert_eq!(selected, [other, cutter, b, a]);
}

#[test]
fn picking_a_group_keeps_dragging_while_ui_switches_layer() {
    let mut s = CanvasSession::new(Tool::Select);
    s.doc.layers.push(document_core::Layer {
        id: 2,
        name: "Body".into(),
        visible: true,
        locked: false,
    });
    let a = s
        .doc
        .add_shape([15., 15.], [20., 20.], [255; 4], Shape::Ellipse, 2);
    let b = s
        .doc
        .add_shape([70., 20.], [10., 10.], [255; 4], Shape::Rectangle, 2);
    for obj in &mut s.doc.objects {
        obj.group = Some(1);
    }
    let p = egui::pos2(102., 102.);
    s.input(egui::Event::PointerMoved(p));
    assert!(!s.button(p, true));
    assert_eq!(s.selected, [a, b]);
    s.layer = 2;
    let end = p + egui::vec2(25., 0.);
    s.input(egui::Event::PointerMoved(end));
    assert!(s.button(end, false));
    let dx = s.doc.objects[0].position[0] - 15.;
    assert!(dx > 0.);
    assert!((s.doc.objects[1].position[0] - 70. - dx).abs() < 0.001);
}

#[test]
fn finishing_a_fill_only_pen_line_with_enter_preserves_its_paint() {
    let mut s = CanvasSession::new(Tool::Pen);
    assert!(!s.click(egui::pos2(105., 130.)));
    assert!(!s.click(egui::pos2(220., 130.)));
    let enter = egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    assert!(s.input(enter));
    let object = &s.doc.objects[0];
    assert!(matches!(&object.shape,Shape::Path{closed:false,nodes} if nodes.len()==2));
    assert_eq!(object.fill, [255; 4]);
    assert_eq!(object.stroke, [0; 4]);
    assert_eq!(object.stroke_width, 0.);
    assert!(s.doc.validate().is_ok());
}

#[test]
fn resizing_rotated_shape_keeps_its_opposite_corner_fixed() {
    let mut doc = Document::default();
    let id = doc.add_shape([15., 20.], [20., 10.], [255; 4], Shape::Rectangle, 1);
    doc.objects[0].rotation = 33.;
    let anchor = Pos2::from(doc.objects[0].local_to_world([0., 0.], 0));
    let start = Pos2::from(doc.objects[0].local_to_world([1., 1.], 0));
    let desired = start + egui::vec2(6., 10.);
    let gesture = Gesture::Resize {
        start,
        id,
        handle: 4,
        offset: Vec2::ZERO,
    };
    let style = CanvasStyle {
        edit_motion: false,
        key_mode: Default::default(),
        fill: [255; 4],
        gradient: None,
        stroke_gradient: None,
        no_fill: false,
        no_stroke: false,
        gradient_channel: crate::canvas::PaintChannel::Fill,
        cap: Default::default(),
        join: Default::default(),
        stroke: [0; 4],
        stroke_width: 0.,
        layer: 1,
        snap: Default::default(),
    };
    assert!(
        gesture
            .apply(&mut doc, 0, desired, &style, egui::Modifiers::NONE)
            .0
    );
    let object = &doc.objects[0];
    assert!(Pos2::from(object.local_to_world([0., 0.], 0)).distance(anchor) < 0.001);
    assert!(Pos2::from(object.local_to_world([1., 1.], 0)).distance(desired) < 0.001);
}

#[test]
fn group_targets_preserve_key_gating_geometry_and_clipboard_order() {
    use crate::{commands, timeline::KeyMode};
    let mut doc = Document::default();
    let first = doc.add_rectangle([20., 11.], [13., 17.], [170; 4]);
    let second = doc.add_rectangle([43., 10.], [9., 24.], [210; 4]);
    // Paint order deliberately differs from numeric ID order.
    doc.objects.swap(0, 1);
    for object in &mut doc.objects {
        object.group = Some(8);
        object.rotation = 31.;
        object.animated.skew = 17.;
    }
    doc.objects[0].keys.insert(0, [44., 11.]);
    for locked in [true, false] {
        let layer = commands::add_layer(&mut doc);
        doc.add_shape([18.; 2], [12.; 2], [88; 4], Shape::Ellipse, layer);
        let state = doc.layers.iter_mut().find(|l| l.id == layer).unwrap();
        state.locked = locked;
        state.visible = locked;
    }
    let requested = [first, doc.objects[2].id, doc.objects[3].id];
    let selected = editable_selection(&doc, &requested);
    assert_eq!(selected, [second, first]);
    let mut style = CanvasStyle {
        edit_motion: false,
        key_mode: KeyMode::Manual,
        fill: [255; 4],
        gradient: None,
        stroke_gradient: None,
        no_fill: false,
        no_stroke: false,
        gradient_channel: crate::canvas::PaintChannel::Fill,
        cap: Default::default(),
        join: Default::default(),
        stroke: [0; 4],
        stroke_width: 0.,
        layer: 1,
        snap: Default::default(),
    };
    let gesture = Gesture::Move {
        start: Pos2::ZERO,
        reference: Pos2::ZERO,
        flips: [false; 2],
        ids: GestureIds::new(selected.clone()),
    };
    let before = doc.clone();
    let delta = egui::vec2(8., -6.);
    assert!(
        !gesture
            .apply(&mut doc, 4, delta.to_pos2(), &style, egui::Modifiers::NONE)
            .0
    );
    assert_eq!(doc, before, "a disallowed key must veto the whole group");
    style.key_mode = KeyMode::Auto;
    assert!(
        gesture
            .apply(&mut doc, 4, delta.to_pos2(), &style, egui::Modifiers::NONE)
            .0
    );
    for (old, moved) in before.objects[..2].iter().zip(&doc.objects) {
        let anchor = |o: &document_core::VectorObject| Pos2::from(o.local_to_world([0.1, 0.9], 4));
        assert!((anchor(moved) - anchor(old) - delta).length() < 0.001);
        assert!(moved.keys.contains_key(&4));
        assert_eq!(moved.rotation, old.rotation);
        assert_eq!(moved.animated.skew, old.animated.skew);
    }
    assert_eq!(doc.objects[2..], before.objects[2..]);
    let mut clipboard = commands::Clipboard::default();
    clipboard.copy(&doc, &selected, 4);
    let mut dest = Document::default();
    assert_eq!(clipboard.paste(&mut dest, 1).len(), 2);
    assert_eq!(dest.objects[0].fill, doc.objects[0].fill);
    assert_eq!(dest.objects[1].fill, doc.objects[1].fill);
    assert!(dest.objects[0].group.is_some());
    assert_eq!(dest.objects[0].group, dest.objects[1].group);
}

#[test]
fn group_corner_and_rotation_handles_transform_members_together() {
    let mut s = CanvasSession::new(Tool::Rectangle);
    assert!(s.drag(egui::pos2(100., 120.), egui::pos2(140., 160.)));
    assert!(s.drag(egui::pos2(180., 120.), egui::pos2(220., 160.)));
    for o in &mut s.doc.objects {
        o.group = Some(3);
        o.stroke = [0, 0, 0, 255];
        o.stroke_width = 2.;
    }
    let keyed = &mut s.doc.objects[1];
    let original_key = keyed.position;
    keyed.keys.insert(0, original_key);
    let original_sizes: Vec<_> = s.doc.objects.iter().map(|o| Vec2::from(o.size)).collect();
    s.timeline.frame = 4;
    s.tool = Tool::Select;
    s.selected.clear();
    assert!(!s.click(egui::pos2(120., 140.)));
    assert_eq!(s.selected.len(), 2);
    let before = group_bounds(&s.doc, 4, &GestureIds::new(s.selected.clone())).unwrap();
    assert!(s.drag(egui::pos2(220., 160.), egui::pos2(280., 180.)));
    let scaled = group_bounds(&s.doc, 4, &GestureIds::new(s.selected.clone())).unwrap();
    assert!(before.min.distance(scaled.min) < 0.001);
    assert!((scaled.size() - 1.5 * before.size()).length() < 0.001);
    for (o, old_size) in s.doc.objects.iter().zip(&original_sizes) {
        assert!((Vec2::from(o.size) - 1.5 * *old_size).length() < 0.001);
        assert_eq!(o.stroke_width, 2.);
    }
    let centers: Vec<_> = s
        .doc
        .objects
        .iter()
        .map(|o| Pos2::from(o.local_to_world([0.5; 2], 4)))
        .collect();
    assert!(s.drag(egui::pos2(190., 102.), egui::pos2(238., 150.)));
    for (o, center) in s.doc.objects.iter().zip(centers) {
        assert!((o.rotation - 90.).abs() < 0.001);
        let expected = gesture::rotate_around(center, scaled.center(), 90.);
        assert!(expected.distance(Pos2::from(o.local_to_world([0.5; 2], 4))) < 0.001);
    }
    assert_eq!(s.doc.objects[1].keys[&0], original_key);
    assert_ne!(s.doc.objects[1].keys[&4], original_key);
}

#[test]
fn resize_and_rotate_handles_edit_the_evaluated_tracks() {
    let mut doc = Document::default();
    let id = doc.add_rectangle([15., 20.], [20., 10.], [255; 4]);
    let object = &mut doc.objects[0];
    object.keys.insert(0, object.position);
    object.rotation_keys.extend([(0, 325.), (4, 395.)]);
    object.animated.skew_keys.extend([(0, -25.), (4, 5.)]);
    object.scale_keys.extend([(0, [1., 1.]), (4, [2., 3.])]);
    let anchor = Pos2::from(object.local_to_world([0., 0.], 2));
    let start = Pos2::from(object.local_to_world([1., 1.], 2));
    let desired = start + egui::vec2(5., 7.);
    let style = CanvasStyle {
        edit_motion: false,
        key_mode: Default::default(),
        fill: [255; 4],
        gradient: None,
        stroke_gradient: None,
        no_fill: false,
        no_stroke: false,
        gradient_channel: crate::canvas::PaintChannel::Fill,
        cap: Default::default(),
        join: Default::default(),
        stroke: [0; 4],
        stroke_width: 0.,
        layer: 1,
        snap: Default::default(),
    };
    assert!(
        Gesture::Resize {
            start,
            id,
            handle: 4,
            offset: Vec2::ZERO,
        }
        .apply(&mut doc, 2, desired, &style, egui::Modifiers::NONE)
        .0
    );
    let object = &doc.objects[0];
    assert_eq!(object.size, [20., 10.]);
    assert!(object.scale_keys.contains_key(&2));
    assert!(Pos2::from(object.local_to_world([0., 0.], 2)).distance(anchor) < 0.001);
    assert!(Pos2::from(object.local_to_world([1., 1.], 2)).distance(desired) < 0.001);
    let center = Pos2::from(object.local_to_world([0.5; 2], 2));
    let start = center + egui::vec2(0., -12.);
    assert!(
        Gesture::Rotate { start, id }
            .apply(
                &mut doc,
                2,
                center + egui::vec2(12., 0.),
                &style,
                egui::Modifiers::NONE
            )
            .0
    );
    assert_eq!(doc.objects[0].rotation_keys[&2], 450.);
    assert_eq!(doc.objects[0].rotation, 0.);
}

#[test]
fn pointer_edits_auto_key_and_manual_requires_an_existing_key() {
    let mut session = CanvasSession::new(Tool::Rectangle);
    assert!(session.drag(egui::pos2(120., 120.), egui::pos2(200., 200.)));
    session.tool = Tool::Select;
    session.timeline.frame = 4;
    session.timeline.key_mode = crate::timeline::KeyMode::Auto;
    let base = session.doc.objects[0].position;
    assert!(session.drag(egui::pos2(140., 140.), egui::pos2(165., 135.)));
    let animated = session.doc.objects[0].clone();
    assert_eq!(animated.position_at(0), base);
    assert_ne!(animated.position_at(4), base);
    session.timeline.frame = 2;
    session.timeline.key_mode = crate::timeline::KeyMode::Manual;
    assert!(!session.drag(egui::pos2(152., 138.), egui::pos2(162., 138.)));
    assert_eq!(session.doc.objects[0], animated);
}

#[test]
fn guide_anchor_drag_moves_the_route_and_preview_keeps_its_start_pose() {
    let mut session = CanvasSession::new(Tool::Select);
    let id = session.doc.add_rectangle([56., 56.], [16., 16.], [255; 4]);
    session.selected = vec![id];
    session.edit_motion = true;
    let o = &mut session.doc.objects[0];
    o.motion = Some(
        document_core::MotionPath::new(
            vec![PathNode::corner([0., 0.]), PathNode::corner([32., 0.])],
            false,
        )
        .unwrap(),
    );
    o.animated.progress_keys.extend([(0, 0.), (4, 1.)]);
    let start = o.position_at(0);
    let old = o.position_at(4);
    // Board at screen (40,40), scale 2.5: the terminal guide node is (280,200).
    assert!(session.drag(egui::pos2(280., 200.), egui::pos2(300., 225.)));
    assert_eq!(session.doc.objects[0].position_at(0), start);
    assert_ne!(session.doc.objects[0].position_at(4), old);
}

#[test]
fn bone_posed_pointer_drag_and_keyboard_keys_stay_in_bind_space() {
    use crate::timeline::KeyMode;
    let mut session = CanvasSession::new(Tool::Select);
    let id = session.doc.add_rectangle([56., 56.], [16., 16.], [255; 4]);
    let bone = session
        .doc
        .add_bone("Hip", None, [64., 64.], [80., 64.])
        .unwrap();
    session.doc.bind_rigid(id, Some(bone)).unwrap();
    session.doc.set_bone_rotation(bone, 90.).unwrap();
    session.timeline.frame = 4;
    session.timeline.key_mode = KeyMode::Auto;
    assert!(session.drag(egui::pos2(200., 200.), egui::pos2(225., 200.)));
    let o = &session.doc.objects[0];
    assert_eq!(o.keys[&0], [56., 56.]);
    assert_eq!(o.keys[&4], [56., 46.]);
    assert_eq!(o.position_at(4), [66., 56.]);
    // The keyboard shortcut samples the authored track when inserting a mid-span key.
    session.timeline.frame = 2;
    let before = session.doc.objects[0].position_at(2);
    assert!(session
        .timeline
        .key_selected_track(&mut session.doc, Some(id)));
    assert_eq!(session.doc.objects[0].position_at(2), before);
    assert_eq!(session.doc.objects[0].keys[&2], [56., 51.]);
}

#[test]
fn posed_guide_handles_follow_a_rotated_bone_and_edit_the_visible_endpoint() {
    let mut s = CanvasSession::new(Tool::Select);
    let id = s.doc.add_rectangle([56., 56.], [16., 16.], [255; 4]);
    s.selected = vec![id];
    s.edit_motion = true;
    let arm = s.doc.add_bone("Arm", None, [64., 64.], [80., 64.]).unwrap();
    s.doc.bind_rigid(id, Some(arm)).unwrap();
    let o = &mut s.doc.objects[0];
    o.motion = Some(
        document_core::MotionPath::new(
            vec![PathNode::corner([0., 0.]), PathNode::corner([32., 0.])],
            false,
        )
        .unwrap(),
    );
    o.animated.progress_keys.extend([(0, 0.), (4, 1.)]);
    s.doc.set_bone_rotation(arm, 90.).unwrap();
    // Board starts at (40,40), with 2.5 screen pixels per artboard pixel.
    assert!(s.drag(egui::pos2(200., 280.), egui::pos2(225., 280.)));
    assert_eq!(s.doc.objects[0].position_at(0), [56., 56.]);
    assert_eq!(s.doc.objects[0].position_at(4), [66., 88.]);
}
