use super::backend::{Entry, Kind};
use super::previews::Image;
use crate::theme;
use eframe::egui::{self, pos2, vec2, Color32, Rect, Response, Sense, Ui};

fn paint_image(ui: &Ui, rect: Rect, image: Image) {
    let scale = (rect.width() / image.size.x).min(rect.height() / image.size.y);
    let target = Rect::from_center_size(rect.center(), image.size * scale);
    ui.painter().image(
        image.id,
        target,
        Rect::from_min_max(pos2(0., 0.), pos2(1., 1.)),
        Color32::WHITE,
    );
}

pub(super) fn asset_tile(
    ui: &mut Ui,
    entry: &Entry,
    image: Option<Image>,
    selected: bool,
    size: egui::Vec2,
) -> Response {
    let (card, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            &entry.name,
        )
    });
    let matte = Rect::from_min_max(card.min, pos2(card.right(), card.bottom() - 19.));
    let surface = if response.hovered() {
        ui.visuals().widgets.hovered.bg_fill
    } else {
        theme::RAISED
    };
    ui.painter().rect_filled(matte, 3., surface);
    if let Some(image) = image {
        paint_image(ui, matte.shrink(4.), image);
    } else if entry.kind == Kind::Folder {
        let b = Rect::from_center_size(matte.center(), vec2(24., 15.));
        let ink = ui.visuals().weak_text_color();
        ui.painter().rect_filled(
            Rect::from_min_size(b.min - vec2(0., 2.), vec2(10., 5.)),
            2.,
            ink,
        );
        ui.painter().rect_filled(b, 2., ink);
    } else {
        let label = match entry.kind {
            Kind::Project => "VANIM",
            Kind::Svg => "SVG",
            _ => "Image",
        };
        ui.painter().text(
            matte.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(11.),
            ui.visuals().weak_text_color(),
        );
    }
    if selected {
        ui.painter()
            .rect_stroke(matte, 3., (1.5, theme::MINT), egui::StrokeKind::Inside);
    }
    let galley = egui::WidgetText::from(&entry.name).into_galley(
        ui,
        Some(egui::TextWrapMode::Truncate),
        size.x,
        egui::FontId::proportional(12.),
    );
    let origin = pos2(card.center().x - galley.size().x * 0.5, matte.bottom() + 2.);
    ui.painter().galley(
        origin,
        galley,
        if selected {
            theme::MINT
        } else {
            ui.visuals().text_color()
        },
    );
    let response = response.on_hover_cursor(if entry.kind == Kind::Folder {
        egui::CursorIcon::PointingHand
    } else {
        egui::CursorIcon::Grab
    });
    response.on_hover_ui(|ui| {
        ui.label(&entry.name);
        ui.weak(match entry.kind {
            Kind::Folder => "Double-click to open folder",
            Kind::Project => "Double-click to open in a tab · Drag to place",
            _ => "Drag onto the canvas to place",
        });
    })
}

pub(super) fn strip(ui: &mut Ui, images: &[Option<Image>], height: f32) -> Response {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click_and_drag());
    let width = rect.width() / images.len() as f32;
    for (i, image) in images.iter().enumerate() {
        let tile = Rect::from_min_size(
            rect.min + vec2(width * i as f32, 0.),
            vec2((width - 2.).max(1.), height),
        );
        ui.painter().rect_filled(tile, 3., theme::RAISED);
        if let Some(image) = image {
            paint_image(ui, tile.shrink(3.), *image);
        }
    }
    response
        .on_hover_text("Drag onto the canvas to place")
        .on_hover_cursor(egui::CursorIcon::Grab)
}
