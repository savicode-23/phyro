use document_core::{AnimationProject, Clip, Document, Layer};
use project_io::{export_still, import_svg, StillFormat, StillOptions, StillTarget};
use std::{fs, io::ErrorKind};
use tiny_skia::Pixmap;

fn fixture() -> (Clip, [u64; 3]) {
    let mut doc = Document {
        width: 24,
        height: 20,
        frames: 2,
        ..Default::default()
    };
    let a = doc.add_rectangle([-16., 3.], [4., 4.], [23, 75, 92, 255]);
    let b = doc.add_rectangle([-4., 7.], [4., 4.], [27, 98, 138, 255]);
    let c = doc.add_rectangle([2., 2.], [1., 2.], [243, 78, 14, 255]);
    doc.objects[0]
        .keys
        .extend([(0, [-16., 3.]), (1, [-8., 3.])]);
    doc.objects[0].stroke_width = 7.;
    doc.objects[0].stroke = [200, 5, 219, 255];
    doc.objects[0].no_stroke = true;
    doc.objects[0].group = Some(1);
    doc.objects[1].group = Some(1);
    doc.rename_group(1, 1, "Wing.Left").unwrap();
    doc.layers.push(Layer {
        id: 2,
        name: "Front".into(),
        visible: true,
        locked: false,
    });
    doc.objects[2].layer = 2;
    doc.objects[2].group = Some(1);
    doc.rename_group(2, 1, "wing.left").unwrap();
    doc.add_rectangle([16., 14.], [4., 4.], [255; 4]);
    (
        AnimationProject::from_document(doc).clips.remove(0),
        [a, b, c],
    )
}

fn options(format: StillFormat) -> StillOptions {
    StillOptions {
        format,
        scale: 2,
        replace: false,
    }
}

#[test]
fn single_and_paired_stills_agree_for_artboards_selections_and_groups() {
    let dir = tempfile::tempdir().unwrap();
    let (clip, [a, b, c]) = fixture();
    let ids = [b, c];
    for (label, target) in [
        ("artboard", StillTarget::Artboard),
        ("selection", StillTarget::Selection(&ids)),
        ("objects", StillTarget::Objects(&ids)),
    ] {
        let out = dir.path().join(label);
        let mut steps = Vec::new();
        let pairs = export_still(
            &out,
            &clip,
            1,
            target,
            options(StillFormat::PngAndSvg),
            |done, total| {
                steps.push((done, total));
                true
            },
        )
        .unwrap();
        let expected: &[&str] = match target {
            StillTarget::Artboard => &["artboard.png", "artboard.svg"],
            StillTarget::Selection(_) => &["selection.png", "selection.svg"],
            StillTarget::Objects(_) => &[
                "Wing.Left.png",
                "Wing.Left.svg",
                "wing.left (2).png",
                "wing.left (2).svg",
            ],
        };
        assert_eq!(
            pairs
                .iter()
                .map(|asset| asset.path.file_name().unwrap().to_str().unwrap())
                .collect::<Vec<_>>(),
            expected
        );
        let total = expected.len() as u32;
        assert_eq!(
            steps,
            (0..=total).map(|done| (done, total)).collect::<Vec<_>>()
        );
        for (index, format) in [StillFormat::Png, StillFormat::Svg].into_iter().enumerate() {
            let path = dir.path().join(format!("{label}-{index}"));
            let singles =
                export_still(&path, &clip, 1, target, options(format), |_, _| true).unwrap();
            for (single, pair) in singles.iter().zip(pairs.iter().skip(index).step_by(2)) {
                assert_eq!(
                    fs::read(&single.path).unwrap(),
                    fs::read(&pair.path).unwrap()
                );
                assert_eq!(
                    (single.origin, single.size, &single.object_ids),
                    (pair.origin, pair.size, &pair.object_ids)
                );
                if index == 0 {
                    let image = Pixmap::load_png(&single.path).unwrap();
                    assert_eq!([image.width(), image.height()], single.size);
                } else {
                    let svg = fs::read_to_string(&single.path).unwrap();
                    let [x, y] = single.origin;
                    let [w, h] = single.size;
                    assert!(svg.contains(&format!(r#"viewBox="{x} {y} {w} {h}""#)));
                }
            }
        }
        match target {
            StillTarget::Artboard => {
                assert_eq!((pairs[0].origin, pairs[0].size), ([0, 0], [48, 40]));
                assert_eq!(pairs[1].size, [24, 20]);
            }
            StillTarget::Selection(_) => {
                assert_eq!(pairs[0].object_ids, [a, b, c]);
                assert_eq!((pairs[0].origin, pairs[0].size), ([-16, 4], [22, 18]));
                assert_eq!((pairs[1].origin, pairs[1].size), ([-8, 2], [11, 9]));
                let doc = import_svg(&pairs[1].path).unwrap();
                assert_eq!(doc.layers.len(), 2);
                assert_eq!(doc.objects.len(), 3);
                assert!(doc.objects[0].no_stroke);
                assert_eq!(doc.objects[0].group, doc.objects[1].group);
                assert_ne!(doc.objects[0].layer, doc.objects[2].layer);
                assert_eq!(doc.objects[0].group_name.as_deref(), Some("Wing.Left"));
                assert_eq!(doc.objects[2].group_name.as_deref(), Some("wing.left"));
            }
            StillTarget::Objects(_) => {
                assert_eq!(pairs[0].object_ids, [a, b]);
                assert_eq!(pairs[2].object_ids, [c]);
            }
        }
    }
}

#[test]
fn cancellation_and_overwrite_cover_each_single_and_folder_target() {
    let dir = tempfile::tempdir().unwrap();
    let (clip, ids) = fixture();
    let path = dir.path().join("output");
    for target in [
        StillTarget::Artboard,
        StillTarget::Selection(&ids),
        StillTarget::Objects(&ids),
    ] {
        for format in [StillFormat::Svg, StillFormat::Png, StillFormat::PngAndSvg] {
            let options = StillOptions {
                replace: true,
                ..options(format)
            };
            let error = export_still(&path, &clip, 1, target, options, |done, total| done < total)
                .unwrap_err();
            assert_eq!(error.kind(), ErrorKind::Interrupted);
            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
            fs::write(&path, b"existing").unwrap();
            let error = export_still(
                &path,
                &clip,
                1,
                target,
                StillOptions {
                    replace: false,
                    ..options
                },
                |_, _| true,
            )
            .unwrap_err();
            assert_eq!(error.kind(), ErrorKind::AlreadyExists);
            if format == StillFormat::PngAndSvg || matches!(target, StillTarget::Objects(_)) {
                assert_eq!(
                    export_still(&path, &clip, 1, target, options, |_, _| true)
                        .unwrap_err()
                        .kind(),
                    ErrorKind::AlreadyExists
                );
            } else {
                let error =
                    export_still(&path, &clip, 1, target, options, |done, total| done < total)
                        .unwrap_err();
                assert_eq!(error.kind(), ErrorKind::Interrupted);
            }
            assert_eq!(fs::read(&path).unwrap(), b"existing");
            fs::remove_file(&path).unwrap();
        }
    }
}

#[test]
fn a_directory_claimed_during_encoding_remains_empty_and_owned_by_its_creator() {
    let dir = tempfile::tempdir().unwrap();
    let (clip, ids) = fixture();
    for (index, target) in [
        StillTarget::Artboard,
        StillTarget::Selection(&ids),
        StillTarget::Objects(&ids),
    ]
    .into_iter()
    .enumerate()
    {
        let destination = dir.path().join(index.to_string());
        let error = export_still(
            &destination,
            &clip,
            1,
            target,
            options(StillFormat::PngAndSvg),
            |done, total| {
                if done == total {
                    fs::create_dir(&destination).unwrap();
                }
                true
            },
        )
        .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::AlreadyExists);
        assert_eq!(fs::read_dir(destination).unwrap().count(), 0);
    }
}

#[test]
fn measured_png_sizes_match_fractional_artboard_selection_and_batch_exports() {
    let dir = tempfile::tempdir().unwrap();
    let (clip, ids) = fixture();
    for (index, target) in [
        StillTarget::Artboard,
        StillTarget::Selection(&ids),
        StillTarget::Objects(&ids),
    ]
    .into_iter()
    .enumerate()
    {
        let sizes = project_io::png_dimensions(&clip, 1, target, 0.7, || true).unwrap();
        let out = dir.path().join(index.to_string());
        let files = export_still(
            &out,
            &clip,
            1,
            target,
            StillOptions {
                format: StillFormat::PngAndSvg,
                scale: 0.7,
                replace: false,
            },
            |_, _| true,
        )
        .unwrap();
        assert_eq!(sizes.assets.len() * 2, files.len());
        for ((_, expected), pair) in sizes.assets.iter().zip(files.as_chunks::<2>().0) {
            let image = Pixmap::load_png(&pair[0].path).unwrap();
            assert_eq!(*expected, [image.width(), image.height()]);
            assert_eq!(pair[0].size, *expected);
        }
        if target == StillTarget::Artboard {
            assert_eq!(sizes.assets[0].1, [17, 14]);
            assert_eq!(files[1].size, [24, 20]);
        }
    }
}

#[test]
fn width_and_height_follow_native_ink_even_for_rasters_with_transparent_margins() {
    let mut doc = Document {
        frames: 1,
        ..Default::default()
    };
    let id = doc.add_rectangle([-51., -7.], [100., 10.], [0; 4]);
    let mut pixels = vec![0; 4 * 100 * 10];
    for y in 2..8 {
        for x in 40..50 {
            let i = (y * 100 + x) * 4;
            pixels[i..i + 4].copy_from_slice(&[75, 255, 90, 255]);
        }
    }
    doc.objects[0].image = Some(std::sync::Arc::new(
        document_core::RasterImage::from_rgba(100, 10, pixels).unwrap(),
    ));
    let project = AnimationProject::from_document(doc);
    let clip = &project.clips[0];
    let ids = [id];
    let target = StillTarget::Selection(&ids);
    let dir = tempfile::tempdir().unwrap();
    for (i, (scale, size)) in [(1., [10, 6]), (0.5, [5, 3]), (1.3, [13, 8]), (5., [50, 30])]
        .into_iter()
        .enumerate()
    {
        let fit = project_io::png_dimensions(clip, 0, target, scale, || true).unwrap();
        assert_eq!(fit.native, [10, 6]);
        assert_eq!(fit.assets[0].1, size);
        let file = dir.path().join(format!("{i}.png"));
        let assets = export_still(
            &file,
            clip,
            0,
            target,
            StillOptions {
                format: StillFormat::Png,
                scale,
                replace: false,
            },
            |_, _| true,
        )
        .unwrap();
        assert_eq!(assets[0].size, size);
        let image = Pixmap::load_png(file).unwrap();
        assert_eq!([image.width(), image.height()], size);
        assert!(image.pixel(size[0] / 2, size[1] / 2).unwrap().alpha() > 240);
    }
}

#[test]
fn huge_vector_selection_can_export_below_native_resolution() {
    let mut doc = Document {
        frames: 1,
        ..Default::default()
    };
    let id = doc.add_rectangle([-2000., -3000.], [8000., 6000.], [255; 4]);
    let project = AnimationProject::from_document(doc);
    let clip = &project.clips[0];
    let ids = [id];
    let target = StillTarget::Selection(&ids);
    let fit = project_io::png_dimensions(clip, 0, target, 0.01, || true).unwrap();
    assert_eq!(fit.assets[0].1, [80, 60]);
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("small.png");
    let files = export_still(
        &out,
        clip,
        0,
        target,
        StillOptions {
            format: StillFormat::Png,
            scale: 0.01,
            replace: false,
        },
        |_, _| true,
    )
    .unwrap();
    assert_eq!(files[0].size, [80, 60]);
    assert_eq!(files[0].origin, [-20, -30]);
    assert_eq!(
        Pixmap::load_png(out)
            .unwrap()
            .pixel(40, 30)
            .unwrap()
            .alpha(),
        255
    );
}
