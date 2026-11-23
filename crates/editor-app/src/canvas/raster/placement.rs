use super::{render_image, Canvas, PageImage};
use document_core::Document;
use eframe::egui;
use std::sync::Arc;

#[derive(Clone, Copy)]
pub(crate) struct PlacementResolution {
    page: f32,
    sprite: f32,
}

pub(crate) struct PlacementPage {
    document: Arc<Document>,
    frame: u32,
    scale: f32,
    image: PageImage,
    sprite: Option<(u64, renderer::MoveSprite)>,
}

impl PlacementPage {
    // Prepare the exact page and the bounded drag preview on the import worker.
    pub(crate) fn render(
        document: &Arc<Document>,
        frame: u32,
        id: u64,
        resolution: PlacementResolution,
    ) -> Option<Self> {
        let image = render_image(document, frame, resolution.page).ok()?;
        let sprite = document
            .objects
            .iter()
            .find(|o| o.id == id)
            .and_then(|o| renderer::move_sprite(o, frame, resolution.sprite))
            .map(|sprite| (id, sprite));
        Some(Self {
            document: document.clone(),
            frame,
            scale: resolution.page,
            image,
            sprite,
        })
    }
}

impl Canvas {
    pub(crate) fn show_duplicated_placement(
        &mut self,
        drawing: &Arc<Document>,
        frame: u32,
        id: u64,
    ) {
        if drawing
            .objects
            .iter()
            .any(|o| o.id == id && (o.is_instance() || o.image.is_some()))
        {
            self.duplicated_placement = Some((drawing.clone(), frame, id));
        }
    }

    pub(crate) fn placement_resolution(&self, ctx: &egui::Context) -> Option<PlacementResolution> {
        let page = self.texture.as_ref()?.scale;
        let sprite = self
            .context_surface
            .as_ref()
            .map_or(page, |(_, view)| view.scale * ctx.pixels_per_point());
        Some(PlacementResolution { page, sprite })
    }

    pub(crate) fn install_placement_page(&mut self, ctx: &egui::Context, page: PlacementPage) {
        // A view change during import can request a different raster size.
        // Leave that decision to the normal canvas renderer.
        if self
            .texture
            .as_ref()
            .is_none_or(|texture| texture.scale != page.scale)
        {
            self.retain_placement_background(page.frame, None);
            return;
        }
        self.refinement = None;
        self.retain_placement_background(page.frame, page.sprite.as_ref().map(|(id, _)| *id));
        if let Some((id, sprite)) = page.sprite {
            self.prime_move_sprite(ctx, &page.document, page.frame, id, sprite);
        }
        self.upload_page(ctx, page.document, page.frame, page.scale, page.image);
    }
}
