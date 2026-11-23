use super::*;
use std::time::Instant;

fn project(count: usize) -> AnimationProject {
    let mut doc = Document::default();
    doc.add_rectangle([2.; 2], [4.; 2], [223; 4]);
    let template = doc.objects.pop().unwrap();
    doc.objects = (0..count)
        .map(|i| {
            let mut o = template.clone();
            o.id = i as u64 + 1;
            o.position[0] += i as f32;
            o
        })
        .collect();
    doc.next_id = count as u64 + 1;
    AnimationProject::from_document(doc)
}

#[test]
fn undo_after_deleting_a_same_named_clip_returns_to_the_removed_clip() {
    let mut project = AnimationProject::default();
    let player = project.add_animation_set("Player").unwrap();
    let deleted = project.add_clip_in_set(player, "Animation").unwrap();
    project.add_clip_in_set(player, "Jump").unwrap();
    let before = project.clone();
    project.remove_clip(deleted).unwrap();
    let mut history = History::default();
    history.record(&before, &project, 0, 0);
    assert!(history.undo(&mut project));
    assert_eq!(project, before);
    assert_eq!(history.location(), Some((deleted, 0)));
}

#[test]
fn history_counts_one_recoverable_clipping_path_across_group_members_and_snapshots() {
    let mut doc = Document::default();
    let a = doc.add_rectangle([1.; 2], [24.; 2], [120; 4]);
    let b = doc.add_rectangle([22., 1.], [25.; 2], [200; 4]);
    group(&mut doc, &[a, b]);
    let cutter = doc.add_rectangle([3., 6.], [40., 12.], [255; 4]);
    doc.objects.last_mut().unwrap().name = "shared recoverable geometry".into();
    doc.set_clip(&[a, b, cutter], 0, None).unwrap();
    let snapshot = doc.clone();
    let mut seen = HashSet::new();
    let first = object_heap_bytes(&doc.objects[0], &mut seen);
    // The stencil stored on each member is independently sized, but the original path is shared.
    let sources = first - object_heap_bytes(&snapshot.objects[0], &mut seen);
    assert!(
        sources >= size_of::<document_core::ClipSource>() + "shared recoverable geometry".len()
    );
    let retained = object_heap_bytes(&doc.objects[1], &mut seen);
    assert_eq!(
        retained + sources,
        object_heap_bytes(&doc.objects[1], &mut HashSet::new())
    );
}

#[test]
fn same_order_history_counts_only_changed_objects_and_their_currently_saved_geometry() {
    let project = project(1_000);
    let before = project.clips[0].drawing_at(0);
    let mut after = before.clone();
    after.objects[7].position[1] += 6.;
    after.objects[977].shape = Shape::Path {
        closed: false,
        nodes: (0..128)
            .map(|i| document_core::PathNode::corner([i as f32 / 127., 0.5]))
            .collect(),
    };
    let mut change = Change::between(before, &after);
    let ObjectEdits::InPlace(saved) = &change.objects else {
        panic!("object order is unchanged");
    };
    assert_eq!(
        saved.iter().map(|(index, _)| *index).collect::<Vec<_>>(),
        [7, 977]
    );
    let bytes = change.bytes(&mut HashSet::new());
    assert_eq!(
        bytes,
        size_of::<Change>() + saved.capacity() * size_of::<(usize, VectorObject)>()
    );
    let geometry = object_heap_bytes(&after.objects[977], &mut HashSet::new());
    change.swap(&mut after);
    assert_eq!(&after, before);
    assert_eq!(change.bytes(&mut HashSet::new()), bytes + geometry);
    change.swap(&mut after);
    assert_eq!(change.bytes(&mut HashSet::new()), bytes);
    // A caller that independently reordered the same objects still receives the
    // correct saved geometry by ID, without restoring the old order.
    after.objects.swap(7, 977);
    change.swap(&mut after);
    assert_eq!(after.objects[7], before.objects[977]);
    assert_eq!(after.objects[977], before.objects[7]);
}

#[test]
fn sparse_property_edits_swap_the_right_objects_across_structural_edits() {
    let mut scene = project(8);
    let mut history = History::default();
    let mut snapshots = vec![scene.clone()];
    for step in 0..3 {
        let doc = scene.clips[0].drawing_at_mut(0);
        if step == 1 {
            doc.objects.rotate_left(3);
            doc.objects.remove(2);
            doc.add_rectangle([58.; 2], [12.; 2], [193; 4]);
        } else {
            for index in [1, 6] {
                doc.objects[index].position = [18.5 * (step + 1) as f32, 24.];
                doc.objects[index].scale = [0.93 * (step + 1) as f32, 0.68];
            }
        }
        history.record(snapshots.last().unwrap(), &scene, 0, 0);
        snapshots.push(scene.clone());
    }
    for _ in 0..2 {
        for original in snapshots[..3].iter().rev() {
            assert!(history.undo(&mut scene));
            assert_eq!(&scene, original);
        }
        for original in &snapshots[1..] {
            assert!(history.redo(&mut scene));
            assert_eq!(&scene, original);
        }
    }
}

#[test]
#[ignore = "release timing benchmark"]
fn sparse_transform_history_at_10k_objects() {
    let before = project(10_000);
    for operation in ["translate", "scale"] {
        let mut samples = [Vec::new(), Vec::new(), Vec::new()];
        for trial in 0..25 {
            let mut current = before.clone();
            let objects = &mut current.clips[0].drawing_at_mut(0).objects;
            for index in [2_500, 7_500] {
                if operation == "translate" {
                    objects[index].position[1] += 3.75;
                } else {
                    objects[index].scale = [1.5, 0.6];
                }
            }
            let mut history = History::default();
            let start = Instant::now();
            history.record(&before, &current, 0, 0);
            let record = start.elapsed();
            // The editor keeps a snapshot of the current drawing on each side
            // of undo/redo. Include the Arc copy on write in the timings.
            let moved = current.clone();
            let start = Instant::now();
            assert!(history.undo(&mut current));
            let undo = start.elapsed();
            assert_eq!(current, before);
            let restored = current.clone();
            let start = Instant::now();
            assert!(history.redo(&mut current));
            let redo = start.elapsed();
            assert_eq!(current, moved);
            assert_eq!(restored, before);
            if trial > 0 {
                for (bucket, duration) in samples.iter_mut().zip([record, undo, redo]) {
                    bucket.push(duration.as_secs_f64() * 1_000.);
                }
            }
        }
        for (action, mut times) in ["record", "undo", "redo"].into_iter().zip(samples) {
            times.sort_by(f64::total_cmp);
            println!(
                "{operation} {action}: p50 {:.3} ms, p95 {:.3} ms",
                times[11], times[22]
            );
        }
    }
}

#[test]
#[ignore = "release timing benchmark"]
fn boolean_history_at_10k_objects() {
    let before = project(10_000);
    let mut edited = before.clone();
    let drawing = edited.clips[0].drawing_at_mut(0);
    let operands = [drawing.objects[2_500].id, drawing.objects[2_501].id];
    assert_eq!(
        drawing
            .boolean(&operands, 0, document_core::BooleanOp::Difference)
            .unwrap(),
        [operands[0]]
    );
    let mut samples = [Vec::new(), Vec::new(), Vec::new()];
    for trial in 0..25 {
        let mut history = History::default();
        let mut current = edited.clone();
        let start = Instant::now();
        history.record(&before, &current, 0, 0);
        let record = start.elapsed();
        let start = Instant::now();
        assert!(history.undo(&mut current));
        let undo = start.elapsed();
        assert_eq!(current, before);
        let restored = current.clone();
        let start = Instant::now();
        assert!(history.redo(&mut current));
        let redo = start.elapsed();
        assert_eq!(current, edited);
        assert_eq!(restored, before);
        if trial > 0 {
            for (bucket, duration) in samples.iter_mut().zip([record, undo, redo]) {
                bucket.push(duration.as_secs_f64() * 1000.);
            }
        }
    }
    for (label, mut times) in ["record", "undo", "redo"].into_iter().zip(samples) {
        times.sort_by(f64::total_cmp);
        println!(
            "10k Difference history {label}: median {:.2} ms, p95 {:.2} ms",
            times[11], times[22]
        );
    }
}

#[test]
fn structural_history_handles_local_and_widely_separated_changes() {
    let original = project(12);
    let before = original.clips[0].drawing_at(0);
    for &(first, last) in &[(0, 1), (4, 5), (10, 11), (1, 10)] {
        let mut edited = before.clone();
        edited.objects[first].fill = [18, 63, 141, 255];
        edited.objects.remove(last);
        edited.add_rectangle([50., 65.], [2., 3.], [16; 4]);
        let new = edited.objects.pop().unwrap();
        edited.objects.insert(first + 1, new);
        let mut change = Change::between(before, &edited);
        let expected = edited.clone();
        for _ in 0..2 {
            change.swap(&mut edited);
            assert_eq!(&edited, before);
            change.swap(&mut edited);
            assert_eq!(edited, expected);
        }
    }
}

#[test]
fn structural_history_restores_order_and_keeps_unrecorded_geometry_by_id() {
    let original = project(10);
    let before = original.clips[0].drawing_at(0);
    let mut doc = before.clone();
    doc.objects[2].rotation = 25.;
    doc.objects.remove(3);
    doc.objects[5..8].rotate_left(1);
    let mut history = Change::between(before, &doc);
    // Another caller subsequently reorders the remaining drawing and edits
    // untouched artwork. Structural history restores order on both passes.
    doc.objects.swap(0, 8);
    let mut restored = before.clone();
    for id in [1, 6] {
        for scene in [&mut restored, &mut doc] {
            scene
                .objects
                .iter_mut()
                .find(|o| o.id == id)
                .unwrap()
                .opacity = 0.37;
        }
    }
    let edited = doc.clone();
    history.swap(&mut doc);
    assert_eq!(doc, restored);
    history.swap(&mut doc);
    assert_eq!(doc, edited);
}

#[test]
fn structural_history_excludes_retained_geometry_from_its_budget() {
    let mut original = project(100);
    let before = original.clips[0].drawing_at_mut(0);
    before.objects[18].shape = Shape::Path {
        closed: false,
        nodes: vec![document_core::PathNode::corner([0.2, 0.5]); 16_384],
    };
    let mut edited = before.clone();
    let operands = [edited.objects[6].id, edited.objects[7].id];
    edited
        .boolean(&operands, 0, document_core::BooleanOp::Difference)
        .unwrap();
    let mut change = Change::between(before, &edited);
    let bytes = |c: &Change| c.bytes(&mut HashSet::new());
    let original_bytes = bytes(&change);
    assert!(original_bytes < object_heap_bytes(&before.objects[18], &mut HashSet::new()));
    let expected = edited.clone();
    change.swap(&mut edited);
    assert!(bytes(&change) < object_heap_bytes(&before.objects[18], &mut HashSet::new()));
    assert_eq!(&edited, before);
    change.swap(&mut edited);
    assert_eq!(edited, expected);
    assert_eq!(bytes(&change), original_bytes);
}
