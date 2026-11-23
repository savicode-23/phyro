use document_core::{Document, PathEffect, PathNode, Shape};

#[test]
fn saved_live_effects_match_canvas_hit_testing_and_exported_svg() {
    let mut doc = Document::default();
    let nodes = [[0., 0.], [0.4, 0.], [0., 1.]]
        .map(PathNode::corner)
        .to_vec();
    let id = doc.add_shape(
        [8., 8.],
        [80., 50.],
        [245, 91, 67, 255],
        Shape::Path {
            nodes,
            closed: true,
        },
        1,
    );
    let source = renderer::render(&doc, 0).unwrap();
    doc.objects[0].effects = vec![
        PathEffect::Mirror { horizontal: true },
        PathEffect::RoundCorners { distance: 4. },
    ];
    assert_eq!(doc.hit_test([82., 18.], 0), Some(id));
    assert_eq!(doc.hit_test([12., 18.], 0), None);
    let rendered = renderer::render(&doc, 0).unwrap();
    assert_ne!(source.data(), rendered.data());
    let folder = tempfile::tempdir().unwrap();
    let project = folder.path().join("effects.vanim");
    project_io::save_project(&project, &doc).unwrap();
    assert_eq!(project_io::load_project(&project).unwrap(), doc);
    let svg = folder.path().join("effects.svg");
    project_io::export_svg(&svg, &doc, 0, false).unwrap();
    let restored = project_io::import_svg(&svg).unwrap();
    let raster = renderer::render(&restored, 0).unwrap();
    assert_eq!(raster.data(), rendered.data());
}
