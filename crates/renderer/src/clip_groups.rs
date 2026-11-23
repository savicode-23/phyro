use super::{appearance, bounds::SymbolBounds, draw_member, Pixmap, Transform, VectorObject};
use std::collections::HashMap;
use tiny_skia::PixmapPaint;

/// Map a member's document slot to the first slot of its compositing run.
/// Unlisted slots stand alone. Layer-local ordering can differ from the order
/// of a document's backing vector.
pub(super) fn shared_slots(doc: &document_core::Document, frame: u32) -> HashMap<usize, usize> {
    let mut shared = HashMap::new();
    if !doc
        .objects
        .iter()
        .any(|o| o.group.is_some() && o.clip_source.is_some())
    {
        return shared;
    }
    let mut previous: HashMap<u64, (usize, usize)> = HashMap::new();
    for (slot, object) in doc.objects.iter().enumerate() {
        if object.group.is_none() || object.clip_source.is_none() {
            previous.remove(&object.layer);
            continue;
        }
        let run = previous
            .get(&object.layer)
            .filter(|(prior, _)| doc.objects[*prior].shares_clip_mask(object, frame))
            .map_or(slot, |(_, run)| *run);
        if run != slot {
            shared.insert(slot, run);
        }
        previous.insert(object.layer, (slot, run));
    }
    shared
}

/// Label each run before visibility or damage filtering. An unrelated object
/// in the stacking order separates runs even if it lies outside a tile.
pub(super) fn runs<'a>(
    objects: impl Iterator<Item = &'a VectorObject>,
    frame: u32,
) -> impl Iterator<Item = (usize, &'a VectorObject)> {
    let mut previous: Option<&VectorObject> = None;
    let mut run = 0;
    objects.enumerate().map(move |(index, object)| {
        if object.clip_source.is_none()
            || !previous.is_some_and(|p| p.shares_clip_mask(object, frame))
        {
            run = index;
        }
        previous = Some(object);
        (run, object)
    })
}

pub(super) fn paint<'a>(
    surface: &mut Pixmap,
    objects: impl Iterator<Item = (usize, &'a VectorObject)>,
    frame: u32,
    transform: Transform,
    page_origin: [f32; 2],
    mut symbols: Option<&mut SymbolBounds>,
    purpose: super::RenderPurpose,
) -> Result<(), String> {
    let mut objects = objects
        .filter(|(_, o)| purpose == super::RenderPurpose::Editor || !o.reference_only)
        .peekable();
    let mut scratch = None;
    let mut group: Option<Pixmap> = None;
    let space = (frame, transform, page_origin);
    while let Some((run, first)) = objects.next() {
        if !objects.peek().is_some_and(|(next, _)| *next == run) {
            draw_member(
                surface,
                first,
                space,
                &mut scratch,
                symbols.as_deref_mut(),
                true,
                purpose,
            )?;
            continue;
        }
        let group = match &mut group {
            Some(pixels) => {
                pixels.data_mut().fill(0);
                pixels
            }
            None => group.insert(
                Pixmap::new(surface.width(), surface.height())
                    .ok_or("Could not allocate a clipped group compositing layer.")?,
            ),
        };
        draw_member(
            group,
            first,
            space,
            &mut scratch,
            symbols.as_deref_mut(),
            false,
            purpose,
        )?;
        while let Some((_, member)) = objects.next_if(|(next, _)| *next == run) {
            draw_member(
                group,
                member,
                space,
                &mut scratch,
                symbols.as_deref_mut(),
                false,
                purpose,
            )?;
        }
        let mask = appearance::mask(surface, first, frame, transform)?;
        surface.draw_pixmap(
            0,
            0,
            group.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            mask.as_ref(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        pasteboard, render, render_artwork, render_editable, sample_artwork, DragPreview, EditPatch,
    };
    use document_core::Document;
    use std::sync::Arc;

    fn drawing(interleaved: bool) -> Document {
        let mut doc = Document::default();
        let first = doc.add_rectangle([-56., 8.], [108., 58.], [207, 38, 15, 173]);
        doc.objects[0].group = Some(4);
        if interleaved {
            // Between the members in z order, outside the sampled portion. Tile
            // and damage culling must still remember that it separates the runs.
            doc.add_rectangle([95., 95.], [5.; 2], [37, 242, 98, 220]);
        }
        let second = doc.add_rectangle([-37., 14.], [103., 44.], [39, 72, 220, 149]);
        doc.objects.last_mut().unwrap().group = Some(4);
        doc.objects.last_mut().unwrap().opacity = 0.84;
        let upper = doc.add_rectangle([-60., 2.], [119., 70.], [0; 4]);
        doc.set_clip(&[first, second, upper], 0, Some([80, 160]))
            .unwrap();
        doc
    }

    #[test]
    fn a_shared_alpha_mask_composites_overlapping_translucent_members_once() {
        let doc = Arc::new(drawing(false));
        let mut plain = (*doc).clone();
        for o in &mut plain.objects {
            o.stencil = None;
            o.clip_source = None;
        }
        let unmasked = render(&plain, 0).unwrap();
        let mask = appearance::mask(&unmasked, &doc.objects[0], 0, Transform::identity())
            .unwrap()
            .unwrap();
        let mut expected = Pixmap::new(doc.width, doc.height).unwrap();
        expected.draw_pixmap(
            0,
            0,
            unmasked.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            Some(&mask),
        );
        let image = render(&doc, 0).unwrap();
        assert!(image.data() == expected.data());
        assert!(render_editable(&doc, 0, 1.).unwrap().0.data() == image.data());
        let (crop, [x, y]) = render_artwork(&doc.paint_order(), 0, 1).unwrap();
        let pixel = image.pixel(18, 30).unwrap();
        assert_eq!(crop.pixel((18 - x) as u32, (30 - y) as u32).unwrap(), pixel);
        assert_eq!(
            sample_artwork(&doc, 0, [18.5, 30.5], [1.; 2]).unwrap(),
            [pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()]
        );
        let scene = pasteboard::Scene::new(&doc, 0, [doc.width, doc.height]).unwrap();
        let p = crop.pixel((-20 - x) as u32, (30 - y) as u32).unwrap();
        let picked = scene.sample(&doc, [-19.5, 30.5]).unwrap();
        assert_eq!(picked, [p.red(), p.green(), p.blue(), p.alpha()]);
        let tile = scene
            .plan([-1, 0], &pasteboard::Damage::all())
            .unwrap()
            .render(&doc)
            .unwrap();
        assert_eq!(tile.pixels.unwrap().pixel(237, 31).unwrap(), p);

        let mut after = (*doc).clone();
        after.objects[1].fill = [52, 122, 33, 182];
        let mut preview = DragPreview::default();
        assert!(
            preview.render(&doc, &after, 0, 1.).unwrap().data()
                == render(&after, 0).unwrap().data()
        );
    }

    #[test]
    fn animated_members_can_separate_and_share_the_mask_again() {
        let mut doc = drawing(false);
        doc.frames = 5;
        let base = doc.objects[1].position;
        doc.objects[1]
            .keys
            .extend([(0, base), (2, [base[0] + 8., base[1]]), (4, base)]);
        for (frame, together) in [(0, true), (2, false), (4, true)] {
            let ordered = doc.paint_order();
            let labels: Vec<_> = runs(ordered.into_iter(), frame).map(|(r, _)| r).collect();
            assert_eq!(labels[0] == labels[1], together);
            let image = render(&doc, frame).unwrap();
            let mut separate = doc.clone();
            for obj in &mut separate.objects {
                obj.group = None;
            }
            assert_eq!(
                image.pixel(17, 30) != render(&separate, frame).unwrap().pixel(17, 30),
                together
            );
        }
    }

    #[test]
    fn interleaved_shapes_keep_stacking_and_tile_and_damage_culling_preserve_run_boundaries() {
        let doc = drawing(true);
        let page = render(&doc, 0).unwrap();
        let mut independent = doc.clone();
        for o in &mut independent.objects {
            o.group = None;
        }
        assert!(page.data() == render(&independent, 0).unwrap().data());
        let scene = pasteboard::Scene::new(&doc, 0, [doc.width, doc.height]).unwrap();
        assert_eq!(
            scene.sample(&doc, [-20., 30.]).unwrap(),
            sample_artwork(&doc, 0, [-20., 30.], [1.; 2]).unwrap()
        );
        let mut after = doc.clone();
        after.objects[0].fill = [55, 66, 23, 123];
        let edited = render(&after, 0).unwrap();
        if let EditPatch::Pixels { origin, pixels } =
            crate::edit_patch(&doc, &after, 0, 1.).unwrap()
        {
            let actual = pixels
                .pixel(18 - origin[0] as u32, 30 - origin[1] as u32)
                .unwrap();
            assert_eq!(actual, edited.pixel(18, 30).unwrap());
        } else {
            panic!("Expected localized repaint");
        }
        after.objects[1].position = [12., 23.];
        after.objects[1].size = [40.; 2];
        let mut reference = after.clone();
        for o in &mut reference.objects {
            o.group = None;
        }
        assert!(render(&after, 0).unwrap().data() == render(&reference, 0).unwrap().data());
    }
}
