use super::*;

fn placement() -> (AnimationProject, u64, usize) {
    let mut p = AnimationProject::default();
    p.clips[0].set_length(7).unwrap();
    let child = p.add_clip("Hand shapes").unwrap();
    p.clips[child].set_length(4).unwrap();
    p.clips[child].set_fps(24).unwrap();
    let id = p.place_clip(p.clips[child].id, 0, 0, 1, [0.; 2]).unwrap();
    let instance = p.clips[0].drawing_at_mut(0).objects[0]
        .clip_instance
        .as_mut()
        .unwrap();
    instance.timing.offset = 1;
    (p, id, child)
}

fn samples(clip: &Clip) -> Vec<u32> {
    (0..clip.frames)
        .map(|t| clip.drawing_at(t).objects[0].instance_at(t).unwrap().1)
        .collect()
}

#[test]
fn keyed_placements_hold_child_ticks_and_restore_the_original_playback() {
    let (mut p, id, _) = placement();
    let clip = &mut p.clips[0];
    let playback = samples(clip);
    assert_eq!(playback, [1, 3, 1, 3, 1, 3, 1]);
    let doc = clip.drawing_at_mut(0);
    doc.set_drawing_key(id, 2, 3).unwrap();
    doc.set_drawing_key(id, 4, 1).unwrap();
    assert_eq!(doc.objects[0].last_keyframe(), Some(4));
    assert_eq!(samples(clip), [0, 0, 3, 3, 1, 1, 1]);
    assert!(clip.set_length(4).is_err());
    let doc = clip.drawing_at_mut(0);
    let before = doc.clone();
    assert!(doc.set_drawing_key(id, 5, 4).is_err());
    assert!(doc.set_drawing_key(id, 7, 0).is_err());
    assert!(doc.move_drawing_key(id, 2, 4).is_err());
    assert!(doc.move_drawing_key(id, 4, 7).is_err());
    assert!(doc.remove_drawing_key(id, 1).is_err());
    assert_eq!(*doc, before);
    doc.layers[0].locked = true;
    assert!(doc.clear_drawing_keys(id).is_err());
    assert!(doc.set_drawing_key(id, 2, 1).is_err());
    assert!(doc.move_drawing_key(id, 2, 1).is_err());
    doc.layers[0].locked = false;
    assert_eq!(*doc, before);
    doc.move_drawing_key(id, 4, 3).unwrap();
    assert_eq!(samples(clip), [0, 0, 3, 1, 1, 1, 1]);
    let doc = clip.drawing_at_mut(0);
    doc.remove_drawing_key(id, 2).unwrap();
    doc.remove_drawing_key(id, 3).unwrap();
    assert_eq!(samples(clip), playback);
    let doc = clip.drawing_at_mut(0);
    doc.set_drawing_key(id, 0, 0).unwrap();
    doc.clear_drawing_keys(id).unwrap();
    assert_eq!(samples(clip), playback);
    p.validate().unwrap();
}

#[test]
fn child_length_edits_clamp_on_playback_and_keep_the_authored_source_tick() {
    let (mut p, id, child) = placement();
    p.clips[0]
        .drawing_at_mut(0)
        .set_drawing_key(id, 1, 3)
        .unwrap();
    p.clips[child].set_length(2).unwrap();
    p.hydrate_composition().unwrap();
    assert_eq!(samples(&p.clips[0]), [0, 1, 1, 1, 1, 1, 1]);
    let instance = p.clips[0].drawing_at(0).objects[0]
        .clip_instance
        .as_ref()
        .unwrap();
    assert_eq!(instance.drawing_keys[&1], 3);
    p.validate().unwrap();
    p.clips[child].set_length(4).unwrap();
    p.hydrate_composition().unwrap();
    assert_eq!(samples(&p.clips[0]), [0, 3, 3, 3, 3, 3, 3]);
    let doc = p.clips[0].drawing_at_mut(0);
    let instance = doc.objects[0].clip_instance.as_mut().unwrap();
    instance.drawing_keys.insert(2, crate::MAX_FRAMES);
    assert!(doc.validate().is_err());
    assert!(p.hydrate_composition().is_err());
}

#[test]
fn resolving_shared_exposures_reuses_each_refreshed_drawing_after_a_child_edit() {
    let (mut p, _, child) = placement();
    let parent = &mut p.clips[0];
    parent.set_length(1000).unwrap();
    let first = parent.cels[&0].clone();
    let mut second = first.clone();
    Arc::make_mut(&mut second).add_rectangle([8.; 2], [10.; 2], [200; 4]);
    for tick in (0..1000).step_by(4) {
        let cel = if tick % 8 == 0 { &first } else { &second };
        parent.cels.insert(tick, Arc::clone(cel));
    }
    let child_object = p.clips[child]
        .drawing_at_mut(0)
        .add_rectangle([1.; 2], [6.; 2], [100; 4]);
    p.hydrate_composition().unwrap();
    let parent = &p.clips[0];
    assert!(!Arc::ptr_eq(&first, &parent.cels[&0]));
    assert!(!Arc::ptr_eq(&second, &parent.cels[&4]));
    assert!(!Arc::ptr_eq(&parent.cels[&0], &parent.cels[&4]));
    for (&start, drawing) in &parent.cels {
        assert!(Arc::ptr_eq(drawing, &parent.cels[&(start % 8)]));
        let (art, tick) = drawing.objects[0].instance_at(start + 1).unwrap();
        assert_eq!(tick, 3);
        assert_eq!(art.objects[0].id, child_object);
        assert_eq!(drawing.objects.len(), if start % 8 == 0 { 1 } else { 2 });
    }
    assert!(first.objects[0]
        .instance_at(0)
        .unwrap()
        .0
        .objects
        .is_empty());
    p.validate().unwrap();

    // A later, independent edit has to validate its own dependency path.
    let parent = &mut p.clips[0];
    let id = parent.id;
    parent.drawing_at_mut(996).objects[0]
        .clip_instance
        .as_mut()
        .unwrap()
        .clip_id = id;
    assert!(p.hydrate_composition().unwrap_err().contains("cycle"));
}

#[test]
fn parent_range_edits_preserve_discrete_drawings_including_the_initial_zero_hold() {
    let (mut p, id, _) = placement();
    let doc = p.clips[0].drawing_at_mut(0);
    doc.set_drawing_key(id, 2, 3).unwrap();
    doc.set_drawing_key(id, 5, 1).unwrap();
    let clip = &p.clips[0];
    let source = samples(clip);
    for first in 0..clip.frames {
        for last in first..clip.frames {
            let mut copy = clip.clone();
            copy.duplicate_range(first, last).unwrap();
            let wanted: Vec<_> = (0..=last)
                .chain(first..=last)
                .chain(last + 1..clip.frames)
                .map(|i| source[i as usize])
                .collect();
            assert_eq!(samples(&copy), wanted);
            copy.validate().unwrap();
            if last - first + 1 < clip.frames {
                let mut deleted = clip.clone();
                deleted.delete_range(first, last).unwrap();
                let wanted: Vec<_> = (0..first)
                    .chain(last + 1..clip.frames)
                    .map(|i| source[i as usize])
                    .collect();
                assert_eq!(samples(&deleted), wanted);
                deleted.validate().unwrap();
            }
        }
    }
    let mut held = clip.clone();
    held.cels.insert(3, held.cels[&0].clone());
    held.set_exposure(0, 5).unwrap();
    assert_eq!(samples(&held), [0, 0, 3, 3, 3, 3, 3, 1, 1]);
    let keys = &held.drawing_at(0).objects[0]
        .clip_instance
        .as_ref()
        .unwrap()
        .drawing_keys;
    assert_eq!(keys, &BTreeMap::from([(2, 3), (7, 1)]));
    held.set_exposure(0, 3).unwrap();
    assert_eq!(samples(&held), source);
}
