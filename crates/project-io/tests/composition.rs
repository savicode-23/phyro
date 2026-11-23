use document_core::{AnimationProject, ClipTiming, Document, LoopMode};
use project_io::{export_svg, load_animation, save_animation};

fn setup() -> (AnimationProject, usize, u64) {
    let mut p = AnimationProject::default();
    let child = p.add_clip("Blink").unwrap();
    let c = &mut p.clips[child];
    c.set_length(3).unwrap();
    c.set_fps(6).unwrap();
    let id = c
        .drawing_at_mut(0)
        .add_rectangle([4., 6.], [5., 4.], [255, 80, 40, 255]);
    c.drawing_at_mut(0).objects[0]
        .keys
        .extend([(0, [4., 6.]), (2, [24., 6.])]);
    let cid = c.id;
    let wrapper = p.place_clip(cid, 0, 0, 1, [8., 0.]).unwrap();
    assert_ne!(id, 0);
    (p, child, wrapper)
}

#[test]
fn resized_artboards_keep_animated_placements_and_independent_artwork_through_save() {
    let (mut project, child, placed) = setup();
    let mark = project.clips[child]
        .drawing_at_mut(0)
        .add_rectangle([12., 15.], [4., 3.], [255; 4]);
    let (symbol, _) = project
        .convert_to_symbol(child, 0, &[mark], "Mark")
        .unwrap();
    project.remove_symbol_keeping_artwork(symbol).unwrap();
    let parent = project.clips[0].drawing_at_mut(0);
    parent.objects[0].rotation_keys.extend([(0, 0.), (4, 11.)]);
    parent.objects[0]
        .scale_keys
        .extend([(0, [1.; 2]), (4, [0.9, 1.1])]);
    project.hydrate_composition().unwrap();
    let original = project.clone();
    let size = [
        original.clips[0].drawing_at(0).width,
        original.clips[0].drawing_at(0).height,
    ];
    let temp = tempfile::tempdir().unwrap();
    let archive = temp.path().join("resized.vanim");
    let version = || {
        let file = std::fs::File::open(&archive).unwrap();
        let mut zip = zip::ZipArchive::new(file).unwrap();
        let manifest: serde_json::Value =
            serde_json::from_reader(zip.by_name("manifest.json").unwrap()).unwrap();
        manifest["format_version"].as_u64().unwrap()
    };
    save_animation(&archive, &project, false).unwrap();
    assert_eq!(version(), 10);
    for invalid in [[0, 64], [16, document_core::MAX_DIMENSION + 1]] {
        assert!(project.resize_artboard(invalid).is_err());
        assert_eq!(project, original);
    }
    let resized = [size[0] + 31, size[1] + 9];
    project.resize_artboard(resized).unwrap();
    project.validate().unwrap();
    save_animation(&archive, &project, true).unwrap();
    assert_eq!(version(), 11);
    let reopened = load_animation(&archive).unwrap();
    assert_eq!(reopened, project);
    let before = original.clips[0].drawing_at(0);
    let after = reopened.clips[0].drawing_at(0);
    assert_eq!(after.hit_test([13., 7.], 0), Some(placed));
    for t in [0, 3, 4] {
        assert_eq!(
            before.objects[0].symbol_transform(t),
            after.objects[0].symbol_transform(t)
        );
        assert_eq!(before.objects[0].bounds(t), after.objects[0].bounds(t));
        let a = renderer::render(before, t).unwrap();
        let b = renderer::render(after, t).unwrap();
        for (row, wide_row) in a
            .data()
            .chunks_exact(size[0] as usize * 4)
            .zip(b.data().chunks_exact(resized[0] as usize * 4))
        {
            assert_eq!(row, &wide_row[..row.len()]);
        }
    }
    project.resize_artboard(size).unwrap();
    assert_eq!(project, original);
    save_animation(&archive, &project, true).unwrap();
    assert_eq!(version(), 10);
}

#[test]
fn nested_clock_transform_bounds_picking_and_svg_sample_the_same_tick() {
    let (mut p, child, wrapper) = setup();
    let cid = p.clips[child].id;
    let c = &mut p.clips[0];
    c.set_length(12).unwrap();
    let o = &mut c.drawing_at_mut(0).objects[0];
    let i = o.clip_instance.as_mut().unwrap();
    i.timing = ClipTiming {
        offset: -1,
        rate_numerator: 1,
        rate_denominator: 1,
        mode: LoopMode::Loop,
    };
    assert_eq!(
        (0..8).map(|f| i.sample(f).unwrap().1).collect::<Vec<_>>(),
        [2, 2, 0, 0, 1, 1, 2, 2]
    );
    i.timing.mode = LoopMode::Clamp;
    assert_eq!(
        (0..8).map(|f| i.sample(f).unwrap().1).collect::<Vec<_>>(),
        [0, 0, 0, 0, 1, 1, 2, 2]
    );
    i.timing.mode = LoopMode::Loop;
    let d = p.clips[0].drawing_at(4);
    assert_eq!(d.objects[0].bounds(4), [22., 6., 27., 10.]);
    assert_eq!(d.hit_test([24., 8.], 4), Some(wrapper));
    assert_eq!(d.hit_test([14., 8.], 4), None);
    assert_eq!(
        renderer::render(d, 4)
            .unwrap()
            .pixel(24, 8)
            .unwrap()
            .alpha(),
        255
    );
    let tmp = tempfile::tempdir().unwrap();
    let svg = tmp.path().join("frame.svg");
    export_svg(&svg, d, 4, false).unwrap();
    let text = std::fs::read_to_string(svg).unwrap();
    assert!(text.contains(r#"<rect x="14" y="6" width="5" height="4""#));
    assert!(text.contains("matrix(1 0 0 1 8 0)"));
    p.rename_clip(child, "Blink twice").unwrap();
    assert_eq!(p.clips[child].id, cid);
    let archive = tmp.path().join("nested.vanim");
    save_animation(&archive, &p, false).unwrap();
    let loaded = load_animation(&archive).unwrap();
    assert_eq!(p, loaded);
    assert_eq!(
        renderer::render(loaded.clips[0].drawing_at(4), 4)
            .unwrap()
            .data(),
        renderer::render(p.clips[0].drawing_at(4), 4)
            .unwrap()
            .data()
    );
}
#[test]
fn child_edit_reaches_grandparent_and_invalid_cycles_preserve_the_project() {
    let (mut p, child, _) = setup();
    let grandparent = p.add_clip("Scene").unwrap();
    let pid = p.clips[0].id;
    p.place_clip(pid, grandparent, 0, 1, [1., 2.]).unwrap();
    p.clips[child].drawing_at_mut(0).objects[0].fill = [20, 220, 90, 255];
    let expected = p.resolved_snapshot().unwrap();
    let sample = renderer::render(expected.clips[grandparent].drawing_at(0), 0).unwrap();
    assert_eq!(sample.pixel(15, 9).unwrap().green(), 220);
    let before = p.clone();
    let gid = p.clips[grandparent].id;
    assert!(p.place_clip(gid, child, 0, 1, [0.; 2]).is_err());
    assert!(p.remove_clip(child).is_err());
    assert_eq!(p, before);
    assert!(p.place_clip(999, 0, 0, 1, [0.; 2]).is_err());
    assert_eq!(p, before);
}
#[test]
fn static_symbols_nest_and_the_inner_definition_remains_linked() {
    let mut scene = Document::default();
    let mark = scene.add_rectangle([4., 3.], [4., 5.], [40, 50, 130, 255]);
    let mut p = AnimationProject::from_document(scene);
    let (inner, placed) = p.convert_to_symbol(0, 0, &[mark], "Mark").unwrap();
    let (outer, wrapped) = p.convert_to_symbol(0, 0, &[placed], "Group").unwrap();
    let before = p.clone();
    assert!(p.nest_symbol(inner, outer).is_err());
    assert_eq!(p, before);
    assert!(p.remove_symbol(inner).is_err());
    let mut edited = (*p.symbols[&inner].artwork).clone();
    edited.objects[0].fill = [128, 255, 40, 255];
    p.replace_symbol_artwork(inner, edited).unwrap();
    let d = p.clips[0].drawing_at(0);
    assert_eq!(d.hit_test([5., 4.], 0), Some(wrapped));
    assert_eq!(
        renderer::render(d, 0).unwrap().pixel(5, 4).unwrap().green(),
        255
    );
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("symbols.vanim");
    save_animation(&path, &p, false).unwrap();
    let restored = load_animation(&path).unwrap();
    assert_eq!(restored, p);
}
#[test]
fn child_viewport_clips_animated_geometry_outside_the_artboard() {
    let mut p = AnimationProject::default();
    let ci = p.add_clip("Overshoot").unwrap();
    let cid = p.clips[ci].id;
    p.clips[ci]
        .drawing_at_mut(0)
        .add_rectangle([-5., 1.], [9., 3.], [255; 4]);
    let placed = p.place_clip(cid, 0, 0, 1, [20., 10.]).unwrap();
    let d = p.clips[0].drawing_at(0);
    let img = renderer::render(d, 0).unwrap();
    assert_eq!(img.pixel(19, 12).unwrap().alpha(), 0);
    assert_eq!(img.pixel(21, 12).unwrap().alpha(), 255);
    assert_eq!(d.hit_test([19., 12.], 0), None);
    assert_eq!(d.hit_test([21., 12.], 0), Some(placed));
    assert_eq!(d.objects[0].bounds(0), [20., 11., 24., 14.]);
}

#[test]
fn oversized_reference_chains_fail_before_modifying_the_destination() {
    use document_core::{SymbolDefinition, SymbolInstance};
    use std::sync::Arc;
    let mut project = AnimationProject::default();
    let mut last = None;
    for i in 0..16 {
        let mut doc = Document {
            frames: 1,
            ..Default::default()
        };
        doc.add_rectangle([0.; 2], [2.; 2], [255; 4]);
        if let Some(id) = last {
            doc.objects[0].symbol = Some(SymbolInstance::new(
                id,
                project.symbols[&id].artwork.clone(),
            ));
        }
        let id = project
            .add_symbol(SymbolDefinition {
                name: format!("Level {i}"),
                artwork: Arc::new(doc),
            })
            .unwrap();
        last = Some(id);
    }
    let old = project.clone();
    assert!(project
        .place_symbol(last.unwrap(), 0, 0, 1, [0.; 2])
        .is_err());
    assert_eq!(old, project);
    project.validate().unwrap();
}
