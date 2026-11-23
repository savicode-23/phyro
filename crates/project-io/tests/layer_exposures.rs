use document_core::{AnimationProject, Document, Layer, Shape};
use project_io::{load_animation, save_animation};

#[test]
fn mixed_layer_animation_roundtrips_with_correct_rendered_pixels_and_sparse_holds() {
    let mut doc = Document {
        width: 16,
        height: 16,
        frames: 1000,
        ..Document::default()
    };
    doc.layers.push(Layer {
        id: 2,
        name: "FX".into(),
        ..Layer::default()
    });
    let body = doc.add_rectangle([0., 0.], [8., 8.], [255, 0, 0, 255]);
    doc.add_shape([8., 0.], [8., 8.], [0, 255, 0, 255], Shape::Rectangle, 2);
    let mut project = AnimationProject::from_document(doc);
    let clip = &mut project.clips[0];
    clip.blank_layer_frame(8, 2).unwrap();
    clip.duplicate_layer_frame(12, 1).unwrap();
    let before = clip.clone();
    clip.drawing_at_mut(9)
        .objects
        .iter_mut()
        .find(|o| o.id == body)
        .unwrap()
        .fill = [0, 0, 255, 255];
    clip.sync_layer_edits(&before, 9).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("effects.vanim");
    save_animation(&path, &project, false).unwrap();
    let opened = load_animation(&path).unwrap();
    assert_eq!(opened, project);
    let clip = &opened.clips[0];
    assert_eq!(clip.layer_exposure(11, 1), (0, 12));
    assert_eq!(clip.layer_exposure(12, 2), (8, 992));
    for (t, body, fx) in [
        (0, [0, 0, 255, 255], [0, 255, 0, 255]),
        (8, [0, 0, 255, 255], [0; 4]),
        (999, [255, 0, 0, 255], [0; 4]),
    ] {
        let pix = renderer::render(clip.drawing_at(t), t).unwrap();
        let sample = |x: usize| {
            let i: usize = (2 * 16 + x) * 4;
            <[u8; 4]>::try_from(&pix.data()[i..i + 4]).unwrap()
        };
        assert_eq!(sample(2), body);
        assert_eq!(sample(10), fx);
    }
}
