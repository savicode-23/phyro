use super::{appearance, number};
use crate::canvas::PaintChannel;
use document_core::{Gradient, GradientKind, GradientStop};
use eframe::egui::{self, Ui};
use std::borrow::Cow;

#[derive(Clone, Copy, PartialEq)]
pub(super) enum PaintType {
    None,
    Solid,
    Linear,
    Radial,
    Rectangular,
}

impl PaintType {
    pub(super) fn of(gradient: Option<&Gradient>) -> Self {
        match gradient.map(|g| &g.kind) {
            None => Self::Solid,
            Some(GradientKind::Linear { .. }) => Self::Linear,
            Some(GradientKind::Radial { .. }) => Self::Radial,
            Some(GradientKind::Rectangular { .. }) => Self::Rectangular,
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::None => "No paint",
            Self::Solid => "Solid",
            Self::Linear => "Linear",
            Self::Radial => "Radial",
            Self::Rectangular => "Rectangular",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Self::None => "Remove paint from this channel",
            Self::Solid => "One uniform color",
            Self::Linear => "Fade in a straight line",
            Self::Radial => "Fade from the center in circles or ellipses",
            Self::Rectangular => "Fade from the center in nested rectangles",
        }
    }

    fn kind(self, previous: Option<&Gradient>) -> Option<GradientKind> {
        let (center, radius) = match previous.map(|g| &g.kind) {
            Some(GradientKind::Linear { start, end }) => (
                [
                    f32::midpoint(start[0], end[0]),
                    f32::midpoint(start[1], end[1]),
                ],
                (end[0] * 0.5 - start[0] * 0.5).hypot(end[1] * 0.5 - start[1] * 0.5),
            ),
            Some(
                GradientKind::Radial { center, radius }
                | GradientKind::Rectangular { center, radius },
            ) => (*center, *radius),
            None => ([0.5; 2], 0.5),
        };
        match self {
            Self::None | Self::Solid => None,
            Self::Linear => Some(GradientKind::Linear {
                start: [center[0] - radius, center[1]],
                end: [center[0] + radius, center[1]],
            }),
            Self::Radial => Some(GradientKind::Radial { center, radius }),
            Self::Rectangular => Some(GradientKind::Rectangular { center, radius }),
        }
    }
}

pub(super) enum Edit {
    Restore,
    Type(PaintType),
    StopsOrGeometry(Gradient),
}

impl Edit {
    pub(super) fn apply(&self, old: Option<&Gradient>, color: [u8; 4]) -> Option<Gradient> {
        match self {
            Self::Restore => old.cloned(),
            Self::StopsOrGeometry(gradient) => Some(gradient.clone()),
            Self::Type(fill_type) => {
                if *fill_type == PaintType::None || PaintType::of(old) == *fill_type {
                    return old.cloned();
                }
                let kind = fill_type.kind(old)?;
                let Some(old) = old else {
                    return Some(Gradient::from_color(kind, color));
                };
                let mut gradient = Gradient {
                    kind,
                    stops: old.stops.clone(),
                };
                if !gradient.valid() {
                    // Imported geometry near floating-point limits can collapse on conversion.
                    gradient.kind = fill_type.kind(None)?;
                }
                Some(gradient)
            }
        }
    }
}

pub(super) fn type_menu(
    ui: &mut Ui,
    channel: PaintChannel,
    current: PaintType,
    multiple: bool,
    width: f32,
) -> (egui::Response, Option<Edit>) {
    let mut choice = None;
    let menu = egui::ComboBox::from_id_salt("paint-type")
        .width(width)
        .selected_text(channel.name())
        .show_ui(ui, |ui| {
            for mode in [
                PaintType::None,
                PaintType::Solid,
                PaintType::Linear,
                PaintType::Radial,
                PaintType::Rectangular,
            ] {
                let active = !multiple && mode == current;
                if ui
                    .selectable_label(active, mode.label())
                    .on_hover_text(mode.hint())
                    .clicked()
                    && !active
                {
                    choice = Some(Edit::Type(mode));
                }
            }
        });
    let response = menu.response.on_hover_text(format!(
        "Change {} paint type",
        channel.name().to_lowercase()
    ));
    (response, choice)
}

pub(super) fn controls(
    ui: &mut Ui,
    channel: PaintChannel,
    gradient: &Gradient,
    multiple: bool,
) -> Option<Edit> {
    if multiple {
        ui.weak("Select one object to edit its gradient stops.");
        return None;
    }
    let mut edited = Cow::Borrowed(gradient);
    let add = ui
        .horizontal(|ui| {
            let label = match channel {
                PaintChannel::Fill => "Fill gradient",
                PaintChannel::Stroke => "Stroke gradient",
            };
            ui.strong(label).on_hover_text(crate::keymap::hint(
                ui.ctx(),
                "Drag gradient handles on the canvas",
                crate::keymap::Action::GradientTool,
            ));
            ui.add_enabled(
                gradient.stops.len() < 256,
                egui::Button::new("Add stop").small(),
            )
            .clicked()
        })
        .inner;
    for (i, original) in gradient.stops.iter().enumerate() {
        let mut stop = *original;
        let mut remove = false;
        ui.push_id(("gradient-stop", i), |ui| {
            ui.horizontal(|ui| {
                let swatch = crate::color_picker::button(ui, &mut stop.color);
                swatch.on_hover_text(format!("Color of stop {}", i + 1));
                let mut percent = stop.offset * 100.;
                let min = i.checked_sub(1).map_or(0., |p| gradient.stops[p].offset);
                let max = gradient.stops.get(i + 1).map_or(1., |next| next.offset);
                if ui
                    .add(
                        egui::DragValue::new(&mut percent)
                            .speed(0.5)
                            .range(min * 100. ..=max * 100.)
                            .suffix("%"),
                    )
                    .on_hover_text("Stop position")
                    .changed()
                {
                    stop.offset = (percent / 100.).clamp(min, max);
                }
                remove = gradient.stops.len() > 2
                    && ui.small_button("−").on_hover_text("Remove stop").clicked();
            });
        });
        if remove {
            edited.to_mut().stops.remove(i);
            break;
        }
        if stop != *original {
            edited.to_mut().stops[i] = stop;
        }
    }
    if add {
        let stops = &mut edited.to_mut().stops;
        let i = stops
            .windows(2)
            .enumerate()
            .max_by(|(_, a), (_, b)| {
                (a[1].offset - a[0].offset).total_cmp(&(b[1].offset - b[0].offset))
            })
            .unwrap()
            .0;
        let (left, right) = (stops[i], stops[i + 1]);
        stops.insert(
            i + 1,
            GradientStop {
                offset: f32::midpoint(left.offset, right.offset),
                color: std::array::from_fn(|i| u8::midpoint(left.color[i], right.color[i])),
            },
        );
    }
    ui.collapsing("Gradient geometry", |ui| {
        let mut kind = gradient.kind.clone();
        match &mut kind {
            GradientKind::Linear { start, end } => {
                appearance::pair(ui, "Start", start, None);
                appearance::pair(ui, "End", end, None);
            }
            GradientKind::Radial { center, radius }
            | GradientKind::Rectangular { center, radius } => {
                appearance::pair(ui, "Center", center, None);
                ui.horizontal(|ui| {
                    ui.label("Radius");
                    number(ui, radius, 0.01, "", Some(0.001));
                });
            }
        }
        if kind != gradient.kind {
            edited.to_mut().kind = kind;
        }
    });
    match edited {
        Cow::Owned(g) if g.valid() => Some(Edit::StopsOrGeometry(g)),
        _ => None,
    }
}
