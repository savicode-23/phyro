use super::*;
use document_core::{Document, RasterImage};

#[test]
fn review_changes_only_the_target_hold_and_rejects_a_locked_layer() {
    let mut drawing = Document::default();
    let id = drawing.add_rectangle([2., 4.], [12., 9.], [10, 20, 30, 255]);
    let mut project = AnimationProject::from_document(drawing);
    let clip = &mut project.clips[0];
    clip.set_length(9).unwrap();
    for tick in [2, 6] {
        clip.cels.insert(tick, Arc::clone(&clip.cels[&0]));
    }
    let clip_id = clip.id;
    let operations = json!([
        {"op":"set_fill","object_id":id,"clip_id":clip_id,"exposure_start":2,"fill":[184,115,51,255]},
        {"op":"set_position_key","object_id":id,"clip_id":clip_id,"exposure_start":2,"frame":4,"position":[10.,4.]}
    ]);
    let review = Review::prepare(Arc::new(project.clone()), 0, operations.clone()).unwrap();
    assert_eq!(review.tick, 4);
    for tick in 0..9 {
        let before = render(&project, clip_id, tick, 1024).unwrap();
        let after = render(&review.after, clip_id, tick, 1024).unwrap();
        assert_eq!(before.data() != after.data(), (2..6).contains(&tick));
    }
    project.clips[0].drawing_at_mut(2).layers[0].locked = true;
    let failure = Review::prepare(Arc::new(project), 0, operations)
        .err()
        .unwrap();
    assert_eq!(failure["details"]["operation_index"], 0);
}

#[test]
fn preview_caps_the_long_side_and_keeps_visible_references() {
    let mut drawing = Document {
        width: 2048,
        height: 256,
        ..Default::default()
    };
    drawing.add_rectangle([0., 0.], [256., 256.], [255; 4]);
    drawing.objects[0].reference_only = true;
    drawing.objects[0].image = Some(Arc::new(
        RasterImage::from_rgba(1, 1, vec![248, 80, 30, 255]).unwrap(),
    ));
    let project = AnimationProject::from_document(drawing);
    let clip = &project.clips[0];
    let preview = render(&project, clip.id, 0, 1024).unwrap();
    assert_eq!((preview.width(), preview.height()), (1024, 128));
    assert!(preview
        .data()
        .as_chunks::<4>()
        .0
        .contains(&[248, 80, 30, 255]));
    assert_ne!(
        preview.data(),
        renderer::render_for_export(clip.drawing_at(0), 0, 0.5)
            .unwrap()
            .data()
    );
}
