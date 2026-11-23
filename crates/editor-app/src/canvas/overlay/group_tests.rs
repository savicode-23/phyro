use super::*;
use egui::epaint::{Mesh, Primitive};

fn overlay_meshes(
    doc: &Document,
    ids: &[u64],
    frame: u32,
    rotation: Option<(Rect, f32)>,
    density: f32,
    optimized: bool,
) -> (usize, Vec<(Rect, Mesh)>) {
    let ctx = egui::Context::default();
    let clip = Rect::from_min_max(egui::pos2(5., 7.), egui::pos2(185., 160.));
    let view = View {
        board: Rect::from_min_size(egui::pos2(2.3, 4.7), Vec2::splat(800.)),
        scale: 1.31,
    };
    let mut input = egui::RawInput {
        screen_rect: Some(clip),
        ..Default::default()
    };
    input
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .native_pixels_per_point = Some(density);
    let out = ctx.run(input, |ctx| {
        let p = ctx
            .layer_painter(egui::LayerId::background())
            .with_clip_rect(clip);
        if optimized {
            group_selection(&p, view, doc, frame, ids, rotation);
        } else {
            selection(&p, view, doc, frame, ids, Tool::Select);
            if let Some((bounds, angle)) = rotation.or_else(|| {
                gesture::group_bounds(doc, frame, &GestureIds::new(ids.to_vec())).map(|b| (b, 0.))
            }) {
                group_handles(&p, view, bounds, angle);
            }
        }
    });
    let shape_count = out.shapes.len();
    let meshes = ctx
        .tessellate(out.shapes, out.pixels_per_point)
        .into_iter()
        .map(|p| {
            let Primitive::Mesh(mesh) = p.primitive else {
                panic!("unexpected paint callback")
            };
            (p.clip_rect, mesh)
        })
        .collect();
    (shape_count, meshes)
}

#[test]
fn multi_selection_preserves_visible_meshes_and_offscreen_group_extents() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lantern-elbow.vanim");
    let project = project_io::load_animation(&path).unwrap();
    let mut doc = project.clips[0].drawing_at(4).clone();
    assert!(doc.objects.iter().any(|o| o.skin.is_some()));
    let crossing = doc.add_rectangle([-200., 50.], [450., 14.], [255; 4]);
    doc.objects.last_mut().unwrap().rotation = 9.;
    let hidden_layer = crate::commands::add_layer(&mut doc);
    let locked_layer = crate::commands::add_layer(&mut doc);
    doc.layers
        .iter_mut()
        .find(|l| l.id == hidden_layer)
        .unwrap()
        .visible = false;
    doc.layers
        .iter_mut()
        .find(|l| l.id == locked_layer)
        .unwrap()
        .locked = true;
    for layer in [hidden_layer, locked_layer] {
        doc.add_shape(
            [9., 20.],
            [71., 39.],
            [255; 4],
            document_core::Shape::Ellipse,
            layer,
        );
    }
    // The far rectangles affect the group pivot while all their individual edges are clipped.
    for i in 0..140 {
        doc.add_rectangle([500. + i as f32 * 3., 76.], [5., 11.], [255; 4]);
    }
    let mut ids: Vec<_> = doc.objects.iter().map(|o| o.id).collect();
    ids.extend([crossing, u64::MAX]);
    let fixed = Rect::from_min_max(egui::pos2(-31., 40.), egui::pos2(220., 170.));
    for density in [0.75, 2.] {
        for frame in [0, 4] {
            for orientation in [None, Some((fixed, 27.))] {
                let (old_count, old) =
                    overlay_meshes(&doc, &ids, frame, orientation, density, false);
                let (new_count, new) =
                    overlay_meshes(&doc, &ids, frame, orientation, density, true);
                assert!(new_count + 500 < old_count);
                assert!(!new.is_empty());
                assert_eq!(new, old);
            }
        }
        // The short, allocation-free ID lookup should paint the identical group too.
        let few = [crossing, ids[0], ids[1], u64::MAX];
        assert_eq!(
            overlay_meshes(&doc, &few, 4, None, density, true).1,
            overlay_meshes(&doc, &few, 4, None, density, false).1,
        );
    }
}
