use std::sync::{Arc, Weak};

use super::{
    artboard, enclosing, paint_objects, visible_bounds, Document, EditPatch, IntRect, Pixmap,
    VectorObject,
};

/// A fully painted artboard just before its topmost object was drawn.
#[derive(Debug)]
pub struct EditBackdrop {
    document: Weak<Document>,
    frame: u32,
    scale: f32,
    slot: usize,
    pixels: Pixmap,
}

fn area(rect: IntRect) -> u64 {
    u64::from(rect.width()) * u64::from(rect.height())
}

/// Retain one extra RGBA surface for a small top object above expensive artwork.
/// A full paint of any other page uses the ordinary rendering path.
pub fn render_editable(
    document: &Arc<Document>,
    frame: u32,
    scale: f32,
) -> Result<(Pixmap, Option<EditBackdrop>), String> {
    let page = artboard(document, frame, scale)?;
    let mut pixels = page.allocate()?;
    let objects = document.paint_order();
    let page_area = area(page.bounds);
    let expensive_background = |o: &VectorObject| {
        (o.is_instance()
            || o.image.is_some()
            || o.gradient.is_some()
            || o.stroke_gradient.is_some()
            || o.stencil.is_some()
            || o.stroke_width_at(frame) > 0. && o.opacity_at(frame) < 1.)
            && visible_bounds(o, frame, &page).is_some_and(|r| area(r) * 4 >= page_area)
    };
    // Keep at most 16 MiB. Simple backgrounds are cheap to repaint in edit_patch.
    let candidate = objects.split_last().filter(|(top, background)| {
        page_area <= 4 * 1024 * 1024
            && objects.len() >= 512
            // The topmost object's appearance can depend on lower members of
            // its clipped group, even after a later transform brings them together.
            && top.clip_source.is_none()
            && visible_bounds(top, frame, &page).is_some_and(|r| area(r) * 24 < page_area)
            && background
                .iter()
                .filter(|o| expensive_background(o))
                .take(4)
                .count()
                == 4
    });
    let Some((&top, background)) = candidate else {
        paint_objects(&mut pixels, &objects, frame, page.transform)?;
        return Ok((pixels, None));
    };
    paint_objects(&mut pixels, background, frame, page.transform)?;
    let slot = document
        .objects
        .iter()
        .position(|o| std::ptr::eq(o, top))
        .expect("paint order borrows the document");
    let backdrop = EditBackdrop {
        document: Arc::downgrade(document),
        frame,
        scale,
        slot,
        pixels: pixels.clone(),
    };
    paint_objects(&mut pixels, &[top], frame, page.transform)?;
    Ok((pixels, Some(backdrop)))
}

impl EditBackdrop {
    /// Patch an image only if its exact source snapshot is still displayed
    /// and everything behind the same top object has kept its paint order and appearance.
    pub fn patch(
        &mut self,
        before: &Arc<Document>,
        after: &Arc<Document>,
        frame: u32,
        scale: f32,
    ) -> Result<Option<EditPatch>, String> {
        let slot = self.slot;
        if self.frame != frame
            || self.scale != scale
            || !self.document.ptr_eq(&Arc::downgrade(before))
            || (before.width, before.height, before.frames)
                != (after.width, after.height, after.frames)
            || before.layers != after.layers
            || before.skeleton != after.skeleton
            || before.objects.len() != after.objects.len()
            || before.objects[slot].id != after.objects[slot].id
            || before.objects[slot].layer != after.objects[slot].layer
            || after.objects[slot].clip_source.is_some()
            || before.objects[..slot] != after.objects[..slot]
            || before.objects[slot + 1..] != after.objects[slot + 1..]
        {
            return Ok(None);
        }
        let page = artboard(after, frame, scale)?;
        let bounds = |object: &VectorObject| visible_bounds(object, frame, &page);
        let area = bounds(&before.objects[slot])
            .into_iter()
            .chain(bounds(&after.objects[slot]))
            .reduce(enclosing);
        let patch = if let Some(area) = area {
            // Painting at the original device coordinates and dimensions keeps
            // antialiasing, masks and gradient sampling identical to a full paint.
            let mut surface = self.pixels.clone();
            paint_objects(&mut surface, &[&after.objects[slot]], frame, page.transform)?;
            EditPatch::Pixels {
                origin: [area.left() as usize, area.top() as usize],
                pixels: surface
                    .clone_rect(area)
                    .ok_or("Could not allocate the preview region.")?,
            }
        } else {
            EditPatch::Unchanged
        };
        self.document = Arc::downgrade(after);
        Ok(Some(patch))
    }
}

#[cfg(test)]
mod tests;
