use super::*;

#[test]
fn cancelling_partial_root_placement_and_switching_drawings_drop_the_draft() {
    let mut panel = RigPanel::default();
    let mut doc = Document::default();
    panel.set_drawing(8, 0);
    panel.begin(&doc, None);
    assert!(panel.placing());
    panel.set_drawing(8, 4);
    assert!(!panel.placing());
    assert!(doc.skeleton.bones.is_empty());
    let id = doc.add_bone("Shoulder", None, [0., 0.], [12., 0.]).unwrap();
    panel.selected = Some(id);
    panel.begin(&doc, Some(id));
    panel.set_drawing(9, 4);
    assert!(!panel.placing());
    assert_eq!(panel.selected, None);
    assert_eq!(doc.skeleton.bones.len(), 1);
}

#[test]
fn binding_a_mixed_selection_rejects_the_whole_edit() {
    let mut doc = Document::default();
    let bone = doc.add_bone("Arm", None, [0., 0.], [4., 0.]).unwrap();
    let id = doc.add_rectangle([0.; 2], [2.; 2], [255; 4]);
    let before = doc.clone();
    let mut panel = RigPanel::default();
    panel.bind(&mut doc, &[id, 999], Some(bone), 0);
    assert!(panel.error.is_some());
    assert_eq!(doc, before);
}

#[test]
fn auto_keys_keep_the_start_pose_and_manual_only_edits_existing_keys() {
    let mut doc = Document::default();
    let arm = doc.add_bone("Arm", None, [1., 1.], [9., 1.]).unwrap();
    doc.set_bone_rotation(arm, 20.).unwrap();
    RigPanel::set_pose_angle(&mut doc, arm, 4, KeyMode::Auto, 60.).unwrap();
    assert_eq!(
        doc.skeleton.bones[0].rotation_keys,
        [(0, 20.), (4, 60.)].into()
    );
    let before = doc.clone();
    assert!(RigPanel::set_pose_angle(&mut doc, arm, 2, KeyMode::Manual, 10.).is_err());
    assert_eq!(doc, before);
    RigPanel::set_pose_angle(&mut doc, arm, 4, KeyMode::Manual, -30.).unwrap();
    assert_eq!(doc.skeleton.bones[0].rotation_at(4), -30.);
}
