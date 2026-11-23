use document_core::{AnimationProject, Document, Shape, SymbolDefinition};
use project_io::{export_sprite_bundle, export_svg, load_animation, save_animation};
use std::fs;
use std::sync::Arc;

fn fixture() -> (AnimationProject, Vec<u64>) {
    let mut doc = Document {
        width: 32,
        height: 24,
        ..Document::default()
    };
    let a = doc.add_rectangle([2., 3.], [5., 4.], [230, 20, 40, 255]);
    let b = doc.add_shape([10., 3.], [3., 3.], [0, 150, 170, 255], Shape::Ellipse, 1);
    (AnimationProject::from_document(doc), vec![a, b])
}

#[test]
fn removing_linked_symbols_keeps_independent_artwork_in_clips_and_nested_definitions() {
    let (mut p, ids) = fixture();
    let (bell, placed_bell) = p.convert_to_symbol(0, 0, &ids, "Bell").unwrap();
    let (badge, first) = p.convert_to_symbol(0, 0, &[placed_bell], "Badge").unwrap();
    let second = p.place_symbol(badge, 0, 0, 1, [13., 10.]).unwrap();
    let instance = &mut p.clips[0].drawing_at_mut(0).objects[1];
    instance.name = "Back badge".into();
    instance.rotation_keys.extend([(0, 15.), (4, 70.)]);
    instance
        .scale_keys
        .extend([(0, [1., 0.9]), (4, [1.7, 0.8])]);
    instance.opacity_keys.insert(2, 0.63);
    let other = p.duplicate_clip(0, "Second clip").unwrap();
    p.clips[other].duplicate_frame(3).unwrap();
    let parent = p.add_clip("Scene").unwrap();
    p.place_clip(p.clips[other].id, parent, 0, 1, [0.; 2])
        .unwrap();
    let pixels = |p: &AnimationProject| {
        p.clips
            .iter()
            .flat_map(|clip| {
                (0..clip.frames).map(|t| renderer::render(clip.drawing_at(t), t).unwrap().take())
            })
            .collect::<Vec<_>>()
    };
    let before = pixels(&p);
    p.remove_symbol_keeping_artwork(badge).unwrap();
    assert_eq!(pixels(&p), before);
    assert!(!p.symbols.contains_key(&badge));
    let doc = p.clips[0].drawing_at(4);
    assert_eq!(doc.objects[0].id, first);
    assert_eq!(doc.objects[1].id, second);
    assert_eq!(doc.objects[0].name, "Badge");
    assert_eq!(doc.objects[1].name, "Back badge");
    assert_eq!(p.clips[0].exposure(4).0, 0);
    assert_eq!(p.clips[other].cels.len(), 3);
    assert!(doc.objects[0].embedded_artwork.as_ref().unwrap().objects[0]
        .symbol
        .is_some());
    let shared = doc.objects[0].static_artwork().unwrap();
    assert!(Arc::ptr_eq(
        shared,
        doc.objects[1].static_artwork().unwrap()
    ));
    // A later removal must also find links inside already independent artwork.
    p.remove_symbol_keeping_artwork(bell).unwrap();
    assert!(p.symbols.is_empty());
    assert_eq!(pixels(&p), before);
    let doc = p.clips[0].drawing_at(4);
    let mut revised = (**doc.objects[0].static_artwork().unwrap()).clone();
    let inside = Arc::make_mut(revised.objects[0].embedded_artwork.as_mut().unwrap());
    inside.objects[0].fill = [10, 30, 240, 255];
    p.replace_embedded_artwork(0, 4, first, revised).unwrap();
    p.validate().unwrap();
    let old = p.clips[0].drawing_at(4).objects[1]
        .static_artwork()
        .unwrap();
    let new = p.clips[0].drawing_at(4).objects[0]
        .static_artwork()
        .unwrap();
    assert_ne!(old, new);
    let after = pixels(&p);
    assert_ne!(after[0], before[0]);
    let count = p.clips[0].frames as usize;
    assert_eq!(
        &after[count..],
        &before[count..],
        "independent clips and their parent should retain every pixel"
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("independent.vanim");
    save_animation(&path, &p, false).unwrap();
    let opened = load_animation(&path).unwrap();
    assert_eq!(opened, p);
    assert_eq!(pixels(&opened), after);
}

#[test]
fn removing_the_child_keeps_outer_links_and_rejects_invalid_embedded_content_atomically() {
    let (mut p, ids) = fixture();
    let (inner, placed) = p.convert_to_symbol(0, 0, &ids, "Mark").unwrap();
    let (outer, _) = p.convert_to_symbol(0, 0, &[placed], "Flag").unwrap();
    let before = renderer::render(p.clips[0].drawing_at(0), 0).unwrap();
    p.remove_symbol_keeping_artwork(inner).unwrap();
    assert!(p.clips[0].drawing_at(0).objects[0].symbol.is_some());
    assert!(p.symbols[&outer].artwork.objects[0]
        .embedded_artwork
        .is_some());
    assert_eq!(
        renderer::render(p.clips[0].drawing_at(0), 0)
            .unwrap()
            .data(),
        before.data()
    );
    // Recursion can cross from independent content back into a named symbol.
    let acyclic = p.clone();
    let mut cyclic = (*p.symbols[&outer].artwork).clone();
    let nested = Arc::make_mut(cyclic.objects[0].embedded_artwork.as_mut().unwrap());
    nested
        .objects
        .push(p.clips[0].drawing_at(0).objects[0].clone());
    assert!(p
        .replace_symbol_artwork(outer, cyclic)
        .unwrap_err()
        .contains("cycle"));
    assert_eq!(p, acyclic);
    p.remove_symbol_keeping_artwork(outer).unwrap();
    let valid = p.clone();
    let copy = &p.clips[0].drawing_at(0).objects[0];
    let id = copy.id;
    let mut bad = (**copy.static_artwork().unwrap()).clone();
    bad.frames = 2;
    assert!(p.replace_embedded_artwork(0, 0, id, bad).is_err());
    assert_eq!(p, valid);
    // Shared allocations still count once per placement towards the painting budget.
    let mut huge = Arc::new(Document {
        frames: 1,
        ..Document::default()
    });
    for _ in 0..6 {
        let mut parent = Document {
            frames: 1,
            ..Document::default()
        };
        for _ in 0..16 {
            parent.add_rectangle([0.; 2], [8.; 2], [0; 4]);
            parent.objects.last_mut().unwrap().embedded_artwork = Some(huge.clone().into());
        }
        huge = Arc::new(parent);
    }
    assert!(p
        .replace_embedded_artwork(0, 0, id, (*huge).clone())
        .unwrap_err()
        .contains("budget"));
    assert_eq!(p, valid);
    assert!(p.remove_symbol_keeping_artwork(u64::MAX).is_err());
    assert_eq!(p, valid);
}

#[test]
fn converting_preserves_pixels_and_hit_testing_uses_visible_children() {
    let (mut p, ids) = fixture();
    let original = renderer::render(p.clips[0].drawing_at(0), 2).unwrap();
    let (symbol, instance) = p.convert_to_symbol(0, 2, &ids, "Bell").unwrap();
    let doc = p.clips[0].drawing_at(0);
    assert_eq!(renderer::render(doc, 2).unwrap().data(), original.data());
    assert_eq!(doc.hit_test([3., 4.], 2), Some(instance));
    assert_eq!(
        doc.hit_test([8.5, 4.], 2),
        None,
        "space between the two shapes is transparent"
    );
    let (nested, _) = SymbolDefinition::capture("Nested", doc, 0, &[instance]).unwrap();
    assert_eq!(
        nested.artwork.objects[0]
            .symbol
            .as_ref()
            .unwrap()
            .definition,
        symbol
    );
    assert!(p.remove_symbol(symbol).is_err());
    let snapshot = p.clone();
    let mut revised = (*p.symbols[&symbol].artwork).clone();
    revised.objects[0].fill = [3, 100, 240, 255];
    p.duplicate_clip(0, "Second").unwrap();
    let second = p.place_symbol(symbol, 1, 0, 1, [15., 10.]).unwrap();
    p.clips[1]
        .drawing_at_mut(0)
        .objects
        .iter_mut()
        .find(|o| o.id == second)
        .unwrap()
        .rotation_keys
        .extend([(0, 90.), (4, 180.)]);
    p.replace_symbol_artwork(symbol, revised).unwrap();
    p.validate().unwrap();
    assert_eq!(
        renderer::render(p.clips[0].drawing_at(0), 2)
            .unwrap()
            .pixel(3, 4)
            .unwrap()
            .blue(),
        240
    );
    assert_eq!(
        renderer::render(snapshot.clips[0].drawing_at(0), 2)
            .unwrap()
            .data(),
        original.data()
    );
    let doc = p.clips[1].drawing_at(2);
    let placed = doc.objects.iter().find(|o| o.id == second).unwrap();
    assert_eq!(placed.rotation_at(2), 135.);
    let hit = placed.local_to_world([0.18, 0.4], 2);
    assert_eq!(doc.hit_test(hit, 2), Some(second));
    assert_eq!(
        renderer::render(doc, 2)
            .unwrap()
            .pixel(hit[0] as u32, hit[1] as u32)
            .unwrap()
            .blue(),
        240
    );
}

#[test]
fn archive_hydrates_references_and_vector_and_raster_exports_use_artwork() {
    let (mut p, ids) = fixture();
    let (symbol, _) = p.convert_to_symbol(0, 0, &ids, "Bell").unwrap();
    let second = p.place_symbol(symbol, 0, 0, 1, [17., 12.]).unwrap();
    let doc = p.clips[0].drawing_at_mut(0);
    let extra = doc.objects.iter_mut().find(|o| o.id == second).unwrap();
    extra.scale = [1.3, 0.8];
    extra.rotation = 25.;
    extra.opacity = 0.5;
    let dir = tempfile::tempdir().unwrap();
    let native = dir.path().join("test.vanim");
    save_animation(&native, &p, false).unwrap();
    let restored = load_animation(&native).unwrap();
    assert_eq!(restored, p);
    let doc = restored.clips[0].drawing_at(4);
    for object in &doc.objects {
        let shared = object.symbol.as_ref().unwrap().shared_artwork().unwrap();
        assert!(Arc::ptr_eq(shared, &restored.symbols[&symbol].artwork));
    }
    let svg = dir.path().join("frame.svg");
    export_svg(&svg, doc, 4, false).unwrap();
    let text = fs::read_to_string(svg).unwrap();
    assert_eq!(text.matches("<ellipse ").count(), 2);
    assert_eq!(text.matches("<rect ").count(), 2);
    assert!(text.contains("matrix("));
    let bundle = dir.path().join("sheet");
    export_sprite_bundle(&bundle, &restored.clips[0], &restored.preset, true).unwrap();
    let png = renderer::Pixmap::load_png(bundle.join("frames/frame_0004.png")).unwrap();
    assert_eq!(png.data(), renderer::render(doc, 4).unwrap().data());
}

#[test]
fn conversion_refuses_to_bake_tracks_or_reorder_unselected_art() {
    let (mut p, ids) = fixture();
    p.clips[0].drawing_at_mut(0).objects[0]
        .opacity_keys
        .insert(2, 0.5);
    let old = p.clone();
    assert!(p.convert_to_symbol(0, 2, &ids, "Animation").is_err());
    assert_eq!(p, old);
    let (frozen, _) =
        SymbolDefinition::capture("Still", p.clips[0].drawing_at(2), 2, &ids).unwrap();
    assert_eq!(frozen.artwork.objects[0].opacity, 0.5);
    assert!(frozen.artwork.objects[0].opacity_keys.is_empty());
    let (mut p, ids) = fixture();
    let doc = p.clips[0].drawing_at_mut(0);
    doc.add_rectangle([4., 4.], [5., 6.], [200; 4]);
    doc.objects.swap(1, 2);
    let old = p.clone();
    assert!(p.convert_to_symbol(0, 0, &ids, "Nonadjacent").is_err());
    assert_eq!(p, old);
}
