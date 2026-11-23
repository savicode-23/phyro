use super::*;
use egui::{pos2, vec2, Pos2};

#[derive(Clone, Copy, Debug)]
enum Drag {
    Move,
    Scale,
    Rotate,
}

impl Drag {
    fn map(self, point: Pos2, pivot: Pos2, from: f32, to: f32) -> Pos2 {
        let offset = point - pivot;
        match self {
            Self::Move => point + vec2(13., 5.) * (to - from),
            Self::Scale => {
                let factor = |n| vec2(1. + n * 0.14, 1. + n * 0.05);
                pivot + offset * (factor(to) / factor(from))
            }
            Self::Rotate => pivot + egui::emath::Rot2::from_angle((to - from) * 0.15) * offset,
        }
    }
}

// Three UV corners distinguish a real affine rotation or stretch from a
// translation or the axis-aligned bounding box of a rotated image.
fn sprite_triangle(window: &EditorWindow, frame: &egui::FullOutput) -> Option<[Pos2; 3]> {
    let manager = window.ctx.tex_manager();
    let textures = manager.read();
    let mesh = frame.shapes.iter().find_map(|s| match &s.shape {
        egui::Shape::Mesh(m) if textures.meta(m.texture_id)?.name == "moving artwork" => Some(m),
        _ => None,
    })?;
    Some(
        [Pos2::ZERO, pos2(1., 0.), pos2(0., 1.)]
            .map(|uv| mesh.vertices.iter().find(|v| v.uv == uv).unwrap().pos),
    )
}

fn check_drag(window: &mut EditorWindow, drag: Drag) {
    let before = window.editor.project.clone();
    let board = window.board();
    let doc = window.editor.scene();
    let screen = |p| board.min + egui::Vec2::from(p) * (board.width() / doc.width as f32);
    let object = &doc.objects[0];
    let id = object.id;
    let corners = [[0., 0.], [1., 0.], [1., 1.], [0., 1.]];
    let initial = corners.map(object.local_to_world_at(0)).map(screen);
    let (start, pivot) = match drag {
        Drag::Move => {
            // Grab painted artwork away from the pivot and transform handles.
            let grab = Pos2::from(object.position_at(0)) + vec2(45., 37.);
            assert_eq!(doc.hit_test(grab.into(), 0), Some(id));
            (screen(grab.into()), Pos2::ZERO)
        }
        Drag::Scale => (initial[2], initial[0]),
        Drag::Rotate => (
            initial[0].lerp(initial[1], 0.5) - vec2(0., 18.),
            screen(object.pivot_at(0)),
        ),
    };
    let density = board.width() / doc.width as f32;
    assert_eq!(window.editor.selection, [id]);
    window.frame(vec![egui::Event::PointerMoved(start), button(start, true)]);
    let mut first = None;
    let check_triangle = |prior: [Pos2; 3], current: [Pos2; 3], n: f32| {
        for (a, b) in prior.into_iter().zip(current) {
            let expected = drag.map(a, pivot, 1., n);
            assert!(
                expected.distance(b) < 0.1,
                "{drag:?} step {n}: {b:?} vs {expected:?}"
            );
        }
    };
    for step in 1..=5 {
        let n = step as f32;
        let pointer = drag.map(start, pivot, 0., n);
        let frame = window.frame(vec![egui::Event::PointerMoved(pointer)]);
        let triangle = sprite_triangle(window, &frame)
            .unwrap_or_else(|| panic!("canvas did not paint the placement for {drag:?} step {n}"));
        if let Some(prior) = first {
            check_triangle(prior, triangle, n);
            let manager = window.ctx.tex_manager();
            let textures = manager.read();
            for (id, _) in &frame.textures_delta.set {
                if let Some(meta) = textures.meta(*id) {
                    assert!(
                        !responsiveness::is_artwork_texture(&meta.name),
                        "{drag:?} step {n} uploaded {}",
                        meta.name
                    );
                }
            }
        } else {
            first = Some(triangle);
        }
    }
    assert_eq!(window.editor.project, before);
    // The final button event can arrive beyond the last held pointer sample.
    let end = drag.map(start, pivot, 0., 7.);
    let release = window.frame(vec![button(end, false)]);
    if let Some(triangle) = sprite_triangle(window, &release) {
        check_triangle(first.unwrap(), triangle, 7.);
    }
    assert_eq!(window.editor.selection, [id]);
    let object = &window.editor.scene().objects[0];
    for (world, original) in corners
        .map(object.local_to_world_at(0))
        .into_iter()
        .zip(initial)
    {
        let actual = board.min + egui::Vec2::from(world) * density;
        let expected = drag.map(original, pivot, 0., 7.);
        assert!(
            actual.distance(expected) < 0.1,
            "{drag:?} released at {actual:?} vs {expected:?}"
        );
    }
    window.click_edit("Undo");
    assert_eq!(window.editor.project, before);
}

pub(super) fn check_placed_clip(window: &mut EditorWindow) {
    check_drag(window, Drag::Move);
    check_drag(window, Drag::Scale);
    let before = window.editor.project.clone();
    let board = window.board();
    let doc = window.editor.scene();
    let density = board.width() / doc.width as f32;
    let to_screen = |p: [f32; 2]| board.min + egui::Vec2::from(p) * density;
    let placement = &doc.objects[0];
    let center = to_screen(placement.pivot_at(0));
    let pivot = to_screen(placement.local_to_world_at(0)([0.2, 0.7]));
    window.drag(center + vec2(4., 0.), pivot - center);
    assert!(to_screen(window.editor.scene().objects[0].pivot_at(0)).distance(pivot) < 0.1);
    check_drag(window, Drag::Rotate);
    window.click_edit("Undo");
    assert_eq!(window.editor.project, before);
}
