use super::*;

#[test]
fn hiding_then_reopening_the_pasteboard_discards_a_late_worker_result() {
    let ctx = egui::Context::default();
    let (tx, rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let mut margins = Pasteboard {
        job: Some(Job {
            key: [-1, 1],
            cancel: cancel.clone(),
            rx,
        }),
        ..Default::default()
    };
    for _ in 0..3 {
        margins.clear(&ctx);
        assert!(cancel.load(Ordering::Relaxed));
        assert!(Arc::ptr_eq(&margins.job.as_ref().unwrap().cancel, &cancel));
    }
    tx.send(Ok(Patch {
        origin: [0, 0],
        size: [TEXTURE as usize; 2],
        pixels: None,
    }))
    .unwrap();
    margins.poll(&ctx);
    assert!(margins.job.is_none() && margins.tiles.is_empty());
    assert!(margins.warning().is_none());
}

#[test]
fn worker_failure_retains_usable_pixels_and_reports_the_failure() {
    let ctx = egui::Context::default();
    let mut margins = Pasteboard::default();
    let key = [2, -1];
    let image = renderer::Pixmap::new(TEXTURE, TEXTURE).unwrap();
    margins.store(
        &ctx,
        key,
        Ok(Patch {
            origin: [0, 0],
            size: [TEXTURE as usize; 2],
            pixels: Some(image),
        }),
    );
    let id = margins.tiles[&key].as_ref().unwrap().id();
    let (tx, rx) = mpsc::channel();
    margins.job = Some(Job {
        key,
        cancel: Arc::new(AtomicBool::new(false)),
        rx,
    });
    drop(tx);
    margins.poll(&ctx);
    assert_eq!(margins.tiles[&key].as_ref().unwrap().id(), id);
    assert!(margins.warning().unwrap().contains("stopped unexpectedly"));
}
