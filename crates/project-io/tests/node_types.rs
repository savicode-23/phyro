use document_core::{
    path_edit::{style_nodes, HandleStyle},
    AnimationProject, Document, PathNode, Property, Shape,
};
use project_io::{load_animation, load_project, save_animation, save_project};
use std::{fs::File, io::Read};

#[test]
fn authored_handle_types_reopen_in_drawings_and_in_geometry_keys_only() {
    let mut doc = Document::default();
    let shape = Shape::Path {
        nodes: [[0., 0.], [0.3, 0.8], [1., 0.]]
            .map(PathNode::corner)
            .to_vec(),
        closed: false,
    };
    doc.add_shape([6.; 2], [60., 30.], [255; 4], shape, 1);
    let original = doc.clone();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nodes.vanim");
    let version = || {
        let mut archive = zip::ZipArchive::new(File::open(&path).unwrap()).unwrap();
        let mut json = String::new();
        archive
            .by_name("manifest.json")
            .unwrap()
            .read_to_string(&mut json)
            .unwrap();
        serde_json::from_str::<serde_json::Value>(&json).unwrap()["format_version"]
            .as_u64()
            .unwrap()
    };
    save_project(&path, &doc).unwrap();
    assert_eq!(version(), u64::from(document_core::FORMAT_VERSION));
    let object = &mut doc.objects[0];
    assert!(style_nodes(
        &mut object.shape.contours_mut()[0],
        false,
        object.size,
        1..=1,
        HandleStyle::Smooth
    ));
    save_project(&path, &doc).unwrap();
    assert_eq!(version(), 12);
    assert_eq!(load_project(&path).unwrap(), doc);
    let mut project = AnimationProject::from_document(original);
    let object = &mut project.clips[0].drawing_at_mut(0).objects[0];
    Property::Geometry.add(object, 0);
    object
        .set_shape_at(4, doc.objects[0].shape.clone())
        .unwrap();
    save_animation(&path, &project, true).unwrap();
    assert_eq!(version(), 12);
    assert_eq!(load_animation(&path).unwrap(), project);
}
