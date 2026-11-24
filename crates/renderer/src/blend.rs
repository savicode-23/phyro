use document_core::BlendMode;
use tiny_skia::BlendMode as Skia;

/// Split textures use source-over in the editor. Blended artwork instead
/// needs its original backdrop when rasterized for a transform preview.
pub fn requires_scene_compositing(document: &document_core::Document) -> bool {
    fn visit(
        doc: &document_core::Document,
        seen: &mut std::collections::HashSet<*const document_core::Document>,
    ) -> bool {
        seen.insert(doc)
            && (doc
                .layers
                .iter()
                .any(|l| l.visible && !l.blend_mode.is_normal())
                || doc.objects.iter().any(|o| {
                    o.uses_blending() || o.static_artwork().is_some_and(|art| visit(art, seen))
                }))
    }
    visit(document, &mut std::collections::HashSet::new())
}

pub(super) fn skia(mode: BlendMode) -> Skia {
    match mode {
        BlendMode::Normal => Skia::SourceOver,
        BlendMode::Multiply => Skia::Multiply,
        BlendMode::Screen => Skia::Screen,
        BlendMode::Overlay => Skia::Overlay,
        BlendMode::Darken => Skia::Darken,
        BlendMode::Lighten => Skia::Lighten,
        BlendMode::ColorDodge => Skia::ColorDodge,
        BlendMode::ColorBurn => Skia::ColorBurn,
        BlendMode::HardLight => Skia::HardLight,
        BlendMode::SoftLight => Skia::SoftLight,
        BlendMode::Difference => Skia::Difference,
        BlendMode::Exclusion => Skia::Exclusion,
        BlendMode::Hue => Skia::Hue,
        BlendMode::Saturation => Skia::Saturation,
        BlendMode::Color => Skia::Color,
        BlendMode::Luminosity => Skia::Luminosity,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        pasteboard, render, render_editable, render_for_export, sample_artwork, DragPreview,
    };
    use document_core::{Document, Layer, Shape, Stencil};
    use std::sync::Arc;

    #[test]
    fn fill_stroke_and_object_blend_at_their_own_levels() {
        let mut doc = Document::default();
        doc.add_rectangle([0.; 2], [50.; 2], [128, 128, 128, 255]);
        doc.add_rectangle([12.; 2], [20.; 2], [128, 128, 128, 255]);
        let top = &mut doc.objects[1];
        top.stroke_width = 4.;
        top.stroke = top.fill;
        top.fill_blend_mode = BlendMode::Multiply;
        top.stroke_blend_mode = BlendMode::Screen;
        let p = render(&doc, 0).unwrap();
        assert_eq!(p.pixel(20, 20).unwrap().red(), 64);
        assert_eq!(p.pixel(13, 20).unwrap().red(), 160); // stroke blends over the fill
        assert_eq!(p.pixel(11, 20).unwrap().red(), 192); // stroke blends over the backdrop

        let top = &mut doc.objects[1];
        top.blend_mode = BlendMode::Multiply;
        top.opacity = 0.5;
        let p = render(&doc, 0).unwrap();
        assert_eq!(p.pixel(20, 20).unwrap().red(), 96);
        assert_eq!(p.pixel(13, 20).unwrap().red(), 112); // internal Screen, then outer Multiply

        let top = &mut doc.objects[1];
        top.blend_mode = BlendMode::Normal;
        top.no_stroke = true;
        top.stencil = Some(Stencil {
            shape: Shape::Rectangle,
            origin: [0.; 2],
            size: [0.8; 2],
            fill_rule: Default::default(),
            alpha: None,
        });
        let p = render(&doc, 0).unwrap();
        assert_eq!(p.pixel(20, 20).unwrap().red(), 96);
        assert_eq!(p.pixel(30, 30).unwrap().red(), 128);
    }

    #[test]
    fn a_blended_layer_composites_as_one_across_page_tiles_selection_and_drag() {
        let mut doc = Document::default();
        doc.layers.push(Layer {
            id: 2,
            blend_mode: BlendMode::Multiply,
            ..Layer::default()
        });
        let gray = [128, 128, 128, 255];
        doc.add_rectangle([-16., 0.], [48., 40.], gray);
        for x in [-10., 6.] {
            doc.add_shape([x, 5.], [42., 25.], gray, Shape::Rectangle, 2);
        }
        let doc = Arc::new(doc);
        let image = render(&doc, 0).unwrap();
        assert_eq!(image.pixel(16, 16).unwrap().red(), 64);
        assert_eq!(image.pixel(38, 16).unwrap().red(), 128); // transparent backdrop
        assert_eq!(render_editable(&doc, 0, 1.).unwrap().0.data(), image.data());
        assert_eq!(render_for_export(&doc, 0, 1.).unwrap().data(), image.data());
        assert_eq!(
            sample_artwork(&doc, 0, [16.5, 16.5], [1.; 2]).unwrap(),
            [64, 64, 64, 255]
        );
        let (crop, at) = crate::render_artwork(&doc.paint_order(), &doc.layers, 0, 1.).unwrap();
        assert_eq!(
            crop.pixel((16 - at[0]) as u32, (16 - at[1]) as u32),
            image.pixel(16, 16)
        );
        let scene = pasteboard::Scene::for_artboard(&doc, 0, [doc.width, doc.height]).unwrap();
        let plan = scene.plan([0, 0], &pasteboard::Damage::all()).unwrap();
        let job = plan.detach(&doc);
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let tile = job.render_until(&cancel).unwrap().pixels.unwrap();
        assert_eq!(tile.pixel(17, 17), image.pixel(16, 16));
        let outside = pasteboard::Scene::new(&doc, 0, [doc.width, doc.height]).unwrap();
        assert_eq!(
            outside.sample(&doc, [-5.5, 16.5]).unwrap(),
            [64, 64, 64, 255]
        );

        let mut after = (*doc).clone();
        after.objects[1].position[0] += 21.;
        assert_eq!(
            DragPreview::default()
                .render(&doc, &after, 0, 1.)
                .unwrap()
                .data(),
            render(&after, 0).unwrap().data()
        );
    }
}
