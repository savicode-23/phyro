use super::Canvas;
use crate::{library::AssetDrop, theme};
use eframe::egui::{
    self, pos2, vec2, Color32, CursorIcon, FontId, Painter, Pos2, Rect, Stroke, Ui, Vec2,
};

impl Canvas {
    pub(crate) fn paint_folder_drop(
        &self,
        ui: &Ui,
        loading: Option<[f32; 2]>,
        blocked: impl FnOnce() -> Option<&'static str>,
    ) {
        let Some((surface, view)) = self.context_surface.as_ref() else {
            return;
        };
        let canvas = surface.rect.intersect(ui.clip_rect());
        if let Some(world) = loading {
            let at = view.screen(world.into());
            if canvas.contains(at) {
                let painter = ui.painter().with_clip_rect(canvas);
                crosshair(&painter, at);
                badge(ui, &painter, at, None, "Placing…", theme::MINT);
            }
        }
        if !ui.input(|i| i.pointer.primary_down()) {
            return;
        }
        let Some(asset) = egui::DragAndDrop::payload::<AssetDrop>(ui.ctx()) else {
            return;
        };
        let Some(at) = ui.ctx().pointer_hover_pos() else {
            return;
        };
        // Match dnd_release_payload's hit test. The canvas includes editable
        // pasteboard space around the page and can be covered by other panels.
        let over_canvas = surface.contains_pointer();
        let reason = over_canvas.then(blocked).flatten();
        let ready = over_canvas && reason.is_none();
        let (hint, color, cursor) = if let Some(reason) = reason {
            (reason, ui.visuals().warn_fg_color, CursorIcon::NotAllowed)
        } else if ready {
            ("Release to place", theme::MINT, CursorIcon::Copy)
        } else {
            (
                "Drag onto canvas",
                Color32::LIGHT_GRAY,
                CursorIcon::Grabbing,
            )
        };
        let painter = ui.ctx().layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            egui::Id::new("folder-placement-feedback"),
        ));
        if ready {
            crosshair(&painter.with_clip_rect(canvas), at);
        }
        ui.ctx().set_cursor_icon(cursor);
        badge(ui, &painter, at, Some(&asset), hint, color);
    }
}

fn crosshair(painter: &Painter, at: Pos2) {
    for (width, ink) in [(3.5, theme::PETROL), (1.5, theme::MINT)] {
        painter.circle_stroke(at, 5., (width, ink));
        for axis in [Vec2::X, -Vec2::X, Vec2::Y, -Vec2::Y] {
            painter.line_segment([at + axis * 6., at + axis * 11.], (width, ink));
        }
    }
}

fn badge(
    ui: &Ui,
    painter: &Painter,
    at: Pos2,
    asset: Option<&AssetDrop>,
    hint: &str,
    accent: Color32,
) {
    let bounds = painter.clip_rect();
    let inset = if asset.is_some() { 46. } else { 10. };
    let limit = (bounds.width() - inset - 20.).clamp(1., 206.);
    let line = |text: &str, size| {
        egui::WidgetText::from(text).into_galley(
            ui,
            Some(egui::TextWrapMode::Truncate),
            limit,
            FontId::proportional(size),
        )
    };
    let title = asset.map(|asset| line(&asset.label, 13.));
    let hint = line(hint, 12.);
    let text_width = title
        .as_ref()
        .map_or(0., |line| line.size().x)
        .max(hint.size().x);
    let height = if asset.is_some() { 43. } else { 25. };
    let size = vec2(inset + text_width + 10., height);
    let mut origin = at + vec2(15., 15.);
    if origin.x + size.x > bounds.right() {
        origin.x = at.x - 15. - size.x;
    }
    if origin.y + size.y > bounds.bottom() {
        origin.y = at.y - 15. - size.y;
    }
    let origin = origin.clamp(bounds.min, (bounds.max - size).max(bounds.min));
    let rect = Rect::from_min_size(origin, size);
    painter.rect(
        rect,
        6.,
        theme::SURFACE,
        (1., accent),
        egui::StrokeKind::Inside,
    );
    let x = rect.left() + inset;
    if let Some(title) = title {
        let icon =
            Rect::from_center_size(pos2(rect.left() + 23., rect.center().y), Vec2::splat(31.));
        painter.rect_filled(icon, 3., theme::RAISED);
        if let Some(image) = asset.and_then(|asset| asset.thumbnail.as_ref()) {
            let dims = image.size_vec2();
            let ratio = (icon.width() / dims.x).min(icon.height() / dims.y);
            painter.image(
                image.id(),
                Rect::from_center_size(icon.center(), dims * ratio),
                Rect::from_min_max(Pos2::ZERO, pos2(1., 1.)),
                Color32::WHITE,
            );
        } else {
            let c = icon.center();
            let sheet = Rect::from_center_size(c, vec2(13., 17.));
            painter.rect_stroke(
                sheet,
                2.,
                Stroke::new(1.5_f32, Color32::LIGHT_GRAY),
                egui::StrokeKind::Inside,
            );
            painter.line_segment(
                [c - vec2(3., 0.), c + vec2(3., 0.)],
                (1., Color32::LIGHT_GRAY),
            );
        }
        painter.galley(pos2(x, rect.top() + 5.), title, Color32::WHITE);
        painter.galley(pos2(x, rect.bottom() - hint.size().y - 5.), hint, accent);
    } else {
        painter.galley(pos2(x, rect.center().y - hint.size().y * 0.5), hint, accent);
    }
}
