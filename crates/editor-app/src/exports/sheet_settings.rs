use super::Exports;
use document_core::AnimationProject;
use eframe::egui;

impl Exports {
    pub(super) fn sheet_settings(
        &mut self,
        ui: &mut egui::Ui,
        project: &mut AnimationProject,
        frames: u32,
    ) {
        ui.horizontal_wrapped(|ui| {
            ui.strong("Sheet layout");
            ui.menu_button("Presets…", |ui| {
                for (name, columns) in [
                    ("Horizontal strip", frames.clamp(1, 4096)),
                    ("Four columns", 4),
                    ("Eight columns", 8),
                ] {
                    if ui.button(name).clicked() {
                        project.preset.columns = columns;
                        project.preset.packed.enabled = false;
                        self.preset_name.clear();
                        ui.close_menu();
                    }
                }
                if !project.presets.is_empty() {
                    ui.separator();
                }
                for (name, preset) in &project.presets {
                    if ui.button(name).clicked() {
                        project.preset = *preset;
                        self.preset_name = name.clone();
                        ui.close_menu();
                    }
                }
                ui.separator();
                ui.label("Project preset name");
                ui.text_edit_singleline(&mut self.preset_name);
                let name = self.preset_name.trim();
                let exists = project.presets.contains_key(name);
                let valid = !name.is_empty()
                    && name.chars().count() <= 80
                    && (exists || project.presets.len() < 128);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            valid,
                            egui::Button::new(if exists {
                                "Update preset"
                            } else {
                                "Save preset"
                            }),
                        )
                        .clicked()
                    {
                        self.changed_animation_options |=
                            project.presets.insert(name.into(), project.preset)
                                != Some(project.preset);
                        ui.close_menu();
                    }
                    if ui
                        .add_enabled(exists, egui::Button::new("Remove preset"))
                        .clicked()
                    {
                        self.changed_animation_options |= project.presets.remove(name).is_some();
                        ui.close_menu();
                    }
                });
            });
        });
        let p = &mut project.preset;
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut p.packed.enabled, false, "Uniform grid");
            ui.selectable_value(&mut p.packed.enabled, true, "Packed atlas");
        });
        egui::Grid::new("sheet-layout-fields")
            .num_columns(2)
            .show(ui, |ui| {
                let mut field = |label: &str, value: &mut u32, limits, suffix| {
                    let label = ui.label(label);
                    ui.add(egui::DragValue::new(value).range(limits).suffix(suffix))
                        .labelled_by(label.id);
                    ui.end_row();
                };
                if p.packed.enabled {
                    field("Maximum side", &mut p.packed.max_size, 1..=4096, " px");
                    field("Extrude edges", &mut p.packed.extrude, 0..=32, " px");
                } else {
                    field("Columns", &mut p.columns, 1..=4096, "");
                }
                field("Padding", &mut p.padding, 0..=4096, " px");
                field("Margin", &mut p.margin, 0..=4096, " px");
            });
        if p.packed.enabled {
            ui.checkbox(&mut p.packed.trim, "Trim transparency");
            ui.checkbox(&mut p.packed.deduplicate, "Share identical frames");
            ui.checkbox(&mut p.packed.allow_rotation, "Allow 90° rotation");
        }
    }
}
