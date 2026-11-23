#[cfg(test)]
use document_core::Shape;
use document_core::{Document, KeyValue, LineCap, LineJoin, Property, VectorObject};
use eframe::egui::{self, DragValue, Ui};

use crate::canvas::CanvasStyle;
use crate::timeline::KeyMode;
mod layer_tree;
mod motion;
use crate::commands::{self, Arrange};
pub(crate) mod appearance;
mod selection_header;
pub(crate) use selection_header::show as selection_title;
pub(crate) mod arrange_ui;
mod color_row;
mod gradient;
#[cfg(test)]
mod object_list_tests;
mod paint;

fn number(ui: &mut Ui, value: &mut f32, speed: f64, suffix: &str, minimum: Option<f32>) -> bool {
    let mut input = *value;
    let mut field = DragValue::new(&mut input).speed(speed).suffix(suffix);
    if let Some(minimum) = minimum {
        field = field
            .range(minimum..=f32::MAX)
            .clamp_existing_to_range(false);
    }
    if ui.add(field).changed() && input.is_finite() && input != *value {
        *value = input;
        true
    } else {
        false
    }
}

#[derive(
    Clone, Copy, Debug, Default, Hash, PartialEq, Eq, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Tab {
    #[default]
    Properties,
    Layers,
    Rig,
    Library,
    Document,
}

impl Tab {
    pub const COUNT: usize = 5;
    pub const ALL: [Self; Self::COUNT] = [
        Self::Properties,
        Self::Layers,
        Self::Rig,
        Self::Library,
        Self::Document,
    ];
    pub const DEFAULT_VISIBLE: [bool; Self::COUNT] = [true, true, true, true, false];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Properties => "Properties",
            Self::Layers => "Layers",
            Self::Rig => "Rig",
            Self::Library => "Library",
            Self::Document => "Document",
        }
    }

    fn tab_button(&mut self, ui: &mut Ui, tab: Self) {
        let selected = *self == tab;
        let text = egui::RichText::new(tab.label()).size(13.);
        let text = if selected { text.strong() } else { text };
        let response = ui.add(
            egui::Button::new(text)
                .frame(false)
                .min_size(egui::vec2(0., 25.)),
        );
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::SelectableLabel,
                ui.is_enabled(),
                selected,
                tab.label(),
            )
        });
        if response.clicked() {
            *self = tab;
        }
        if *self == tab {
            ui.painter().hline(
                response.rect.x_range(),
                response.rect.bottom(),
                egui::Stroke::new(2.0_f32, ui.visuals().selection.stroke.color),
            );
        }
    }

    pub fn scroll(self) -> egui::ScrollArea {
        // Extremely long layer names or numeric values may require horizontal
        // scrolling. Their minimum width must never push the canvas sideways.
        egui::ScrollArea::both()
            .id_salt(("inspector-scroll", self))
            .auto_shrink([false, false])
    }
}

pub fn show(
    ui: &mut Ui,
    clip: &mut document_core::Clip,
    selected: &mut Vec<u64>,
    frame: u32,
    style: &mut CanvasStyle,
    edit_selection: bool,
    snapshot: &std::sync::Arc<Document>,
) -> Option<crate::canvas::PaintChannel> {
    let id = ui.make_persistent_id("symbol-inspector-tab");
    let mut tab: Tab = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    let reveal = align_shortcut(ui);
    if reveal {
        tab = Tab::Properties;
    }
    ui.horizontal(|ui| {
        for panel in [Tab::Properties, Tab::Layers] {
            tab.tab_button(ui, panel);
        }
    });
    ui.separator();
    ui.data_mut(|d| d.insert_temp(id, tab));
    if tab == Tab::Layers {
        layers(ui, clip, selected, frame, style);
        None
    } else {
        let doc = clip.drawing_at_mut(frame);
        tab.scroll()
            .show(ui, |ui| {
                selection_title(ui, doc, Some(snapshot), selected, frame);
                let picker = properties(ui, doc, selected, frame, style, edit_selection, reveal);
                snapping(ui, doc, style, true);
                picker
            })
            .inner
    }
}

pub(crate) fn properties(
    ui: &mut Ui,
    doc: &mut Document,
    selected: &mut Vec<u64>,
    frame: u32,
    style: &mut CanvasStyle,
    edit_selection: bool,
    reveal: bool,
) -> Option<crate::canvas::PaintChannel> {
    let mut picker = None;
    egui::CollapsingHeader::new(if selected.is_empty() {
        "Tool paint"
    } else {
        "Appearance"
    })
    .default_open(true)
    .show_unindented(ui, |ui| {
        picker = paint::show(ui, doc, selected, style, frame);
    });
    if selected.len() == 1 {
        ui.separator();
        egui::CollapsingHeader::new("Transform")
            .default_open(true)
            .show_unindented(ui, |ui| {
                transform(ui, doc, selected, frame, style.key_mode);
            });
    }
    if !selected.is_empty() {
        ui.separator();
    }
    appearance::show(ui, doc, selected, frame);
    if !selected.is_empty() || reveal {
        let align = egui::CollapsingHeader::new("Align and Distribute")
            .open(reveal.then_some(true))
            .show(ui, |ui| {
                arrange_ui::panel(ui, doc, selected, frame, style.key_mode)
            });
        if reveal {
            align.header_response.scroll_to_me(Some(egui::Align::Min));
        }
        if selected.len() >= 2 {
            crate::path_ops::show(ui, doc, selected, frame);
        }
    }
    if edit_selection && selected.len() == 1 {
        if let Some(index) = doc.objects.iter().position(|o| {
            o.id == selected[0]
                && o.text.is_none()
                && commands::has_vector_paint(o)
                && commands::is_editable(doc, o.layer)
        }) {
            crate::path_editor::show(ui, &mut doc.objects[index], frame, style.key_mode);
        }
    }
    if edit_selection {
        crate::path_editor::split_or_join(ui, doc, selected, frame);
    }
    if let [id] = selected.as_slice() {
        if let Some(object) = doc.objects.iter_mut().find(|o| o.id == *id) {
            if commands::has_vector_paint(object) {
                let previous = (object.cap, object.join);
                ui.collapsing("Paint options", |ui| paint_options(ui, object));
                if object.cap != previous.0 {
                    style.cap = object.cap;
                }
                if object.join != previous.1 {
                    style.join = object.join;
                }
            }
            more_transforms(ui, object, frame, style.key_mode);
        }
    }
    if edit_selection && !selected.is_empty() {
        motion::show(ui, doc, selected, frame, style);
    }
    if !selected.is_empty() {
        ui.collapsing("Arrange", |ui| arrangement(ui, doc, selected, frame));
    }
    picker
}

pub(crate) fn align_shortcut(ui: &mut Ui) -> bool {
    let map = crate::keymap::active(ui.ctx());
    ui.is_enabled()
        && ui.input(|i| i.focused && !i.pointer.any_down())
        && !crate::keymap::widget_owns_shortcuts(ui.ctx())
        && ui.memory(|m| {
            m.top_modal_layer()
                .is_none_or(|modal| modal == ui.layer_id())
        })
        && ui.input_mut(|i| {
            let before = i.events.len();
            i.events.retain(|e| {
                !matches!(e, egui::Event::Key { key, modifiers, pressed: true, .. }
                if map.resolve(*key, *modifiers) == Some(crate::keymap::Action::Align))
            });
            i.events.len() < before
        })
}

pub(crate) fn snapping(ui: &mut Ui, doc: &mut Document, style: &mut CanvasStyle, standalone: bool) {
    ui.collapsing("Snapping and guides", |ui| {
        ui.checkbox(&mut style.snap.enabled, "Enable snapping");
        ui.add_enabled_ui(style.snap.enabled, |ui| {
            if standalone {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut style.snap.grid, "Snap to grid");
                    ui.add_enabled(
                        style.snap.grid,
                        DragValue::new(&mut style.snap.grid_settings.spacing)
                            .range(document_core::GridSettings::SPACING_RANGE)
                            .speed(0.5)
                            .suffix(" px"),
                    );
                });
            }
            ui.checkbox(&mut style.snap.guides, "Guides");
            ui.checkbox(&mut style.snap.bounds, "Object edges and centers");
            ui.checkbox(&mut style.snap.anchors, "Path anchors");
            ui.horizontal(|ui| {
                ui.label("Snap distance");
                ui.add(
                    DragValue::new(&mut style.snap.tolerance)
                        .range(2.0..=24.)
                        .suffix(" screen px"),
                );
            });
        });
        guides(ui, "Vertical", &mut doc.guides_x, doc.width);
        guides(ui, "Horizontal", &mut doc.guides_y, doc.height);
    });
}

fn paint_options(ui: &mut Ui, object: &mut VectorObject) {
    if !object.shape.contours().is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.label("Fill rule");
            ui.selectable_value(
                &mut object.fill_rule,
                document_core::FillRule::NonZero,
                "Nonzero",
            );
            ui.selectable_value(
                &mut object.fill_rule,
                document_core::FillRule::EvenOdd,
                "Even-odd",
            );
        });
    }
    ui.collapsing("Stroke ends and corners", |ui| {
        if !object.stroke_profile.is_empty() {
            ui.weak("Pressure strokes use round caps and joins.");
            return;
        }
        egui::ComboBox::from_label("Cap")
            .selected_text(format!("{:?}", object.cap))
            .show_ui(ui, |ui| {
                for cap in [LineCap::Butt, LineCap::Round, LineCap::Square] {
                    ui.selectable_value(&mut object.cap, cap, format!("{cap:?}"));
                }
            });
        egui::ComboBox::from_label("Join")
            .selected_text(format!("{:?}", object.join))
            .show_ui(ui, |ui| {
                for join in [LineJoin::Miter, LineJoin::Round, LineJoin::Bevel] {
                    let choice = ui.selectable_value(&mut object.join, join, format!("{join:?}"));
                    if join == LineJoin::Miter {
                        choice.on_hover_text("Very sharp corners bevel to prevent long spikes.");
                    }
                }
            });
    });
}

fn opacity(ui: &mut Ui, doc: &mut Document, selected: &[u64], frame: u32, mode: KeyMode) {
    let Some(primary) = selected
        .last()
        .and_then(|id| doc.objects.iter().find(|o| o.id == *id))
    else {
        return;
    };
    let mut percent = primary.opacity_at(frame) * 100.;
    let changed = ui
        .horizontal(|ui| {
            color_row::number_label(ui, "Opacity");
            ui.add(
                DragValue::new(&mut percent)
                    .range(0.0..=100.0)
                    .speed(0.5)
                    .suffix("%"),
            )
            .changed()
        })
        .inner;
    if changed && percent.is_finite() {
        for o in &mut doc.objects {
            if selected.contains(&o.id) && mode.prepare(o, frame, Property::Opacity) {
                o.set_opacity_at(frame, percent / 100.);
            }
        }
    }
}

fn transform(ui: &mut Ui, doc: &mut Document, selected: &[u64], frame: u32, mode: KeyMode) {
    let [id] = selected else {
        return;
    };
    let Some(obj) = doc.objects.iter_mut().find(|o| o.id == *id) else {
        return;
    };
    let previous_position = obj.position_at(frame);
    let mut position = previous_position;
    let mut size = obj.size;
    let mut rotation = obj.rotation_at(frame);
    for (labels, values, min) in [
        (["X", "Y"], &mut position, None),
        (["W", "H"], &mut size, Some(f32::MIN_POSITIVE)),
    ] {
        ui.columns(2, |columns| {
            for ((column, label), value) in columns.iter_mut().zip(labels).zip(values) {
                column.horizontal(|ui| {
                    ui.add_sized(
                        [14., ui.spacing().interact_size.y],
                        egui::Label::new(label).halign(egui::Align::Min),
                    );
                    ui.spacing_mut().interact_size.x = ui.available_width().clamp(38., 120.);
                    number(ui, value, 0.25, " px", min);
                });
            }
        });
    }
    let size_changed = size != obj.size && obj.set_base_size(size);
    ui.horizontal(|ui| {
        ui.label("Rotation");
        if number(ui, &mut rotation, 0.5, "°", None) && mode.prepare(obj, frame, Property::Rotation)
        {
            obj.set_rotation_at(frame, rotation);
        }
    });
    if (position != previous_position
        || size_changed && obj.pivot.is_none() && position != obj.position_at(frame))
        && mode.prepare(obj, frame, Property::Position)
    {
        obj.set_position_at(frame, position);
    }
}
fn more_transforms(ui: &mut Ui, obj: &mut VectorObject, frame: u32, mode: KeyMode) {
    let signed_scale = obj.scale_at(frame);
    let mut scale = signed_scale.map(f32::abs);
    ui.collapsing("Scale and skew", |ui| {
        let mut visible = obj.visible_at(frame);
        if ui.checkbox(&mut visible, "Visible").changed() {
            mode.set(obj, frame, Property::Visibility, KeyValue::Visible(visible));
        }
        egui::Grid::new("scale-fields").show(ui, |ui| {
            for (label, value) in ["Scale X", "Scale Y"].into_iter().zip(&mut scale) {
                ui.label(label);
                number(ui, value, 0.01, "×", Some(f32::MIN_POSITIVE));
                ui.end_row();
            }
            ui.label("Skew");
            let mut skew = obj.skew_at(frame);
            if ui
                .add(
                    DragValue::new(&mut skew)
                        .speed(0.25)
                        .range(-85.0..=85.)
                        .suffix("°"),
                )
                .changed()
            {
                mode.set(obj, frame, Property::Skew, KeyValue::Scalar(skew));
            }
            ui.end_row();
        });
    });
    if scale != signed_scale.map(f32::abs) && mode.prepare(obj, frame, Property::Scale) {
        obj.set_scale_at(
            frame,
            std::array::from_fn(|i| scale[i].copysign(signed_scale[i])),
        );
    }
}

pub(crate) use layer_tree::{finish_rename_on_blur, layers};

fn arrangement(ui: &mut Ui, doc: &mut Document, selected: &mut Vec<u64>, frame: u32) {
    ui.add_enabled_ui(!selected.is_empty(), |ui| {
        ui.horizontal_wrapped(|ui| {
            let grouping = commands::grouping_options(doc, selected);
            if ui
                .add_enabled(grouping.can_group, egui::Button::new("Group"))
                .on_hover_text("Group two or more objects on the same layer")
                .clicked()
            {
                commands::GroupAction::Group.apply(doc, selected);
            }
            if ui
                .add_enabled(grouping.can_ungroup, egui::Button::new("Ungroup"))
                .clicked()
            {
                commands::GroupAction::Ungroup.apply(doc, selected);
            }
            if ui
                .button("To front")
                .on_hover_text("Bring to front (Home)")
                .clicked()
            {
                commands::arrange(doc, selected, frame, Arrange::ToFront);
            }
            if ui
                .button("To back")
                .on_hover_text("Send to back (End)")
                .clicked()
            {
                commands::arrange(doc, selected, frame, Arrange::ToBack);
            }
        });
    });
}

fn guides(ui: &mut Ui, axis: &str, positions: &mut Vec<f32>, extent: u32) {
    ui.push_id(axis, |ui| {
        if ui.small_button(format!("+ {axis} guide")).clicked() {
            positions.push(extent as f32 / 2.0);
        }
        let mut remove = None;
        for (index, position) in positions.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.label(axis);
                number(ui, position, 0.5, " px", None);
                if ui.small_button("Remove").clicked() {
                    remove = Some(index);
                }
            });
        }
        if let Some(index) = remove {
            positions.remove(index);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspecting_imported_geometry_preserves_subpixel_dimensions() {
        let mut doc = Document::default();
        let id = doc.add_rectangle([0., 0.], [0.002, 1_000_001.], [255; 4]);
        let original = doc.clone();
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                transform(ui, &mut doc, &[id], 0, KeyMode::default())
            });
        });
        assert_eq!(doc, original);
    }
}
