use std::sync::{mpsc, Arc};

use super::display_filter::ARTWORK_TEXTURE;
use document_core::Document;
use eframe::egui::{self, ColorImage};

use super::{Canvas, FrameTexture};

#[cfg(test)]
mod held_tests;
pub(super) mod placement;

impl FrameTexture {
    fn keep_sample(image: &Arc<ColorImage>) -> Option<Arc<ColorImage>> {
        (image.pixels.len() <= 4 * 1024 * 1024).then(|| image.clone())
    }

    fn upload(&mut self, image: ColorImage) {
        let image = Arc::new(image);
        self.sample_pixels = Self::keep_sample(&image);
        self.handle.set(image, ARTWORK_TEXTURE);
    }

    pub(super) fn sample_texel(
        &self,
        doc: &Document,
        frame: u32,
        point: [f32; 2],
    ) -> Option<[u8; 4]> {
        let image = self.sample_pixels.as_ref()?;
        if !(0.0..doc.width as f32).contains(&point[0])
            || !(0.0..doc.height as f32).contains(&point[1])
            || self.pending_split.is_some()
            || !self.matches(doc, frame)
        {
            return None;
        }
        let [w, h] = image.size;
        let x = ((point[0] / doc.width as f32 * w as f32) as usize).min(w - 1);
        let y = ((point[1] / doc.height as f32 * h as f32) as usize).min(h - 1);
        Some(image[(x, y)].to_array())
    }

    // Node controls and snapping must see a newly inserted node immediately.
    // document continues to describe the pixels in handle until the worker finishes.
    pub(super) fn authored(&self) -> &Arc<Document> {
        self.pending_split.as_ref().unwrap_or(&self.document)
    }

    pub(super) fn can_refine_split(
        &self,
        doc: &Document,
        frame: u32,
        object: &document_core::VectorObject,
    ) -> bool {
        // Splitting a plain filled cubic preserves its silhouette; subdivision
        // can still change edge antialiasing. Strokes and effects can change the
        // appearance of the new join, so paint those synchronously.
        self.failed_refinement.is_none()
            && self.frame == frame
            && self.scale > 2.
            && matches!(
                object.shape,
                document_core::Shape::Path { closed: true, .. }
                    | document_core::Shape::Compound { .. }
            )
            && object.shape.contours().iter().map(Vec::len).sum::<usize>() >= 2048
            && (object.no_stroke || object.stroke_width == 0.)
            && object.effects.is_empty()
            && (object.no_fill || object.gradient.is_none())
            && object.stencil.is_none()
            && !object.is_instance()
            && object.rigid_bone.is_none()
            && object.motion.is_none()
            && object.last_keyframe().is_none()
            && self.document.as_ref() == doc
    }

    fn finish_refinement(&mut self, result: Result<PageImage, String>, scale: f32) {
        match result {
            Ok((image, backdrop)) => {
                self.backdrop = backdrop;
                self.upload(image);
                self.scale = scale;
                if let Some(target) = self.pending_split.take() {
                    self.document = target;
                    self.frame_independent = Default::default();
                }
                self.failed_refinement = None;
            }
            Err(error) => {
                self.failed_refinement = Some(FailedRefinement {
                    scale,
                    message: format!("Full-detail preview unavailable. {error}"),
                });
            }
        }
    }

    pub(super) fn matches(&self, document: &Document, frame: u32) -> bool {
        frame < document.frames
            && (self.frame == frame
                || *self
                    .frame_independent
                    .get_or_init(|| self.document.frame_independent()))
            && (std::ptr::eq(self.document.as_ref(), document)
                || self.document.as_ref() == document)
    }

    fn patch(&mut self, document: &Arc<Document>, frame: u32) -> Result<bool, String> {
        if self.frame != frame {
            return Ok(false);
        }
        if Arc::ptr_eq(&self.document, document) || self.document.as_ref() == document.as_ref() {
            return Ok(true);
        }
        let prepared = if let Some(backdrop) = self.backdrop.as_mut() {
            backdrop.patch(&self.document, document, frame, self.scale)?
        } else {
            None
        };
        let patch = match prepared {
            Some(patch) => patch,
            None => {
                self.backdrop = None;
                renderer::edit_patch(&self.document, document, frame, self.scale)?
            }
        };
        match patch {
            renderer::EditPatch::FullRender => return Ok(false),
            renderer::EditPatch::Unchanged => {}
            renderer::EditPatch::Pixels { origin, pixels } => {
                let patch = color_image(pixels);
                if let Some(full) = self.sample_pixels.as_mut().and_then(Arc::get_mut) {
                    for (y, row) in patch.pixels.chunks_exact(patch.size[0]).enumerate() {
                        let start = (y + origin[1]) * full.size[0] + origin[0];
                        full.pixels[start..start + row.len()].copy_from_slice(row);
                    }
                } else {
                    // An outstanding upload owns this buffer. Never copy the whole page for a patch.
                    self.sample_pixels = None;
                }
                self.handle.set_partial(origin, patch, ARTWORK_TEXTURE);
            }
        }
        self.document = Arc::clone(document);
        self.frame_independent = Default::default();
        Ok(true)
    }
}

pub(super) struct FailedRefinement {
    scale: f32,
    message: String,
}

pub(super) type PageImage = (ColorImage, Option<renderer::EditBackdrop>);

pub(super) struct Refinement<T = PageImage> {
    document: Arc<Document>,
    frame: u32,
    scale: f32,
    result: mpsc::Receiver<Result<T, String>>,
}

fn render_image(document: &Arc<Document>, frame: u32, scale: f32) -> Result<PageImage, String> {
    let (pixels, backdrop) = if dense_artboard(document) {
        renderer::render_editable(document, frame, scale)?
    } else {
        (renderer::render_scaled(document, frame, scale)?, None)
    };
    Ok((color_image(pixels), backdrop))
}

// Retry a transient failure once on the rendering thread. Convert a renderer
// panic into a completion so the idle viewport is woken in the usual way.
fn refined_image<T>(
    render: impl Fn() -> Result<T, String> + std::panic::RefUnwindSafe,
) -> Result<T, String> {
    std::panic::catch_unwind(|| render().or_else(|_| render()))
        .unwrap_or_else(|_| Err("The preview renderer stopped unexpectedly.".into()))
}

fn color_image(pixels: renderer::Pixmap) -> ColorImage {
    ColorImage::from_rgba_premultiplied(
        [pixels.width() as usize, pixels.height() as usize],
        pixels.data(),
    )
}

pub(super) fn dense_artboard(doc: &Document) -> bool {
    // A large pasteboard can contain just a handful of on-screen shapes. Their
    // full redraw is cheap; keep the reduced-density compositor for those drags.
    // The authored origin is only a cost estimate. Exact paint bounds determine
    // which pixels the renderer can reuse.
    const MIN_OBJECTS: usize = 512;
    doc.objects.len() >= MIN_OBJECTS
        && doc
            .objects
            .iter()
            .filter(|o| {
                (0.0..doc.width as f32).contains(&o.position[0])
                    && (0.0..doc.height as f32).contains(&o.position[1])
            })
            .nth(MIN_OBJECTS - 1)
            .is_some()
}

// The authored texture remains usable for pasteboard tiles and for cancellation.
// Only the page image changes while a single transformed object stays inside it.
pub(super) struct PageDrag {
    source: Arc<Document>,
    object: document_core::VectorObject,
    frame: u32,
    scale: f32,
    handle: egui::TextureHandle,
    sample_pixels: Option<Arc<ColorImage>>,
}

impl Refinement {
    pub(super) fn page(
        ctx: &egui::Context,
        document: &Arc<Document>,
        frame: u32,
        scale: f32,
    ) -> Option<Self> {
        Self::spawn(ctx, Arc::clone(document), frame, scale, render_image)
    }

    fn start(
        ctx: &egui::Context,
        texture: &FrameTexture,
        scale: f32,
        render: fn(&Arc<Document>, u32, f32) -> Result<PageImage, String>,
    ) -> Option<Self> {
        let document = Arc::clone(texture.authored());
        let frame = texture.frame;
        Self::spawn(ctx, document, frame, scale, render)
    }
}

pub(super) type SettledArtwork = (PageImage, Option<renderer::MoveSprite>);

impl Refinement<SettledArtwork> {
    pub(super) fn settled_transform(
        ctx: &egui::Context,
        document: &Arc<Document>,
        frame: u32,
        scale: f32,
        refresh: Option<(u64, f32)>,
    ) -> Option<Self> {
        Self::spawn(
            ctx,
            Arc::clone(document),
            frame,
            scale,
            move |doc, tick, density| {
                let image = render_image(doc, tick, density)?;
                let sprite = refresh.and_then(|(id, requested_scale)| {
                    let object = doc.objects.iter().find(|o| o.id == id)?;
                    renderer::move_sprite(object, tick, requested_scale)
                });
                Ok((image, sprite))
            },
        )
    }
}

impl<T: Send + 'static> Refinement<T> {
    pub(super) fn is_target(&self, document: &Arc<Document>, frame: u32, scale: f32) -> bool {
        Arc::ptr_eq(&self.document, document) && self.frame == frame && self.scale == scale
    }

    pub(super) fn completed(&self) -> Option<Result<T, String>> {
        match self.result.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err("The preview renderer stopped unexpectedly.".into()))
            }
        }
    }

    fn spawn(
        ctx: &egui::Context,
        document: Arc<Document>,
        frame: u32,
        scale: f32,
        render: impl Fn(&Arc<Document>, u32, f32) -> Result<T, String>
            + Send
            + std::panic::RefUnwindSafe
            + 'static,
    ) -> Option<Self> {
        let ctx = ctx.clone();
        let viewport = ctx.viewport_id();
        let target = document.clone();
        let (send, result) = mpsc::channel();
        std::thread::Builder::new()
            .name("phyro-page-render".into())
            .spawn(move || {
                if send
                    .send(refined_image(|| render(&document, frame, scale)))
                    .is_ok()
                {
                    ctx.request_repaint_of(viewport);
                }
            })
            .ok()?;
        Some(Self {
            document: target,
            frame,
            scale,
            result,
        })
    }
}

impl Canvas {
    pub(super) fn page_transform_target(
        &self,
        gesture: &super::Gesture,
        frame: u32,
        view: super::View,
        ppp: f32,
        viewport: egui::Rect,
    ) -> Option<(&document_core::VectorObject, f32)> {
        if !matches!(
            gesture,
            super::Gesture::Move { .. } | super::Gesture::Resize { .. }
        ) {
            return None;
        }
        let id = gesture.single_transform()?;
        let source = self.drag_targets.as_ref()?.original();
        let texture = self.texture.as_ref()?;
        let size = [source.width, source.height];
        let physical = view.scale * ppp;
        if source.objects.len() < 512
            || physical * size[0].max(size[1]) as f32 > 2048.
            || texture.frame != frame
            || texture.pending_split.is_some()
            || !Arc::ptr_eq(source, texture.authored())
        {
            return None;
        }
        // Exterior tiles retain the authored image. Allow this shortcut only when
        // the old and new painted extents are safely inside the lowest-resolution grid.
        let outside_density = self
            .pasteboard
            .preview_density(view, viewport, size, physical);
        let density = view
            .scale
            .min(physical)
            .min(texture.scale)
            .min(outside_density);
        let object = source.objects.iter().find(|o| o.id == id)?;
        let known = self.page_drag.as_ref().is_some_and(|p| {
            p.frame == frame && p.object.id == id && Arc::ptr_eq(&p.source, source)
        });
        if super::is_editable(source, object.layer)
            && !(object.group.is_some() && object.clip_source.is_some())
            && (known || !dense_artboard(source))
            && renderer::wholly_on_artboard(object, frame, size, density)
        {
            Some((object, density))
        } else {
            None
        }
    }

    pub(super) fn object_page_texture(
        &mut self,
        ctx: &egui::Context,
        object: &document_core::VectorObject,
        frame: u32,
        scale: f32,
    ) -> Result<egui::TextureId, String> {
        let source = self
            .texture
            .as_ref()
            .expect("object preview has an authored texture")
            .authored();
        if let Some(previous) = self.page_drag.as_ref().filter(|p| {
            Arc::ptr_eq(&p.source, source)
                && p.frame == frame
                && p.scale == scale
                && p.object == *object
        }) {
            return Ok(previous.handle.id());
        }
        let renderer = self.drag_preview.get_or_insert_with(Default::default);
        let image = Arc::new(color_image(
            renderer.render_object_edit(source, object, frame, scale)?,
        ));
        let sample_pixels = FrameTexture::keep_sample(&image);
        let handle = if let Some(mut previous) = self.page_drag.take() {
            previous.handle.set(image, ARTWORK_TEXTURE);
            previous.handle
        } else {
            ctx.load_texture("transformed artboard", image, ARTWORK_TEXTURE)
        };
        let id = handle.id();
        self.page_drag = Some(PageDrag {
            source: source.clone(),
            object: object.clone(),
            frame,
            scale,
            handle,
            sample_pixels,
        });
        Ok(id)
    }

    pub(super) fn page_commit_target(&self, gesture: &super::Gesture, frame: u32) -> Option<u64> {
        let page = self.page_drag.as_ref()?;
        let id = gesture.single_transform()?;
        (id == page.object.id
            && page.frame == frame
            && Arc::ptr_eq(&page.source, self.drag_targets.as_ref()?.original()))
        .then_some(id)
    }

    pub(super) fn commit_object_page(
        &mut self,
        ctx: &egui::Context,
        authored: &Arc<Document>,
        id: u64,
        scale: f32,
    ) -> Result<(), String> {
        let Some(mut page) = self.page_drag.take() else {
            return Ok(());
        };
        let Some(mut renderer) = self.drag_preview.take() else {
            return Ok(());
        };
        let Some(object) = authored.objects.iter().find(|o| o.id == id) else {
            return Ok(());
        };
        if object != &page.object {
            let pixels =
                renderer.render_object_edit(&page.source, object, page.frame, page.scale)?;
            let image = Arc::new(color_image(pixels));
            page.sample_pixels = FrameTexture::keep_sample(&image);
            page.handle.set(image, ARTWORK_TEXTURE);
        }
        // The committed drawing can share its immutable snapshot with history
        // and the zoom worker. Its last preview is already on the GPU.
        self.texture = Some(FrameTexture {
            document: Arc::clone(authored),
            frame: page.frame,
            scale: page.scale,
            handle: page.handle,
            sample_pixels: page.sample_pixels,
            frame_independent: Default::default(),
            pending_split: None,
            failed_refinement: None,
            backdrop: None,
        });
        if page.scale != scale {
            self.sharpen_at = Some(ctx.input(|i| i.time));
        }
        Ok(())
    }

    pub(crate) fn refinement_warning(&self) -> Option<&str> {
        self.texture
            .as_ref()
            .and_then(|t| t.failed_refinement.as_ref())
            .map(|f| f.message.as_str())
            .or_else(|| self.detail.warning())
            .or_else(|| self.pasteboard.warning())
    }

    #[cfg(test)]
    pub(super) fn frame_texture(
        &mut self,
        ctx: &egui::Context,
        document: &Document,
        frame: u32,
        scale: f32,
    ) -> Result<egui::TextureId, String> {
        self.artwork_texture(ctx, &Arc::new(document.clone()), frame, scale, None)
    }

    pub(super) fn artwork_texture(
        &mut self,
        ctx: &egui::Context,
        original: &Arc<Document>,
        frame: u32,
        scale: f32,
        preview: Option<&Arc<Document>>,
    ) -> Result<egui::TextureId, String> {
        let document = preview.unwrap_or(original);
        let settling_drag = preview.is_none()
            && (self.patching_drag || self.drag_preview.is_some() && self.page_drag.is_none());
        if preview.is_some() {
            self.page_drag = None;
        }
        if let Some(cached) = self.texture.as_mut() {
            // While recovering at a new density, keep the inserted node available
            // to controls and snapping until that worker replaces the old pixels.
            let split_changed = cached.pending_split.as_ref().is_some_and(|target| {
                preview.is_some()
                    || cached.frame != frame
                    || cached.scale != scale && cached.failed_refinement.is_none()
                    || (!Arc::ptr_eq(target, document) && target.as_ref() != document.as_ref())
            });
            if split_changed {
                cached.pending_split = None;
            }
            if cached.failed_refinement.is_some()
                && (preview.is_some()
                    || split_changed
                    || cached.pending_split.is_none() && !cached.matches(document, frame))
            {
                cached.failed_refinement = None;
                ctx.request_repaint();
            }
        }
        // For dense scenes a local update avoids rasterizing thousands of
        // unchanged objects on the first pointer movement. A larger edit keeps
        // the regular drag compositor for the rest of the same gesture.
        if self.drag_preview.is_none()
            && (self.patching_drag || preview.is_some() && dense_artboard(document))
        {
            if let Some(cached) = self.texture.as_mut() {
                if cached.patch(document, frame)? {
                    self.patching_drag = preview.is_some();
                    if self.patching_drag {
                        return Ok(cached.handle.id());
                    }
                    if cached.scale != scale {
                        self.sharpen_at = Some(ctx.input(|i| i.time));
                    }
                }
            }
        }
        self.patching_drag = false;
        if preview.is_none() {
            self.finish_drag_texture(ctx, document, frame, scale)?;
        }
        // Keep just one job in flight. A completion wakes the viewport, which can
        // start the latest request if the artist changed view in the meantime.
        let mut completed = None;
        if let Some(job) = self.refinement.take() {
            match job.completed() {
                Some(result) => completed = Some((job, result)),
                None => self.refinement = Some(job),
            }
        }
        let settled = self
            .sharpen_at
            .is_some_and(|time| time <= ctx.input(|i| i.time));
        if let Some(cached) = self
            .texture
            .as_mut()
            .filter(|cached| cached.pending_split.is_some() || cached.matches(document, frame))
        {
            if cached
                .failed_refinement
                .as_ref()
                .is_some_and(|f| f.scale == scale)
            {
                return Ok(cached.handle.id());
            }
            let recovering = cached.failed_refinement.is_some();
            if cached.scale == scale && cached.pending_split.is_none() {
                if settling_drag
                    && self.refinement.is_none()
                    && cached.backdrop.is_none()
                    && !Arc::ptr_eq(&cached.document, document)
                {
                    // A committed pose can equal the last staged pose byte for
                    // byte. Keep the same texture and share the committed drawing.
                    cached.document = Arc::clone(document);
                }
                if recovering {
                    cached.failed_refinement = None;
                    ctx.request_repaint();
                }
                if settled {
                    self.sharpen_at = None;
                }
                return Ok(cached.handle.id());
            }
            if cached.pending_split.is_some() || settled || recovering {
                let result = if let Some((_, result)) = completed.filter(|(job, _)| {
                    Arc::ptr_eq(&job.document, cached.authored())
                        && job.frame == cached.frame
                        && job.scale == scale
                }) {
                    result
                } else {
                    if self.refinement.is_none() {
                        self.refinement = Refinement::start(ctx, cached, scale, render_image);
                    }
                    if self.refinement.is_some() {
                        return Ok(cached.handle.id());
                    }
                    // The worker could not start. Try on this thread.
                    let target = cached.authored();
                    let tick = cached.frame;
                    refined_image(|| render_image(target, tick, scale))
                };
                cached.finish_refinement(result, scale);
                if recovering || cached.failed_refinement.is_some() {
                    ctx.request_repaint();
                }
                if settled {
                    self.sharpen_at = None;
                }
                return Ok(cached.handle.id());
            }
        }
        // First display, content edits and timeline changes are visible in this frame.
        // The cached image already has this frame and density. Local committed
        // edits, including inserted or deleted artwork and Undo/Redo, can update it in place.
        if preview.is_none() {
            if let Some(cached) = self
                .texture
                .as_mut()
                .filter(|c| c.frame == frame && c.scale == scale)
            {
                if dense_artboard(document) && cached.patch(document, frame)? {
                    return Ok(cached.handle.id());
                }
            }
        }
        let (image, backdrop) = match preview {
            Some(_) => {
                let scene = self.drag_preview.get_or_insert_with(Default::default);
                let snapped = self.drag_targets.as_ref().map(|drag| drag.original());
                let single = snapped.and_then(|source| {
                    let id = self.gesture.as_ref()?.single_transform()?;
                    document
                        .objects
                        .iter()
                        .find(|o| o.id == id)
                        .map(|o| (source, o))
                });
                let previous =
                    snapped.or_else(|| self.texture.as_ref().map(FrameTexture::authored));
                let pixels = match (single, previous) {
                    // Snapping validated the authored drawing for this pointer
                    // event. This gesture writes only the transformed object.
                    (Some((source, object)), _) => {
                        scene.render_object_edit(source, object, frame, scale)?
                    }
                    (_, Some(before)) => {
                        scene.render_from_snapshot(before, document, frame, scale)?
                    }
                    _ => scene.render(original, document, frame, scale)?,
                };
                (color_image(pixels), None)
            }
            None => render_image(document, frame, scale)?,
        };
        let id = self.upload_page(ctx, Arc::clone(document), frame, scale, (image, backdrop));
        if settled {
            self.sharpen_at = None;
        }
        Ok(id)
    }

    pub(super) fn upload_page(
        &mut self,
        ctx: &egui::Context,
        snapshot: Arc<Document>,
        frame: u32,
        scale: f32,
        (image, backdrop): PageImage,
    ) -> egui::TextureId {
        let image = Arc::new(image);
        let sample_pixels = FrameTexture::keep_sample(&image);
        let handle = if let Some(mut cached) = self.texture.take() {
            cached.handle.set(image, ARTWORK_TEXTURE);
            cached.handle
        } else {
            ctx.load_texture("artboard artwork", image, ARTWORK_TEXTURE)
        };
        let id = handle.id();
        self.texture = Some(FrameTexture {
            sample_pixels,
            pending_split: None,
            failed_refinement: None,
            document: snapshot,
            backdrop,
            frame_independent: Default::default(),
            frame,
            scale,
            handle,
        });
        id
    }

    fn finish_drag_texture(
        &mut self,
        ctx: &egui::Context,
        document: &Arc<Document>,
        frame: u32,
        scale: f32,
    ) -> Result<(), String> {
        let Some(mut drag) = self.drag_preview.take() else {
            return Ok(());
        };
        let Some(texture) = self.texture.as_mut() else {
            return Ok(());
        };
        let page_drag = self.page_drag.take();
        let drag_scale = page_drag.as_ref().map_or(texture.scale, |p| p.scale);
        if texture.frame != frame
            || drag_scale >= scale
            || (texture.document.width, texture.document.height)
                != (document.width, document.height)
        {
            return Ok(());
        }
        // A release may carry a new position, and Escape restores the original
        // geometry. Show those pixels now at drag density, then sharpen off-thread.
        if !Arc::ptr_eq(&texture.document, document)
            && texture.document.as_ref() != document.as_ref()
        {
            let pixels = if let Some(page) = &page_drag {
                drag.render_from_snapshot(&page.source, document, frame, drag_scale)?
            } else {
                drag.render(document, document, frame, drag_scale)?
            };
            let image = color_image(pixels);
            texture.upload(image);
            texture.document = Arc::clone(document);
            texture.frame_independent = Default::default();
            texture.scale = drag_scale;
            texture.backdrop = None;
        }
        if texture.scale != scale {
            self.sharpen_at = Some(ctx.input(|i| i.time));
        }
        Ok(())
    }
}
