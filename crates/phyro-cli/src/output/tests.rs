use super::*;
use document_core::Document;
use renderer::Pixmap;

fn project() -> AnimationProject {
    let mut artwork = Document {
        width: 12,
        height: 10,
        frames: 5,
        fps: 20,
        ..Default::default()
    };
    artwork.add_rectangle([1., 1.], [3., 3.], [220, 40, 70, 255]);
    let mut project = AnimationProject::from_document(artwork);
    let clip = &mut project.clips[0];
    clip.origin = [2.5, 7.];
    let mut drawing = clip.drawing_at(0).clone();
    drawing.objects[0].fill = [16, 100, 200, 255];
    drawing.objects[0]
        .keys
        .extend([(2, [1., 1.]), (4, [5., 1.])]);
    clip.cels.insert(2, std::sync::Arc::new(drawing));
    project.preset.scale = 2;
    project
}

#[test]
fn preview_and_all_exports_agree_with_the_active_held_drawing() {
    let tmp = tempfile::tempdir().unwrap();
    let project = project();
    let clip = &project.clips[0];
    let png = tmp.path().join("preview.png");
    let result = preview(&project, clip.id, 3, &png, 2.).unwrap();
    assert_eq!(result["files"], json!([png]));
    assert_eq!(result["frame_size"], json!([24, 20]));
    assert_eq!(result["pivot"], json!([5., 14.]));
    assert_eq!(result["exposure_start"], 2);
    assert_eq!(result["exposure_duration"], 3);
    let expected = renderer::render_scaled(clip.drawing_at(3), 3, 2.).unwrap();
    assert_eq!(Pixmap::load_png(&png).unwrap().data(), expected.data());
    for (format, file_count) in [
        ("frames", 5),
        ("sheet", 2),
        ("sheet-and-frames", 7),
        ("packed", 3),
    ] {
        let path = tmp.path().join(format);
        let columns = matches!(format, "sheet" | "sheet-and-frames").then_some(2);
        let result = export(&project, clip.id, &path, format, columns).unwrap();
        assert_eq!(result["frame_count"], 5);
        assert_eq!(result["fps"], 20);
        assert_eq!(result["frame_size"], json!([24, 20]));
        assert_eq!(result["pivot"], json!([5., 14.]));
        let files = result["files"].as_array().unwrap();
        assert_eq!(files.len(), file_count);
        assert!(files
            .iter()
            .all(|file| Path::new(file.as_str().unwrap()).is_file()));
        if format == "frames" {
            assert!(result["metadata"].is_null());
            assert_eq!(
                fs::read(path.join("frame_0003.png")).unwrap(),
                fs::read(&png).unwrap()
            );
        } else {
            let meta: Value =
                serde_json::from_slice(&fs::read(result["metadata"].as_str().unwrap()).unwrap())
                    .unwrap();
            assert_eq!(meta["frames"][3]["pivot"], result["pivot"]);
            assert_eq!(meta["fps"], result["fps"]);
            if columns.is_some() {
                assert_eq!(meta["columns"], 2);
                let frame = &meta["frames"][3];
                let sheet = Pixmap::load_png(path.join("sheet.png")).unwrap();
                let x = frame["x"].as_u64().unwrap() as u32;
                let y = frame["y"].as_u64().unwrap() as u32;
                for row in 0..expected.height() {
                    for col in 0..expected.width() {
                        assert_eq!(expected.pixel(col, row), sheet.pixel(x + col, y + row));
                    }
                }
            }
        }
    }
}

#[test]
fn bad_requests_and_existing_paths_preserve_the_destination() {
    let tmp = tempfile::tempdir().unwrap();
    let project = project();
    let id = project.clips[0].id;
    let out = tmp.path().join("destination");
    assert!(preview(&project, id + 1, 0, &out, 1.).is_err());
    assert_eq!(
        preview(&project, id, 5, &out, 1.).unwrap_err().code,
        "frame_out_of_range"
    );
    assert!(preview(&project, id, 1, &out, 0.).is_err());
    assert!(export(&project, id, &out, "frames", Some(4)).is_err());
    assert!(export(&project, id, &out, "packed", Some(4)).is_err());
    assert!(export(&project, id, &out, "sheet", Some(0)).is_err());
    assert!(export(&project, id, &out, "unknown", None).is_err());
    assert!(preview(&project, id, 1, &out, 33.).is_err());
    assert!(!out.exists());
    fs::write(&out, b"artist's existing file").unwrap();
    assert_eq!(
        preview(&project, id, 1, &out, 1.).unwrap_err().code,
        "output_exists"
    );
    assert_eq!(
        export(&project, id, &out, "sheet", None).unwrap_err().code,
        "output_exists"
    );
    assert_eq!(fs::read(&out).unwrap(), b"artist's existing file");
    assert_eq!(
        export(&project, id, tmp.path(), "frames", None)
            .unwrap_err()
            .code,
        "output_exists"
    );
    assert_eq!(tmp.path().read_dir().unwrap().count(), 1);
}
