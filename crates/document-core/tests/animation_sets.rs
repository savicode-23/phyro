use document_core::{AnimationProject, DEFAULT_ANIMATION_SET_ID};
use std::sync::Arc;

#[test]
fn a_new_actor_starts_clean_then_its_clips_inherit_its_own_setup() {
    let mut project = AnimationProject::default();
    project.clips[0].origin = [43., 61.];
    project.clips[0].set_fps(30).unwrap();
    let old = project.clips[0].drawing_at_mut(0);
    old.layers[0].name = "Rival body".into();
    old.add_bone("Rival skeleton", None, [0.; 2], [10., 0.])
        .unwrap();
    let set = project.add_animation_set("Hero").unwrap();
    let first = project.add_clip_in_set(set, "Idle").unwrap();
    let initial = project.clips[first].drawing_at(0);
    assert!(initial.skeleton.bones.is_empty());
    assert_ne!(initial.layers[0].name, "Rival body");
    assert_eq!([initial.width, initial.height], [1000; 2]);
    assert_eq!(project.clips[first].fps, 30);
    assert_eq!(project.clips[first].origin, [500., 1000.]);
    let rig = project.clips[first].drawing_at_mut(0);
    rig.layers[0].name = "Hero body".into();
    rig.add_bone("Hip", None, [0.; 2], [0., 6.]).unwrap();
    project.clips[first].origin = [12., 9.];
    let walk = project.add_clip_in_set(set, "Walk").unwrap();
    assert_eq!(project.clips[walk].origin, [12., 9.]);
    let next = project.clips[walk].drawing_at(0);
    assert_eq!(next.skeleton.bones[0].name, "Hip");
    assert_eq!(next.layers[0].name, "Hero body");
}

#[test]
fn clips_share_local_names_across_sets_and_keep_their_references() {
    let mut project = AnimationProject::default();
    let player = project.add_animation_set("Player").unwrap();
    let enemy = project.add_animation_set("Enemy").unwrap();
    let idle = project.add_clip_in_set(player, "Idle").unwrap();
    project.clips[idle]
        .drawing_at_mut(0)
        .add_rectangle([0.; 2], [6.; 2], [255; 4]);
    let id = project.clips[idle].id;
    let copy = project.duplicate_clip(idle, "Jump").unwrap();
    assert_eq!(project.clips[copy].set_id, player);
    assert!(Arc::ptr_eq(
        &project.clips[copy].cels[&0],
        &project.clips[idle].cels[&0]
    ));
    project.place_clip(id, 0, 0, 1, [2., 3.]).unwrap();
    project.add_clip_in_set(enemy, "Idle").unwrap();
    let before = project.clone();
    assert!(project.move_clip_to_set(idle, enemy).is_err());
    assert!(project.add_clip_in_set(player, "IDLE").is_err());
    assert!(project.rename_animation_set(enemy, "PLAYER").is_err());
    assert_eq!(project, before);
    project.move_clip_to_set(copy, enemy).unwrap();
    project.rename_clip(copy, "Run").unwrap();
    project.remove_animation_set(player).unwrap();
    assert_eq!(project.clips[idle].set_id, DEFAULT_ANIMATION_SET_ID);
    assert_eq!(project.clip_index(id), Some(idle));
    assert_eq!(project.clip_usage(id), 1);
    assert_eq!(project.clips[copy].set_id, enemy);
    assert!(!project.remove_clip(idle).unwrap_err().is_empty());
    project.validate().unwrap();
}

#[test]
fn removing_a_set_preserves_uncontested_names_and_uses_safe_unique_suffixes() {
    let mut project = AnimationProject::default();
    let default = DEFAULT_ANIMATION_SET_ID;
    let set = project.add_animation_set("Effects").unwrap();
    let long_name = "é".repeat(80);
    let colliding = project.add_clip_in_set(set, "Animation").unwrap();
    let unique = project.add_clip_in_set(set, "Animation (2)").unwrap();
    project.add_clip_in_set(default, &long_name).unwrap();
    let long_clip = project.add_clip_in_set(set, &long_name).unwrap();
    project.remove_animation_set(set).unwrap();
    assert_eq!(project.clips[colliding].name, "Animation (3)");
    assert_eq!(project.clips[unique].name, "Animation (2)");
    assert_eq!(
        project.clips[long_clip].name,
        format!("{} (2)", "é".repeat(76))
    );
    assert!(project.clips.iter().all(|c| c.set_id == default));
    assert!(project.remove_animation_set(default).is_err());
    assert!(project.add_animation_set("Next").unwrap() > set);
    project.validate().unwrap();
}

#[test]
fn placing_an_external_clip_remaps_source_set_ids_and_reuses_the_local_clip() {
    let mut source = AnimationProject::default();
    let actor = source.add_animation_set("Actor").unwrap();
    let source_index = source.add_clip_in_set(actor, "Idle").unwrap();
    source.clips[source_index].include_in_set_export = false;
    let source_id = source.clips[source_index].id;
    let mut destination = AnimationProject::default();
    destination.add_animation_set("Unrelated").unwrap();
    let props = destination.add_animation_set("Props").unwrap();
    let parent = destination.add_clip_in_set(props, "Scene").unwrap();
    let existing = destination.add_clip("Hero · Actor / Idle").unwrap();
    destination
        .place_external_clip(&source, source_id, "Hero", (parent, 0, 1), [4., 3.])
        .unwrap();
    let imported_id = destination.clips[parent].drawing_at(0).objects[0]
        .clip_instance
        .as_ref()
        .unwrap()
        .clip_id;
    let index = destination.clip_index(imported_id).unwrap();
    assert!(!destination.clips[index].include_in_set_export);
    destination.clips[index].include_in_set_export = true;
    destination
        .place_external_clip(&source, source_id, "Hero", (parent, 0, 1), [8., 3.])
        .unwrap();
    let objects = &destination.clips[parent].drawing_at(0).objects;
    let placed = objects[0].clip_instance.as_ref().unwrap().clip_id;
    assert_eq!(objects[1].clip_instance.as_ref().unwrap().clip_id, placed);
    assert_ne!(placed, destination.clips[existing].id);
    let imported = &destination.clips[destination.clip_index(placed).unwrap()];
    assert_eq!(imported.set_id, props);
    assert!(imported.include_in_set_export);
    assert_eq!(imported.name, "Hero · Actor / Idle");
    destination.validate().unwrap();
}
