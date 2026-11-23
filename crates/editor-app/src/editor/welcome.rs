use super::welcome_preview::Preview;
use crate::theme;
use eframe::egui::{
    self, pos2, vec2, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2,
};
use std::path::{Path, PathBuf};

const BACKGROUND: Color32 = Color32::from_rgb(19, 26, 29);
const BORDER: Color32 = Color32::from_rgb(61, 77, 78);
const INK: Color32 = Color32::from_rgb(233, 242, 237);
const MUTED: Color32 = Color32::from_rgb(164, 184, 179);
const ROW: f32 = 45.;

pub(super) enum Action {
    New,
    Open,
    Recent(PathBuf),
    RecentIssues,
}

pub(super) fn show(
    ctx: &egui::Context,
    recent: &[PathBuf],
    status: &str,
    has_issues: bool,
    preview: &mut Preview,
) -> Option<Action> {
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(BACKGROUND))
        .show(ctx, |ui| {
            let viewport_height = ui.available_height();
            egui::ScrollArea::vertical()
                .id_salt("welcome-page")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let room = ui.available_width();
                    let margin = if room < 380. { 8. } else { 22. };
                    let width = (room - 2. * margin)
                        .min(if recent.is_empty() { 662. } else { 940. })
                        .max(1.);
                    let pad = if width < 440. { 16. } else { 26. };
                    let inner = (width - 2. * pad).max(1.);
                    let stacked = inner < 545.;
                    let header = if stacked { 101. } else { 60. };
                    let gap = 21.;
                    let list_width = if stacked {
                        inner
                    } else {
                        (inner * 0.36).min(300.)
                    };
                    let preview_width = if stacked {
                        inner
                    } else {
                        inner - list_width - 26.
                    };
                    let image_height = (preview_width * 0.55).clamp(105., 275.);
                    let list_height = if stacked {
                        (recent.len() as f32 * (ROW + 3.)).min(140.)
                    } else {
                        image_height + 45.
                    };
                    let preview_height = 29. + image_height + 45.;
                    let body = if recent.is_empty() {
                        66.
                    } else if stacked {
                        29. + list_height + gap + preview_height
                    } else {
                        preview_height
                    };
                    let has_footer = has_issues || !status.is_empty();
                    let height =
                        pad * 2. + header + gap * 2. + body + if has_footer { 33. } else { 0. };
                    let content_height = viewport_height.max(height + margin * 2.);
                    let (_, page) = ui.allocate_space(vec2(room, content_height));
                    let card = Rect::from_center_size(page.center(), vec2(width, height));
                    ui.painter()
                        .rect(card, 14, theme::SURFACE, (1., BORDER), StrokeKind::Inside);
                    let panel = card.shrink(pad);
                    let mut action = header_actions(ui, panel, header, stacked, recent.is_empty());
                    let divider = panel.top() + header + gap;
                    ui.painter().hline(panel.x_range(), divider, (1., BORDER));
                    let y = divider + gap;
                    title(ui, pos2(panel.left(), y), "Recent projects");
                    if recent.is_empty() {
                        preview.clear_selection(ui.ctx());
                        ui.painter().text(
                            pos2(panel.left(), y + 36.),
                            Align2::LEFT_TOP,
                            "No recent projects yet",
                            FontId::proportional(14.),
                            MUTED,
                        );
                    } else {
                        let list = Rect::from_min_size(
                            pos2(panel.left(), y + 29.),
                            vec2(list_width, list_height),
                        );
                        let memory = ui.id().with("selected-recent-project");
                        let remembered = ui.data(|d| d.get_temp::<PathBuf>(memory));
                        let mut selected = remembered
                            .as_deref()
                            .and_then(|path| recent.iter().position(|p| p == path))
                            .unwrap_or(0);
                        let was = selected;
                        if let Some(open) = recent_rows(ui, list, recent, &mut selected) {
                            action = Some(Action::Recent(open));
                        }
                        if was != selected || remembered.as_ref() != Some(&recent[selected]) {
                            ui.data_mut(|d| d.insert_temp(memory, recent[selected].clone()));
                        }
                        let top = if stacked {
                            pos2(panel.left(), list.bottom() + gap)
                        } else {
                            pos2(list.right() + 26., y)
                        };
                        title(ui, top, "Preview");
                        let image = Rect::from_min_size(
                            top + vec2(0., 29.),
                            vec2(preview_width, image_height),
                        );
                        let mut image_ui = ui.new_child(egui::UiBuilder::new().max_rect(image));
                        preview.show(&mut image_ui, Some(&recent[selected]), image.size());
                        let open = Rect::from_min_size(
                            pos2(image.left(), image.bottom() + 9.),
                            vec2(158_f32.min(image.width()), 36.),
                        );
                        if button(ui, open, "Open selected project", true) {
                            action = Some(Action::Recent(recent[selected].clone()));
                        }
                    }
                    if has_footer {
                        let footer =
                            Rect::from_min_max(pos2(panel.left(), panel.bottom() - 23.), panel.max);
                        let mut text = footer;
                        if has_issues {
                            let button = Rect::from_min_max(
                                pos2((footer.right() - 100.).max(footer.left()), footer.top()),
                                footer.max,
                            );
                            text.max.x = button.left() - 8.;
                            if ui.put(button, egui::Button::new("Recent issues")).clicked() {
                                action = Some(Action::RecentIssues);
                            }
                        }
                        if !status.is_empty() && text.width() > 0. {
                            ui.put(text, egui::Label::new(status).truncate())
                                .on_hover_text(status);
                        }
                    }
                    action
                })
                .inner
        })
        .inner
}

fn header_actions(
    ui: &mut Ui,
    panel: Rect,
    height: f32,
    stacked: bool,
    empty: bool,
) -> Option<Action> {
    let side = if stacked { 45. } else { 60. };
    let icon = Rect::from_min_size(panel.min, Vec2::splat(side));
    let key = egui::Id::new("phyro-welcome-logo");
    let texture = ui
        .data(|d| d.get_temp::<egui::TextureHandle>(key))
        .unwrap_or_else(|| {
            let bytes =
                eframe::icon_data::from_png_bytes(include_bytes!("../../assets/phyro-icon.png"))
                    .expect("embedded Phyro logo");
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [bytes.width as usize, bytes.height as usize],
                &bytes.rgba,
            );
            let texture = ui
                .ctx()
                .load_texture("Phyro hare", image, egui::TextureOptions::LINEAR);
            ui.data_mut(|d| d.insert_temp(key, texture.clone()));
            texture
        });
    ui.painter().image(
        texture.id(),
        icon,
        Rect::from_min_max(Pos2::ZERO, pos2(1., 1.)),
        Color32::WHITE,
    );
    ui.painter().text(
        pos2(icon.right() + 12., icon.center().y - 1.),
        Align2::LEFT_CENTER,
        "Phyro",
        FontId::proportional(if stacked { 35. } else { 43. }),
        INK,
    );
    let gap = 8.;
    let width = if stacked {
        (panel.width() - gap) * 0.5
    } else {
        123.
    };
    let left = if stacked {
        panel.left()
    } else {
        panel.right() - 2. * width - gap
    };
    let top = if stacked {
        panel.top() + height - 38.
    } else {
        icon.center().y - 19.
    };
    let new = Rect::from_min_size(pos2(left, top), vec2(width, 38.));
    if button(ui, new, "New project", empty) {
        return Some(Action::New);
    }
    let open = new.translate(vec2(width + gap, 0.));
    button(ui, open, "Open project…", false).then_some(Action::Open)
}

fn button(ui: &mut Ui, rect: Rect, label: &str, primary: bool) -> bool {
    let ink = if primary { theme::PETROL } else { INK };
    let fill = if primary { theme::MINT } else { theme::RAISED };
    ui.put(
        rect,
        egui::Button::new(egui::RichText::new(label).color(ink).size(14.))
            .fill(fill)
            .stroke(if primary {
                Stroke::NONE
            } else {
                Stroke::new(1_f32, BORDER)
            })
            .corner_radius(6)
            .min_size(rect.size()),
    )
    .clicked()
}

fn title(ui: &Ui, pos: Pos2, label: &str) {
    ui.painter()
        .text(pos, Align2::LEFT_TOP, label, FontId::proportional(18.), INK);
}

fn recent_rows(
    ui: &mut Ui,
    rect: Rect,
    recent: &[PathBuf],
    selected: &mut usize,
) -> Option<PathBuf> {
    let mut list = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("recent-files")
            .max_rect(rect),
    );
    list.spacing_mut().item_spacing.y = 3.;
    let mut open = None;
    egui::ScrollArea::vertical()
        .id_salt("rows")
        .max_height(rect.height())
        .auto_shrink([false, false])
        .show_rows(&mut list, ROW, recent.len(), |ui, rows| {
            for index in rows {
                let path = &recent[index];
                let name = path
                    .file_name()
                    .unwrap_or(path.as_os_str())
                    .to_string_lossy();
                let location = parent_label(path, recent);
                let (rect, response) =
                    ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
                let active = *selected == index;
                response.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::WidgetType::SelectableLabel,
                        ui.is_enabled(),
                        active,
                        format!("{name}, {location}"),
                    )
                });
                if response.double_clicked()
                    || (active
                        && response.has_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                {
                    open = Some(path.clone());
                }
                if response.clicked() {
                    *selected = index;
                }
                if active || response.hovered() || response.has_focus() {
                    let fill = if active {
                        theme::SELECTED
                    } else {
                        theme::RAISED
                    };
                    ui.painter().rect_filled(rect, 6, fill);
                }
                if response.has_focus() {
                    ui.painter()
                        .rect_stroke(rect, 6, (1., theme::MINT), StrokeKind::Inside);
                }
                let x = rect.left() + 12.;
                let y = rect.center().y;
                let pen = Stroke::new(1.3_f32, if active { theme::MINT } else { MUTED });
                ui.painter().add(egui::Shape::closed_line(
                    vec![
                        pos2(x, y - 8.),
                        pos2(x + 8., y - 8.),
                        pos2(x + 13., y - 3.),
                        pos2(x + 13., y + 8.),
                        pos2(x, y + 8.),
                    ],
                    pen,
                ));
                ui.painter().add(egui::Shape::line(
                    vec![
                        pos2(x + 8., y - 8.),
                        pos2(x + 8., y - 3.),
                        pos2(x + 13., y - 3.),
                    ],
                    pen,
                ));
                let text_x = x + 24.;
                let width = (rect.right() - text_x - 5.).max(1.);
                for (text, y, size, ink) in [
                    (name.as_ref(), y - 17., 15., INK),
                    (&location, y + 3., 11.5, MUTED),
                ] {
                    let galley = egui::WidgetText::from(text).into_galley(
                        ui,
                        Some(egui::TextWrapMode::Truncate),
                        width,
                        FontId::proportional(size),
                    );
                    ui.painter().galley(pos2(text_x, y), galley, ink);
                }
                response.on_hover_text(path.to_string_lossy());
            }
        });
    open
}

fn parent_label(path: &Path, recent: &[PathBuf]) -> String {
    let parent = path.parent().unwrap_or(Path::new(""));
    let shared = recent
        .iter()
        .filter(|p| p.as_path() != path && p.file_name() == path.file_name())
        .filter_map(|p| p.parent())
        .map(|p| {
            p.components()
                .rev()
                .zip(parent.components().rev())
                .take_while(|(a, b)| a == b)
                .count()
        })
        .max()
        .unwrap_or(0);
    let depth = parent.components().count().saturating_sub(shared + 1);
    let label: PathBuf = parent.components().skip(depth).collect();
    label.to_string_lossy().into_owned()
}
