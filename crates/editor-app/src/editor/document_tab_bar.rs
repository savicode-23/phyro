use super::Editor;
use crate::keymap::{self, Action};
use crate::theme;
use eframe::egui::{self, pos2, vec2, Align, Color32, Pos2, Rect, Sense, Stroke, Ui};
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const HEIGHT: f32 = 26.;

struct Names {
    labels: HashMap<u64, (String, Option<PathBuf>)>,
}

impl Names {
    fn read(editor: &Editor) -> Self {
        let mut labels = HashMap::new();
        let mut matching = HashMap::<String, Vec<(Vec<OsString>, u64)>>::new();
        for i in 0..editor.document_tab_count() {
            let (id, name, _, path) = editor.document_tab_info(i);
            if let Some(parent) = path.and_then(Path::parent) {
                let parts = parent
                    .components()
                    .rev()
                    .map(|p| p.as_os_str().to_owned())
                    .collect();
                matching
                    .entry(name.to_lowercase())
                    .or_default()
                    .push((parts, id));
            }
            labels.insert(id, (name, path.map(Path::to_owned)));
        }
        // Qualify duplicate filenames with the shortest distinguishing parent
        // path. Compare neighbors in reversed directory order once per change.
        for siblings in matching.values_mut().filter(|g| g.len() > 1) {
            siblings.sort_unstable();
            for (i, (dirs, id)) in siblings.iter().enumerate() {
                let shared = [i.checked_sub(1), Some(i + 1)]
                    .into_iter()
                    .flatten()
                    .filter_map(|j| siblings.get(j))
                    .map(|(other, _)| dirs.iter().zip(other).take_while(|(a, b)| a == b).count())
                    .max()
                    .unwrap_or(0);
                let parent: PathBuf = dirs.iter().take(shared + 1).rev().collect();
                if let Some((name, _)) = labels.get_mut(id) {
                    *name = format!("{} / {name}", parent.display());
                }
            }
        }
        Self { labels }
    }

    fn cached(editor: &Editor, ctx: &egui::Context, active: u64, path: Option<&Path>) -> Arc<Self> {
        let key = egui::Id::new("phyro-document-tab-labels");
        let cached = ctx.data(|m| m.get_temp::<Arc<Self>>(key));
        if let Some(cache) = cached {
            if cache.labels.len() == editor.document_tab_count()
                && cache
                    .labels
                    .get(&active)
                    .is_some_and(|(_, p)| p.as_deref() == path)
            {
                return cache;
            }
        }
        let names = Arc::new(Self::read(editor));
        ctx.data_mut(|m| m.insert_temp(key, Arc::clone(&names)));
        names
    }
}

enum TabAction {
    Select(usize),
    Close(usize),
}

impl Editor {
    pub(super) fn document_tab_shortcuts(&mut self, ctx: &egui::Context) -> bool {
        if !ctx.input(|i| {
            i.events.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::Key {
                        pressed: true,
                        repeat: false,
                        ..
                    }
                )
            })
        }) {
            return false;
        }
        let active = self.active_document_tab();
        let count = self.document_tab_count();
        if keymap::pressed(ctx, Action::CloseTab) {
            self.close_document_tab(active);
        } else if keymap::pressed(ctx, Action::NextTab) {
            self.select_document_tab((active + 1) % count);
        } else if keymap::pressed(ctx, Action::PreviousTab) {
            self.select_document_tab((active + count - 1) % count);
        } else {
            return false;
        }
        ctx.request_repaint();
        true
    }

    pub(super) fn document_tab_menu(&mut self, ui: &mut Ui, enabled: bool) {
        let button =
            egui::Button::new("Close tab").shortcut_text(keymap::label(ui.ctx(), Action::CloseTab));
        if ui.add_enabled(enabled, button).clicked() {
            ui.close_menu();
            self.close_document_tab(self.active_document_tab());
        }
    }

    pub(super) fn document_tab_bar(&mut self, ctx: &egui::Context, enabled: bool) {
        let count = self.document_tab_count();
        if count == 1 && self.path.is_none() && self.recovered_name.is_none() {
            return;
        }
        let active = self.active_document_tab();
        let (active_id, _, _, path) = self.document_tab_info(active);
        let names = Names::cached(self, ctx, active_id, path);
        let mut action = None;
        egui::TopBottomPanel::top("document-tabs")
            .exact_height(HEIGHT)
            .frame(egui::Frame::NONE.fill(theme::SURFACE))
            .show(ctx, |ui| {
                ui.add_enabled_ui(enabled, |ui| {
                    let area = ui.available_rect_before_wrap();
                    let width = (area.width() / count as f32).clamp(135., 196.);
                    let overflow = count as f32 * width > area.width() + 1.;
                    let strip = if overflow {
                        Rect::from_min_max(area.min, pos2(area.right() - 54., area.bottom()))
                    } else {
                        area
                    };
                    ui.scope_builder(egui::UiBuilder::new().max_rect(strip), |ui| {
                        let memory = egui::Id::new("document-tab-scroll-state");
                        let geometry = (active_id, active, width, strip.width());
                        let (previous, old_offset) = ctx
                            .data(|d| d.get_temp::<((u64, usize, f32, f32), f32)>(memory))
                            .unwrap_or_default();
                        let mut scroll = egui::ScrollArea::horizontal()
                            .id_salt("document-tabs-scroll")
                            .auto_shrink([false, false])
                            .scroll_bar_visibility(
                                egui::scroll_area::ScrollBarVisibility::AlwaysHidden,
                            );
                        if geometry != previous {
                            let x = active as f32 * width;
                            scroll = scroll.horizontal_scroll_offset(
                                old_offset.clamp((x + width - strip.width()).clamp(0., x), x),
                            );
                        }
                        let output = scroll.show_viewport(ui, |ui, viewport| {
                            let (_, row) = ui.allocate_space(vec2(count as f32 * width, HEIGHT));
                            let first = (viewport.left().max(0.) / width).floor() as usize;
                            let end = ((viewport.right() / width).ceil() as usize + 1).min(count);
                            for i in first..end {
                                let rect = Rect::from_min_size(
                                    row.min + vec2(i as f32 * width, 0.),
                                    vec2(width, HEIGHT),
                                );
                                let (id, name, dirty, path) = self.document_tab_info(i);
                                let label = names
                                    .labels
                                    .get(&id)
                                    .map_or(name.as_str(), |e| e.0.as_str());
                                let header = Header {
                                    index: i,
                                    id,
                                    label,
                                    path,
                                    dirty,
                                    active: i == active,
                                };
                                if let Some(choice) = header.show(ui, rect) {
                                    action = Some(choice);
                                }
                            }
                        });
                        ctx.data_mut(|d| d.insert_temp(memory, (geometry, output.state.offset.x)));
                    });
                    if overflow {
                        let menu =
                            Rect::from_min_max(pos2(strip.right() + 2., area.top()), area.max);
                        ui.scope_builder(
                            egui::UiBuilder::new()
                                .max_rect(menu)
                                .layout(egui::Layout::left_to_right(Align::Center)),
                            |ui| {
                                if let Some(i) = self.all_document_tabs_menu(ui, &names) {
                                    action = Some(TabAction::Select(i));
                                }
                            },
                        );
                    }
                });
            });
        if let Some(action) = action {
            match action {
                TabAction::Select(i) => self.select_document_tab(i),
                TabAction::Close(i) => self.close_document_tab(i),
            }
            ctx.request_repaint();
        }
    }

    fn all_document_tabs_menu(&self, ui: &mut Ui, names: &Names) -> Option<usize> {
        let mut selected = None;
        ui.menu_button("Tabs", |ui| {
            egui::ScrollArea::vertical()
                .id_salt("all-document-tabs")
                .max_height(285.)
                .show_rows(
                    ui,
                    ui.spacing().interact_size.y,
                    self.document_tab_count(),
                    |ui, rows| {
                        for index in rows {
                            let (id, name, dirty, path) = self.document_tab_info(index);
                            let label = names
                                .labels
                                .get(&id)
                                .map_or(name.as_str(), |e| e.0.as_str());
                            let label = if dirty {
                                format!("{label} *")
                            } else {
                                label.to_owned()
                            };
                            let response = ui
                                .selectable_label(index == self.active_document_tab(), label)
                                .on_hover_ui(|ui| {
                                    if let Some(p) = path {
                                        ui.label(p.display().to_string());
                                    }
                                });
                            if response.clicked() {
                                selected = Some(index);
                                ui.close_menu();
                            }
                        }
                    },
                );
        });
        selected
    }
}

struct Header<'a> {
    index: usize,
    id: u64,
    label: &'a str,
    path: Option<&'a Path>,
    dirty: bool,
    active: bool,
}

impl Header<'_> {
    fn show(self, ui: &mut Ui, rect: Rect) -> Option<TabAction> {
        let Self {
            index,
            id,
            label,
            path,
            dirty,
            active,
        } = self;
        let split = rect.right() - 25.;
        let body = Rect::from_min_max(rect.min, pos2(split, rect.bottom()));
        let close = Rect::from_min_max(pos2(split, rect.top()), rect.max).shrink2(vec2(1., 2.));
        let title = ui.interact(body, egui::Id::new(("document-title", id)), Sense::click());
        let cross = ui.interact(close, egui::Id::new(("close-document", id)), Sense::click());
        title.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::SelectableLabel,
                ui.is_enabled(),
                active,
                if dirty {
                    format!("{label}, unsaved")
                } else {
                    label.to_owned()
                },
            )
        });
        cross.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                ui.is_enabled(),
                format!("Close {label}"),
            )
        });
        if active || title.hovered() || title.has_focus() {
            ui.painter().rect_filled(rect, 0., theme::RAISED);
        }
        if active {
            ui.painter()
                .hline(rect.x_range(), rect.bottom() - 1., (2., theme::MINT));
        }
        let x = if dirty { 17. } else { 8. };
        let text = egui::WidgetText::from(label).into_galley(
            ui,
            Some(egui::TextWrapMode::Truncate),
            (body.width() - x - 2.).max(1.),
            egui::FontId::proportional(13.),
        );
        let ink = if active {
            ui.visuals().strong_text_color()
        } else {
            ui.visuals().text_color()
        };
        ui.painter().galley(
            pos2(body.left() + x, body.center().y - text.size().y * 0.5),
            text,
            ink,
        );
        if dirty {
            ui.painter()
                .circle_filled(pos2(body.left() + 8., body.center().y), 2.7, theme::MINT);
        }
        if cross.hovered() || cross.has_focus() {
            ui.painter()
                .rect_filled(close.shrink(1.), 2., ui.visuals().widgets.hovered.bg_fill);
        }
        let c = close.center();
        let d = vec2(3., 3.);
        let pen = Stroke::new(1.25_f32, ui.visuals().text_color());
        ui.painter().line_segment([c - d, c + d], pen);
        ui.painter().line_segment(
            [
                Pos2::new(c.x - d.x, c.y + d.y),
                Pos2::new(c.x + d.x, c.y - d.y),
            ],
            pen,
        );
        ui.painter().vline(
            rect.right() - 0.5,
            rect.y_range(),
            (1., Color32::from_black_alpha(55)),
        );
        let select = title.clicked();
        let close_tab = cross.clicked();
        title.on_hover_ui(|ui| {
            ui.label(path.map_or_else(|| label.to_owned(), |p| p.display().to_string()));
            if dirty {
                ui.weak("Unsaved changes");
            }
        });
        cross.on_hover_text("Close tab");
        if close_tab {
            Some(TabAction::Close(index))
        } else if select {
            Some(TabAction::Select(index))
        } else {
            None
        }
    }
}
