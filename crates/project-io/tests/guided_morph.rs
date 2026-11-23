use document_core::{
    morph::{MorphGuide, MorphPoint},
    AnimationProject, Document, Easing, PathNode, Property, Shape,
};
use project_io::{export_svg, import_svg, load_animation, save_animation};

#[test]
fn a_sparse_guided_morph_reopens_picks_and_exports_at_the_playhead() {
    let source = Shape::Path {
        closed: true,
        nodes: [[0.1, 0.8], [0.35, 0.1], [0.9, 0.7]]
            .map(PathNode::corner)
            .to_vec(),
    };
    let mut target_nodes = [
        [0., 0.5],
        [0.5, 0.36],
        [0.72, 0.03],
        [1., 0.4],
        [0.76, 0.92],
        [0.25, 0.9],
    ]
    .map(PathNode::corner);
    target_nodes[4].outgoing = [0.48, 1.04];
    let target = Shape::Path {
        closed: true,
        nodes: target_nodes.to_vec(),
    };
    let mut doc = Document {
        frames: 1000,
        ..Document::default()
    };
    let id = doc.add_shape([8., 8.], [92., 92.], [32, 180, 146, 255], source.clone(), 1);
    let object = &mut doc.objects[0];
    let guide = MorphGuide {
        from: MorphPoint {
            segment: 0,
            t: 0.35,
        },
        to: MorphPoint { segment: 1, t: 0.6 },
    };
    object
        .set_guided_morph(0, 999, source.clone(), target.clone(), vec![guide])
        .unwrap();
    Property::Geometry.set_easing(object, 0, Easing::EaseIn);
    let project = AnimationProject::from_document(doc);
    let tmp = tempfile::tempdir().unwrap();
    let native = tmp.path().join("two-poses.vanim");
    save_animation(&native, &project, false).unwrap();
    let reopened = load_animation(&native).unwrap();
    assert_eq!(reopened, project);
    let clip = &reopened.clips[0];
    assert_eq!(clip.cels.len(), 1);
    let tick = 499;
    let scene = clip.drawing_at(tick);
    let object = &scene.objects[0];
    assert_eq!(Property::Geometry.frames(object), [0, 999]);
    assert_eq!(object.shape_at(0).as_ref(), &source);
    assert_eq!(object.shape_at(999).as_ref(), &target);
    assert_eq!(object.morph_guides(0, 999), Some([guide].as_slice()));
    let start = guide.from.position(&source).unwrap();
    let end = guide.to.position(&target).unwrap();
    let weight = Easing::EaseIn.sample(f64::from(tick) / 999.) as f32;
    let at: [f32; 2] = std::array::from_fn(|i| start[i] + (end[i] - start[i]) * weight);
    assert!(object.shape_at(tick).contours()[0]
        .iter()
        .any(|n| (n.point[0] - at[0]).hypot(n.point[1] - at[1]) < 1e-4));

    let image = renderer::render(scene, tick).unwrap();
    let svg = tmp.path().join("in-between.svg");
    export_svg(&svg, scene, tick, false).unwrap();
    let reimport = import_svg(&svg).unwrap();
    let svg_image = renderer::render(&reimport, 0).unwrap();
    let mut interior = 0;
    for y in (0..scene.height).step_by(3) {
        for x in (0..scene.width).step_by(3) {
            let alpha = image.pixel(x, y).unwrap().alpha();
            if alpha == 0 || alpha == 255 {
                let point = [x as f32 + 0.5, y as f32 + 0.5];
                assert_eq!(scene.hit_test(point, tick) == Some(id), alpha == 255);
                assert!(svg_image.pixel(x, y).unwrap().alpha().abs_diff(alpha) <= 2);
                interior += usize::from(alpha == 255);
            }
        }
    }
    assert!(
        interior > 100,
        "The filled silhouette collapsed at the sampled frame"
    );
}
