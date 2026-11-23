use super::*;
use egui::{vec2, Event, Modifiers, PointerButton, Pos2};
use std::fs;

mod handles;
mod mixed;
mod real_project;

fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    }
}

fn place(
    file: &Path,
    cache: &Path,
    dpi: f32,
    destination: &AnimationProject,
    top: bool,
) -> Viewport {
    let mut view = Viewport {
        window: EditorWindow::new(vec2(1100., 740.)),
        dpi,
        time: 0.,
        board: egui::Rect::NOTHING,
    };
    view.window
        .editor
        .replace_project(destination.clone(), None);
    if top {
        view.window.editor.style.layer = view.window.editor.scene().layers.last().unwrap().id;
    }
    let order = if top { "in front" } else { "behind artwork" };
    println!(
        "\n{}, {order}, 1000×1000 artboard, 1100×740 window at {dpi}×",
        file.file_name().unwrap().to_string_lossy()
    );
    let id = drop_asset(&mut view, file, cache, [0.5; 2]);
    assert_eq!(
        view.window.editor.scene().paint_order().last().unwrap().id == id,
        top
    );
    view
}

fn drop_asset(view: &mut Viewport, file: &Path, cache: &Path, at: [f32; 2]) -> u64 {
    let name = file.file_name().unwrap().to_str().unwrap();
    let initial_count = view.window.editor.scene().objects.len();
    view.window.editor.inspector_tab = crate::inspector::Tab::Library;
    let library = &mut view.window.editor.library;
    library.set_cache_directory(cache.to_owned());
    library.set_directory(file.parent().unwrap().to_owned());
    library.set_folders_active(true);
    let until = Instant::now() + Duration::from_secs(20);
    let tile = loop {
        let input = view.input(Vec::new(), 1. / 60.);
        let output = view.window.frame_with_input(input);
        view.board = EditorWindow::board_rect(&output);
        if view.window.editor.library.resident_previews() > 0 {
            if let Some(tile) = output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == name => {
                    let center = text.galley.rect.translate(text.pos.to_vec2()).center();
                    shape.clip_rect.contains(center).then_some(center)
                }
                _ => None,
            }) {
                break tile;
            }
        }
        assert!(Instant::now() < until, "missing Library tile for {name}");
        std::thread::sleep(Duration::from_millis(3));
    };
    let drop_at = view.board.min + view.board.size() * egui::Vec2::from(at);
    view.frame(vec![Event::PointerMoved(tile)], 1. / 60.);
    view.frame(vec![button(tile, true)], 1. / 60.);
    report(
        "Library to canvas",
        view.frames(24, |i| {
            vec![Event::PointerMoved(
                tile.lerp(drop_at, (i + 1) as f32 / 24.),
            )]
        }),
    );
    let start = Instant::now();
    let drop = view.frame(vec![button(drop_at, false)], 1. / 60.);
    let mut waiting = vec![drop];
    while view.window.editor.scene().objects.len() == initial_count {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "{}",
            view.window.editor.status
        );
        std::thread::sleep(Duration::from_millis(2));
        waiting.push(view.frame(Vec::new(), 1. / 60.));
    }
    println!(
        "Drop to published selection: {:.2} ms elapsed",
        start.elapsed().as_secs_f64() * 1000.
    );
    report("Drop and placement", waiting);
    let before_paint = view.frame(Vec::new(), 1. / 60.);
    report("First painted placement", vec![before_paint]);
    let editor = &view.window.editor;
    let object = editor.scene().objects.last().unwrap();
    assert_eq!(editor.selection, [object.id]);
    object.id
}

fn measure(file: &Path, cache: &Path, dpi: f32, destination: &AnimationProject, top: bool) {
    let mut view = place(file, cache, dpi, destination, top);
    let editor = &view.window.editor;
    let object = editor.scene().objects.last().unwrap();
    let id = object.id;
    let before = object.position_at(0);
    let size = object.size;
    let scale = object.scale_at(0);
    // Find real ink, away from the pivot and bounding-box handles. Works for
    // hollow and sparse placed clips, without assuming their first pose.
    let to_world = object.local_to_world_at(0);
    let inside = [0.35, 0.65, 0.42, 0.58];
    let point = inside
        .into_iter()
        .flat_map(|y| inside.into_iter().map(move |x| [x, y]))
        .map(to_world)
        .find(|p| editor.scene().hit_test(*p, 0) == Some(id))
        .expect("placed artwork hit");
    let from = view.board.min + view.board.size() * vec2(point[0] / 1000., point[1] / 1000.);
    view.frame(vec![Event::PointerMoved(from)], 1. / 60.);
    let press = view.frame(vec![button(from, true)], 1. / 60.);
    let point = |i: usize| from + vec2(9. + i as f32 * 0.65, (i as f32 / 11.).sin() * 8.);
    let first = view.frame(vec![Event::PointerMoved(point(0))], 1. / 60.);
    report("Move press", vec![press]);
    report("First move", vec![first]);
    report(
        "Moving placement",
        view.frames(90, |i| vec![Event::PointerMoved(point(i + 1))]),
    );
    let commit = view.frame(vec![button(point(90), false)], 1. / 60.);
    report("Move release", vec![commit]);
    let drawn = view.frame(Vec::new(), 1. / 60.);
    report("First paint after release", vec![drawn]);
    let placed = view.window.editor.scene().objects.last().unwrap();
    assert_eq!(placed.id, id);
    let after = placed.position_at(0);
    assert_eq!(
        (placed.size, placed.scale_at(0)),
        (size, scale),
        "expected a translation"
    );
    assert!(
        after[0] - before[0] > 10.,
        "drag must move {}: {before:?} -> {after:?}",
        file.display()
    );
    view.window.editor.restore(false);
    assert_eq!(
        view.window
            .editor
            .scene()
            .objects
            .last()
            .unwrap()
            .position_at(0),
        before
    );
    view.window.editor.restore(false);
    assert_eq!(&view.window.editor.project, destination);
}

fn layered_destination(keeper: &Document) -> AnimationProject {
    let mut scene = keeper.clone();
    scene.width = 1000;
    scene.height = 1000;
    for object in &mut scene.objects {
        object.position[0] += 420.;
        object.position[1] += 415.;
    }
    AnimationProject::from_document(scene)
}

fn asset_file(root: &Path, folder: &str, filename: &str) -> std::path::PathBuf {
    let folder = root.join(folder);
    fs::create_dir(&folder).unwrap();
    folder.join(filename)
}

fn clip_fixtures(root: &Path, examples: &Path, keeper: &Document) -> [std::path::PathBuf; 2] {
    let walk = asset_file(root, "walk", "Walk.vanim");
    fs::copy(examples.join("lantern-walk.vanim"), &walk).unwrap();
    let tiles = asset_file(root, "tiles", "Tiles.vanim");
    let project = AnimationProject::from_document(tiled_scene(keeper, 1000));
    project_io::save_animation(&tiles, &project, false).unwrap();
    [walk, tiles]
}

#[test]
#[ignore = "release timing benchmark of real Library drops and the first selection drag"]
fn placed_library_asset_move() {
    let dir = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let keeper = project_io::load_animation(&examples.join("lantern-keeper.vanim")).unwrap();
    let art = keeper.clips[0].drawing_at(0);
    let destination = layered_destination(art);
    let svg = asset_file(dir.path(), "svg", "Box.svg");
    fs::write(&svg, r#"<svg xmlns="http://www.w3.org/2000/svg" width="128" height="128"><rect x="8" y="8" width="96" height="96" fill="teal"/></svg>"#).unwrap();
    let raster = asset_file(dir.path(), "raster", "Keeper.png");
    renderer::render_scaled(art, 0, 4.)
        .unwrap()
        .save_png(&raster)
        .unwrap();
    let [walk, tiles] = clip_fixtures(dir.path(), &examples, art);
    for file in [svg, raster, walk, tiles] {
        for dpi in [1., 2.] {
            measure(&file, cache.path(), dpi, &destination, true);
        }
        measure(&file, cache.path(), 1., &destination, false);
    }
}

#[test]
#[ignore = "release timing benchmark of a placed clip moving over a dense scene"]
fn move_library_clip_over_dense_destination() {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let source = project_io::load_animation(&examples.join("lantern-keeper.vanim")).unwrap();
    let mut scene = tiled_scene(source.clips[0].drawing_at(0), 1000);
    scene.width = 1000;
    scene.height = 1000;
    let folder = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let walk = folder.path().join("Walk.vanim");
    fs::copy(examples.join("lantern-walk.vanim"), &walk).unwrap();
    let destination = AnimationProject::from_document(scene);
    for dpi in [1., 2.] {
        measure(&walk, cache.path(), dpi, &destination, true);
    }
}
