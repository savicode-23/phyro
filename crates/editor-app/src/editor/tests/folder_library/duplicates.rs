use super::*;

fn duplicate_key(pressed: bool) -> egui::Event {
    egui::Event::Key {
        key: egui::Key::D,
        physical_key: Some(egui::Key::D),
        pressed,
        repeat: false,
        modifiers: egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        },
    }
}

fn visit_textures(shape: &egui::Shape, visit: &mut impl FnMut(egui::TextureId)) {
    match shape {
        egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| visit_textures(s, visit)),
        egui::Shape::Mesh(mesh) => visit(mesh.texture_id),
        _ => {}
    }
}

#[test]
fn duplicating_a_placed_clip_displays_each_copy_and_undo_restores_the_original() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("glow.vanim");
    let mut clip = Document::default();
    clip.add_rectangle([0., 0.], [64., 64.], [80, 200, 190, 128]);
    project_io::save_animation(&path, &AnimationProject::from_document(clip), false).unwrap();
    let mut window = EditorWindow::new(egui::vec2(950., 640.));
    window.editor.inspector_tab = crate::inspector::Tab::Properties;
    window.frame(Vec::new());
    window
        .editor
        .place_folder_asset(&window.ctx, path, None, [160., 160.]);
    wait_for(&mut window, |w| !w.editor.selection.is_empty());
    let single = window.editor.project.clone();
    let original = window.editor.selection[0];
    for count in 2..=3 {
        let before = window.editor.selection[0];
        let output = window.frame(vec![duplicate_key(true), duplicate_key(false)]);
        let copy = window.editor.selection[0];
        assert_ne!(before, copy);
        assert_eq!(window.editor.scene().objects.len(), count);
        // Coincident translucent copies must each be drawn, even when all the
        // instances share one uploaded texture.
        let manager = window.ctx.tex_manager();
        let textures = manager.read();
        let mut sprites = Vec::new();
        for shape in output.shapes {
            visit_textures(&shape.shape, &mut |id| {
                if textures
                    .meta(id)
                    .is_some_and(|t| t.name == "moving artwork")
                {
                    sprites.push(id);
                }
            });
        }
        assert_eq!(sprites.len(), count);
        assert!(sprites.iter().all(|id| *id == sprites[0]));
    }
    // Undo while the exact page for the newest copy can still be in flight.
    for count in [2, 1] {
        window.editor.restore(false);
        window.frame(Vec::new());
        assert_eq!(window.editor.scene().objects.len(), count);
    }
    assert_eq!(window.editor.project, single);
    assert_eq!(window.editor.scene().objects[0].id, original);
}
