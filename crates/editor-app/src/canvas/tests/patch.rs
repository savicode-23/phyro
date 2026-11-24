use super::*;
use std::sync::Arc;

#[test]
fn dragging_a_clip_in_from_the_pasteboard_hides_its_old_ink_until_release_or_cancel() {
    let ink = [34, 180, 96, 255];
    let page = Document {
        width: 128,
        height: 128,
        ..Default::default()
    };
    let mut project = document_core::AnimationProject::from_document(page);
    let child = project.add_clip("Brush").unwrap();
    project.clips[child]
        .drawing_at_mut(0)
        .add_rectangle([0., 4.], [22., 26.], ink);
    project
        .place_clip(project.clips[child].id, 0, 0, 1, [-4., 20.])
        .unwrap();
    let mut s = CanvasSession::new(Tool::Select);
    s.doc = project.clips[0].drawing_at(0).clone();
    let paint = |s: &mut CanvasSession, events| {
        s.paint(egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.))),
            events,
            ..Default::default()
        })
        .1
    };
    paint(&mut s, vec![]);
    let view = s.canvas.context_surface.as_ref().unwrap().1;
    let start = view.screen(egui::pos2(4., 30.));
    let end = view.screen(egui::pos2(48., 30.));
    for cancel in [true, false] {
        s.input(egui::Event::PointerMoved(start));
        s.button(start, true);
        let held = paint(&mut s, vec![egui::Event::PointerMoved(end)]);
        let parts = s.ctx.tessellate(held.shapes, held.pixels_per_point);
        let manager = s.ctx.tex_manager();
        let textures = manager.read();
        let mut moving = false;
        for part in parts {
            if let egui::epaint::Primitive::Mesh(mesh) = part.primitive {
                let name = &textures.meta(mesh.texture_id).unwrap().name;
                assert!(
                    !matches!(
                        name.as_str(),
                        "artboard artwork" | "pasteboard artwork" | "artboard detail"
                    ),
                    "stale texture drawn: {name}"
                );
                moving |= name == "moving artwork" && mesh.calc_bounds().contains(end);
            }
        }
        drop(textures);
        assert!(moving);
        if cancel {
            s.canvas.cancel_drag();
        }
        assert_eq!(s.button(end, false), !cancel);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            paint(&mut s, vec![]);
            let page = s.canvas.texture.as_ref().unwrap();
            if page.matches(&s.doc, 0) {
                assert_eq!(
                    page.sample_texel(&s.doc, 0, [4., 30.]),
                    Some(if cancel { ink } else { [0; 4] })
                );
                assert_eq!(
                    page.sample_texel(&s.doc, 0, [48., 30.]),
                    Some(if cancel { [0; 4] } else { ink })
                );
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
}

#[test]
fn on_page_snapped_move_draws_the_exact_pose_without_copying_the_drawing() {
    let mut s = CanvasSession::new(Tool::Select);
    s.snap.enabled = true;
    let id = s
        .doc
        .add_rectangle([50., 50.], [28., 28.], [80, 175, 125, 255]);
    let template = s.doc.objects[0].clone();
    for i in 0..512 {
        let mut parked = template.clone();
        parked.id = s.doc.next_id;
        s.doc.next_id += 1;
        parked.position = [200. + i as f32, 250.];
        s.doc.objects.push(parked);
    }
    let start = egui::pos2(200., 200.);
    let end = start + egui::vec2(19., 27.);
    s.input(egui::Event::PointerMoved(start));
    let source = Arc::clone(s.canvas.texture.as_ref().unwrap().authored());
    let source_id = s.canvas.texture.as_ref().unwrap().handle.id();
    assert!(!s.button(start, true));
    let movement = || egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.))),
        events: vec![egui::Event::PointerMoved(end)],
        ..Default::default()
    };
    let (changed, output) = s.paint(movement());
    assert!(!changed);
    assert_eq!(s.selected, [id]);
    assert_eq!(s.doc, *source);
    assert!(s.canvas.page_drag.is_some());
    assert!(Arc::ptr_eq(
        &source,
        s.canvas.texture.as_ref().unwrap().authored()
    ));
    let (pose_id, delta) = output
        .textures_delta
        .set
        .into_iter()
        .find(|(id, d)| id != &source_id && d.pos.is_none() && d.image.size()[0] > 256)
        .expect("separate page texture for the moving pose");
    let egui::ImageData::Color(image) = delta.image else {
        panic!("expected an RGBA image")
    };
    let (_, same) = s.paint(movement());
    assert!(same.textures_delta.set.iter().all(|(id, _)| *id != pose_id));
    let release = |pos| egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.))),
        events: vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            },
        ],
        ..Default::default()
    };
    let (changed, released) = s.paint(release(end));
    assert!(changed);
    assert_eq!(s.canvas.texture.as_ref().unwrap().handle.id(), pose_id);
    assert!(released
        .textures_delta
        .set
        .iter()
        .all(|(id, _)| *id != pose_id));
    assert!(s.canvas.page_drag.is_none());
    let density = image.size[0] as f32 / s.doc.width as f32;
    let full = renderer::render_scaled(&s.doc, 0, density).unwrap();
    assert_eq!(
        image.pixels,
        egui::ColorImage::from_rgba_premultiplied(image.size, full.data()).pixels
    );

    let committed = Arc::clone(s.canvas.texture.as_ref().unwrap().authored());
    let body = end + egui::vec2(15., 9.);
    s.input(egui::Event::PointerMoved(body));
    s.button(body, true);
    s.input(egui::Event::PointerMoved(body + egui::vec2(-18., 5.)));
    assert!(s.canvas.page_drag.is_some());
    s.canvas.cancel_drag();
    s.button(body, false);
    assert!(s.canvas.page_drag.is_none());
    assert!(Arc::ptr_eq(
        &committed,
        s.canvas.texture.as_ref().unwrap().authored()
    ));
    assert_eq!(s.doc, *committed);

    s.button(body, true);
    s.input(egui::Event::PointerMoved(body + egui::vec2(-18., 5.)));
    assert!(s.canvas.page_drag.is_some());
    let (changed, _) = s.paint(release(body + egui::vec2(-26., 7.)));
    assert!(changed);
    let final_pose = s.canvas.texture.as_ref().unwrap();
    assert_eq!(final_pose.authored().as_ref(), &s.doc);
    let expected = renderer::render_scaled(&s.doc, 0, final_pose.scale).unwrap();
    assert_eq!(
        final_pose.sample_pixels.as_ref().unwrap().pixels,
        egui::ColorImage::from_rgba_premultiplied(
            [expected.width() as usize, expected.height() as usize],
            expected.data()
        )
        .pixels
    );
}

#[test]
fn dense_drag_updates_the_displayed_image_through_release_cancel_and_frame_switches() {
    let mut s = CanvasSession::new(Tool::Select);
    for _ in 0..512 {
        s.doc.add_rectangle([121., 120.], [2., 2.], [45; 4]);
    }
    for i in 0..4 {
        s.doc.add_shape(
            [2.3 + i as f32, 5.6],
            [115., 118.],
            [36, 170, 221, 151],
            Shape::Ellipse,
            1,
        );
        let background = s.doc.objects.last_mut().unwrap();
        background.opacity = 0.7;
        background.stroke_width = 1.5;
        background.stroke = [31, 35, 93, 171];
    }
    s.doc.add_shape(
        [23., 31.],
        [10., 15.],
        [80, 204, 134, 158],
        Shape::Ellipse,
        1,
    );
    let original = s.doc.clone();
    // Warm at a lower density, then let the normal zoom worker publish the new
    // image and its matching background together before the first pointer edit.
    let _ = s.ctx.run(Default::default(), |ctx| {
        s.canvas.frame_texture(ctx, &original, 0, 1.).unwrap();
    });
    assert!(s.canvas.texture.as_ref().unwrap().backdrop.is_some());
    s.canvas.sharpen_at = Some(0.);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut screen = loop {
        let output = s.ctx.run(Default::default(), |ctx| {
            s.canvas.frame_texture(ctx, &original, 0, 2.).unwrap();
        });
        let texture = s.canvas.texture.as_ref().unwrap();
        if texture.scale == 2. {
            let (_, delta) = output
                .textures_delta
                .set
                .into_iter()
                .find(|(id, _)| *id == texture.handle.id())
                .unwrap();
            let egui::ImageData::Color(image) = delta.image else {
                panic!("expected artboard pixels")
            };
            assert_eq!(delta.pos, None);
            assert!(texture.backdrop.is_some());
            break (*image).clone();
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    };
    let mut check = |s: &mut CanvasSession, frame: u32, expected: &Document, preview: bool| {
        let input = egui::RawInput::default();
        let staged = preview.then(|| Arc::new(expected.clone()));
        let output = s.ctx.run(input, |ctx| {
            s.canvas
                .artwork_texture(
                    ctx,
                    &Arc::new(s.doc.clone()),
                    frame,
                    if preview { 1. } else { 2. },
                    staged.as_ref(),
                )
                .unwrap();
        });
        let texture = s.canvas.texture.as_ref().unwrap();
        let changed = output
            .textures_delta
            .set
            .into_iter()
            .find(|(id, _)| *id == texture.handle.id());
        let mut partial = false;
        if let Some((_, delta)) = changed {
            let egui::ImageData::Color(image) = delta.image else {
                panic!("expected an RGBA artboard")
            };
            if let Some([x, y]) = delta.pos {
                partial = true;
                for (dy, row) in image.pixels.chunks_exact(image.size[0]).enumerate() {
                    let first = (dy + y) * screen.size[0] + x;
                    screen.pixels[first..first + row.len()].copy_from_slice(row);
                }
            } else {
                screen = (*image).clone();
            }
        }
        let reference = renderer::render_scaled(expected, frame, texture.scale).unwrap();
        let exact = egui::ColorImage::from_rgba_premultiplied(
            [reference.width() as usize, reference.height() as usize],
            reference.data(),
        );
        assert!(screen == exact);
        partial
    };
    let mut moved = original.clone();
    moved.objects.last_mut().unwrap().position[0] += 2.5;
    assert!(check(&mut s, 0, &moved, true));
    assert!(s.canvas.texture.as_ref().unwrap().backdrop.is_some());
    assert!(check(&mut s, 0, &original, false)); // Cancel restores the original pixels.
    assert!(check(&mut s, 0, &moved, true));
    let mut release = moved.clone();
    release.objects.last_mut().unwrap().position[1] -= 1.25;
    s.doc = release.clone();
    assert!(check(&mut s, 0, &release, false)); // Final movement arrived with the release.
    assert!(s.canvas.texture.as_ref().unwrap().backdrop.is_some());
    let mut other = release.clone();
    other.objects[0].position = [68., 71.];
    assert!(check(&mut s, 0, &other, true)); // Switch the object being edited.
    assert!(s.canvas.texture.as_ref().unwrap().backdrop.is_none());
    other.objects[0].keys.insert(1, [78., 81.]);
    s.doc = other.clone();
    assert!(!check(&mut s, 1, &other, false));
    s.doc.add_rectangle([12., 17.], [18., 14.], [105; 4]);
    let pasted = s.doc.clone();
    assert!(check(&mut s, 1, &pasted, false));
    let operands = [
        s.doc.objects[s.doc.objects.len() - 2].id,
        s.doc.objects.last().unwrap().id,
    ];
    s.doc
        .boolean(&operands, 1, document_core::BooleanOp::Difference)
        .unwrap();
    let cut = s.doc.clone();
    assert!(check(&mut s, 1, &cut, false));
    s.doc = pasted.clone();
    assert!(check(&mut s, 1, &pasted, false));
    s.doc = cut.clone();
    assert!(check(&mut s, 1, &cut, false));
    assert!(!check(&mut s, 1, &cut, false)); // Already displayed.

    // Undo at a different display density requires a full-sized replacement.
    let output = s.ctx.run(Default::default(), |ctx| {
        s.canvas.frame_texture(ctx, &pasted, 1, 1.5).unwrap();
    });
    let texture = s.canvas.texture.as_ref().unwrap();
    let (_, delta) = output
        .textures_delta
        .set
        .into_iter()
        .find(|(id, _)| *id == texture.handle.id())
        .unwrap();
    assert_eq!(delta.pos, None);
    assert_eq!(texture.scale, 1.5);
    let egui::ImageData::Color(image) = delta.image else {
        panic!("expected an RGBA artboard")
    };
    let pixels = renderer::render_scaled(&pasted, 1, 1.5).unwrap();
    assert!(
        *image
            == egui::ColorImage::from_rgba_premultiplied(
                [pixels.width() as usize, pixels.height() as usize],
                pixels.data()
            )
    );
}

#[test]
fn reused_transform_snapshots_follow_concurrent_artwork_edits_and_exact_pixels() {
    let mut s = CanvasSession::new(Tool::Select);
    s.doc
        .add_rectangle([50., 50.], [28., 28.], [90, 160, 130, 255]);
    let sample = s.doc.objects[0].clone();
    for n in 0..600 {
        let mut o = sample.clone();
        o.id = s.doc.next_id;
        s.doc.next_id += 1;
        o.position = [300. + n as f32, 300.];
        s.doc.objects.push(o);
    }
    let mut control = CanvasSession::new(Tool::Select);
    control.doc = s.doc.clone();
    let start = egui::pos2(200., 200.);
    for session in [&mut s, &mut control] {
        session.input(egui::Event::PointerMoved(start));
        assert!(!session.button(start, true));
    }
    let mut end = start;
    for step in 1..8 {
        for session in [&mut s, &mut control] {
            match step {
                3 => {
                    session.doc.objects[1].position = [20., 20.];
                    session.doc.objects[1].fill = [180, 40, 60, 255];
                }
                4 => {
                    session.doc.objects.swap(0, 23);
                }
                5 => {
                    let selected = session.selected[0];
                    session
                        .doc
                        .objects
                        .iter_mut()
                        .find(|o| o.id == selected)
                        .unwrap()
                        .fill = [0, 20, 60, 255];
                }
                6 => {
                    session.doc.guides_x.push(22.);
                }
                _ => {}
            }
        }
        // Each control frame starts from a full, independent copy of authored artwork.
        control.canvas.previous_drag_document = None;
        end = start + egui::vec2(3. * step as f32, 2. * step as f32);
        let paint = |session: &mut CanvasSession| {
            let (_, out) = session.paint(egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.))),
                events: vec![egui::Event::PointerMoved(end)],
                ..Default::default()
            });
            let t = session.canvas.texture.as_ref().unwrap();
            let id = t.handle.id();
            let uploads = out
                .textures_delta
                .set
                .into_iter()
                .filter(|(i, _)| *i == id)
                .map(|(_, d)| (d.pos, d.image))
                .collect::<Vec<_>>();
            (t.authored().clone(), uploads)
        };
        let (actual, uploads) = paint(&mut s);
        let (expected, expected_uploads) = paint(&mut control);
        assert_eq!(actual, expected);
        assert_eq!(s.doc, control.doc);
        assert!(s.doc != *actual, "the pose is still preview-only");
        assert_eq!(uploads.len(), expected_uploads.len());
        for ((pos, image), (wanted, baseline)) in uploads.into_iter().zip(expected_uploads) {
            assert_eq!(pos, wanted);
            match (image, baseline) {
                (egui::ImageData::Color(a), egui::ImageData::Color(b)) => {
                    assert_eq!(a.pixels, b.pixels)
                }
                _ => panic!("artboard must be a color texture"),
            }
        }
    }
    assert!(s.button(end, false));
    assert!(control.button(end, false));
    assert_eq!(s.doc, control.doc);
    s.input(egui::Event::PointerMoved(end));
    assert!(s.canvas.previous_drag_document.is_none());
}
