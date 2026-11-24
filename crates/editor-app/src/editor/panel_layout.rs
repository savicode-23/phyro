use super::Editor;
use crate::diagnostics::Action;
use crate::inspector::Tab;
use eframe::egui;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Left,
    Right,
}

impl Side {
    fn slots(self) -> [usize; 2] {
        if self == Self::Left {
            [0, 1]
        } else {
            [2, 3]
        }
    }

    fn panel_id(self) -> &'static str {
        if self == Self::Left {
            "inspector-left"
        } else {
            "inspector"
        }
    }
}

const SLOT_LABELS: [&str; 4] = ["Upper left", "Lower left", "Upper right", "Lower right"];

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Group {
    tabs: Vec<Tab>,
    active: Tab,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct PanelLayout {
    groups: [Group; 4],
    widths: [f32; 2],
    splits: [f32; 2],
}

impl Default for PanelLayout {
    fn default() -> Self {
        let mut groups: [Group; 4] = Default::default();
        groups[2].tabs = Tab::ALL.to_vec();
        Self {
            groups,
            widths: [256.; 2],
            splits: [0.5; 2],
        }
    }
}

impl PanelLayout {
    pub(super) fn normalize(&mut self) {
        let mut seen = [false; Tab::COUNT];
        for g in &mut self.groups {
            g.tabs
                .retain(|t| !std::mem::replace(&mut seen[*t as usize], true));
        }
        self.groups[2]
            .tabs
            .extend(Tab::ALL.into_iter().filter(|t| !seen[*t as usize]));
        for g in &mut self.groups {
            if !g.tabs.contains(&g.active) {
                g.active = g.tabs.first().copied().unwrap_or_default();
            }
        }
        self.widths = self.widths.map(|w| {
            if w.is_finite() {
                w.clamp(180., 2000.)
            } else {
                256.
            }
        });
        self.splits = self.splits.map(|s| {
            if s.is_finite() {
                s.clamp(0.1, 0.9)
            } else {
                0.5
            }
        });
    }

    fn move_tab(&mut self, tab: Tab, slot: usize, before: Option<Tab>) {
        if before == Some(tab) {
            return;
        }
        for group in &mut self.groups {
            group.tabs.retain(|&item| item != tab);
            if group.active == tab {
                group.active = group.tabs.first().copied().unwrap_or_default();
            }
        }
        let group = &mut self.groups[slot];
        let i = before
            .and_then(|b| group.tabs.iter().position(|&t| t == b))
            .unwrap_or(group.tabs.len());
        group.tabs.insert(i, tab);
        group.active = tab;
    }

    fn activate(&mut self, tab: Tab) {
        if let Some(g) = self.groups.iter_mut().find(|g| g.tabs.contains(&tab)) {
            g.active = tab;
        }
    }

    fn active(&self, slot: usize, visible: &[bool; Tab::COUNT]) -> Option<Tab> {
        let g = &self.groups[slot];
        g.tabs
            .iter()
            .copied()
            .find(|&t| t == g.active && visible[t as usize])
            .or_else(|| g.tabs.iter().copied().find(|t| visible[*t as usize]))
    }

    pub(super) fn stacked() -> Self {
        let mut layout = Self::default();
        layout.move_tab(Tab::Layers, 3, None);
        layout.activate(Tab::Properties);
        layout
    }
}

#[derive(Clone, Copy)]
enum LayoutAction {
    Select(Tab),
    Move {
        tab: Tab,
        slot: usize,
        before: Option<Tab>,
    },
    Hide(Tab),
}

fn placement_menu(ui: &mut egui::Ui, tab: Tab, current: usize, action: &mut Option<LayoutAction>) {
    for (slot, label) in SLOT_LABELS.into_iter().enumerate() {
        if ui.selectable_label(current == slot, label).clicked() {
            *action = Some(LayoutAction::Move {
                tab,
                slot,
                before: None,
            });
            ui.close_menu();
        }
    }
    ui.separator();
    if ui.button("Hide panel").clicked() {
        *action = Some(LayoutAction::Hide(tab));
        ui.close_menu();
    }
}

fn header(
    ui: &mut egui::Ui,
    slot: usize,
    active: Tab,
    tabs: &[Tab],
    visible: &[bool; Tab::COUNT],
) -> Option<LayoutAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        let enabled = tabs.iter().copied().filter(|t| visible[*t as usize]);
        let font = egui::FontId::proportional(13.);
        let text_width = enabled
            .clone()
            .map(|t| {
                ui.fonts(|f| {
                    f.layout_no_wrap(t.label().into(), font.clone(), egui::Color32::WHITE)
                        .size()
                        .x
                })
            })
            .sum::<f32>();
        let free = ui.available_width() - text_width - 28.;
        let gaps = enabled.clone().count().saturating_sub(1) as f32;
        if free >= 4. * gaps {
            ui.spacing_mut().item_spacing.x = (free / gaps.max(1.)).clamp(4., 10.);
            for tab in enabled {
                let text = egui::RichText::new(tab.label()).size(13.);
                let text = if tab == active { text.strong() } else { text };
                let response = ui.add(
                    egui::Button::new(text)
                        .frame(false)
                        .min_size(egui::vec2(0., 25.))
                        .sense(egui::Sense::click_and_drag()),
                );
                response.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::WidgetType::SelectableLabel,
                        ui.is_enabled(),
                        active == tab,
                        tab.label(),
                    )
                });
                if response.clicked() {
                    action = Some(LayoutAction::Select(tab));
                }
                if active == tab {
                    ui.painter().hline(
                        response.rect.x_range(),
                        response.rect.bottom(),
                        egui::Stroke::new(2_f32, ui.visuals().selection.stroke.color),
                    );
                }
                response.dnd_set_drag_payload(tab);
                let insert_after = ui
                    .ctx()
                    .pointer_hover_pos()
                    .is_some_and(|p| p.x > response.rect.center().x);
                if response.dnd_hover_payload::<Tab>().is_some() {
                    ui.painter().vline(
                        if insert_after {
                            response.rect.right()
                        } else {
                            response.rect.left()
                        },
                        response.rect.y_range(),
                        egui::Stroke::new(2_f32, ui.visuals().selection.stroke.color),
                    );
                }
                if let Some(moved) = response.dnd_release_payload::<Tab>() {
                    action = Some(LayoutAction::Move {
                        tab: *moved,
                        slot,
                        before: if insert_after {
                            tabs.iter().skip_while(|&&t| t != tab).nth(1).copied()
                        } else {
                            Some(tab)
                        },
                    });
                }
                response.context_menu(|ui| placement_menu(ui, tab, slot, &mut action));
            }
        } else {
            egui::ComboBox::from_id_salt("active-panel")
                .width((ui.available_width() - 26.).min(140.))
                .selected_text(active.label())
                .show_ui(ui, |ui| {
                    for tab in enabled {
                        if ui.selectable_label(active == tab, tab.label()).clicked() {
                            action = Some(LayoutAction::Select(tab));
                        }
                    }
                });
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.menu_button("…", |ui| placement_menu(ui, active, slot, &mut action))
                .response
                .on_hover_text("Move or hide this panel");
        });
    });
    ui.separator();
    action
}

impl Editor {
    pub(super) fn panel_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Panels", |ui| {
            for tab in Tab::ALL {
                let label = match tab {
                    Tab::Library => "Artwork library",
                    Tab::Document => "Document Properties",
                    _ => tab.label(),
                };
                if ui
                    .checkbox(&mut self.visible_panels[tab as usize], label)
                    .changed()
                {
                    if self.visible_panels[tab as usize] {
                        self.show_inspector = true;
                        self.inspector_tab = tab;
                        ui.close_menu();
                    } else if self.inspector_tab == tab {
                        self.inspector_tab = Tab::ALL
                            .into_iter()
                            .find(|t| self.visible_panels[*t as usize])
                            .unwrap_or_default();
                    }
                }
            }
            ui.separator();
            if ui.button("Layers and Properties together").clicked() {
                self.panel_layout = PanelLayout::stacked();
                self.visible_panels[Tab::Layers as usize] = true;
                self.visible_panels[Tab::Properties as usize] = true;
                self.inspector_tab = Tab::Properties;
                self.show_inspector = true;
                clear_sizes(ui.ctx());
                ui.close_menu();
            }
            if ui.button("Group panels into tabs").clicked() {
                self.panel_layout = Default::default();
                self.panel_layout.activate(self.inspector_tab);
                self.show_inspector = true;
                clear_sizes(ui.ctx());
                ui.close_menu();
            }
            if ui.button("Reset panel layout").clicked() {
                self.panel_layout = PanelLayout::stacked();
                self.inspector_tab = Tab::Properties;
                self.visible_panels = Tab::DEFAULT_VISIBLE;
                self.show_inspector = true;
                clear_sizes(ui.ctx());
                ui.close_menu();
            }
        });
    }

    fn layout_action(&mut self, action: LayoutAction) {
        match action {
            LayoutAction::Select(tab) => {
                if tab == Tab::Library && self.inspector_tab != tab {
                    self.note(Action::Library);
                }
                self.inspector_tab = tab;
                self.panel_layout.activate(tab);
            }
            LayoutAction::Move { tab, slot, before } => {
                self.panel_layout.move_tab(tab, slot, before);
                self.inspector_tab = tab;
            }
            LayoutAction::Hide(tab) => {
                self.visible_panels[tab as usize] = false;
                if self.inspector_tab == tab {
                    self.inspector_tab = Tab::ALL
                        .into_iter()
                        .find(|t| self.visible_panels[*t as usize])
                        .unwrap_or_default();
                }
            }
        }
    }

    pub(super) fn dock_panels(&mut self, ctx: &egui::Context, correct_text: bool, reveal: bool) {
        if reveal {
            self.inspector_tab = Tab::Properties;
            self.visible_panels[Tab::Properties as usize] = true;
        }
        self.panel_layout.activate(self.inspector_tab);
        let previous = self.selection.clone();
        let mut next = None;
        let right_visible = Side::Right.slots().into_iter().any(|slot| {
            self.panel_layout
                .active(slot, &self.visible_panels)
                .is_some()
        });
        let all_hidden = !self.visible_panels.into_iter().any(|visible| visible);
        for side in [Side::Left, Side::Right] {
            let slots = side.slots();
            let active = slots.map(|slot| self.panel_layout.active(slot, &self.visible_panels));
            if correct_text && side == Side::Left {
                continue;
            }
            if active == [None, None] && !(side == Side::Right && (correct_text || all_hidden)) {
                continue;
            }
            let index = side as usize;
            let reserve_other = if side == Side::Left && right_visible {
                180.
            } else {
                0.
            };
            let limit = (ctx.available_rect().width() - 200. - reserve_other).max(180.);
            let dock = match side {
                Side::Left => egui::SidePanel::left(side.panel_id()),
                Side::Right => egui::SidePanel::right(side.panel_id()),
            };
            let result = dock
                .default_width(self.panel_layout.widths[index])
                .width_range(180.0..=limit)
                .resizable(true)
                .show(ctx, |ui| {
                    ui.set_min_width(ui.max_rect().width());
                    self.timeline.leave_if_pressed(ui);
                    if correct_text {
                        self.canvas.text_controls(ui);
                        return;
                    }
                    let available = ui.available_rect_before_wrap();
                    let mut rects = [available; 2];
                    if active.iter().all(Option::is_some) {
                        let height = available.height().max(1.);
                        let grip = 6.;
                        let min = (height * 0.4).clamp(grip, 80.);
                        let mid = (self.panel_layout.splits[index] * height)
                            .clamp(min, height.max(2. * min) - min);
                        let y = available.top() + mid;
                        let hit = egui::Rect::from_x_y_ranges(
                            available.x_range(),
                            (y - grip / 2.)..=(y + grip / 2.),
                        );
                        let response = ui
                            .interact(
                                hit,
                                ui.make_persistent_id("stack-splitter"),
                                egui::Sense::drag(),
                            )
                            .on_hover_cursor(egui::CursorIcon::ResizeVertical);
                        if response.dragged() {
                            if let Some(point) = response.interact_pointer_pos() {
                                self.panel_layout.splits[index] = ((point.y - available.top())
                                    .clamp(min, height.max(2. * min) - min)
                                    / height)
                                    .clamp(0.1, 0.9);
                                ui.ctx().request_repaint();
                            }
                        }
                        rects[0].max.y = (y - grip / 2.).min(available.bottom());
                        rects[1].min.y = (y + grip / 2.).min(available.bottom());
                        ui.painter().hline(
                            hit.x_range(),
                            y,
                            if response.hovered() || response.dragged() {
                                ui.visuals().selection.stroke
                            } else {
                                ui.visuals().widgets.noninteractive.bg_stroke
                            },
                        );
                    }
                    for (row, tab) in active.into_iter().enumerate() {
                        let Some(tab) = tab else { continue };
                        let mut group = ui.new_child(
                            egui::UiBuilder::new()
                                .id_salt(("panel-group", slots[row]))
                                .max_rect(rects[row])
                                .layout(egui::Layout::top_down(egui::Align::Min)),
                        );
                        group.set_clip_rect(ui.clip_rect().intersect(rects[row]));
                        if let Some(action) = header(
                            &mut group,
                            slots[row],
                            tab,
                            &self.panel_layout.groups[slots[row]].tabs,
                            &self.visible_panels,
                        ) {
                            next = Some(action);
                        }
                        group.add_enabled_ui(!self.timeline.is_playing(), |ui| {
                            if tab == Tab::Layers
                                || (tab == Tab::Library && self.library.folders_active())
                            {
                                self.inspector_body(ui, tab, false);
                            } else {
                                tab.scroll(ui).show(ui, |ui| {
                                    self.inspector_body(ui, tab, reveal && tab == Tab::Properties)
                                });
                            }
                        });
                    }
                });
            if result.response.rect.width() < limit || self.panel_layout.widths[index] <= limit {
                self.panel_layout.widths[index] = result.response.rect.width();
            }
        }
        if let Some(action) = next.or_else(|| dock_targets(ctx)) {
            self.layout_action(action);
            ctx.request_repaint();
        }
        if !(0..4).any(|s| self.panel_layout.active(s, &self.visible_panels) == Some(Tab::Rig)) {
            self.canvas.rig.leave_inspector();
        }
        if previous != self.selection {
            self.canvas.cancel_drag();
            self.tidy_selection();
            if !self.selection.is_empty() {
                self.tool = crate::canvas::Tool::Select;
            }
            self.sync_paint();
        }
    }
}

/// Empty dock positions stay available while dragging, without reserving canvas space at rest.
fn dock_targets(ctx: &egui::Context) -> Option<LayoutAction> {
    let tab = egui::DragAndDrop::payload::<Tab>(ctx)?;
    let area = ctx.available_rect().shrink(8.);
    if area.width() < 130. || area.height() < 62. {
        return None;
    }
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("panel-drop-targets"),
    ));
    let style = ctx.style();
    let visuals = &style.visuals;
    let (hover, released) = ctx.input(|i| (i.pointer.interact_pos(), i.pointer.primary_released()));
    let size = egui::vec2((area.width() * 0.4).min(94.), 26.);
    let mut result = None;
    for (slot, corner) in [
        area.left_top(),
        area.left_bottom(),
        area.right_top(),
        area.right_bottom(),
    ]
    .into_iter()
    .enumerate()
    {
        let rect = egui::Rect::from_min_size(
            egui::pos2(
                if slot < 2 {
                    corner.x
                } else {
                    corner.x - size.x
                },
                if slot % 2 == 0 {
                    corner.y
                } else {
                    corner.y - size.y
                },
            ),
            size,
        );
        let over = hover.is_some_and(|p| rect.contains(p));
        painter.rect(
            rect,
            4.,
            if over {
                visuals.selection.bg_fill
            } else {
                visuals.window_fill()
            },
            visuals.selection.stroke,
            egui::StrokeKind::Inside,
        );
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            SLOT_LABELS[slot],
            egui::FontId::proportional(12.),
            visuals.text_color(),
        );
        if over && released {
            egui::DragAndDrop::take_payload::<Tab>(ctx);
            result = Some(LayoutAction::Move {
                tab: *tab,
                slot,
                before: None,
            });
        }
    }
    result
}

fn clear_sizes(ctx: &egui::Context) {
    ctx.data_mut(|d| {
        for side in [Side::Left, Side::Right] {
            d.remove::<egui::containers::panel::PanelState>(egui::Id::new(side.panel_id()));
        }
    });
}
