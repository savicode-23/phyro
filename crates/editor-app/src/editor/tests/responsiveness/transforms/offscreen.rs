use super::*;
use document_core::{PathNode, Shape};

const BOARD: u32 = 512;
const SLEEVE: [f32; 2] = [71., 71.];

// Each path gets its own two-pixel cell, entirely inside the lower right corner.
fn marker_position(i: usize) -> [f32; 2] {
    [(300 + 2 * (i % 100)) as f32, (300 + 2 * (i / 100)) as f32]
}

fn add_markers(doc: &mut Document, count: usize) {
    if count == 0 {
        return;
    }
    let shape = Shape::Path {
        closed: true,
        nodes: [[0.5, 0.], [1., 0.5], [0.5, 1.], [0., 0.5]]
            .map(PathNode::corner)
            .to_vec(),
    };
    let layer = doc.layers.last().unwrap().id;
    doc.add_shape(
        marker_position(0),
        [2., 2.],
        [215, 103, 61, 255],
        shape,
        layer,
    );
    let first = doc.objects.last().unwrap().clone();
    for i in 1..count {
        let mut copy = first.clone();
        copy.id = doc.next_id;
        doc.next_id += 1;
        copy.position = marker_position(i);
        doc.objects.push(copy);
    }
}

fn visible_artboard(view: &mut Viewport) -> (egui::Rect, f32) {
    let input = view.input(Vec::new(), 1. / 60.);
    let output = view.window.frame_with_input(input);
    let (board, clip) = output
        .shapes
        .iter()
        .find_map(|s| crate::canvas::painted_artboard(&s.shape).map(|board| (board, s.clip_rect)))
        .expect("visible artboard");
    let scale = board.width() / BOARD as f32;
    assert!((board.height() / BOARD as f32 - scale).abs() < 0.001);
    let clipped = board.intersect(clip);
    let world = |p: egui::Pos2| ((p - board.min) / scale).to_pos2();
    let rect = egui::Rect::from_min_max(world(clipped.min), world(clipped.max));
    let textures = view.window.ctx.tex_manager();
    let [width, height] = textures
        .read()
        .allocated()
        .find(|(_, texture)| texture.name == "artboard artwork")
        .expect("settled artwork texture")
        .1
        .size;
    assert_eq!(width, height);
    let raster_scale = width as f32 / BOARD as f32;
    (rect, raster_scale)
}

#[test]
#[ignore = "release timing benchmark"]
fn viewport_responsiveness_offscreen() {
    let sample = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lantern-walk.vanim");
    let mut walk = project_io::load_animation(&sample).unwrap();
    let parent = walk.clips[0].drawing_at_mut(0);
    parent.width = BOARD;
    parent.height = BOARD;
    let sleeve = parent.hit_test(SLEEVE, 0).unwrap();
    assert_eq!(
        parent.objects.iter().find(|o| o.id == sleeve).unwrap().name,
        "Sleeve fabric"
    );
    let mut expected_view = None;
    for count in [0, 1_000, 10_000] {
        let mut project = walk.clone();
        add_markers(project.clips[0].drawing_at_mut(0), count);
        let mut view = Viewport {
            window: EditorWindow::new(egui::vec2(1100., 740.)),
            dpi: 2.,
            time: 0.,
            board: egui::Rect::NOTHING,
        };
        view.window.editor.replace_project(project.clone(), None);
        view.window.editor.tool = Tool::Select;
        view.frames(5, |_| Vec::new());
        for _ in 0..2 {
            let anchor = on_screen(&view, SLEEVE);
            wheel_zoom(&mut view, anchor);
        }
        let (visible, scale) = visible_artboard(&mut view);
        assert!(visible.contains(SLEEVE.into()));
        assert!(!visible.intersects(egui::Rect::from_min_max(
            [300., 300.].into(),
            [500., 500.].into()
        )));
        assert_eq!(
            *expected_view.get_or_insert((visible, scale)),
            (visible, scale)
        );
        let image = renderer::render_scaled(project.clips[0].drawing_at(0), 0, scale).unwrap();
        let clean = renderer::render_scaled(walk.clips[0].drawing_at(0), 0, scale).unwrap();
        for i in 0..count {
            let [x, y] = marker_position(i).map(|v| ((v + 1.) * scale) as u32);
            assert_eq!(clean.pixel(x, y).unwrap().alpha(), 0);
            assert!(image.pixel(x, y).unwrap().alpha() > 0, "missing marker {i}");
        }
        let left = (visible.min.x * scale).floor() as usize;
        let right = (visible.max.x * scale).ceil().min(image.width() as f32) as usize;
        for y in (visible.min.y * scale).floor() as usize
            ..(visible.max.y * scale).ceil().min(image.height() as f32) as usize
        {
            let start = (y * image.width() as usize + left) * 4;
            let end = (y * image.width() as usize + right) * 4;
            assert!(
                image.data()[start..end] == clean.data()[start..end],
                "visible row {y} changed"
            );
        }
        println!("\nLantern Walk + {count} on-artboard/offscreen paths at 1100x740 @2x, 4x fit, crop {visible:?}");
        let hit = on_screen(&view, SLEEVE);
        report(
            "hover",
            view.frames(20, |i| {
                vec![egui::Event::PointerMoved(
                    hit + egui::vec2((i % 4) as f32, 0.),
                )]
            }),
        );
        for rep in 1..=3 {
            println!("trial {rep}");
            for edit in [Edit::Move, Edit::Scale, Edit::Rotate] {
                gesture(&mut view, &project, &[sleeve], SLEEVE, edit);
            }
        }
    }
}
