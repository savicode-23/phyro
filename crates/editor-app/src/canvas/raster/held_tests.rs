use super::*;
use document_core::AnimationProject;

mod animated_snapshots;

impl Refinement {
    pub(in crate::canvas) fn inject_error(&mut self) {
        let (send, result) = mpsc::channel();
        send.send(Err("Injected renderer failure.".into())).unwrap();
        self.result = result;
    }
}

#[test]
fn worker_retries_transient_errors_once() {
    use std::sync::atomic::{AtomicU32, Ordering};
    let attempts = AtomicU32::new(0);
    let transient = refined_image::<PageImage>(|| {
        if attempts.fetch_add(1, Ordering::Relaxed) == 0 {
            Err("Temporary failure".into())
        } else {
            Ok((ColorImage::new([1, 1], egui::Color32::WHITE), None))
        }
    });
    assert!(transient.is_ok());
    assert_eq!(attempts.swap(0, Ordering::Relaxed), 2);
    assert!(refined_image::<PageImage>(|| {
        attempts.fetch_add(1, Ordering::Relaxed);
        Err("Persistent failure".into())
    })
    .is_err());
    assert_eq!(attempts.load(Ordering::Relaxed), 2);
}

#[test]
fn worker_unwind_delivers_failure_and_wakes_the_idle_viewport() {
    let (mut canvas, ctx) = (Canvas::default(), egui::Context::default());
    assert!(uploaded(&mut canvas, &ctx, &Document::default(), 0, 1.));
    for _ in 0..3 {
        let _ = ctx.run(Default::default(), |_| {});
    }
    let (wake, wakeups) = mpsc::channel();
    ctx.set_request_repaint_callback(move |event| {
        let _ = wake.send(event.viewport_id);
    });
    let job = Refinement::start(&ctx, canvas.texture.as_ref().unwrap(), 2., |_, _, _| {
        panic!("Injected renderer unwind")
    })
    .unwrap();
    let timeout = std::time::Duration::from_secs(5);
    assert!(job
        .result
        .recv_timeout(timeout)
        .unwrap()
        .unwrap_err()
        .contains("stopped unexpectedly"));
    assert_eq!(
        wakeups.recv_timeout(timeout).unwrap(),
        egui::ViewportId::ROOT
    );
}

#[test]
fn failed_zoom_keeps_the_preview_until_density_changes_and_ignores_stale_failures() {
    let (mut canvas, ctx) = (Canvas::default(), egui::Context::default());
    let mut doc = Document::default();
    doc.add_rectangle([2.; 2], [5.; 2], [120; 4]);
    assert!(uploaded(&mut canvas, &ctx, &doc, 0, 1.));
    let snapshot = canvas.texture.as_ref().unwrap().document.clone();
    let (send, result) = mpsc::channel();
    send.send(Err("Injected zoom failure".into())).unwrap();
    canvas.refinement = Some(Refinement {
        document: snapshot.clone(),
        frame: 0,
        scale: 2.,
        result,
    });
    canvas.sharpen_at = Some(0.5);
    for frame in [0, 0, 1, 2] {
        assert!(!uploaded(&mut canvas, &ctx, &doc, frame, 2.));
        assert!(canvas.refinement.is_none());
        assert!(canvas
            .refinement_warning()
            .unwrap()
            .contains("Injected zoom failure"));
        assert_eq!(canvas.texture.as_ref().unwrap().scale, 1.);
    }
    // A new density queues a real worker and leaves the last painted image in place.
    assert!(!uploaded(&mut canvas, &ctx, &doc, 0, 1.5));
    assert!(canvas.refinement.is_some());
    assert!(canvas.refinement_warning().is_some());
    assert_eq!(canvas.texture.as_ref().unwrap().scale, 1.);
    // Change density again while that worker is running. A late success for
    // the abandoned density must leave both the old artwork and warning intact.
    let (finish_old, old_result) = mpsc::channel();
    canvas.refinement.as_mut().unwrap().result = old_result;
    assert!(!uploaded(&mut canvas, &ctx, &doc, 0, 1.75));
    assert_eq!(canvas.refinement.as_ref().unwrap().scale, 1.5);
    finish_old
        .send(Ok((ColorImage::new([1, 1], egui::Color32::RED), None)))
        .unwrap();
    assert!(!uploaded(&mut canvas, &ctx, &doc, 0, 1.75));
    assert_eq!(canvas.refinement.as_ref().unwrap().scale, 1.75);
    assert_eq!(canvas.texture.as_ref().unwrap().scale, 1.);
    assert!(canvas.refinement_warning().is_some());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !uploaded(&mut canvas, &ctx, &doc, 0, 1.75) {
        assert!(
            std::time::Instant::now() < deadline,
            "density recovery timed out"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(canvas.refinement.is_none());
    assert!(canvas.refinement_warning().is_none());
    assert_eq!(canvas.texture.as_ref().unwrap().scale, 1.75);
    let (send, result) = mpsc::channel();
    send.send(Err("Stale failure".into())).unwrap();
    canvas.refinement = Some(Refinement {
        document: snapshot,
        frame: 0,
        scale: 1.5,
        result,
    });
    doc.objects[0].fill = [255; 4];
    assert!(uploaded(&mut canvas, &ctx, &doc, 0, 1.5));
    assert!(canvas.refinement_warning().is_none());
}

fn uploaded(
    canvas: &mut Canvas,
    ctx: &egui::Context,
    doc: &Document,
    frame: u32,
    scale: f32,
) -> bool {
    let mut id = None;
    let out = ctx.run(
        egui::RawInput {
            time: Some(1.),
            ..Default::default()
        },
        |ctx| {
            id = Some(canvas.frame_texture(ctx, doc, frame, scale).unwrap());
        },
    );
    out.textures_delta.set.iter().any(|(k, _)| Some(*k) == id)
}

#[test]
fn worker_placement_reuses_exact_page_pixels_only_at_the_matching_density() {
    let ctx = egui::Context::default();
    let mut canvas = Canvas::default();
    let mut base = Document::default();
    base.add_rectangle([8.; 2], [72.; 2], [32, 96, 64, 255]);
    assert!(uploaded(&mut canvas, &ctx, &base, 0, 1.));
    let resolution = canvas.placement_resolution(&ctx).unwrap();
    let mut placed = base.clone();
    let id = placed.add_rectangle([17.; 2], [35.; 2], [0; 4]);
    let mut artwork = Document::default();
    artwork.add_rectangle([2., 20.], [85., 73.], [240, 180, 100, 171]);
    placed.objects.last_mut().unwrap().embedded_artwork = Some(Arc::new(artwork).into());
    let placed = Arc::new(placed);
    let page = super::placement::PlacementPage::render(&placed, 0, id, resolution).unwrap();
    canvas.install_placement_page(&ctx, page);
    // The upload performed by installation is delivered at the next end_pass.
    let _ = ctx.run(Default::default(), |_| {});
    assert!(!uploaded(&mut canvas, &ctx, &placed, 0, 1.));
    let expected = color_image(renderer::render_scaled(&placed, 0, 1.).unwrap());
    assert_eq!(
        canvas.texture.as_ref().unwrap().sample_pixels.as_deref(),
        Some(&expected)
    );

    let mut zoomed = Canvas::default();
    assert!(uploaded(&mut zoomed, &ctx, &base, 0, 2.));
    let stale = super::placement::PlacementPage::render(&placed, 0, id, resolution).unwrap();
    zoomed.install_placement_page(&ctx, stale);
    assert_eq!(zoomed.texture.as_ref().unwrap().authored().as_ref(), &base);
    assert!(uploaded(&mut zoomed, &ctx, &placed, 0, 2.));
}

#[test]
fn pending_split_promotes_only_the_current_drawing_frame_and_density() {
    let mut base = Document::default();
    base.add_rectangle([8.; 2], [14.; 2], [120; 4]);
    let mut target = base.clone();
    target.objects[0].size[0] += 1.;
    let target = Arc::new(target);
    let mut recolored = target.as_ref().clone();
    recolored.objects[0].fill = [240; 4];

    for (drawing, frame, scale, accepted) in [
        (target.as_ref(), 0, 1., true),
        (&base, 0, 1., false), // Undo before the worker completes.
        (&recolored, 0, 1., false),
        (target.as_ref(), 1, 1., false),
        (target.as_ref(), 0, 2., false),
    ] {
        let (mut canvas, ctx) = (Canvas::default(), egui::Context::default());
        assert!(uploaded(&mut canvas, &ctx, &base, 0, 1.));
        let (send, result) = mpsc::channel();
        canvas.texture.as_mut().unwrap().pending_split = Some(Arc::clone(&target));
        canvas.refinement = Some(Refinement {
            document: Arc::clone(&target),
            frame: 0,
            scale: 1.,
            result,
        });
        uploaded(&mut canvas, &ctx, drawing, frame, scale);
        assert_eq!(
            canvas.texture.as_ref().unwrap().pending_split.is_some(),
            accepted
        );
        send.send(render_image(&target, 0, 1.)).unwrap();
        assert_eq!(uploaded(&mut canvas, &ctx, drawing, frame, scale), accepted);
        let cached = canvas.texture.as_ref().unwrap();
        assert!(cached.pending_split.is_none());
        assert_eq!(cached.document.as_ref(), drawing);
        assert_eq!(cached.scale, scale);
    }
}

#[test]
fn static_nested_art_reuses_pixels_across_ticks_until_the_source_changes() {
    let mut p = AnimationProject::default();
    let child = p.add_clip("Still drawing").unwrap();
    p.clips[child].set_length(5).unwrap();
    p.clips[child]
        .drawing_at_mut(0)
        .add_rectangle([6.; 2], [12.; 2], [255; 4]);
    p.place_clip(p.clips[child].id, 0, 0, 1, [0.; 2]).unwrap();
    let (mut canvas, ctx) = (Canvas::default(), egui::Context::default());
    let parent = p.clips[0].drawing_at(0);
    assert!(uploaded(&mut canvas, &ctx, parent, 0, 1.));
    assert!(!uploaded(&mut canvas, &ctx, parent, 3, 1.));
    assert!(canvas
        .frame_texture(&ctx, parent, parent.frames, 1.)
        .is_err());

    // A settled zoom can finish after the playhead advances on the same still.
    let cached = canvas.texture.as_ref().unwrap();
    let (send, result) = mpsc::channel();
    send.send(render_image(&cached.document, 0, 2.)).unwrap();
    canvas.refinement = Some(Refinement {
        document: Arc::clone(&cached.document),
        frame: 0,
        scale: 2.,
        result,
    });
    canvas.sharpen_at = Some(0.5);
    assert!(uploaded(&mut canvas, &ctx, parent, 4, 2.));
    assert_eq!(canvas.texture.as_ref().unwrap().scale, 2.);
    assert!(canvas.refinement.is_none());

    p.clips[child].drawing_at_mut(0).objects[0].fill = [150, 90, 20, 255];
    p.hydrate_composition().unwrap();
    let parent = p.clips[0].drawing_at(0);
    assert!(uploaded(&mut canvas, &ctx, parent, 2, 2.));
    assert!(!uploaded(&mut canvas, &ctx, parent, 1, 2.));
    p.clips[child].drawing_at_mut(0).objects[0]
        .keys
        .extend([(0, [6.; 2]), (4, [18.; 2])]);
    p.hydrate_composition().unwrap();
    let parent = p.clips[0].drawing_at(0);
    assert!(uploaded(&mut canvas, &ctx, parent, 1, 2.));
    assert!(uploaded(&mut canvas, &ctx, parent, 2, 2.));
    p.clips[child].drawing_at_mut(0).objects[0].keys.clear();
    p.clips[child].blank_frame(0).unwrap();
    p.hydrate_composition().unwrap();
    let parent = p.clips[0].drawing_at(0);
    assert!(uploaded(&mut canvas, &ctx, parent, 0, 2.));
    assert!(uploaded(&mut canvas, &ctx, parent, 1, 2.));
}

#[test]
fn keys_on_hidden_artwork_motion_and_bones_remain_frame_dependent() {
    let mut doc = Document::default();
    doc.add_rectangle([0.; 2], [8.; 2], [0; 4]);
    let object = &mut doc.objects[0];
    object.animated.visible = false;
    object.opacity = 0.;
    object.animated.progress_keys.insert(3, 1.);
    assert!(!doc.frame_independent());
    doc.objects[0].animated.progress_keys.clear();
    doc.objects[0].animated.visible_keys.insert(1, true);
    assert!(!doc.frame_independent());
    doc.objects[0].animated.visible_keys.clear();
    assert!(doc.frame_independent());
    let bone = doc.add_bone("Arm", None, [0.; 2], [14., 0.]).unwrap();
    doc.set_bone_rotation_key(bone, 3, 36.).unwrap();
    assert!(!doc.frame_independent());
}
