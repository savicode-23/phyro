use super::*;
use std::sync::Arc;

fn three_cels() -> AnimationProject {
    let mut project = AnimationProject::default();
    for at in [0, 2, 4] {
        let mut drawing = Document::default();
        drawing.add_rectangle([at as f32, 5.], [7.; 2], [185; 4]);
        project.clips[0].cels.insert(at, Arc::new(drawing));
    }
    project
}

#[test]
fn navigation_shares_all_cels_until_an_edit() {
    let mut w = EditorWindow::new(egui::vec2(1100., 740.));
    w.editor.replace_project(three_cels(), None);
    for tick in [0, 1, 2, 3, 4, 0, 3, 4] {
        w.editor.timeline.frame = tick;
        w.frame(vec![]);
        let clip = &w.editor.project.clips[0];
        for (at, cel) in &clip.cels {
            assert!(
                Arc::ptr_eq(cel, &w.editor.baseline.clips[0].cels[at]),
                "navigating to frame {tick} copied the drawing at {at}"
            );
        }
    }
    assert!(!w.editor.can_undo());

    // Switch drawings on the same update as an edit; keep both Undo and Redo exact.
    let before = w.editor.project.clone();
    w.editor.project.clips[0].drawing_at_mut(4).objects[0].position = [29., 43.];
    let after = w.editor.project.clone();
    for (at, cel) in &after.clips[0].cels {
        assert_eq!(Arc::ptr_eq(cel, &before.clips[0].cels[at]), *at != 4);
    }
    w.editor.timeline.frame = 2;
    w.frame(vec![]);
    assert_eq!(w.editor.project, after);
    w.editor.restore(false);
    assert_eq!(w.editor.project, before);
    w.editor.restore(true);
    assert_eq!(w.editor.project, after);
}

#[test]
fn reordering_or_removing_clips_cannot_substitute_another_cel_with_the_same_tick() {
    let mut w = EditorWindow::new(egui::vec2(1100., 740.));
    let mut p = three_cels();
    p.add_clip("Other").unwrap();
    w.editor.replace_project(p, None);
    w.frame(vec![]);
    let id = w.editor.project.clips[0].session_id();
    w.editor.project.clips.swap(0, 1);
    let other = w.editor.project.clips[0].session_id();
    w.editor.enter_cel(other, 0);
    assert!(Arc::ptr_eq(
        &w.editor.project.clips[1].cels[&0],
        &w.editor.baseline.clips[0].cels[&0]
    ));
    w.editor.project.clips.remove(0);
    w.editor.enter_cel(id, 0);
    assert_eq!(w.editor.project.clips[0].session_id(), id);
}

#[test]
fn retiring_a_cel_preserves_changed_hydrated_clip_art() {
    let mut p = three_cels();
    let nested = p.add_clip("Changing source").unwrap();
    p.clips[nested]
        .drawing_at_mut(0)
        .add_rectangle([3.; 2], [7.; 2], [133; 4]);
    let child = p.clips[nested].id;
    p.place_clip(child, 0, 0, 1, [0.; 2]).unwrap();
    let mut w = EditorWindow::new(egui::vec2(1100., 740.));
    w.editor.replace_project(p, None);
    w.frame(vec![]);
    let scene = &mut w.editor.project;
    scene.clips[nested].drawing_at_mut(0).objects[0].fill = [37, 162, 189, 255];
    scene.hydrate_composition().unwrap();
    let parent = scene.clips[0].session_id();
    w.editor.enter_cel(parent, 2);
    let current = &w.editor.project.clips[0].cels[&0];
    assert!(!Arc::ptr_eq(current, &w.editor.baseline.clips[0].cels[&0]));
    let inst = current
        .objects
        .last()
        .unwrap()
        .clip_instance
        .as_ref()
        .unwrap();
    let (art, frame) = inst.sample(0).unwrap();
    assert_eq!(art.objects[0].fill_at(frame), [37, 162, 189, 255]);
}
