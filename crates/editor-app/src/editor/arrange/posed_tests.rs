use super::*;
use document_core::{AnimationProject, PathNode, Shape, SymbolDefinition};
use std::sync::Arc;

#[test]
fn a_posed_skin_aligns_at_its_painted_edge_or_rejects_every_move_and_key() {
    let mut doc = Document::default();
    let ordinary = doc.add_rectangle([4., 12.], [4.; 2], [255; 4]);
    let arm = doc.add_bone("Arm", None, [10., 10.], [20., 10.]).unwrap();
    let wrist = doc
        .add_bone("Wrist", Some(arm), [20., 10.], [30., 10.])
        .unwrap();
    let skin = doc.add_shape(
        [20., 10.],
        [10., 5.],
        [255; 4],
        Shape::Path {
            closed: true,
            nodes: [[0., 0.], [1., 0.], [1., 1.], [0., 1.]]
                .map(PathNode::corner)
                .to_vec(),
        },
        1,
    );
    doc.bind_skin_at(skin, wrist, 0).unwrap();
    doc.set_bone_rotation_key(wrist, 0, 0.).unwrap();
    doc.set_bone_rotation_key(wrist, 4, 90.).unwrap();
    let anchor = doc.add_rectangle([60., 25.], [40.; 2], [255; 4]);
    let ids = [ordinary, skin, anchor];
    let action = Action::Align(Alignment::Left, Reference::LargestArea);
    let mixed = doc.clone();
    assert!(apply(&mut doc, &ids, 4, KeyMode::Auto, action).is_err());
    assert_eq!(
        doc, mixed,
        "an earlier movable object must not acquire a key"
    );

    for n in 0..4 {
        doc.set_skin_weight(skin, 0, n, 1.).unwrap();
    }
    let before = doc.clone();
    assert!(apply(&mut doc, &ids, 4, KeyMode::Auto, action).unwrap());
    assert_eq!(doc.objects[2], before.objects[2]);
    assert_eq!(doc.objects[1].skin, before.objects[1].skin);
    assert_eq!(doc.objects[1].bounds(0), before.objects[1].bounds(0));
    let [l, t, r, b] = doc.objects[1].bounds(4);
    assert!((l - 60.).abs() < 0.0001 && (r - 65.).abs() < 0.0001);
    assert!((t - 10.).abs() < 0.0001 && (b - 20.).abs() < 0.0001);
    assert_eq!(doc.objects[0].bounds(4)[0], 60.);
}

#[test]
fn linked_placements_use_the_childs_current_painted_extent_and_keep_their_sources() {
    let mut project = AnimationProject::default();
    let mut art = Document {
        frames: 1,
        ..Default::default()
    };
    art.add_rectangle([12., 8.], [12., 9.], [255; 4]);
    let symbol = project
        .add_symbol(SymbolDefinition {
            name: "Badge".into(),
            artwork: art.into(),
        })
        .unwrap();
    let child = project.add_clip("Growing square").unwrap();
    project.clips[child].set_length(4).unwrap();
    let cd = project.clips[child].drawing_at_mut(0);
    cd.add_rectangle([2., 6.], [10., 8.], [255; 4]);
    cd.objects[0]
        .scale_keys
        .extend([(0, [1.; 2]), (3, [3.; 2])]);
    let clip_id = project.clips[child].id;
    let s = project.place_symbol(symbol, 0, 0, 1, [45., 15.]).unwrap();
    let c = project.place_clip(clip_id, 0, 0, 1, [19., 9.]).unwrap();
    let saved_child = project.clips[child].clone();
    let doc = project.clips[0].drawing_at_mut(0);
    let area = |b: [f32; 4]| (b[2] - b[0]) * (b[3] - b[1]);
    assert!(area(doc.objects[0].bounds(0)) > area(doc.objects[1].bounds(0)));
    assert!(area(doc.objects[0].bounds(3)) < area(doc.objects[1].bounds(3)));
    let symbol_source = doc.objects[0]
        .symbol
        .as_ref()
        .unwrap()
        .shared_artwork()
        .unwrap()
        .clone();
    let clip_source = doc.objects[1]
        .clip_instance
        .as_ref()
        .unwrap()
        .shared_clip()
        .unwrap()
        .clone();
    let anchor = doc.objects[1].clone();
    assert!(apply(
        doc,
        &[s, c],
        3,
        KeyMode::Tracked,
        Action::Align(Alignment::Left, Reference::LargestArea)
    )
    .unwrap());
    assert_eq!(doc.objects[1], anchor);
    assert_eq!(doc.objects[0].bounds(3)[0], anchor.bounds(3)[0]);
    assert!(Arc::ptr_eq(
        doc.objects[0]
            .symbol
            .as_ref()
            .unwrap()
            .shared_artwork()
            .unwrap(),
        &symbol_source
    ));
    assert!(Arc::ptr_eq(
        doc.objects[1]
            .clip_instance
            .as_ref()
            .unwrap()
            .shared_clip()
            .unwrap(),
        &clip_source
    ));
    assert!(Arc::ptr_eq(
        &project.symbols[&symbol].artwork,
        &symbol_source
    ));
    assert_eq!(project.clips[child], saved_child);
}
