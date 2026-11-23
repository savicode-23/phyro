use super::{gradient, paint::ChannelPaint};
use crate::canvas::{paint_picker_icon, PaintChannel};
use document_core::{Gradient, GradientStop};
use eframe::egui::{self, Ui};

fn hex(rgb: [u8; 4]) -> String {
    format!("{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2])
}

fn parse(text: &str, alpha: u8) -> Option<[u8; 4]> {
    let s = text.trim();
    let s = s.strip_prefix('#').unwrap_or(s);
    if !(s.len() == 6 || s.len() == 8) || !s.is_ascii() {
        return None;
    }
    let byte = |n| u8::from_str_radix(&s[n..n + 2], 16).ok();
    Some([
        byte(0)?,
        byte(2)?,
        byte(4)?,
        if s.len() == 8 { byte(6)? } else { alpha },
    ])
}

fn label_width(ui: &Ui) -> f32 {
    (ui.available_width() * 0.32).clamp(39., 84.)
}

pub(super) fn number_label(ui: &mut Ui, label: &str) {
    // Keep the numerical rows aligned with the hex value above them.
    let width = (label_width(ui) + (ui.available_width() * 0.15).clamp(20., 40.) + 4.).max(74.);
    ui.add_sized(
        [width, ui.spacing().interact_size.y],
        egui::Label::new(label).halign(egui::Align::Min),
    );
    ui.spacing_mut().interact_size.x = ui.available_width().clamp(38., 160.);
}

#[derive(Default)]
pub(super) struct Row {
    pub gradient_edit: Option<gradient::Edit>,
    pub color_changed: bool,
    pub picked: bool,
    pub activate: bool,
}

pub(super) fn show(
    ui: &mut Ui,
    channel: PaintChannel,
    paint: &mut ChannelPaint<'_>,
    multiple: bool,
    active: bool,
    enabled: bool,
) -> Row {
    let name = channel.name();
    let pick_name = match channel {
        PaintChannel::Fill => "Pick fill color",
        PaintChannel::Stroke => "Pick stroke color",
    };
    let kind = if paint.disabled {
        gradient::PaintType::None
    } else {
        gradient::PaintType::of(paint.gradient)
    };
    let rgba = &mut paint.color;
    let before = *rgba;
    let width = ui.available_width();
    let label = label_width(ui).max(59.);
    let chip = (width * 0.15).clamp(20., 40.);
    let height = ui.spacing().interact_size.y.max(20.);
    let mut row = Row::default();
    let type_menu = |ui: &mut Ui| {
        ui.add_enabled_ui(enabled, |ui| {
            gradient::type_menu(ui, channel, kind, multiple, label)
        })
        .inner
    };
    let above = (width < 194.).then(|| type_menu(ui));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.;
        let (type_response, edit) = above.unwrap_or_else(|| type_menu(ui));
        let caption = type_response.id;
        row.activate = type_response.clicked() || edit.is_some();
        row.gradient_edit = edit;
        let restore = !multiple && paint.disabled;
        let clear = match (channel, restore) {
            (PaintChannel::Fill, false) => "No fill",
            (PaintChannel::Stroke, false) => "No stroke",
            (PaintChannel::Fill, true) => "Restore fill",
            (PaintChannel::Stroke, true) => "Restore stroke",
        };
        let clear_button = ui.add_enabled(
            enabled,
            egui::Button::new("×")
                .selected(restore)
                .min_size(egui::vec2(height, height)),
        );
        clear_button.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, restore, clear)
        });
        if clear_button.on_hover_text(clear).clicked() {
            row.activate = true;
            row.gradient_edit = Some(if restore {
                gradient::Edit::Restore
            } else {
                gradient::Edit::Type(gradient::PaintType::None)
            });
        }
        if paint.disabled {
            ui.weak("No paint").labelled_by(caption);
        } else if let Some(gradient) = paint.gradient {
            let sample = preview(ui, gradient, chip, height).labelled_by(caption);
            let edit = ui
                .selectable_label(active, kind.label())
                .labelled_by(caption);
            let hint = format!(
                "Edit {} gradient stops and canvas handles",
                name.to_lowercase()
            );
            sample.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &hint)
            });
            row.activate |=
                sample.on_hover_text(&hint).clicked() || edit.on_hover_text(hint).clicked();
        } else {
            let response = ui
                .scope(|ui| {
                    ui.spacing_mut().interact_size = egui::vec2(chip, height);
                    crate::color_picker::button(ui, rgba)
                })
                .inner
                .labelled_by(caption);
            response
                .clone()
                .on_hover_text(format!("Choose {name} color. Right-click for None."));
            row.activate |= response.clicked() || response.secondary_clicked();
            response.context_menu(|ui| {
                if ui.button("None").clicked() {
                    row.gradient_edit = Some(gradient::Edit::Type(gradient::PaintType::None));
                    ui.close_menu();
                }
            });
            let id = ui.make_persistent_id("hex");
            let focused = ui.memory(|m| m.has_focus(id));
            let mut text = if focused {
                ui.data(|d| d.get_temp::<String>(id))
                    .unwrap_or_else(|| hex(*rgba))
            } else {
                hex(*rgba)
            };
            let field = (ui.available_width() - height - 4.).clamp(55., 160.);
            let response = ui
                .add(
                    egui::TextEdit::singleline(&mut text)
                        .id(id)
                        .desired_width(field - 8.)
                        .font(egui::TextStyle::Monospace)
                        .char_limit(9),
                )
                .labelled_by(caption);
            row.activate |= response.gained_focus();
            if response.changed() {
                if let Some(value) = parse(&text, rgba[3]) {
                    *rgba = value;
                    crate::color_picker::remember(ui.ctx(), value);
                } else if text.eq_ignore_ascii_case("none") {
                    row.gradient_edit = Some(gradient::Edit::Type(gradient::PaintType::None));
                }
            }
            if response.has_focus() {
                ui.data_mut(|d| d.insert_temp(id, text));
            } else {
                ui.data_mut(|d| d.remove::<String>(id));
            }
            response.on_hover_text("Hex color (RRGGBB or RRGGBBAA). Enter None to clear.");
            let (_, rect) = ui.allocate_space(egui::vec2(height, height));
            let button = ui.interact(
                rect,
                egui::Id::new(("sample-property-color", ui.layer_id(), name)),
                egui::Sense::click(),
            );
            button.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), pick_name)
            });
            let visuals = ui.style().interact(&button);
            ui.painter().rect(
                button.rect,
                3.,
                visuals.weak_bg_fill,
                visuals.bg_stroke,
                egui::StrokeKind::Inside,
            );
            paint_picker_icon(ui.painter(), button.rect.center(), visuals.fg_stroke.color);
            row.picked = button
                .on_hover_text(format!("{pick_name} from the canvas"))
                .clicked();
        }
    });
    row.color_changed = before != *rgba;
    row.activate |= row.color_changed || row.picked;
    row
}

// Paint the actual stops, including hard stops and the padded colors at either end.
fn preview(ui: &mut Ui, gradient: &Gradient, width: f32, height: f32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let rect = rect.shrink(1.);
        egui::color_picker::show_color_at(ui.painter(), egui::Color32::TRANSPARENT, rect);
        let mut mesh = egui::Mesh::default();
        let first = GradientStop {
            offset: 0.,
            ..gradient.stops[0]
        };
        let last = GradientStop {
            offset: 1.,
            ..gradient.stops[gradient.stops.len() - 1]
        };
        for stop in std::iter::once(&first)
            .chain(&gradient.stops)
            .chain([&last])
        {
            let [r, g, b, a] = stop.color;
            let color = egui::Color32::from_rgba_unmultiplied(r, g, b, a);
            let x = egui::lerp(rect.x_range(), stop.offset);
            let i = mesh.vertices.len() as u32;
            mesh.colored_vertex(egui::pos2(x, rect.top()), color);
            mesh.colored_vertex(egui::pos2(x, rect.bottom()), color);
            if i > 0 {
                mesh.add_triangle(i - 2, i - 1, i);
                mesh.add_triangle(i - 1, i + 1, i);
            }
        }
        ui.painter().add(mesh);
        ui.painter().rect_stroke(
            rect,
            2.,
            ui.style().interact(&response).bg_stroke,
            egui::StrokeKind::Inside,
        );
    }
    response
}
