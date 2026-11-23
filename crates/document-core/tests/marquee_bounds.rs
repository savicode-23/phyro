use document_core::{
    Document, LineCap, LineJoin, PathEffect, PathNode, Shape, SymbolInstance, VectorObject,
};

fn check_edges(object: &VectorObject, frame: u32) {
    let [l, t, r, b] = object.bounds(frame);
    for dx in [-1., 0., 1.] {
        for dy in [-1., 0., 1.] {
            for area in [
                [l + dx, t + dy, r, b],
                [l, t, r + dx, b + dy],
                [l + dx, t, r, b + dy],
                [l, t + dy, r + dx, b],
                [r + 12., b + 15., r + 24., b + 30.],
            ] {
                let expected = area[0] <= l && area[1] <= t && r <= area[2] && b <= area[3];
                assert_eq!(
                    object.bounds_within(area, frame),
                    expected,
                    "object {} at tick {frame}, exact bounds {l},{t},{r},{b}, query {area:?}",
                    object.id
                );
            }
        }
    }
}

#[test]
fn marquee_containment_matches_exact_stroked_geometry_at_each_boundary() {
    let mut doc = Document::default();
    let mut nodes = [[0.2, 0.7], [0.5, 0.1], [0.60, 0.95], [1.3, 0.25]].map(PathNode::corner);
    nodes[0].outgoing = [-0.47, -0.65];
    nodes[1].incoming = [0.4, -0.3];
    for (i, shape) in [
        Shape::Rectangle,
        Shape::Ellipse,
        Shape::Path {
            closed: false,
            nodes: nodes.to_vec(),
        },
        Shape::Compound {
            contours: vec![
                nodes.to_vec(),
                vec![
                    PathNode::corner([0.3, 0.4]),
                    PathNode::corner([0.7, 0.5]),
                    PathNode::corner([0.5, 0.7]),
                ],
            ],
        },
    ]
    .into_iter()
    .enumerate()
    {
        doc.add_shape([12., 15.], [35., 19.], [255; 4], shape, 1);
        let object = doc.objects.last_mut().unwrap();
        object
            .keys
            .extend([(0, [-3_500., 7_000.]), (4, [25., 21.])]);
        object.rotation_keys.extend([(0, 32.), (4, -11.)]);
        object
            .scale_keys
            .extend([(0, [0.9, 0.75]), (4, [1.5, 1.1])]);
        object.animated.skew = 13.;
        if i == 3 {
            object.effects.push(PathEffect::Mirror { horizontal: true });
        }
        for (cap, join) in [
            (LineCap::Butt, LineJoin::Bevel),
            (LineCap::Square, LineJoin::Miter),
            (LineCap::Round, LineJoin::Round),
        ] {
            object.cap = cap;
            object.join = join;
            for width in [0., 7.] {
                object.stroke_width = width;
                for frame in [0, 2, 4] {
                    check_edges(object, frame);
                }
            }
        }
    }
    // An instance can contain artwork outside its nominal placement box.
    let mut child = Document::default();
    child.add_rectangle([-256., 330.], [12., 15.], [255; 4]);
    doc.add_rectangle([18., 12.], [41., 33.], [255; 4]);
    let instance = doc.objects.last_mut().unwrap();
    instance.symbol = Some(SymbolInstance::new(1, child.into()));
    instance.rotation = 17.;
    check_edges(instance, 0);
}
