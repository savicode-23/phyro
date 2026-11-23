use super::{toolbar::Icon, unique_name, Timeline};
use crate::library::{AssetDrop, AssetSource};
use document_core::{AnimationProject, Clip, DEFAULT_ANIMATION_SET_ID};
use eframe::egui::{self, Ui};

fn clip_row(ui: &mut Ui, source: &Clip, active: bool, list_row: bool) -> egui::Response {
    let sense = if active {
        egui::Sense::click()
    } else {
        egui::Sense::click_and_drag()
    };
    let button = egui::Button::new(&source.name)
        .selected(active)
        .truncate()
        .sense(sense);
    let response = if list_row {
        ui.add_sized([ui.available_width(), ui.spacing().interact_size.y], button)
    } else {
        ui.add(button.frame(false))
    };
    if active {
        return response.on_hover_text(&source.name);
    }
    if response.drag_started() {
        response.dnd_set_drag_payload(AssetDrop {
            source: AssetSource::ProjectClip {
                id: source.id,
                session: source.session_id(),
            },
            label: source.name.clone(),
            thumbnail: None,
        });
        if !list_row {
            ui.memory_mut(|m| m.close_popup());
        }
    }
    response.on_hover_ui(|ui| {
        ui.strong(&source.name);
        ui.label("Click to open. Drag onto the canvas to place a linked clip.");
    })
}

#[derive(Clone, Copy)]
enum Action {
    NewSet,
    RenameSet(u64),
    RemoveSet(u64),
    NewClip(u64),
    RenameClip(u64),
    DeleteClip(u64),
}

pub(super) struct Dialog {
    action: Action,
    name: String,
    focus: bool,
    error: Option<String>,
}

impl Dialog {
    fn new(action: Action, name: String) -> Self {
        Self {
            action,
            name,
            focus: true,
            error: None,
        }
    }

    fn apply(&self, project: &mut AnimationProject) -> Result<Option<usize>, String> {
        let name = self.name.trim();
        match self.action {
            Action::NewSet => {
                let mut draft = project.clone();
                let set = draft.add_animation_set(name)?;
                let clip = draft.add_clip_in_set(set, "Animation")?;
                *project = draft;
                Ok(Some(clip))
            }
            Action::RenameSet(set) => project.rename_animation_set(set, name).map(|_| None),
            Action::RemoveSet(set) => project.remove_animation_set(set).map(|_| None),
            Action::NewClip(set) => project.add_clip_in_set(set, name).map(Some),
            Action::RenameClip(id) | Action::DeleteClip(id) => {
                let index = project.clip_index(id).ok_or("Unknown clip.")?;
                if matches!(self.action, Action::RenameClip(_)) {
                    project.rename_clip(index, name)?;
                    return Ok(None);
                }
                let set = project.clips[index].set_id;
                project.remove_clip(index)?;
                let next = project
                    .clips
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| c.set_id == set)
                    .min_by_key(|(i, _)| i.abs_diff(index))
                    .map(|(i, _)| i)
                    .unwrap_or_else(|| index.min(project.clips.len() - 1));
                Ok(Some(next))
            }
        }
    }
}

impl Timeline {
    pub(super) fn navigator(&mut self, ui: &mut Ui, project: &mut AnimationProject) -> bool {
        let mut changed = false;
        if ui.available_width() >= 940. && ui.available_height() >= 128. {
            egui::SidePanel::left("timeline-clip-list")
                .exact_width(174.)
                .resizable(false)
                .show_inside(ui, |ui| {
                    self.set_menu(ui, project);
                    ui.horizontal(|ui| {
                        self.set_picker(ui, project);
                        self.export_set_button(ui, project.clips[self.clip].set_id);
                    });
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.;
                        ui.strong("Clips");
                        changed |= self.clip_actions(ui, project);
                    });
                    let set = project.clips[self.clip].set_id;
                    let indices: Vec<_> = project
                        .clips
                        .iter()
                        .enumerate()
                        .filter_map(|(i, clip)| (clip.set_id == set).then_some(i))
                        .collect();
                    let row = ui.spacing().interact_size.y;
                    egui::ScrollArea::vertical()
                        .id_salt(("set-clips", set))
                        .auto_shrink([false, false])
                        .show_rows(ui, row, indices.len(), |ui, visible| {
                            for &i in &indices[visible] {
                                if clip_row(ui, &project.clips[i], self.clip == i, true).clicked()
                                    && self.clip != i
                                {
                                    self.jump_to_clip(i);
                                }
                            }
                        });
                });
        } else {
            ui.horizontal(|ui| {
                self.set_menu(ui, project);
                self.set_picker(ui, project);
                let set = project.clips[self.clip].set_id;
                self.export_set_button(ui, set);
                ui.separator();
                egui::ComboBox::from_id_salt("active-animation")
                    .width(110.)
                    .truncate()
                    .selected_text(&project.clips[self.clip].name)
                    .show_ui(ui, |ui| {
                        for (i, clip) in project
                            .clips
                            .iter()
                            .enumerate()
                            .filter(|(_, c)| c.set_id == set)
                        {
                            if clip_row(ui, clip, self.clip == i, false).clicked() {
                                if self.clip != i {
                                    self.jump_to_clip(i);
                                }
                                ui.memory_mut(|m| m.close_popup());
                            }
                        }
                    })
                    .response
                    .on_hover_text("Active clip");
                changed |= self.clip_actions(ui, project);
            });
        }
        changed |= self.set_dialog(ui.ctx(), project);
        changed
    }

    fn clip_actions(&mut self, ui: &mut Ui, project: &mut AnimationProject) -> bool {
        let mut changed = false;
        let clip = &project.clips[self.clip];
        let set = clip.set_id;
        if Icon::NewClip
            .show(ui, false)
            .on_hover_text("New clip")
            .clicked()
        {
            self.navigator_dialog = Some(Dialog::new(
                Action::NewClip(set),
                unique_name(project, set, "Animation"),
            ));
        }
        if Icon::DuplicateClip
            .show(ui, false)
            .on_hover_text("Duplicate clip")
            .clicked()
        {
            let stem = format!("{} copy", clip.name.chars().take(61).collect::<String>());
            let name = unique_name(project, set, &stem);
            if let Some(index) = self.result(project.duplicate_clip(self.clip, &name)) {
                self.jump_to_clip(index);
                changed = true;
            }
        }
        let clip = &project.clips[self.clip];
        ui.add_enabled_ui(project.clips.len() > 1, |ui| {
            if Icon::DeleteClip
                .show(ui, false)
                .on_hover_text("Delete clip")
                .on_disabled_hover_text("Keep at least one clip in the project.")
                .clicked()
            {
                self.navigator_dialog =
                    Some(Dialog::new(Action::DeleteClip(clip.id), clip.name.clone()));
            }
        });
        if Icon::ExportClip
            .show(ui, false)
            .on_hover_text("Export clip")
            .clicked()
        {
            self.export_request = Some(crate::exports::AnimationExport::Clip(clip.id));
        }
        ui.menu_button("⋯", |ui| {
            if ui.button("Rename clip…").clicked() {
                let clip = &project.clips[self.clip];
                self.navigator_dialog =
                    Some(Dialog::new(Action::RenameClip(clip.id), clip.name.clone()));
                ui.close_menu();
            }
            if project.animation_sets.len() > 1 {
                let destination = ui
                    .menu_button("Move to set", |ui| {
                        let mut selected = None;
                        for other in project.animation_sets.iter().filter(|s| s.id != set) {
                            if ui.button(&other.name).clicked() {
                                selected = Some(other.id);
                                ui.close_menu();
                            }
                        }
                        selected
                    })
                    .inner
                    .flatten();
                if let Some(id) = destination {
                    changed |= self
                        .result(project.move_clip_to_set(self.clip, id))
                        .is_some();
                }
            }
            ui.separator();
            let clip = &mut project.clips[self.clip];
            ui.strong("Sprite origin");
            let mut origin = clip.origin;
            ui.horizontal(|ui| {
                ui.add(
                    egui::DragValue::new(&mut origin[0])
                        .prefix("X ")
                        .speed(0.25),
                );
                ui.add(
                    egui::DragValue::new(&mut origin[1])
                        .prefix("Y ")
                        .speed(0.25),
                );
            });
            ui.horizontal(|ui| {
                let scene = clip.drawing_at(0);
                if ui.button("Center").clicked() {
                    origin = [scene.width as f32 / 2., scene.height as f32 / 2.];
                }
                if ui.button("Feet").on_hover_text("Bottom center").clicked() {
                    origin = [scene.width as f32 / 2., scene.height as f32];
                }
            });
            if origin != clip.origin && origin.into_iter().all(f32::is_finite) {
                clip.origin = origin;
                changed = true;
            }
        })
        .response
        .on_hover_text("Clip settings");
        changed
    }

    fn export_set_button(&mut self, ui: &mut Ui, set: u64) {
        if Icon::ExportSet
            .show(ui, false)
            .on_hover_text("Export animation set")
            .clicked()
        {
            self.export_request = Some(crate::exports::AnimationExport::Set(set));
        }
    }

    fn set_picker(&mut self, ui: &mut Ui, project: &AnimationProject) {
        let set = project
            .animation_set(project.clips[self.clip].set_id)
            .unwrap();
        egui::ComboBox::from_id_salt("timeline-animation-set")
            .height(288.)
            .width(ui.available_width().min(125.))
            .truncate()
            .selected_text(&set.name)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show_ui(ui, |ui| {
                for other in &project.animation_sets {
                    let first = project
                        .clips
                        .iter()
                        .position(|clip| clip.set_id == other.id);
                    let label = match first {
                        Some(_) => other.name.clone(),
                        None => format!("{} · Add first clip…", other.name),
                    };
                    let select =
                        |ui: &mut Ui| ui.selectable_label(other.id == set.id, label).clicked();
                    let selected = if first.is_some() && other.id != set.id {
                        let id = egui::Id::new((
                            "browse-set-clips",
                            project.clips[0].session_id(),
                            other.id,
                        ));
                        let header =
                            egui::collapsing_header::CollapsingState::load_with_default_open(
                                ui.ctx(),
                                id,
                                false,
                            )
                            .show_header(ui, select);
                        let (toggle, heading, _) = header.body(|ui| {
                            for (i, clip) in project
                                .clips
                                .iter()
                                .enumerate()
                                .filter(|(_, c)| c.set_id == other.id)
                            {
                                if clip_row(ui, clip, false, false).clicked() {
                                    self.jump_to_clip(i);
                                    ui.memory_mut(|m| m.close_popup());
                                }
                            }
                        });
                        toggle.on_hover_text("Browse clips to open or drag onto the canvas");
                        heading.inner
                    } else {
                        select(ui)
                    };
                    if selected {
                        ui.memory_mut(|m| m.close_popup());
                        if other.id == set.id {
                            continue;
                        }
                        if let Some(index) = first {
                            self.jump_to_clip(index);
                        } else {
                            self.navigator_dialog =
                                Some(Dialog::new(Action::NewClip(other.id), "Animation".into()));
                        }
                    }
                }
            })
            .response
            .on_hover_text("Choose the active animation set");
    }

    fn set_menu(&mut self, ui: &mut Ui, project: &AnimationProject) {
        let set = project
            .animation_set(project.clips[self.clip].set_id)
            .unwrap();
        ui.menu_button("Sets", |ui| {
            if ui.button("New set…").clicked() {
                self.navigator_dialog = Some(Dialog::new(Action::NewSet, String::new()));
                ui.close_menu();
            }
            if ui.button("Rename set…").clicked() {
                self.navigator_dialog =
                    Some(Dialog::new(Action::RenameSet(set.id), set.name.clone()));
                ui.close_menu();
            }
            if ui
                .add_enabled(
                    set.id != DEFAULT_ANIMATION_SET_ID,
                    egui::Button::new("Remove set…"),
                )
                .clicked()
            {
                self.navigator_dialog =
                    Some(Dialog::new(Action::RemoveSet(set.id), set.name.clone()));
                ui.close_menu();
            }
        })
        .response
        .on_hover_text("Manage animation sets");
    }

    fn set_dialog(&mut self, ctx: &egui::Context, project: &mut AnimationProject) -> bool {
        let Some(dialog) = &mut self.navigator_dialog else {
            return false;
        };
        let (title, button) = match dialog.action {
            Action::NewSet => ("New animation set", "Create set"),
            Action::RenameSet(_) => ("Rename animation set", "Rename"),
            Action::RemoveSet(_) => ("Remove animation set", "Move clips and remove"),
            Action::NewClip(_) => ("New clip", "Create clip"),
            Action::RenameClip(_) => ("Rename clip", "Rename"),
            Action::DeleteClip(_) => ("Delete clip", "Delete clip"),
        };
        let mut apply = false;
        let mut cancel = false;
        let modal = egui::Modal::new("animation-set-dialog".into()).show(ctx, |ui| {
            ui.set_min_width(255.);
            ui.set_max_width(340.);
            ui.heading(title);
            if matches!(dialog.action, Action::RemoveSet(_)) {
                let default = &project.animation_set(DEFAULT_ANIMATION_SET_ID).unwrap().name;
                ui.label(format!("Move all clips in “{}” to “{default}”? Clips with matching names receive a numbered suffix.", dialog.name));
            } else if matches!(dialog.action, Action::DeleteClip(_)) {
                ui.label(format!("Delete “{}” from this project?", dialog.name));
            } else {
                let mut field = egui::TextEdit::singleline(&mut dialog.name)
                    .hint_text("Name")
                    .desired_width(244.)
                    .show(ui);
                if dialog.focus {
                    field.response.request_focus();
                    let range = egui::text::CCursorRange::two(
                        egui::text::CCursor::default(),
                        egui::text::CCursor::new(dialog.name.chars().count()),
                    );
                    field.state.cursor.set_char_range(Some(range));
                    field.state.store(ctx, field.response.id);
                    dialog.focus = false;
                }
                apply = field.response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            }
            if let Some(error) = &dialog.error { ui.colored_label(ui.visuals().error_fg_color, error); }
            ui.horizontal(|ui| {
                apply |= ui.button(button).clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        if apply {
            match dialog.apply(project) {
                Ok(selected) => {
                    self.navigator_dialog = None;
                    if let Some(index) = selected {
                        self.jump_to_clip(index);
                    }
                    return true;
                }
                Err(message) => dialog.error = Some(message),
            }
        } else if cancel || modal.should_close() {
            self.navigator_dialog = None;
        }
        false
    }
}
