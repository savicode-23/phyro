use document_core::{AnimationProject, Clip, Document, Layer, Shape};
use std::sync::Arc;

fn scene() -> Clip {
    let mut doc = Document {
        frames: 1000,
        ..Document::default()
    };
    doc.layers.push(Layer {
        id: 2,
        name: "Effects".into(),
        ..Layer::default()
    });
    doc.add_rectangle([0.; 2], [16.; 2], [200; 4]);
    doc.add_shape([32.; 2], [16.; 2], [150; 4], Shape::Ellipse, 2);
    AnimationProject::from_document(doc).clips.remove(0)
}

fn ink(c: &Clip, tick: u32, layer: u64) -> Vec<(u64, [u8; 4])> {
    c.drawing_at(tick)
        .objects
        .iter()
        .filter(|o| o.layer == layer)
        .map(|o| (o.id, o.fill))
        .collect()
}

#[test]
fn layers_hold_editable_drawings_independently_through_other_rows() {
    let mut clip = scene();
    clip.blank_layer_frame(300, 2).unwrap();
    clip.duplicate_layer_frame(600, 1).unwrap();
    assert_eq!(clip.layer_exposure(599, 1), (0, 600));
    assert_eq!(clip.layer_exposure(600, 2), (300, 700));
    assert_eq!(
        clip.layer_drawing_starts(2, 400..=800).collect::<Vec<_>>(),
        [300]
    );
    let before = clip.clone();
    let new = clip.drawing_at_mut(400);
    new.objects[0].fill = [100; 4];
    new.objects[0].keys.insert(950, [40., 80.]);
    let mark = new.add_shape([8.; 2], [8.; 2], [30; 4], Shape::Ellipse, 2);
    clip.sync_layer_edits(&before, 400).unwrap();
    assert_eq!(ink(&clip, 0, 1), vec![(1, [100; 4])]);
    assert_eq!(ink(&clip, 999, 1), vec![(1, [200; 4])]);
    assert_eq!(ink(&clip, 299, 2), vec![(2, [150; 4])]);
    assert_eq!(ink(&clip, 999, 2), vec![(mark, [30; 4])]);
    assert_eq!(clip.drawing_at(0).objects[0].keys[&950], [40., 80.]);
    assert!(clip.drawing_at(999).objects[0].keys.is_empty());
    assert_eq!(clip.cels.len(), 3);
    assert_eq!(ink(&before, 400, 2), vec![]);
    clip.validate().unwrap();
}

#[test]
fn sliding_one_row_retains_the_other_rows_and_does_not_move_a_clip_event() {
    let mut clip = scene();
    clip.blank_layer_frame(200, 2).unwrap();
    clip.duplicate_layer_frame(500, 2).unwrap();
    let before = clip.clone();
    clip.drawing_at_mut(550)
        .add_shape([8.; 2], [4.; 2], [60; 4], Shape::Ellipse, 2);
    clip.sync_layer_edits(&before, 550).unwrap();
    clip.duplicate_layer_frame(350, 1).unwrap();
    clip.blank_layer_frame(700, 1).unwrap();
    clip.add_event(350, "landing").unwrap();
    let character = |c: &Clip| [100, 349, 350, 499, 500, 700, 999].map(|t| ink(c, t, 1));
    let expected = character(&clip);
    clip.move_layer_boundary(500, 250, 2).unwrap();
    assert_eq!(ink(&clip, 250, 2), ink(&clip, 999, 2));
    assert!(clip.layer_frame_is_blank(249, 2));
    clip.set_layer_exposure(210, 2, 400).unwrap();
    assert!(clip.layer_frame_is_blank(599, 2));
    assert!(!clip.layer_frame_is_blank(600, 2));
    assert_eq!(character(&clip), expected);
    assert_eq!(clip.layer_exposure(350, 1), (350, 350));
    assert_eq!(clip.frames, 1000);
    assert!(clip.events.contains_key(&350));
    assert!(clip.move_layer_boundary(600, 200, 2).is_err());
    clip.validate().unwrap();
}

#[test]
fn whole_clip_splices_preserve_boundaries_and_old_drawings_remain_independent() {
    let mut clip = scene();
    clip.cels.insert(40, Arc::clone(&clip.cels[&0]));
    clip.blank_layer_frame(15, 2).unwrap();
    assert_eq!(clip.layer_exposure(20, 1), (0, 40));
    assert_eq!(clip.layer_exposure(40, 1), (40, 960));
    clip.duplicate_range(10, 20).unwrap();
    assert_eq!(clip.layer_exposure(26, 2).0, 26);
    assert_eq!(clip.layer_exposure(26, 1).0, 21);
    assert_eq!(clip.layer_exposure(32, 1).0, 32);
    clip.delete_range(21, 31).unwrap();
    assert_eq!(clip.layer_exposure(15, 2).0, 15);
    assert_eq!(clip.layer_exposure(40, 1).0, 40);
    clip.delete_range(5, 19).unwrap();
    assert_eq!(clip.layer_exposure(5, 2).0, 5);
    assert_eq!(clip.layer_exposure(5, 1).0, 0);
    clip.validate().unwrap();
}

#[test]
fn indexed_row_queries_survive_edits_to_warm_and_cloned_timelines() {
    let mut clip = scene();
    for tick in 1..1000 {
        clip.duplicate_layer_frame(tick, 2).unwrap();
    }
    assert_eq!(clip.cels.len(), 1000);
    assert_eq!(
        clip.layer_drawing_starts(1, 990..=999).collect::<Vec<_>>(),
        [0]
    );
    let previous = clip.clone();
    clip.duplicate_layer_frame(995, 1).unwrap();
    assert_eq!(
        clip.layer_drawing_starts(1, 990..=999).collect::<Vec<_>>(),
        [0, 995]
    );
    assert_eq!(
        previous
            .layer_drawing_starts(1, 990..=999)
            .collect::<Vec<_>>(),
        [0]
    );
    assert_eq!(
        clip.layer_drawing_starts(2, 994..=997).collect::<Vec<_>>(),
        [994, 995, 996, 997]
    );
}

#[test]
fn editing_held_background_reuses_a_snapshot_across_a_thousand_matching_exposures() {
    let mut clip = scene();
    for frame in 1..1000 {
        clip.duplicate_layer_frame(frame, 2).unwrap();
    }
    let before = clip.clone();
    clip.drawing_at_mut(558).objects[0].fill = [41; 4];
    clip.sync_layer_edits(&before, 558).unwrap();
    assert!(clip
        .cels
        .values()
        .all(|doc| Arc::ptr_eq(doc, &clip.cels[&558])));
    assert_eq!(before.drawing_at(558).objects[0].fill, [200; 4]);
}

#[test]
fn a_shared_nested_placement_keeps_its_keys_across_effects_on_the_same_layer() {
    let mut project = AnimationProject::from_document(scene().drawing_at(0).clone());
    let child = project.add_clip("Character").unwrap();
    project.clips[child].set_length(10).unwrap();
    let placed = project
        .place_clip(project.clips[child].id, 0, 0, 2, [0.; 2])
        .unwrap();
    let clip = &mut project.clips[0];
    clip.blank_layer_frame(15, 2).unwrap();
    clip.blank_layer_frame(33, 2).unwrap();
    let before = clip.clone();
    clip.drawing_at_mut(28)
        .set_drawing_key(placed, 28, 7)
        .unwrap();
    clip.sync_layer_edits(&before, 28).unwrap();
    for t in [0, 14, 15, 28, 33, 900] {
        let placement = clip
            .drawing_at(t)
            .objects
            .iter()
            .find(|o| o.id == placed)
            .unwrap();
        assert_eq!(
            placement.clip_instance.as_ref().unwrap().drawing_keys[&28],
            7
        );
    }
    project.validate().unwrap();
}

#[test]
fn layer_structure_follows_a_family_of_held_drawings() {
    let mut clip = scene();
    clip.blank_layer_frame(10, 2).unwrap();
    clip.blank_layer_frame(20, 2).unwrap();
    let before = clip.clone();
    let doc = clip.drawing_at_mut(12);
    doc.layers[1].name = "New effects name".into();
    doc.layers[1].visible = false;
    doc.layers.push(Layer {
        id: 3,
        name: "Weather".into(),
        ..Layer::default()
    });
    let new = doc.add_shape([3.; 2], [2.; 2], [99; 4], Shape::Rectangle, 3);
    clip.sync_layer_edits(&before, 12).unwrap();
    let first_pass = clip.clone();
    clip.sync_layer_edits(&before, 12).unwrap();
    assert_eq!(clip, first_pass);
    for frame in [0, 10, 20] {
        assert_eq!(clip.drawing_at(frame).layers[1].name, "New effects name");
        assert!(!clip.drawing_at(frame).layers[1].visible);
        assert_eq!(ink(&clip, frame, 3)[0].0, new);
    }
    assert!(clip.blank_layer_frame(13, 2).is_err());
    clip.validate().unwrap();
}

#[test]
fn independent_legacy_drawings_can_reuse_ids_and_conflicting_retimes_are_atomic() {
    let mut clip = scene();
    let mut other = clip.drawing_at(0).clone();
    other.objects[0].id = 2;
    other.objects[1].id = 1;
    clip.cels.insert(40, Arc::new(other));
    clip.blank_layer_frame(10, 2).unwrap();
    let before = clip.clone();
    let id = clip
        .drawing_at_mut(5)
        .add_rectangle([5.; 2], [4.; 2], [62; 4]);
    clip.sync_layer_edits(&before, 5).unwrap();
    assert!(ink(&clip, 10, 1).iter().any(|(i, _)| *i == id));
    assert!(!ink(&clip, 40, 1).iter().any(|(i, _)| *i == id));
    let saved = clip.clone();
    assert!(clip.move_layer_boundary(40, 5, 1).is_err());
    assert_eq!(clip, saved);
    clip.validate().unwrap();
}

#[test]
fn invalid_or_inconsistent_layer_metadata_is_rejected() {
    let mut clip = scene();
    clip.blank_layer_frame(10, 2).unwrap();
    let mut invalid = clip.clone();
    invalid.layer_starts.insert(15, [2].into());
    assert!(invalid.validate().is_err());
    let mut invalid = clip.clone();
    invalid.layer_starts.insert(0, [1].into());
    assert!(invalid.validate().is_err());
    let mut invalid = clip.clone();
    invalid.layer_starts.insert(10, [72].into());
    assert!(invalid.validate().is_err());
    clip.drawing_at_mut(10).objects[0].stroke_width = 77.;
    assert!(clip.validate().is_err());
}

#[test]
fn adding_a_layer_to_a_legacy_clip_gives_it_one_hold_across_existing_drawings_and_rigs() {
    let mut clip = scene();
    let mut second = clip.drawing_at(0).clone();
    second.objects[0].fill = [17; 4];
    second.next_id = 72;
    second.layers.push(Layer {
        id: 7,
        name: "Smoke".into(),
        ..Layer::default()
    });
    second
        .add_bone("Other character", None, [0.; 2], [10., 0.])
        .unwrap();
    clip.cels.insert(40, Arc::new(second));
    assert_eq!(clip.next_layer_id(), 8);
    let before = clip.clone();
    clip.drawing_at_mut(0).layers.push(Layer {
        id: 8,
        name: "Weather".into(),
        ..Layer::default()
    });
    clip.sync_layer_edits(&before, 0).unwrap();
    assert_eq!(clip.layer_exposure(50, 8), (0, 1000));
    assert_eq!(clip.layer_exposure(50, 1), (40, 960));
    assert_eq!(clip.rig_exposure(50), (40, 960));
    let before = clip.clone();
    let mark = clip
        .drawing_at_mut(50)
        .add_shape([1.; 2], [4.; 2], [31; 4], Shape::Ellipse, 8);
    clip.sync_layer_edits(&before, 50).unwrap();
    assert!(mark >= 72);
    assert_eq!(ink(&clip, 0, 8), ink(&clip, 50, 8));
    assert_ne!(ink(&clip, 0, 1), ink(&clip, 50, 1));
    assert!(clip.drawing_at(0).skeleton.bones.is_empty());
    assert_eq!(clip.drawing_at(50).skeleton.bones.len(), 1);
    clip.blank_layer_frame(14, 8).unwrap();
    assert!(ink(&clip, 50, 8).is_empty());
    assert!(!ink(&clip, 13, 8).is_empty());
    clip.validate().unwrap();
}

#[test]
fn renaming_and_reordering_legacy_layers_preserve_unrelated_local_stacks() {
    let mut clip = scene();
    let mut second = clip.drawing_at(0).clone();
    second.layers.insert(
        1,
        Layer {
            id: 7,
            name: "Local".into(),
            ..Layer::default()
        },
    );
    let mut third = second.clone();
    third.layers[0].name = "Another meaning".into();
    clip.cels.insert(30, Arc::new(second));
    clip.cels.insert(60, Arc::new(third));
    let before = clip.clone();
    let layers = &mut clip.drawing_at_mut(0).layers;
    layers[0].name = "Character".into();
    layers.swap(0, 1);
    clip.sync_layer_edits(&before, 0).unwrap();
    let names = |frame| {
        clip.drawing_at(frame)
            .layers
            .iter()
            .map(|l| l.name.as_str())
            .collect::<Vec<_>>()
    };
    assert_eq!(names(30), ["Effects", "Local", "Character"]);
    assert_eq!(names(60), ["Another meaning", "Local", "Effects"]);
    assert!(clip.layer_starts.is_empty());
    clip.validate().unwrap();
}

#[test]
fn imported_animation_and_placed_symbol_reach_the_whole_target_layer_before_validation() {
    let mut project = AnimationProject::from_document(scene().drawing_at(0).clone());
    project.clips[0].blank_layer_frame(25, 2).unwrap();
    let mut artist = AnimationProject::default();
    artist.clips[0]
        .drawing_at_mut(0)
        .add_rectangle([1.; 2], [4.; 2], [255; 4]);
    let source = artist.clips[0].id;
    let id = project
        .place_external_clip(&artist, source, "Actor", (0, 50, 1), [16.; 2])
        .unwrap();
    let (symbol, _) = project.convert_to_symbol(0, 50, &[1], "Body").unwrap();
    let instance = project.place_symbol(symbol, 0, 50, 1, [10.; 2]).unwrap();
    for frame in [0, 25, 999] {
        let doc = project.clips[0].drawing_at(frame);
        assert!(doc
            .objects
            .iter()
            .any(|o| o.id == id && o.clip_instance.is_some()));
        assert!(doc
            .objects
            .iter()
            .any(|o| o.id == instance && o.symbol.is_some()));
    }
    project.validate().unwrap();
}
