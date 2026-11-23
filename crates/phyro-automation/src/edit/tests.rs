use super::*;
use document_core::{arrange, Document, MotionPath, PathNode, Shape};

fn fixture() -> (AnimationProject, u64, u64) {
    let mut drawing = Document::default();
    let id = drawing.add_rectangle([2., 3.], [16., 12.], [25, 60, 150, 255]);
    let project = AnimationProject::from_document(drawing);
    let clip = project.clips[0].id;
    (project, clip, id)
}

#[test]
fn set_fill_restores_paint_even_when_the_new_color_is_transparent() {
    let (mut project, clip, id) = fixture();
    let object = &mut project.clips[0].drawing_at_mut(0).objects[0];
    object.no_fill = true;
    object.gradient = Some(document_core::Gradient::from_color(
        document_core::GradientKind::Radial {
            center: [0.5, 0.5],
            radius: 0.5,
        },
        object.fill,
    ));
    let edit = json!([{"op":"set_fill","clip_id":clip,"exposure_start":0,"object_id":id,"fill":[11,22,33,0]}]);
    apply(&mut project, &edit).unwrap();
    let object = &project.clips[0].drawing_at(0).objects[0];
    assert!(!object.no_fill);
    assert!(object.gradient.is_none());
    assert_eq!(object.fill, [11, 22, 33, 0]);
}

#[test]
fn edits_are_atomic_and_errors_identify_the_failing_scope() {
    let (mut project, clip, id) = fixture();
    project.clips[0].duplicate_frame(2).unwrap();
    let original = project.clone();
    for bad in [
        json!({"op":"set_fill","clip_id":clip,"exposure_start":0,"object_id":999,"fill":[0,0,0,0]}),
        json!({"op":"set_fill","clip_id":clip,"exposure_start":1,"object_id":id,"fill":[0,0,0,0]}),
        json!({"op":"set_position_key","clip_id":clip,"exposure_start":0,"object_id":id,"frame":3,"position":[1,1]}),
        json!({"op":"set_fill","clip_id":clip,"exposure_start":0,"object_id":id,"fill":[0,0,0,0],"typo":1}),
    ] {
        let good = json!({"op":"set_fill","clip_id":clip,"exposure_start":0,"object_id":id,"fill":[1,2,3,255]});
        let error = apply(&mut project, &json!([good, bad])).unwrap_err();
        let details = error.details.unwrap();
        assert_eq!(details["operation_index"], 1);
        assert_eq!(details["exposure_start"], bad["exposure_start"]);
        assert_eq!(details["object_id"], bad["object_id"]);
        assert_eq!(project, original);
    }
}

#[test]
fn named_paths_and_text_share_the_editors_alignment_and_boolean_operations() {
    let (mut p, clip, base) = fixture();
    let operations = json!([
        {"op":"create_path","clip_id":clip,"exposure_start":0,"layer_id":1,"name":"Cutter","closed":true,"fill":[200,124,60,255],"nodes":[{"point":[8,6]},{"point":[12,6]},{"point":[12,10]},{"point":[8,10]}]},
        {"op":"create_text","clip_id":clip,"exposure_start":0,"layer_id":1,"name":"Caption","position":[5,4],"content":"Night","fill":[4,5,6,255],"font_size":8}
    ]);
    let results = apply(&mut p, &operations).unwrap();
    let path = results[0]["created_ids"][0].as_u64().unwrap();
    let text = results[1]["created_ids"][0].as_u64().unwrap();
    let mut oracle = p.clips[0].drawing_at(0).clone();
    let moves = arrange::position_updates(
        &oracle,
        &[base, path],
        0,
        arrange::Action::Align(
            arrange::Alignment::CenterX,
            arrange::Reference::BiggestByDirection,
        ),
    )
    .unwrap();
    for (i, pos) in moves {
        oracle.objects[i].set_position_at(0, pos);
    }
    let expected = oracle
        .boolean(&[base, path], 0, document_core::BooleanOp::Difference)
        .unwrap();
    let result = apply(&mut p,&json!([
        {"op":"set_text","clip_id":clip,"exposure_start":0,"object_id":text,"content":"New\nline"},
        {"op":"align","clip_id":clip,"exposure_start":0,"object_ids":[base,path],"frame":0,"direction":"center_x"},
        {"op":"boolean","clip_id":clip,"exposure_start":0,"object_ids":[base,path],"frame":0,"operation":"difference"}
    ])).unwrap();
    let doc = p.clips[0].drawing_at(0);
    assert_eq!(result[2]["result_ids"], json!(expected));
    assert_eq!(doc.objects[0], oracle.objects[0]);
    let caption = doc.objects.iter().find(|o| o.id == text).unwrap();
    assert_eq!(caption.text.as_ref().unwrap().content, "New\nline");
    assert_eq!(caption.text.as_ref().unwrap().size, 8.);
    assert_eq!(caption.name, "Caption");
    assert!(matches!(caption.shape, Shape::Compound { .. }));
}

#[test]
fn changing_a_key_preserves_held_drawing_identity_and_hydrates_nested_art() {
    let (mut p, clip_id, id) = fixture();
    let child = p.add_clip("Parent").unwrap();
    p.place_clip(clip_id, child, 0, 1, [0., 0.]).unwrap();
    apply(&mut p,&json!([{"op":"set_position_key","clip_id":clip_id,"exposure_start":0,"object_id":id,"frame":4,"position":[18,11]}])).unwrap();
    let clip = &p.clips[0];
    assert_eq!(clip.cels.len(), 1);
    assert_eq!(clip.drawing_at(0).objects[0].position_at(0), [2., 3.]);
    assert_eq!(clip.drawing_at(2).objects[0].position_at(2), [10., 7.]);
    let placement = p.clips[child].drawing_at(0).objects[0]
        .clip_instance
        .as_ref()
        .unwrap();
    assert_eq!(
        placement.shared_clip().unwrap().drawing_at(4).objects[0].position_at(4),
        [18., 11.]
    );
}

#[test]
fn position_keys_accept_artboard_coordinates_with_motion_and_an_animated_rig() {
    let (mut project, clip, id) = fixture();
    let drawing = project.clips[0].drawing_at_mut(0);
    let bone = drawing.add_bone("Arm", None, [2., 3.], [13., 3.]).unwrap();
    drawing.bind_rigid(id, Some(bone)).unwrap();
    drawing.set_bone_rotation_key(bone, 0, 23.).unwrap();
    drawing.set_bone_rotation_key(bone, 4, 68.).unwrap();
    let object = &mut drawing.objects[0];
    object.motion = Some(
        MotionPath::new(
            vec![PathNode::corner([1., 2.]), PathNode::corner([9., 13.])],
            false,
        )
        .unwrap(),
    );
    object.animated.progress_keys.extend([(0, 0.2), (4, 0.7)]);
    let first = object.position_at(0);
    apply(&mut project,&json!([{"op":"set_position_key","clip_id":clip,"exposure_start":0,"object_id":id,"frame":4,"position":[31,17]}])).unwrap();
    let object = &project.clips[0].drawing_at(0).objects[0];
    assert_eq!(object.position_at(0), first);
    assert!(object.keys.contains_key(&0) && object.keys.contains_key(&4));
    for (actual, target) in object.position_at(4).into_iter().zip([31., 17.]) {
        assert!((actual - target).abs() < 0.0001);
    }
}

#[test]
fn every_discovery_example_parses_and_each_variant_rejects_unknown_fields() {
    for spec in describe().as_array().unwrap() {
        let mut example = spec["example"].clone();
        serde_json::from_value::<Edit>(example.clone()).unwrap();
        for field in example.as_object().unwrap().keys() {
            assert!(spec["schema"]["properties"].get(field).is_some());
        }
        example["unknown_field"] = json!(true);
        assert!(serde_json::from_value::<Edit>(example).is_err());
    }
}
