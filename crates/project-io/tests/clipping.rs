use document_core::{AnimationProject, Document, PathNode, Shape};
use project_io::{
    export_svg, import_svg, load_animation, load_animation_bytes, load_project, save_animation,
    save_project,
};
use std::fs::File;
use std::io::{Cursor, Read, Write};
use std::path::Path;
use std::sync::Arc;
use zip::{ZipArchive, ZipWriter};

fn group_with_path() -> (Document, [u64; 3]) {
    let mut d = Document::default();
    let first = d.add_rectangle([5., 10.], [37., 35.], [33, 112, 131, 255]);
    let second = d.add_rectangle([34., 15.], [36., 35.], [216, 89, 56, 255]);
    for o in &mut d.objects {
        o.group = Some(7);
    }
    let cutter = d.add_shape(
        [12., 7.],
        [47., 39.],
        [233, 190, 54, 250],
        Shape::Path {
            nodes: [[0.5, 0.], [1., 0.5], [0.5, 1.], [0., 0.5]]
                .map(PathNode::corner)
                .to_vec(),
            closed: true,
        },
        1,
    );
    let path = d.objects.last_mut().unwrap();
    path.name = "Window outline".into();
    path.stroke = [21, 21, 21, 200];
    path.stroke_width = 2.5;
    (d, [first, second, cutter])
}

fn entries(path: &Path) -> (serde_json::Value, serde_json::Value) {
    let mut archive = ZipArchive::new(File::open(path).unwrap()).unwrap();
    let mut read = |name| {
        let mut data = String::new();
        archive
            .by_name(name)
            .unwrap()
            .read_to_string(&mut data)
            .unwrap();
        serde_json::from_str(&data).unwrap()
    };
    (read("manifest.json"), read("document.json"))
}

fn encoded(head: &serde_json::Value, doc: &serde_json::Value) -> Vec<u8> {
    let mut z = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, payload) in [("manifest.json", head), ("document.json", doc)] {
        z.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        z.write_all(&serde_json::to_vec(payload).unwrap()).unwrap();
    }
    z.finish().unwrap().into_inner()
}

#[test]
fn resaving_either_project_header_preserves_the_original_wire_format() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lantern-walk.vanim");
    let (mut manifest, document) = entries(&source);
    let project = load_animation(&source).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let saved = temp.path().join("walk.vanim");
    let original_manifest = manifest.clone();
    assert_eq!(original_manifest["format"], "Loopline");
    for name in ["Loopline", "Phyro"] {
        manifest["format"] = name.into();
        let reopened = load_animation_bytes(&encoded(&manifest, &document)).unwrap();
        assert_eq!(reopened, project);
        save_animation(&saved, &reopened, true).unwrap();
        assert_eq!(entries(&saved).0, original_manifest);
    }
}

#[test]
fn rectangular_paint_selects_version_17_in_drawings_clips_and_symbol_artwork() {
    gradient_archive_roundtrip(17, |o, gradient| o.gradient = Some(gradient));
}

#[test]
fn stroke_paint_selects_version_18_in_drawings_clips_and_symbol_artwork() {
    gradient_archive_roundtrip(18, |o, mut gradient| {
        gradient.kind = document_core::GradientKind::Linear {
            start: [0., 0.5],
            end: [1., 0.5],
        };
        o.stroke_gradient = Some(gradient);
    });
}

fn gradient_archive_roundtrip(
    version: u32,
    apply: impl FnOnce(&mut document_core::VectorObject, document_core::Gradient),
) {
    use document_core::{Gradient, GradientKind};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("box-color.vanim");
    let (mut doc, ids) = group_with_path();
    let gradient = Gradient::from_color(
        GradientKind::Rectangular {
            center: [0.2, 0.35],
            radius: 0.8,
        },
        [122, 92, 14, 197],
    );
    doc.objects[0].pivot = Some([0.; 2]);
    save_project(&path, &doc).unwrap();
    assert_eq!(entries(&path).0["format_version"], 16);
    assert_eq!(load_project(&path).unwrap(), doc);
    apply(&mut doc.objects[2], gradient);
    save_project(&path, &doc).unwrap();
    assert_eq!(entries(&path).0["format_version"], version);
    assert_eq!(load_project(&path).unwrap(), doc);

    doc.set_clip(&ids, 0, None).unwrap();
    assert!(doc
        .objects
        .iter()
        .all(|o| o.gradient.is_none() && o.stroke_gradient.is_none()));
    let mut animation = AnimationProject::from_document(doc);
    save_animation(&path, &animation, true).unwrap();
    let (mut header, payload) = entries(&path);
    assert_eq!(header["format_version"], version);
    assert_eq!(load_animation(&path).unwrap(), animation);
    header["format_version"] = (version - 1).into();
    assert!(load_animation_bytes(&encoded(&header, &payload))
        .unwrap_err()
        .to_string()
        .contains(&format!("format {version}")));

    let (symbol, _) = animation
        .convert_to_symbol(0, 0, &ids[..2], "Window")
        .unwrap();
    for detach in [false, true] {
        if detach {
            animation.remove_symbol_keeping_artwork(symbol).unwrap();
        }
        save_animation(&path, &animation, true).unwrap();
        assert_eq!(entries(&path).0["format_version"], version);
        assert_eq!(load_animation(&path).unwrap(), animation);
    }
}

#[test]
fn a_transform_pivot_survives_native_save_clipping_and_sampled_svg_export() {
    let (mut drawing, ids) = group_with_path();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hinge.vanim");
    let cutter = drawing.objects.last_mut().unwrap();
    let hinge = cutter.local_to_world([0., 0.5], 0);
    assert!(cutter.set_pivot_at(0, hinge));
    cutter.rotation_keys.extend([(0, 0.), (6, 90.)]);
    drawing.frames = 7;
    save_project(&path, &drawing).unwrap();
    let reopened = load_project(&path).unwrap();
    assert_eq!(reopened, drawing);
    let halfway = reopened.objects.last().unwrap().pivot_at(3);
    assert!((halfway[0] - hinge[0]).hypot(halfway[1] - hinge[1]) < 0.001);

    let svg = dir.path().join("pose.svg");
    export_svg(&svg, &reopened, 3, false).unwrap();
    let snapshot = import_svg(&svg).unwrap();
    let from_svg = renderer::render(&snapshot, 0).unwrap();
    let native = renderer::render(&reopened, 3).unwrap();
    assert_eq!(from_svg.width(), native.width());
    assert_eq!(from_svg.height(), native.height());
    // The SVG reader bakes the transform into path coordinates. Allow its
    // subpixel rounding on antialiased edges when comparing the sampled pose.
    assert!(from_svg
        .data()
        .iter()
        .zip(native.data())
        .all(|(&a, &b)| a.abs_diff(b) <= 5));

    drawing.set_clip(&ids, 0, None).unwrap();
    assert!(drawing.objects.iter().all(|o| o.pivot.is_none()));
    let project = AnimationProject::from_document(drawing);
    save_animation(&path, &project, true).unwrap();
    let (mut manifest, payload) = entries(&path);
    assert_eq!(manifest["format_version"], 16);
    assert_eq!(load_animation(&path).unwrap(), project);
    manifest["format_version"] = 15.into();
    assert!(load_animation_bytes(&encoded(&manifest, &payload))
        .unwrap_err()
        .to_string()
        .contains("format 16"));
}

#[test]
fn native_group_names_follow_their_layer_and_survive_symbol_capture() {
    let (mut doc, [a, b, _]) = group_with_path();
    doc.layers.push(document_core::Layer {
        id: 2,
        ..Default::default()
    });
    let other = doc.add_rectangle([3.; 2], [8.; 2], [255; 4]);
    let other_object = doc.objects.iter_mut().find(|o| o.id == other).unwrap();
    other_object.layer = 2;
    other_object.group = Some(7);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("groups.vanim");
    save_project(&path, &doc).unwrap();
    assert_eq!(entries(&path).0["format_version"], 2);
    let original = load_project(&path).unwrap();
    let mut doc = original.clone();
    doc.rename_group(1, 7, "  Étincelles ✦ ").unwrap();
    for o in doc.objects.iter().filter(|o| [a, b].contains(&o.id)) {
        assert_eq!(o.group_name.as_deref(), Some("Étincelles ✦"));
    }
    assert!(doc
        .objects
        .iter()
        .find(|o| o.id == other)
        .unwrap()
        .group_name
        .is_none());
    save_project(&path, &doc).unwrap();
    assert_eq!(load_project(&path).unwrap(), doc);
    let (mut header, body) = entries(&path);
    assert_eq!(header["format_version"], 15);
    header["format_version"] = 14.into();
    assert!(load_animation_bytes(&encoded(&header, &body))
        .unwrap_err()
        .to_string()
        .contains("format 15"));

    let mut p = AnimationProject::from_document(doc.clone());
    let (symbol, _) = p.convert_to_symbol(0, 0, &[a, b], "Sparks").unwrap();
    let artwork = &p.symbols[&symbol].artwork;
    assert!(artwork
        .objects
        .iter()
        .all(|o| o.group_name.as_deref() == Some("Étincelles ✦")));
    assert!(artwork.objects[0].group.is_some());
    assert_eq!(artwork.objects[0].group, artwork.objects[1].group);
    save_animation(&path, &p, true).unwrap();
    assert_eq!(entries(&path).0["format_version"], 15);
    assert_eq!(load_animation(&path).unwrap(), p);

    doc.rename_group(1, 7, "  ").unwrap();
    assert_eq!(doc, original);
    save_project(&path, &doc).unwrap();
    assert_eq!(entries(&path).0["format_version"], 2);
}

#[test]
fn invalid_group_names_leave_artwork_unchanged_and_inconsistent_native_members_are_rejected() {
    let (mut doc, _) = group_with_path();
    let original = doc.clone();
    assert!(doc.rename_group(1, 7, &"x".repeat(81)).is_err());
    assert!(doc.rename_group(1, 7, "a\nb").is_err());
    assert!(doc.rename_group(1, 99, "Missing").is_err());
    assert_eq!(doc, original);
    doc.layers[0].locked = true;
    assert!(doc.rename_group(1, 7, "Locked").is_err());
    doc.layers[0].locked = false;
    assert_eq!(doc, original);
    doc.objects[1].group_name = Some("Inconsistent".into());
    assert!(doc.validate().is_err());
    doc.objects[1].group = None;
    assert!(doc.validate().is_err());
}

#[test]
fn pressure_paint_keeps_its_centerline_color_and_width_profile_across_native_roundtrips() {
    let mut doc = Document {
        width: 128,
        height: 64,
        ..Default::default()
    };
    let id = doc.add_shape(
        [12., 28.],
        [100., 8.],
        [0; 4],
        Shape::Path {
            nodes: [[0., 0.5], [1., 0.5]].map(PathNode::corner).into(),
            closed: false,
        },
        1,
    );
    let pen = &mut doc.objects[0];
    pen.stroke = [31, 94, 163, 255];
    pen.stroke_width = 20.;
    pen.stroke_profile = vec![[0., 0.2], [0.3, 1.], [0.65, 0.2], [1., 0.8]];
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("pressure.vanim");
    save_project(&file, &doc).unwrap();
    assert_eq!(entries(&file).0["format_version"], 14);
    assert_eq!(load_project(&file).unwrap(), doc);

    let pixels = renderer::render(&doc, 0).unwrap();
    assert_eq!(pixels.pixel(42, 39).unwrap().blue(), 163);
    assert_eq!(pixels.pixel(77, 39).unwrap().alpha(), 0);
    assert_eq!(doc.hit_test([42.5, 39.5], 0), Some(id));
    assert_eq!(doc.hit_test([77.5, 39.5], 0), None);
    let mut project = AnimationProject::from_document(doc);
    project.convert_to_symbol(0, 0, &[id], "Ink").unwrap();
    let linked = renderer::render(project.clips[0].drawing_at(0), 0).unwrap();
    // Rebasing a definition at its own origin can change edge antialiasing.
    assert!(linked
        .pixels()
        .iter()
        .zip(pixels.pixels())
        .all(|(a, b)| (a.alpha() >= 128) == (b.alpha() >= 128)));
    save_animation(&file, &project, true).unwrap();
    let (mut header, data) = entries(&file);
    assert_eq!(header["format_version"], 14);
    let reopened = load_animation(&file).unwrap();
    assert_eq!(reopened, project);
    assert!(
        renderer::render(reopened.clips[0].drawing_at(0), 0)
            .unwrap()
            .data()
            == linked.data()
    );
    header["format_version"] = 13.into();
    assert!(load_animation_bytes(&encoded(&header, &data))
        .unwrap_err()
        .to_string()
        .contains("format 14"));
}

#[test]
fn reflected_scales_roundtrip_in_drawings_animations_and_captured_cutters() {
    let (mut doc, ids) = group_with_path();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("flipped.vanim");
    let cutter = doc.objects.last_mut().unwrap();
    cutter.scale_keys.extend([(0, [1., 1.]), (3, [-1., 1.])]);
    save_project(&path, &doc).unwrap();
    let (mut header, data) = entries(&path);
    assert_eq!(header["format_version"], 13);
    assert_eq!(load_project(&path).unwrap(), doc);
    header["format_version"] = 12.into();
    let downgraded = dir.path().join("downgraded.vanim");
    std::fs::write(&downgraded, encoded(&header, &data)).unwrap();
    assert!(load_project(&downgraded)
        .unwrap_err()
        .to_string()
        .contains("format 13"));
    assert!(load_animation(&downgraded)
        .unwrap_err()
        .to_string()
        .contains("format 13"));

    doc.set_clip(&ids, 0, None).unwrap();
    assert!(doc.objects.iter().all(|o| o.scale_keys.is_empty()));
    let project = AnimationProject::from_document(doc);
    save_animation(&path, &project, true).unwrap();
    assert_eq!(entries(&path).0["format_version"], 13);
    assert_eq!(load_animation(&path).unwrap(), project);
}

#[test]
fn clipped_group_saves_one_original_exports_visible_pixels_and_releases_after_open() {
    let (mut drawing, [a, b, cutter]) = group_with_path();
    let original = drawing.clone();
    drawing.set_clip(&[cutter, a, b], 0, None).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let docfile = dir.path().join("drawing.vanim");
    save_project(&docfile, &drawing).unwrap();
    assert_eq!(load_project(&docfile).unwrap(), drawing);
    assert_eq!(
        load_animation(&docfile).unwrap().clips[0].drawing_at(0),
        &drawing
    );

    let mut p = AnimationProject::from_document(drawing.clone());
    p.duplicate_clip(0, "Copy").unwrap();
    let file = dir.path().join("clipped.vanim");
    save_animation(&file, &p, false).unwrap();
    let (header, data) = entries(&file);
    assert_eq!(header["format_version"], 12);
    assert_eq!(data["clipping"]["sources"].as_array().unwrap().len(), 1);
    let mut reopened = load_animation(&file).unwrap();
    assert_eq!(reopened, p);
    assert!(load_project(&file).is_err());
    let other = reopened.clips[1].drawing_at(0).objects[1]
        .clip_source
        .as_ref()
        .unwrap()
        .source
        .clone();
    let editable = reopened.clips[0].drawing_at_mut(0);
    assert!(Arc::ptr_eq(
        &editable.objects[0].clip_source.as_ref().unwrap().source,
        &other
    ));

    let svg = dir.path().join("clipped.svg");
    export_svg(&svg, editable, 0, false).unwrap();
    let imported = import_svg(&svg).unwrap();
    let pixels = renderer::render(editable, 0).unwrap();
    assert_eq!(
        renderer::render(&imported, 0).unwrap().data(),
        pixels.data()
    );
    assert_eq!(pixels.pixel(10, 16).unwrap().alpha(), 0);
    assert!(pixels.pixel(32, 28).unwrap().alpha() > 245);
    editable.release_clip(&[a, b], 0).unwrap();
    assert_eq!(editable.objects, original.objects);
    assert_eq!(
        renderer::render(editable, 0).unwrap().data(),
        renderer::render(&original, 0).unwrap().data()
    );
    assert_eq!(reopened.clips[1].drawing_at(0), &drawing);
}

#[test]
fn a_thousand_exposures_share_the_original_path_inside_independent_artwork() {
    let (mut art, ids) = group_with_path();
    art.frames = 1;
    art.set_clip(&ids, 0, None).unwrap();
    let source = Arc::new(art);
    let mut drawing = Document {
        frames: 1_000,
        ..Document::default()
    };
    drawing.add_rectangle([0.; 2], [128.; 2], [0; 4]);
    drawing.objects[0].embedded_artwork = Some(source.into());
    let mut project = AnimationProject::from_document(drawing);
    let clip = &mut project.clips[0];
    let exposure = clip.cels[&0].clone();
    for frame in 1..clip.frames {
        clip.cels.insert(frame, exposure.clone());
    }
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("held-cutout.vanim");
    save_animation(&file, &project, false).unwrap();
    let (header, data) = entries(&file);
    assert_eq!(header["format_version"], 12);
    assert_eq!(data["artwork"].as_object().unwrap().len(), 1);
    assert_eq!(data["clipping"]["sources"].as_array().unwrap().len(), 1);
    let reopened = load_animation(&file).unwrap();
    let nested = |frame| {
        reopened.clips[0].drawing_at(frame).objects[0]
            .embedded_artwork
            .as_ref()
            .unwrap()
    };
    assert!(Arc::ptr_eq(nested(0), nested(999)));
    let group = &nested(999).objects;
    assert!(Arc::ptr_eq(
        &group[0].clip_source.as_ref().unwrap().source,
        &group[1].clip_source.as_ref().unwrap().source
    ));
    assert_eq!(
        group[0].clip_source.as_ref().unwrap().source.object.name,
        "Window outline"
    );
}

#[test]
fn missing_or_downgraded_clip_sources_fail_to_open() {
    let (mut drawing, ids) = group_with_path();
    drawing.set_clip(&ids, 0, None).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("clips.vanim");
    save_animation(&file, &AnimationProject::from_document(drawing), false).unwrap();
    let (mut header, mut data) = entries(&file);
    header["format_version"] = 11.into();
    assert!(load_animation_bytes(&encoded(&header, &data))
        .unwrap_err()
        .to_string()
        .contains("format 12"));
    header["format_version"] = 12.into();
    data["clipping"]["sources"] = serde_json::json!([]);
    assert!(load_animation_bytes(&encoded(&header, &data)).is_err());
}
