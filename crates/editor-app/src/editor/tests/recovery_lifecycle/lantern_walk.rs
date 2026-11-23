use document_core::{AnimationProject, Clip, Document, ExportPreset, VectorObject};
use std::{path::Path, sync::Arc};

const MANUAL_EXPOSURE: u32 = 500;
const MANUAL_KEY: u32 = 720;
const PUBLISHED_EXPOSURE: u32 = 512;
const PUBLISHED_KEY: u32 = 750;
// The broad teal hood in the bundled walk is an authored cubic path.
const HOOD: u64 = 7;

fn hood(drawing: &Document) -> &VectorObject {
    drawing.objects.iter().find(|o| o.id == HOOD).unwrap()
}

fn hood_mut(drawing: &mut Document) -> &mut VectorObject {
    drawing.objects.iter_mut().find(|o| o.id == HOOD).unwrap()
}

pub(super) fn manual_source(path: &Path) -> AnimationProject {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lantern-walk.vanim");
    let mut project = project_io::load_animation(&fixture).unwrap();
    let clip = &mut project.clips[0];
    clip.set_length(1000).unwrap();
    clip.cels
        .insert(MANUAL_EXPOSURE, Arc::clone(&clip.cels[&0]));
    let hood = hood_mut(clip.drawing_at_mut(MANUAL_EXPOSURE));
    hood.keys.extend([
        (MANUAL_EXPOSURE, hood.position),
        (MANUAL_KEY, [21., 15.]),
        (999, [19., 14.]),
    ]);
    project_io::save_animation(path, &project, false).unwrap();
    project
}

pub(super) fn edited(mut project: AnimationProject, variant: &str) -> AnimationProject {
    let (start, key, position, point, incoming, outgoing) = match variant {
        "Published" => (
            PUBLISHED_EXPOSURE,
            PUBLISHED_KEY,
            [29., 20.],
            [0.52, -0.05],
            [0.26, 0.04],
            [0.70, 0.12],
        ),
        "Replacement" => (
            625,
            821,
            [9., 23.],
            [0.40, 0.18],
            [0.21, 0.22],
            [0.61, 0.25],
        ),
        _ => panic!("Unknown checkpoint revision {variant}"),
    };
    let clip = &mut project.clips[0];
    let drawing = clip.cels.remove(&MANUAL_EXPOSURE).unwrap();
    clip.cels.insert(start, drawing);
    let hood = hood_mut(clip.drawing_at_mut(start));
    let node = &mut hood.shape.contours_mut()[0][1];
    node.point = point;
    node.incoming = incoming;
    node.outgoing = outgoing;
    hood.keys.remove(&MANUAL_EXPOSURE).unwrap();
    hood.keys.remove(&MANUAL_KEY).unwrap();
    hood.keys.extend([(start, hood.position), (key, position)]);
    project
}

fn sheet(root: &Path, name: &str, clip: &Clip) -> renderer::Pixmap {
    let directory = root.join(name);
    let preset = ExportPreset {
        columns: 40,
        ..Default::default()
    };
    project_io::export_sprite_bundle(&directory, clip, &preset, false).unwrap();
    let sheet = renderer::Pixmap::load_png(directory.join("sheet.png")).unwrap();
    assert_eq!((sheet.width(), sheet.height()), (40 * 128, 25 * 128));
    sheet
}

pub(super) fn assert_checkpoint(
    root: &Path,
    expected: &AnimationProject,
    actual: &AnimationProject,
    all_pixels: bool,
) {
    assert_eq!(actual, expected);
    let recovered = &actual.clips[0];
    let reference = &expected.clips[0];
    assert_eq!((recovered.frames, recovered.fps), (1000, 8));
    assert_eq!(recovered.exposure(511), (0, 512));
    assert_eq!(recovered.exposure(999), (512, 488));
    let object = hood(recovered.drawing_at(PUBLISHED_EXPOSURE));
    let node = &object.shape.contours()[0][1];
    assert_eq!(node.point, [0.52, -0.05]);
    assert_eq!(node.incoming, [0.26, 0.04]);
    assert_eq!(node.outgoing, [0.70, 0.12]);
    assert_eq!(
        object.keys.keys().copied().collect::<Vec<_>>(),
        [512, 750, 999]
    );
    assert_eq!(object.position_at(PUBLISHED_KEY), [29., 20.]);
    let manual = project_io::load_animation(&root.join("Original.vanim")).unwrap();
    let original = &manual.clips[0];
    let before = renderer::render(original.drawing_at(PUBLISHED_KEY), PUBLISHED_KEY).unwrap();
    let after = renderer::render(reference.drawing_at(PUBLISHED_KEY), PUBLISHED_KEY).unwrap();
    assert!(
        before.data() != after.data(),
        "checkpoint edits must change visible artwork"
    );

    if all_pixels {
        let expected_sheet = sheet(root, "expected-sheet", reference);
        let actual_sheet = sheet(root, "recovered-sheet", recovered);
        assert!(
            actual_sheet.data() == expected_sheet.data(),
            "recovered 1,000-frame sheet differs"
        );
    } else {
        for tick in [0, 4, 499, 511, 512, 513, 749, 750, 751, 999] {
            let expected_png = root.join("expected-frame.png");
            let recovered_png = root.join("recovered-frame.png");
            project_io::export_clip_frame(&expected_png, reference, tick, 1, true).unwrap();
            project_io::export_clip_frame(&recovered_png, recovered, tick, 1, true).unwrap();
            let expected_image = renderer::Pixmap::load_png(expected_png).unwrap();
            let recovered_image = renderer::Pixmap::load_png(recovered_png).unwrap();
            assert!(
                expected_image.data() == recovered_image.data(),
                "recovered frame {tick} differs"
            );
        }
    }
}
