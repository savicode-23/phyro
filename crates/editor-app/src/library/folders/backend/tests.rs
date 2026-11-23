use super::*;

fn eventually<T>(mut f: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(value) = f() {
            return value;
        }
        assert!(Instant::now() < deadline, "asset worker timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn listing_never_decodes_files_and_navigation_replaces_pending_work() {
    let dir = tempfile::tempdir().unwrap();
    let children = dir.path().join("other");
    fs::create_dir(&children).unwrap();
    let trash = dir.path().join("readme.txt");
    fs::write(&trash, "hello").unwrap();
    for i in 0..40 {
        fs::write(dir.path().join(format!("{i}.PNG")), "undecodable").unwrap();
    }
    let cache = dir.path().join("cache");
    let mut browser = Backend::with_cache(&egui::Context::default(), cache.clone());
    browser.navigate(dir.path().to_path_buf(), Vec::new());
    let entries = eventually(|| browser.poll()).unwrap();
    assert_eq!(entries.len(), 41);
    assert_eq!(entries[0].kind, Kind::Folder);
    assert!(!cache.exists());
    let keys: Vec<_> = entries[1..].iter().map(PreviewKey::thumbnail).collect();
    browser.set_requests(&keys, true);
    assert!(browser.shared.state.lock().unwrap().started.is_empty());
    browser.set_requests(&keys, false);
    eventually(|| (browser.shared.state.lock().unwrap().ready.len() == MAX_READY).then_some(()));
    assert_eq!(
        browser.shared.state.lock().unwrap().started.len(),
        MAX_READY
    );
    browser.navigate(children, entries);
    assert!(eventually(|| browser.poll()).unwrap().is_empty());
    assert!(browser.take_preview().is_none());
    assert!(!cache.exists());
}

#[test]
fn artwork_thumbnails_and_clip_metadata_survive_a_warm_cache_and_change_on_rewrite() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("flame.vanim");
    let sample = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/guided-flame.vanim");
    fs::copy(&sample, &path).unwrap();
    let entry = Entry {
        name: "flame.vanim".into(),
        path: path.clone(),
        kind: Kind::Project,
    };
    let key = PreviewKey::thumbnail(&entry);
    let old = stamp(&path).unwrap();
    let cache = dir.path().join("cache");
    let mut renderer = assets::Renderer::new(Some(cache.clone()));
    let initial = renderer.render(&key, old).unwrap();
    let clip = &initial.clips[0];
    assert!(clip.frames > 1);
    assert!(!clip.name.is_empty());
    assert!(initial.image.size.iter().all(|&n| n <= 128));
    let frame = renderer
        .render(&PreviewKey::frame(&entry, clip.id, clip.frames / 2), old)
        .unwrap();
    assert!(frame.clips.is_empty());
    assert_ne!(initial.image.pixels, frame.image.pixels);
    assert_eq!(fs::read_dir(&cache).unwrap().count(), 1);
    let new = stamp(&path).unwrap();
    fs::write(&path, b"corrupt").unwrap();
    let mut warm = assets::Renderer::new(Some(cache));
    assert_eq!(
        warm.render(&key, new).unwrap().image.pixels,
        initial.image.pixels
    );
    assert!(warm.render(&key, stamp(&path).unwrap()).is_err());
}

#[test]
fn thumbnails_fit_nonsquare_images_and_svg_pages() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wide.png");
    tiny_skia::Pixmap::new(320, 80)
        .unwrap()
        .save_png(&path)
        .unwrap();
    let svg = dir.path().join("wide.svg");
    fs::write(&svg, r#"<svg xmlns="http://www.w3.org/2000/svg" width="320" height="80"><rect x="0" y="0" width="20" height="10"/></svg>"#).unwrap();
    let mut renderer = assets::Renderer::new(None);
    for (kind, path) in [(Kind::Image, path.clone()), (Kind::Svg, svg)] {
        let entry = Entry {
            path,
            kind,
            name: String::new(),
        };
        let image = renderer
            .render(&PreviewKey::thumbnail(&entry), stamp(&entry.path).unwrap())
            .unwrap()
            .image;
        assert_eq!(image.size, [128, 32]);
    }
}

#[test]
fn a_visible_source_edited_in_place_gets_a_fresh_preview_without_rescanning() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("changing.png");
    tiny_skia::Pixmap::new(16, 20)
        .unwrap()
        .save_png(&path)
        .unwrap();
    let mut browser = Backend::with_cache(&egui::Context::default(), dir.path().join("cache"));
    browser.navigate(dir.path().to_owned(), Vec::new());
    let entries = eventually(|| browser.poll()).unwrap();
    let keys = [PreviewKey::thumbnail(&entries[0])];
    browser.watch(std::slice::from_ref(&path));
    browser.set_requests(&keys, false);
    assert_eq!(
        eventually(|| browser.take_preview()).1.unwrap().image.size,
        [16, 20]
    );
    browser.set_requests(&[], false);
    fs::write(&path, b"broken").unwrap();
    assert_eq!(eventually(|| browser.take_changed()), path);
    browser.set_requests(&keys, false);
    assert!(eventually(|| browser.take_preview()).1.is_err());
    assert!(browser.poll().is_none());
}

#[test]
fn requesting_a_new_animation_frame_invalidates_old_queued_frames_and_clip_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fox.vanim");
    let mut source = document_core::AnimationProject::from_document(document_core::Document {
        width: 32,
        height: 32,
        ..Default::default()
    });
    let id = source.clips[0].id;
    source.clips[0].name = "Run".into();
    source.clips[0].set_length(4).unwrap();
    project_io::save_animation(&path, &source, false).unwrap();
    let mut browser = Backend::with_cache(&egui::Context::default(), dir.path().join("cache"));
    browser.navigate(dir.path().to_owned(), Vec::new());
    let entries = eventually(|| browser.poll()).unwrap();
    let poster = PreviewKey::thumbnail(&entries[0]);
    let first = PreviewKey::frame(&entries[0], id, 1);
    let next = PreviewKey::frame(&entries[0], id, 2);
    // No watcher is registered here. A frame read itself must invalidate the old poster.
    browser.set_requests(&[poster.clone(), first.clone()], false);
    eventually(|| (browser.shared.state.lock().unwrap().ready.len() == 2).then_some(()));
    let (key, initial) = browser.take_preview().unwrap();
    assert_eq!(key, poster);
    assert_eq!(initial.unwrap().clips[0].name, "Run");
    source.resize_artboard([64, 32]).unwrap();
    source.clips[0].name = "Summer run".into();
    project_io::save_animation(&path, &source, true).unwrap();
    browser.set_requests(&[next.clone(), first.clone()], false);
    assert_eq!(eventually(|| browser.take_changed()), path);
    for expected in [next, first] {
        let (key, frame) = eventually(|| browser.take_preview());
        assert_eq!(key, expected);
        assert_eq!(frame.unwrap().image.size, [128, 64]);
    }
    browser.set_requests(&[poster], false);
    let refreshed = eventually(|| browser.take_preview()).1.unwrap();
    assert_eq!(refreshed.clips[0].name, "Summer run");
    assert_eq!(refreshed.image.size, [128, 64]);
}
