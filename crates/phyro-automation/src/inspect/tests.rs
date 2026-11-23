use super::*;
use std::sync::Arc;

fn character() -> AnimationProject {
    let mut doc = Document {
        frames: 1_000,
        ..Default::default()
    };
    doc.add_rectangle([2., 4.], [12., 16.], [30, 60, 90, 255]);
    doc.objects[0].name = "Sleeve".into();
    doc.objects[0].keys.extend([(0, [2., 4.]), (4, [10., 12.])]);
    let bone = doc.add_bone("Elbow", None, [1., 2.], [4., 5.]).unwrap();
    doc.set_bone_rotation_key(bone, 3, 17.).unwrap();
    let mut project = AnimationProject::from_document(doc);
    let second = Arc::clone(&project.clips[0].cels[&0]);
    project.clips[0].cels.insert(7, second);
    project
}

#[test]
fn inspection_scopes_a_held_drawing_and_reports_pose_and_keyframes() {
    let mut project = character();
    project.clips[0].include_in_set_export = false;
    let overview = inspect(&project, None, None).unwrap();
    let clip = &overview["clips"][0];
    assert_eq!(overview["animation_sets"][0]["name"], "Animations");
    assert_eq!(overview["animation_sets"][0]["id"], clip["set_id"]);
    assert_eq!(clip["include_in_set_export"], false);
    assert_eq!(clip["exposure_count"], 2);
    assert_eq!(clip["exposures"][1]["exposure_duration"], 993);
    assert!(clip.get("drawing").is_none());
    let view = inspect(&project, Some(1), Some(2)).unwrap();
    let clip = &view["clips"][0];
    assert_eq!(clip["exposures"].as_array().unwrap().len(), 1);
    let drawing = &clip["drawing"];
    assert_eq!(drawing["exposure_start"], 0);
    assert_eq!(drawing["exposure_duration"], 7);
    assert_eq!(drawing["layers"][0]["layer_id"], 1);
    assert_eq!(drawing["bones"][0]["name"], "Elbow");
    assert_eq!(drawing["bones"][0]["rotation_keys"], json!({"3":17.}));
    let object = &drawing["objects"][0];
    assert_eq!(object["position"], json!([6., 8.]));
    assert_eq!(object["base_position"], json!([2., 4.]));
    assert_eq!(object["keyframes"]["position"], json!([0, 4]));
    assert_eq!(object["editable_properties"], json!(["position", "fill"]));
    let key = lookup(&project, None, Some(1), Some(1), Some(4)).unwrap();
    assert_eq!(
        key["keys_at_frame"]["position"],
        json!({"value": [10., 12.], "outgoing_easing": "Linear"})
    );
    let between = lookup(&project, None, Some(1), Some(1), Some(2)).unwrap();
    assert_eq!(between["keys_at_frame"], json!({}));
}

#[test]
fn inspection_reports_the_planted_tip_and_solved_joint_angles() {
    let mut doc = Document::default();
    let thigh = doc.add_bone("Thigh", None, [20., 20.], [32., 35.]).unwrap();
    let shin = doc
        .add_bone("Shin", Some(thigh), [32., 35.], [20., 50.])
        .unwrap();
    doc.pin_foot(shin, 0).unwrap();
    doc.set_bone_rotation_key(thigh, 3, 40.).unwrap();
    let project = AnimationProject::from_document(doc);
    let report = inspect(&project, Some(1), Some(3)).unwrap();
    let bones = &report["clips"][0]["drawing"]["bones"];
    assert!(bones[0]["rotation"].as_f64().unwrap().abs() < 0.001);
    assert_eq!(bones[0]["rotation_keys"]["3"], json!(40.));
    assert_eq!(bones[1]["foot_pin"], json!([20., 50.]));
}

#[test]
fn repeated_object_ids_and_names_return_exposure_and_clip_candidates() {
    let mut project = character();
    let other = project.add_clip("Second").unwrap();
    let drawing = project.clips[other].drawing_at_mut(0);
    drawing.add_rectangle([4.; 2], [3.; 2], [10; 4]);
    drawing.objects[0].name = "Sleeve".into();
    for (name, id) in [(Some("Sleeve"), None), (None, Some(1))] {
        let error = lookup(&project, name, id, None, None).unwrap_err();
        assert_eq!(error.code, "ambiguous_object");
        let details = error.details.unwrap();
        let matches = details["matches"].as_array().unwrap();
        assert_eq!(matches.len(), 3);
        assert_eq!(matches[0]["exposure_duration"], 7);
        assert_eq!(matches[1]["exposure_start"], 7);
        assert_eq!(matches[2]["clip_id"], project.clips[other].id);
    }
    let found = lookup(&project, Some("Sleeve"), None, None, Some(999)).unwrap();
    assert_eq!(found["object_id"], 1);
    assert_eq!(found["sampled_frame"], 999);
    assert_eq!(found["exposure_start"], 7);
    let error = lookup(&project, Some("sleeve"), None, None, None).unwrap_err();
    assert_eq!(error.code, "object_not_found");
    assert_eq!(
        lookup(&project, Some("Sleeve"), Some(1), None, None)
            .unwrap_err()
            .code,
        "invalid_arguments"
    );
    assert_eq!(
        inspect(&project, Some(99), None).unwrap_err().code,
        "unknown_clip"
    );
    assert_eq!(
        inspect(&project, Some(1), Some(1000)).unwrap_err().code,
        "frame_out_of_range"
    );
}

#[test]
fn discovery_tracks_layer_protection_and_animated_fill() {
    let mut project = character();
    let clip = &mut project.clips[0];
    let drawing = clip.drawing_at_mut(0);
    drawing.objects[0].animated.fill_keys.insert(0, [20; 4]);
    Property::Position.set_muted(&mut drawing.objects[0], true);
    let found = lookup(&project, None, Some(1), Some(1), Some(5)).unwrap();
    assert_eq!(found["editable_properties"], json!([]));
    assert_eq!(found["muted_properties"], json!(["position"]));
    let drawing = project.clips[0].drawing_at_mut(7);
    drawing.layers[0].locked = true;
    let found = lookup(&project, None, Some(1), Some(1), Some(7)).unwrap();
    assert_eq!(found["editable_properties"], json!([]));
}
