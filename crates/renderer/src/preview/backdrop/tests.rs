use super::*;
use document_core::{Gradient, GradientKind, GradientStop, Layer, Shape, Stencil, SymbolInstance};

fn scene() -> (Arc<Document>, usize) {
    let mut document = Document::default();
    let gradient = Gradient {
        kind: GradientKind::Radial {
            center: [0.3, 0.6],
            radius: 0.74,
        },
        stops: vec![
            GradientStop {
                offset: 0.,
                color: [214, 71, 42, 206],
            },
            GradientStop {
                offset: 1.,
                color: [21, 170, 234, 79],
            },
        ],
    };
    for i in 0..4 {
        document.add_shape(
            [1.43, 2.12 + i as f32],
            [121.8, 119.6],
            [96; 4],
            Shape::Ellipse,
            1,
        );
        let shape = document.objects.last_mut().unwrap();
        shape.gradient = Some(gradient.clone());
        shape.stroke = [120, 32, 65, 157];
        shape.stroke_width = 1.4;
        shape.opacity = 0.8;
    }
    for _ in 0..508 {
        document.add_rectangle([6.; 2], [0.5; 2], [52; 4]);
    }
    document.layers.push(Layer {
        id: 2,
        name: "Cutter".into(),
        visible: true,
        locked: false,
    });
    document.add_shape(
        [40.2, 37.1],
        [11.7, 10.6],
        [161, 81, 34, 199],
        Shape::Ellipse,
        2,
    );
    let slot = document.objects.len() - 1;
    // The last object in storage is on a lower layer. Include nested artwork,
    // a mask and fractional gradients in the actual pixels below the cutter.
    let mut child = Document::default();
    child.add_rectangle([0.; 2], [128.; 2], [127; 4]);
    child.objects[0].gradient = Some(gradient);
    document.add_rectangle([14.7, 12.3], [85.3, 94.1], [255; 4]);
    let placed = document.objects.last_mut().unwrap();
    placed.symbol = Some(SymbolInstance::new(2, Arc::new(child)));
    placed.stencil = Some(Stencil {
        shape: Shape::Ellipse,
        origin: [0.03, 0.04],
        size: [0.93, 0.85],
        fill_rule: Default::default(),
        alpha: Some([50, 217]),
    });
    (Arc::new(document), slot)
}

#[test]
fn patches_match_full_raster_and_refuse_stale_backgrounds() {
    let (mut previous, slot) = scene();
    let density = 1.61;
    let (mut displayed, backdrop) = render_editable(&previous, 0, density).unwrap();
    assert!(displayed.data() == crate::render_scaled(&previous, 0, density).unwrap().data());
    let mut backdrop = backdrop.unwrap();
    assert_eq!(Arc::strong_count(&previous), 1);
    for step in 0..5 {
        let mut changed = (*previous).clone();
        let cutter = &mut changed.objects[slot];
        match step {
            0 => {
                cutter.position = [42.93, 31.15];
                cutter.rotation = 11.7;
            }
            1 => {
                cutter.scale = [1.34, 0.72];
                cutter.opacity = 0.77;
                cutter.stroke = [20, 141, 77, 218];
                cutter.stroke_width = 3.17;
            }
            2 => cutter.animated.visible = false,
            3 => {
                cutter.animated.visible = true;
                cutter.position = [-102., 21.];
            }
            _ => cutter.position = [28.2, 33.71],
        }
        let next = Arc::new(changed);
        match backdrop
            .patch(&previous, &next, 0, density)
            .unwrap()
            .unwrap()
        {
            EditPatch::Pixels {
                origin: [x, y],
                pixels,
            } => {
                let stride = displayed.width() as usize;
                for (dy, row) in pixels
                    .pixels()
                    .chunks_exact(pixels.width() as usize)
                    .enumerate()
                {
                    let start = (y + dy) * stride + x;
                    displayed.pixels_mut()[start..start + row.len()].copy_from_slice(row);
                }
            }
            EditPatch::Unchanged => {}
            EditPatch::FullRender => panic!("expected the retained background"),
        }
        assert!(displayed.data() == crate::render_scaled(&next, 0, density).unwrap().data());
        previous = next;
    }
    let independent = Arc::new(previous.as_ref().clone());
    assert!(backdrop
        .patch(&independent, &previous, 0, density)
        .unwrap()
        .is_none());
    assert!(backdrop
        .patch(&previous, &previous, 1, density)
        .unwrap()
        .is_none());
    assert!(backdrop
        .patch(&previous, &previous, 0, 2.)
        .unwrap()
        .is_none());
    for mutation in 0..8 {
        let mut actual = previous.as_ref().clone();
        match mutation {
            0 => actual.objects[0].fill = [88; 4],
            1 => actual.objects.swap(slot, 0),
            2 => actual.layers[0].visible = false,
            3 => actual.objects[slot].layer = 1,
            4 => {
                actual.add_bone("Arm", None, [0.; 2], [5., 9.]).unwrap();
            }
            5 => actual.objects[slot + 1].symbol = None,
            6 => {
                actual.objects.pop();
            }
            _ => actual.width += 1,
        }
        assert!(backdrop
            .patch(&previous, &Arc::new(actual), 0, density)
            .unwrap()
            .is_none());
    }
}
