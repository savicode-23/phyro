use document_core::Document;
use project_io::{
    export_frame, export_sequence, export_sequence_with_progress, load_project, save_project,
};
use std::fs;
use std::io::{ErrorKind, Write};

fn moving_rectangle() -> Document {
    let mut document = Document::default();
    document.add_rectangle([2.0, 4.0], [3.0, 3.0], [255, 64, 32, 255]);
    document.objects[0].keys.insert(0, [2.0, 4.0]);
    document.objects[0].keys.insert(4, [18.0, 4.0]);
    document
}

#[test]
fn bundled_example_exports_the_expected_motion() {
    let project = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/moving-rectangle.vanim");
    let doc = load_project(&project).unwrap();
    let directory = tempfile::tempdir().unwrap();
    for (frame, x) in [12, 32, 52, 72, 92].into_iter().enumerate() {
        let path = directory.path().join(format!("{frame}.png"));
        export_frame(&path, &doc, frame as u32).unwrap();
        let image = tiny_skia::Pixmap::load_png(path).unwrap();
        assert_eq!(image.pixel(x - 1, 52).unwrap().alpha(), 0);
        assert_eq!(image.pixel(x, 52).unwrap().alpha(), 255);
        assert_eq!(image.pixel(x + 23, 75).unwrap().alpha(), 255);
        assert_eq!(image.pixel(x + 24, 75).unwrap().alpha(), 0);
    }
}

#[test]
fn reopen_and_export_five_aligned_transparent_frames() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("walk.vanim");
    let sequence = temp.path().join("walk");
    let single = temp.path().join("selected.png");
    let original = moving_rectangle();
    save_project(&project, &original).unwrap();
    let restored = load_project(&project).unwrap();
    assert_eq!(original, restored);
    export_sequence(&sequence, &restored).unwrap();
    assert_eq!(fs::read_dir(&sequence).unwrap().count(), 5);
    for frame in 0..5 {
        let path = sequence.join(format!("frame_{frame:04}.png"));
        let image = tiny_skia::Pixmap::load_png(path).unwrap();
        assert_eq!((image.width(), image.height()), (128, 128));
        let pixel = image.pixel(3 + 4 * frame, 5).unwrap();
        assert_eq!(
            (pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()),
            (255, 64, 32, 255)
        );
        assert_eq!(image.pixel(1 + 4 * frame, 5).unwrap().alpha(), 0);
        assert_eq!(image.pixel(0, 0).unwrap().alpha(), 0);
    }
    export_frame(&single, &restored, 2).unwrap();
    assert_eq!(
        fs::read(single).unwrap(),
        fs::read(sequence.join("frame_0002.png")).unwrap()
    );
}

#[test]
fn failed_operations_preserve_existing_artwork() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("art.vanim");
    let png = temp.path().join("art.png");
    let mut document = moving_rectangle();
    save_project(&project, &document).unwrap();
    let saved = fs::read(&project).unwrap();
    document.fps = 0;
    assert!(save_project(&project, &document).is_err());
    assert_eq!(fs::read(&project).unwrap(), saved);
    fs::write(&png, b"keep").unwrap();
    assert!(export_frame(&png, &moving_rectangle(), 99).is_err());
    assert_eq!(fs::read(&png).unwrap(), b"keep");
    let document = moving_rectangle();
    let conflict = project_io::save_new_project(&project, &document).unwrap_err();
    assert_eq!(conflict.kind(), ErrorKind::AlreadyExists);
    let conflict = project_io::export_new_frame(&png, &document, 0).unwrap_err();
    assert_eq!(conflict.kind(), ErrorKind::AlreadyExists);
    assert_eq!(fs::read(&png).unwrap(), b"keep");
    assert!(load_project(&png).is_err());
}

#[test]
fn cancelled_sequence_can_be_retried_without_clobbering() {
    let temp = tempfile::tempdir().unwrap();
    let sequence = temp.path().join("clip");
    let document = moving_rectangle();
    let error =
        export_sequence_with_progress(&sequence, &document, |done, _| done < 2).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Interrupted);
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
    export_sequence(&sequence, &document).unwrap();
    let first = sequence.join("frame_0000.png");
    fs::write(&first, b"edited frame").unwrap();
    assert_eq!(
        export_sequence(&sequence, &document).unwrap_err().kind(),
        ErrorKind::AlreadyExists
    );
    assert_eq!(fs::read(first).unwrap(), b"edited frame");
}

#[test]
fn rejects_future_formats_and_broken_scene_data_in_valid_containers() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("open.vanim");
    for (version, expected) in [(3, "Unsupported project format"), (1, "Invalid document")] {
        let file = fs::File::create(&project).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("manifest.json", options).unwrap();
        let header = serde_json::json!({ "format": "Loopline", "format_version": version });
        serde_json::to_writer(&mut zip, &header).unwrap();
        zip.start_file("document.json", options).unwrap();
        zip.write_all(b"{}").unwrap();
        drop(zip.finish().unwrap());
        let error = load_project(&project).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidData);
        assert!(error.to_string().contains(expected), "{error}");
    }
}
