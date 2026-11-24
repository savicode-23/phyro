use super::{artboard, paint_objects, Artboard, Document, Pixmap, VectorObject};
use std::sync::Arc;
use tiny_skia::IntRect;

/// Cache the unchanged artwork below and above a continuous edit.
#[derive(Default)]
pub struct DragPreview {
    background: Option<Background>,
}

struct Background {
    artwork: StaticArtwork,
    prefix: usize,
    frame: u32,
    size: [u32; 2],
    scale: f32,
    pixels: Pixmap,
    page: Artboard,
    foreground: Option<Foreground>,
}

struct Foreground {
    len: usize,
    // Slot, compositing run and page extent of artwork that can cover a drag.
    visible: Vec<(usize, usize, IntRect)>,
    // The complete scene with the edited object(s) omitted.
    static_scene: Pixmap,
}

enum StaticArtwork {
    Snapshot {
        document: Arc<Document>,
        // Slots of both static spans in layer paint order.
        slots: Vec<usize>,
    },
    Objects(Vec<VectorObject>),
}

mod backdrop;
pub use backdrop::{render_editable, EditBackdrop};
mod patch;
pub use patch::{edit_patch, EditPatch};

impl Background {
    fn matches(&self, objects: &[&VectorObject], edit: Option<(&Arc<Document>, usize)>) -> bool {
        let suffix = self.foreground.as_ref().map_or(0, |fg| fg.len);
        if objects.len() < self.prefix + suffix
            || splits_mask(objects, self.prefix, self.frame)
            || splits_mask(objects, objects.len() - suffix, self.frame)
        {
            return false;
        }
        if let (StaticArtwork::Snapshot { document, .. }, Some((source, slot))) =
            (&self.artwork, edit)
        {
            // Both static spans came from this exact immutable source. Only
            // the supplied replacement can differ in the current paint order.
            if Arc::ptr_eq(document, source)
                && (self.prefix..objects.len() - suffix).contains(&slot)
            {
                return true;
            }
        }
        let current = objects[..self.prefix]
            .iter()
            .chain(&objects[objects.len() - suffix..]);
        match &self.artwork {
            StaticArtwork::Objects(owned) => owned.iter().eq(current.copied()),
            StaticArtwork::Snapshot { document, slots } => slots
                .iter()
                .map(|&slot| &document.objects[slot])
                .eq(current.copied()),
        }
    }
}

fn splits_mask(objects: &[&VectorObject], at: usize, frame: u32) -> bool {
    at > 0 && at < objects.len() && objects[at - 1].shares_clip_mask(objects[at], frame)
}

impl DragPreview {
    pub fn render(
        &mut self,
        before: &Document,
        preview: &Document,
        frame: u32,
        scale: f32,
    ) -> Result<Pixmap, String> {
        self.render_edit(before, preview, None, None, frame, scale)
    }

    /// Reuse unchanged artwork from an immutable drawing the editor already holds.
    /// The preview can be recycled immediately after this call.
    pub fn render_from_snapshot(
        &mut self,
        before: &Arc<Document>,
        preview: &Document,
        frame: u32,
        scale: f32,
    ) -> Result<Pixmap, String> {
        self.render_edit(before, preview, Some(before), None, frame, scale)
    }

    /// Preview a single transform in its original stacking slot. All other
    /// artwork is read directly from the immutable source drawing.
    pub fn render_object_edit(
        &mut self,
        before: &Arc<Document>,
        replacement: &VectorObject,
        frame: u32,
        scale: f32,
    ) -> Result<Pixmap, String> {
        self.render_edit(
            before,
            before,
            Some(before),
            Some(replacement),
            frame,
            scale,
        )
    }

    fn render_edit(
        &mut self,
        before: &Document,
        preview: &Document,
        snapshot: Option<&Arc<Document>>,
        replacement: Option<&VectorObject>,
        frame: u32,
        scale: f32,
    ) -> Result<Pixmap, String> {
        let mut objects = preview.paint_order();
        let edited_slot = replacement.and_then(|replacement| {
            let slot = objects
                .iter()
                .position(|o| o.id == replacement.id && o.layer == replacement.layer)?;
            objects[slot] = replacement;
            Some(slot)
        });
        if preview
            .layers
            .iter()
            .any(|layer| !layer.blend_mode.is_normal())
        {
            // Cached prefixes and suffixes can split an isolated blend layer.
            let page = artboard(preview, frame, scale)?;
            let mut pixels = page.allocate()?;
            crate::clip_groups::paint_with_layers(
                &mut pixels,
                crate::clip_groups::runs(objects.into_iter(), frame),
                &preview.layers,
                (frame, page.transform, [0.; 2]),
                Some(&mut crate::bounds::SymbolBounds::new()),
                crate::RenderPurpose::Editor,
            )?;
            return Ok(pixels);
        }
        let size = [preview.width, preview.height];
        let background = self.background.as_ref().filter(|bg| {
            bg.frame == frame
                && frame < preview.frames
                && bg.size == size
                && bg.scale == scale
                && bg.matches(&objects, snapshot.zip(edited_slot))
        });
        if background.is_none() {
            // For a single replacement the unaffected prefix and suffix still
            // borrow the original objects. General edits compare both orders.
            let collected;
            let original = if edited_slot.is_some() {
                objects.as_slice()
            } else {
                collected = before.paint_order();
                collected.as_slice()
            };
            let mut common = edited_slot.unwrap_or_else(|| {
                original
                    .iter()
                    .zip(&objects)
                    .take_while(|(a, b)| a == b)
                    .count()
            });
            while splits_mask(&objects, common, frame) {
                common -= 1;
            }
            let mut suffix = edited_slot.map_or_else(
                || {
                    original[common..]
                        .iter()
                        .rev()
                        .zip(objects[common..].iter().rev())
                        .take_while(|(a, b)| a == b)
                        .count()
                },
                |slot| objects.len() - slot - 1,
            );
            while splits_mask(&objects, objects.len() - suffix, frame) {
                suffix -= 1;
            }
            let page = artboard(preview, frame, scale)?;
            let mut pixels = page.allocate()?;
            let transform = page.transform;
            paint_objects(&mut pixels, &objects[..common], frame, transform)?;
            let foreground = if suffix > 0 {
                let top = &objects[objects.len() - suffix..];
                let mut static_scene = pixels.clone();
                // Resolve the bounds once. Parked objects cannot affect this page,
                // even if their paint order separates two clipped runs.
                let visible: Vec<_> = crate::clip_groups::runs(top.iter().copied(), frame)
                    .enumerate()
                    .filter_map(|(slot, (run, object))| {
                        visible_bounds(object, frame, &page).map(|bounds| (slot, run, bounds))
                    })
                    .collect();
                let mut symbols = crate::bounds::SymbolBounds::new();
                crate::clip_groups::paint(
                    &mut static_scene,
                    visible.iter().map(|&(slot, run, _)| (run, top[slot])),
                    frame,
                    transform,
                    [0.; 2],
                    Some(&mut symbols),
                    crate::RenderPurpose::Editor,
                )?;
                Some(Foreground {
                    len: suffix,
                    visible,
                    static_scene,
                })
            } else {
                None
            };
            let unchanged = original[..common]
                .iter()
                .chain(&original[original.len() - suffix..])
                .copied();
            let artwork = match snapshot {
                Some(snapshot) if common + suffix > 0 => StaticArtwork::Snapshot {
                    document: Arc::clone(snapshot),
                    slots: unchanged
                        .map(|o| {
                            snapshot
                                .objects
                                .element_offset(o)
                                .expect("paint order borrows the original drawing")
                        })
                        .collect(),
                },
                _ => StaticArtwork::Objects(unchanged.cloned().collect()),
            };
            self.background = Some(Background {
                artwork,
                prefix: common,
                frame,
                size,
                scale,
                pixels,
                page,
                foreground,
            });
        }
        let bg = self.background.as_ref().expect("prepared above");
        let pending = &objects[bg.prefix..];
        let Some(fg) = &bg.foreground else {
            let mut pixels = bg.pixels.clone();
            paint_objects(&mut pixels, pending, frame, bg.page.transform)?;
            return Ok(pixels);
        };
        let (edited, foreground) = pending.split_at(pending.len() - fg.len);
        let area = edited
            .iter()
            .filter_map(|o| visible_bounds(o, frame, &bg.page))
            .reduce(enclosing);
        let nearby: Vec<_> = fg
            .visible
            .iter()
            .filter_map(|&(slot, run, bounds)| {
                area.and_then(|a| bounds.intersect(&a))
                    .map(|_| (run, foreground[slot]))
            })
            .collect();
        if nearby.is_empty() {
            // The edit can be painted directly onto the static scene. This also
            // evaluates any nested references that have moved entirely off-board.
            let mut pixels = fg.static_scene.clone();
            paint_objects(&mut pixels, edited, frame, bg.page.transform)?;
            return Ok(pixels);
        }
        let mut pixels = bg.pixels.clone();
        paint_objects(&mut pixels, edited, frame, bg.page.transform)?;
        let mut symbols = crate::bounds::SymbolBounds::new();
        crate::clip_groups::paint(
            &mut pixels,
            nearby.iter().copied(),
            frame,
            bg.page.transform,
            [0.; 2],
            Some(&mut symbols),
            crate::RenderPurpose::Editor,
        )?;
        if nearby.len() < fg.visible.len() {
            restore_outside(&mut pixels, &fg.static_scene, area.unwrap());
        }
        Ok(pixels)
    }
}

/// Keep rendering in the original pixel coordinates. Repainting an intersecting
/// translucent object can change pixels outside the edit, so restore those bytes.
fn restore_outside(target: &mut Pixmap, source: &Pixmap, area: IntRect) {
    let stride = target.width() as usize;
    let first = area.top() as usize * stride;
    let end = area.bottom() as usize * stride;
    let src = source.pixels();
    let dst = target.pixels_mut();
    dst[..first].copy_from_slice(&src[..first]);
    dst[end..].copy_from_slice(&src[end..]);
    for (dst, src) in dst[first..end]
        .chunks_exact_mut(stride)
        .zip(src[first..end].chunks_exact(stride))
    {
        let left = area.left() as usize;
        let right = area.right() as usize;
        dst[..left].copy_from_slice(&src[..left]);
        dst[right..].copy_from_slice(&src[right..]);
    }
}

fn enclosing(a: IntRect, b: IntRect) -> IntRect {
    IntRect::from_ltrb(
        a.left().min(b.left()),
        a.top().min(b.top()),
        a.right().max(b.right()),
        a.bottom().max(b.bottom()),
    )
    .unwrap()
}

fn visible_bounds(o: &VectorObject, frame: u32, page: &Artboard) -> Option<IntRect> {
    if !o.visible_at(frame) || o.opacity_at(frame) <= 0. {
        return None;
    }
    super::bounds::pixel_bounds(o, frame, page.transform)
        .unwrap_or(page.bounds)
        .intersect(&page.bounds)
}

#[cfg(test)]
mod tests;
