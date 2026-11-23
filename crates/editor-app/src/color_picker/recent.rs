use eframe::egui::{self, color_picker, Color32, Sense, Ui};

const LIMIT: usize = 8;
type Rgba = [u8; 4];

#[derive(Clone, Default)]
struct Palette {
    colors: Vec<Rgba>,
    used: Vec<Rgba>,
    pending: Option<Rgba>,
}

fn id() -> egui::Id {
    egui::Id::new("recent paint colors")
}

fn promote(colors: &mut Vec<Rgba>, color: Rgba) {
    colors.retain(|other| *other != color);
    colors.insert(0, color);
    colors.truncate(LIMIT);
}

pub(crate) fn restore(ctx: &egui::Context, colors: &[Rgba]) {
    let mut list = Vec::new();
    for &color in colors {
        if !list.contains(&color) {
            list.push(color)
        }
        if list.len() == LIMIT {
            break;
        }
    }
    ctx.data_mut(|d| {
        d.insert_temp(
            id(),
            Palette {
                colors: list,
                ..Default::default()
            },
        )
    });
}

pub(crate) fn remember(ctx: &egui::Context, color: Rgba) {
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Palette>(id()).pending = Some(color));
}

pub(crate) fn finish(ctx: &egui::Context) {
    let (pressing, released) =
        ctx.input(|i| (i.pointer.primary_down(), i.pointer.primary_released()));
    if pressing || (!released && ctx.wants_keyboard_input()) {
        return;
    }
    flush(ctx);
}

fn flush(ctx: &egui::Context) {
    ctx.data_mut(|d| {
        let state = d.get_temp_mut_or_default::<Palette>(id());
        if let Some(color) = state.pending.take() {
            promote(&mut state.colors, color);
            promote(&mut state.used, color);
        }
    });
}

pub(crate) fn colors(ctx: &egui::Context) -> Vec<Rgba> {
    flush(ctx);
    ctx.data(|d| d.get_temp::<Palette>(id()))
        .unwrap_or_default()
        .colors
}

pub(crate) fn merge(ctx: &egui::Context, stored: &mut Vec<Rgba>) {
    flush(ctx);
    let used = ctx
        .data(|d| d.get_temp::<Palette>(id()))
        .unwrap_or_default()
        .used;
    for color in used.into_iter().rev() {
        promote(stored, color);
    }
}

pub(super) fn swatches(ui: &mut Ui, color: &mut Rgba) {
    let colors = ui
        .data(|d| d.get_temp::<Palette>(id()))
        .unwrap_or_default()
        .colors;
    if colors.is_empty() {
        return;
    }
    ui.horizontal(|ui| {
        ui.small("Recent");
        ui.spacing_mut().item_spacing.x = 3.;
        for rgba in colors {
            let (rect, response) = ui.allocate_exact_size(egui::vec2(18., 18.), Sense::click());
            let [r, g, b, a] = rgba;
            let hex = format!("#{r:02X}{g:02X}{b:02X}{a:02X}");
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::ColorButton, ui.is_enabled(), &hex)
            });
            color_picker::show_color_at(
                ui.painter(),
                Color32::from_rgba_unmultiplied(r, g, b, a),
                rect.shrink(1.),
            );
            ui.painter().rect_stroke(
                rect,
                2.,
                ui.style().interact(&response).bg_stroke,
                egui::StrokeKind::Inside,
            );
            if response.on_hover_text(hex).clicked() {
                *color = rgba;
                remember(ui.ctx(), rgba);
            }
        }
    });
}
