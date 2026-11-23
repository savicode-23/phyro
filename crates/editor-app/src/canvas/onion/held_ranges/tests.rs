use super::super::OnionSkins;
use document_core::{AnimationProject, Clip, Document};
use std::sync::Arc;

#[test]
fn cached_nested_holds_match_fresh_search_after_seeks_retiming_and_child_edits() {
    let mut project = AnimationProject::default();
    project.clips[0].set_length(1_000).unwrap();
    let child = project.add_clip("Held child").unwrap();
    let child_clip = &mut project.clips[child];
    child_clip.set_length(1_000).unwrap();
    child_clip
        .drawing_at_mut(0)
        .add_rectangle([3.; 2], [8.; 2], [255; 4]);
    let same = Arc::clone(&child_clip.cels[&0]);
    child_clip.cels.insert(600, Arc::clone(&same));
    let mut other = same.as_ref().clone();
    other.objects[0].size[0] += 2.;
    child_clip.cels.insert(900, Arc::new(other));
    let placed = project
        .place_clip(project.clips[child].id, 0, 0, 1, [0.; 2])
        .unwrap();
    let mut skins = OnionSkins::default();
    let mut compare = |clip: &Clip, tick: u32, painted: &Arc<Document>| {
        let sides = [(8, [150, 30, 20, 110]), (8, [40, 90, 210, 140])];
        skins.set(clip, tick, sides, true, Some(painted));
        let mut control = OnionSkins::default();
        control.set(clip, tick, sides, true, None);
        let poses = |s: &OnionSkins| {
            s.ghosts
                .iter()
                .map(|g| (g.frame, g.tint))
                .collect::<Vec<_>>()
        };
        assert_eq!(poses(&skins), poses(&control), "frame {tick}");
        skins.ghosts.iter().map(|g| g.frame).collect::<Vec<_>>()
    };
    let original = project.clone();
    let old = Arc::new(project.clips[0].drawing_at(0).clone());
    assert_eq!(compare(&project.clips[0], 501, &old), [900]);
    for tick in [502, 599, 600, 899, 900, 901, 0, 999, 502] {
        compare(&project.clips[0], tick, &old);
    }

    // Equal-valued but independently editable nested cels are distinct poses.
    project.clips[child]
        .cels
        .insert(600, Arc::new(same.as_ref().clone()));
    project.hydrate_composition().unwrap();
    assert_eq!(old.as_ref(), project.clips[0].drawing_at(0));
    assert_eq!(compare(&project.clips[0], 501, &old), [600, 900]);
    let new = Arc::new(project.clips[0].drawing_at(0).clone());
    assert_eq!(compare(&project.clips[0], 502, &new), [600, 900]);
    let doc = project.clips[0].drawing_at_mut(0);
    doc.set_drawing_key(placed, 0, 0).unwrap();
    doc.set_drawing_key(placed, 530, 900).unwrap();
    assert_eq!(compare(&project.clips[0], 501, &new), [530]);
    assert_eq!(compare(&original.clips[0], 501, &new), [900]);
    let mut local = original.clone();
    let drawing = local.clips[0].drawing_at_mut(0);
    drawing.add_rectangle([40., 40.], [12.; 2], [255; 4]);
    let keyed = drawing.objects.last_mut().unwrap();
    keyed.interpolation = document_core::Interpolation::Hold;
    keyed.opacity_keys.extend([(0, 1.), (520, 0.), (850, 1.)]);
    assert_eq!(compare(&local.clips[0], 501, &old), [520, 850, 900]);
    let painted = Arc::new(local.clips[0].drawing_at(0).clone());
    compare(&local.clips[0], 502, &painted);
    local.clips[0].drawing_at_mut(0).objects[1]
        .opacity_keys
        .insert(515, 0.25);
    assert_eq!(
        compare(&local.clips[0], 503, &painted),
        [515, 520, 850, 900]
    );
    let mut looping = original.clone();
    let timing = &mut looping.clips[0].drawing_at_mut(0).objects[0]
        .clip_instance
        .as_mut()
        .unwrap()
        .timing;
    timing.offset = -213;
    timing.rate_numerator = 3;
    let painted = Arc::new(looping.clips[0].drawing_at(0).clone());
    for n in 0..50 {
        compare(&looping.clips[0], (491 * n + 155) % 1_000, &painted);
    }
    assert!(skins.holds.intervals.iter().any(|[lo, hi]| hi - lo >= 16));
}
