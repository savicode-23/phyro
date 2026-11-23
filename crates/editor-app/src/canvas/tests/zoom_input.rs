use super::*;

#[test]
fn high_zoom_on_a_thousand_pixel_page_sharpens_only_visible_tiles() {
    let mut s = CanvasSession::new(Tool::Select);
    s.doc.width = 1000;
    s.doc.height = 1000;
    s.doc.add_rectangle([495.; 2], [10.; 2], [210, 20, 10, 120]);
    let screen = Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.));
    let paint = |s: &mut CanvasSession, time, events| {
        s.paint(egui::RawInput {
            screen_rect: Some(screen),
            time: Some(time),
            events,
            ..Default::default()
        })
        .1
    };
    paint(&mut s, 0., vec![egui::Event::PointerMoved(screen.center())]);
    let overview = s.canvas.texture.as_ref().unwrap().handle.id();
    paint(&mut s, 0.02, vec![egui::Event::Zoom(2048.)]);
    let sharpen = std::time::Instant::now();
    loop {
        let output = paint(&mut s, 0.2 + sharpen.elapsed().as_secs_f64(), vec![]);
        let triangles = s.ctx.tessellate(output.shapes, output.pixels_per_point);
        let manager = s.ctx.tex_manager();
        let textures = manager.read();
        let mut detailed = false;
        let mut overview_visible = false;
        for shape in triangles {
            if let egui::epaint::Primitive::Mesh(mesh) = shape.primitive {
                overview_visible |= mesh.texture_id == overview;
                if textures
                    .meta(mesh.texture_id)
                    .is_some_and(|m| m.name == "artboard detail")
                {
                    assert_eq!(textures.meta(mesh.texture_id).unwrap().size, [258, 258]);
                    detailed = true;
                }
            }
        }
        if detailed && !overview_visible {
            break;
        }
        assert!(
            sharpen.elapsed().as_secs() < 10,
            "detail tiles did not replace the page preview"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!((s.canvas.detail.density().unwrap() - 64.).abs() < 0.001);
    assert!(s.canvas.texture.as_ref().unwrap().handle.size()[0] <= 2048);
    let warm = paint(&mut s, 20., vec![]);
    assert!(
        warm.textures_delta.set.is_empty(),
        "a stationary view uploaded again"
    );
    let wheel = paint(&mut s, 20.02, vec![egui::Event::Zoom(0.5)]);
    assert!(
        wheel.textures_delta.set.is_empty(),
        "wheel motion rebuilt the old tile grid"
    );
}

#[test]
fn pinch_and_ctrl_wheel_scale_once_about_the_pointer_with_an_existing_pan() {
    let mut session = CanvasSession::new(Tool::Select);
    session.canvas.pan = egui::vec2(32., -18.);
    let point = egui::pos2(145., 162.);
    let offset = point - egui::pos2(200., 200.);
    let board_offset = offset - session.canvas.pan;
    session.input(egui::Event::PointerMoved(point));
    let texture = session.canvas.texture.as_ref().unwrap().handle.id();
    let original = session.doc.clone();
    for (index, event) in [
        egui::Event::Zoom(1.6),
        egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0., 6.),
            modifiers: egui::Modifiers::CTRL,
        },
        egui::Event::Zoom(0.7),
    ]
    .into_iter()
    .enumerate()
    {
        let before = session.canvas.zoom_steps;
        let (_, output) = session.paint(egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.))),
            events: vec![event],
            time: Some((index + 2) as f64 / 60.),
            ..Default::default()
        });
        let factor = session.ctx.input(|i| i.zoom_delta());
        assert_ne!(factor, 1.);
        assert!((session.canvas.zoom_steps - before - factor.log2()).abs() < 0.0001);
        let adjusted = (offset - session.canvas.pan) / session.canvas.zoom_steps.exp2();
        assert!((adjusted - board_offset).length() < 0.001);
        assert!(!output
            .textures_delta
            .set
            .iter()
            .any(|(id, _)| *id == texture));
    }
    assert_eq!(session.doc, original);
    session.input(egui::Event::PointerMoved(egui::pos2(600., 200.)));
    let before = session.canvas.zoom_steps;
    session.input(egui::Event::Zoom(2.));
    assert_eq!(session.canvas.zoom_steps, before);
}
