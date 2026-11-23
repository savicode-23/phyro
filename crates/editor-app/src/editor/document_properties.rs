use super::Editor;
use document_core::{GridKind, GridSettings, GridVisibility};
use eframe::egui::{self, DragValue, Ui};
use std::ops::RangeInclusive;

impl Editor {
    pub(super) fn document_properties(&mut self, ui: &mut Ui) {
        ui.heading("Document Properties");
        ui.horizontal_wrapped(|ui| {
            ui.label(format!(
                "Artboard  {} × {} px",
                self.scene().width,
                self.scene().height
            ));
            if ui.button("Resize…").clicked() {
                self.open_artboard_size();
            }
        });
        ui.separator();

        ui.heading("Grid");
        let mut grid = self.project.grid;
        egui::Grid::new("document-grid-settings")
            .num_columns(2)
            .show(ui, |ui| {
                let kind_label = |kind| match kind {
                    GridKind::Rectangular => "Rectangular",
                    GridKind::Axonometric => "Axonometric",
                };
                let label = ui.label("Type");
                egui::ComboBox::from_id_salt("grid-type")
                    .width(114.)
                    .selected_text(kind_label(grid.kind))
                    .show_ui(ui, |ui| {
                        for kind in [GridKind::Rectangular, GridKind::Axonometric] {
                            ui.selectable_value(&mut grid.kind, kind, kind_label(kind));
                        }
                    })
                    .response
                    .labelled_by(label.id);
                ui.end_row();

                if self.project.grid == GridSettings::default()
                    && grid.kind == GridKind::Axonometric
                {
                    grid.spacing = 32.;
                    grid.visibility = GridVisibility::Visible;
                }

                let visibility_label = |visibility| match visibility {
                    GridVisibility::WhenSnapping => "When snapping",
                    GridVisibility::Visible => "Always show",
                    GridVisibility::Hidden => "Hidden",
                };
                let label = ui.label("Visibility");
                egui::ComboBox::from_id_salt("grid-visibility")
                    .width(114.)
                    .selected_text(visibility_label(grid.visibility))
                    .show_ui(ui, |ui| {
                        for visibility in [
                            GridVisibility::WhenSnapping,
                            GridVisibility::Visible,
                            GridVisibility::Hidden,
                        ] {
                            ui.selectable_value(
                                &mut grid.visibility,
                                visibility,
                                visibility_label(visibility),
                            );
                        }
                    })
                    .response
                    .labelled_by(label.id);
                ui.end_row();

                let spacing_label = if grid.kind == GridKind::Axonometric {
                    "Y spacing"
                } else {
                    "Spacing"
                };
                number(
                    ui,
                    spacing_label,
                    &mut grid.spacing,
                    GridSettings::SPACING_RANGE,
                    " px",
                );
                number(
                    ui,
                    "Origin X",
                    &mut grid.origin[0],
                    GridSettings::ORIGIN_RANGE,
                    " px",
                );
                number(
                    ui,
                    "Origin Y",
                    &mut grid.origin[1],
                    GridSettings::ORIGIN_RANGE,
                    " px",
                );
                if grid.kind == GridKind::Axonometric {
                    number(
                        ui,
                        "X angle",
                        &mut grid.angle_x,
                        GridSettings::ANGLE_RANGE,
                        "°",
                    );
                    number(
                        ui,
                        "Z angle",
                        &mut grid.angle_z,
                        GridSettings::ANGLE_RANGE,
                        "°",
                    );
                }
            });
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut self.style.snap.grid, "Snap to grid")
                .on_hover_text("The Snap toggle in the toolbar enables snapping.");
            if ui
                .add_enabled(
                    grid != GridSettings::default(),
                    egui::Button::new("Reset grid"),
                )
                .clicked()
            {
                grid = GridSettings::default();
            }
        });
        if grid != self.project.grid && grid.validate().is_ok() {
            self.project.grid = grid;
        }
    }
}

fn number(ui: &mut Ui, label: &str, value: &mut f32, range: RangeInclusive<f32>, unit: &str) {
    let label = ui.label(label);
    ui.add(DragValue::new(value).range(range).speed(0.5).suffix(unit))
        .labelled_by(label.id);
    ui.end_row();
}
