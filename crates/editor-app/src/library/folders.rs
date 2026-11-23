mod backend;
mod previews;
mod tiles;

use backend::{Backend, Entry, Kind, PreviewKey};
use eframe::egui;
use previews::Previews;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Default)]
pub(super) struct FolderPanel {
    directory: Option<PathBuf>,
    backend: Option<Backend>,
    entries: Vec<Entry>,
    generation: u64,
    needs_scan: bool,
    loading: bool,
    error: Option<String>,
    selected: Option<usize>,
    clip: Option<u64>,
    playback: Option<Playback>,
    cache: Previews,
    visible_files: Vec<PathBuf>,
    seen: bool,
    busy: bool,
    paused: bool,
    pub(super) open_source: Option<PathBuf>,
    #[cfg(test)]
    pub(super) cache_directory: Option<PathBuf>,
}

struct Playback {
    sample: u32,
    last_ready: u32,
    next_at: f64,
}

impl FolderPanel {
    pub(super) fn directory(&self) -> Option<&Path> {
        self.directory.as_deref()
    }

    pub(super) fn navigate(&mut self, path: PathBuf) {
        self.directory = Some(path);
        self.needs_scan = true;
        self.loading = true;
        self.error = None;
        self.selected = None;
        self.clip = None;
        self.playback = None;
        self.cache.clear();
        self.visible_files.clear();
        self.generation = self.generation.wrapping_add(1);
    }

    pub(super) fn begin_frame(&mut self, busy: bool) {
        self.seen = false;
        self.busy = busy;
        if busy {
            self.pause();
        }
    }

    pub(super) fn end_frame(&mut self, ctx: &egui::Context, resume_after_input: bool) {
        if !self.seen {
            self.pause();
            self.playback = None;
        } else if self.busy && resume_after_input {
            // The last gesture frame still belongs to the canvas. Wake once
            // more to resume even when the pointer subsequently stays still.
            ctx.request_repaint();
        }
    }

    fn pause(&mut self) {
        if !self.paused {
            if let Some(worker) = &mut self.backend {
                worker.set_requests(&[], true);
            }
            self.paused = true;
        }
    }

    fn choose_folder(&mut self) {
        let mut dialog = rfd::FileDialog::new().set_title("Choose asset folder");
        if let Some(path) = &self.directory {
            dialog = dialog.set_directory(path);
        }
        if let Some(path) = crate::diagnostics::native_dialog(|| dialog.pick_folder()) {
            self.navigate(path);
        }
    }

    fn folder_bar(&mut self, ui: &mut egui::Ui) {
        let path = self.directory.as_deref().expect("open folder");
        let name = path
            .file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy();
        let mut next = None;
        let mut choose = false;
        ui.horizontal(|ui| {
            if ui
                .add_enabled(path.parent().is_some(), egui::Button::new("Up"))
                .on_hover_text("Parent folder")
                .clicked()
            {
                next = path.parent().map(Path::to_owned);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.menu_button("…", |ui| {
                    if ui.button("Choose folder…").clicked() {
                        choose = true;
                        ui.close_menu();
                    }
                    if ui.button("Refresh folder").clicked() {
                        next = Some(path.into());
                        ui.close_menu();
                    }
                });
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add(egui::Label::new(egui::RichText::new(name).strong()).truncate())
                        .on_hover_text(path.display().to_string());
                });
            });
        });
        if choose {
            self.choose_folder();
        } else if let Some(path) = next {
            self.navigate(path);
        }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        if self.needs_scan {
            self.needs_scan = false;
            let worker = self.backend.get_or_insert_with(|| {
                #[cfg(test)]
                if let Some(cache) = &self.cache_directory {
                    return Backend::with_cache(ctx, cache.clone());
                }
                Backend::new(ctx)
            });
            worker.navigate(
                self.directory.clone().expect("open folder"),
                std::mem::take(&mut self.entries),
            );
        }
        let Some(worker) = &mut self.backend else {
            return;
        };
        if self.paused {
            worker.set_requests(&[], false);
            self.paused = false;
        }
        if let Some(result) = worker.poll() {
            self.loading = false;
            match result {
                Ok(entries) => self.entries = entries,
                Err(message) => self.error = Some(message),
            }
        }
        while let Some(path) = worker.take_changed() {
            self.cache.invalidate(&path);
            if self
                .selected
                .and_then(|i| self.entries.get(i))
                .is_some_and(|e| e.path == path)
            {
                self.playback = None;
            }
        }
        self.cache.receive(ctx, worker);
    }

    pub(super) fn show(&mut self, ui: &mut egui::Ui) {
        self.seen = true;
        if self.directory.is_none() {
            if ui.button("Choose folder…").clicked() {
                self.choose_folder();
            }
            ui.label("Browse Phyro projects, SVGs and images from a folder on your computer.");
            return;
        }
        self.folder_bar(ui);
        if !self.busy {
            self.poll(ui.ctx());
        }
        if self.loading {
            ui.weak("Reading folder…");
            return;
        }
        if let Some(error) = &self.error {
            ui.colored_label(ui.visuals().warn_fg_color, error);
            return;
        }
        if self.entries.is_empty() {
            ui.weak("No supported assets or folders here.");
            return;
        }

        self.cache.begin();
        self.visible_files.clear();
        let height = ui.available_height().min(ui.clip_rect().height());
        // Keep a separate scroll area for details in short/split docks. Folder rows
        // always have their own finite viewport, even inside another ScrollArea.
        let gallery_height = if self.selected.is_some() {
            (height * 0.49).clamp(77., 370.)
        } else {
            (height - 4.).clamp(77., 720.)
        };
        let open_folder = self.gallery(ui, gallery_height);
        if let Some(path) = open_folder {
            self.navigate(path);
            ui.ctx().request_repaint();
            return;
        }
        if let Some(index) = self.selected {
            ui.separator();
            egui::ScrollArea::vertical()
                .id_salt("asset-details")
                .auto_shrink([false, true])
                .show(ui, |ui| self.detail(ui, index));
        }
        if !self.busy {
            if let Some(worker) = &mut self.backend {
                worker.watch(&self.visible_files);
                worker.set_requests(self.cache.requests(), false);
                self.paused = false;
            }
        }
    }

    fn select(&mut self, index: usize) {
        if self.selected != Some(index) {
            self.selected = Some(index);
            self.clip = None;
            self.playback = None;
        }
    }

    fn gallery(&mut self, ui: &mut egui::Ui, height: f32) -> Option<PathBuf> {
        const ROW: f32 = 75.;
        let columns = ((ui.available_width() / 80.) as usize).clamp(1, 5);
        let rows = self.entries.len().div_ceil(columns);
        let height = height.min(rows as f32 * (ROW + ui.spacing().item_spacing.y));
        let mut open_folder = None;
        egui::ScrollArea::vertical()
            .id_salt(("folder-assets", self.generation))
            .max_height(height)
            .min_scrolled_height(height)
            .auto_shrink([false, false])
            .show_rows(ui, ROW, rows, |ui, rows| {
                let gap = ui.spacing().item_spacing.x;
                let width =
                    ((ui.available_width() - gap * (columns - 1) as f32) / columns as f32).max(1.);
                for row in rows {
                    ui.horizontal(|ui| {
                        for i in row * columns..((row + 1) * columns).min(self.entries.len()) {
                            let entry = &self.entries[i];
                            let selected = self.selected == Some(i);
                            let texture = if entry.kind != Kind::Folder
                                && self.visible_files.len() < previews::MAX_VISIBLE
                            {
                                self.visible_files.push(entry.path.clone());
                                self.cache.texture(PreviewKey::thumbnail(entry), false)
                            } else {
                                None
                            };
                            let response = tiles::asset_tile(
                                ui,
                                entry,
                                texture,
                                selected,
                                egui::vec2(width, ROW),
                            );
                            if entry.kind != Kind::Folder {
                                self.cache.start_drag(
                                    &response,
                                    entry,
                                    selected.then_some(self.clip).flatten(),
                                );
                            }
                            if response.double_clicked() {
                                match entry.kind {
                                    Kind::Folder => open_folder = Some(entry.path.clone()),
                                    Kind::Project => self.open_source = Some(entry.path.clone()),
                                    _ => (),
                                }
                            }
                            if response.clicked() && entry.kind != Kind::Folder {
                                self.select(i);
                            }
                        }
                    });
                }
            });
        open_folder
    }

    fn detail(&mut self, ui: &mut egui::Ui, index: usize) {
        let entry = &self.entries[index];
        let key = PreviewKey::thumbnail(entry);
        self.visible_files.push(entry.path.clone());
        let thumbnail = self.cache.texture(key.clone(), true);
        let heading = ui.add(
            egui::Label::new(egui::RichText::new(&entry.name).strong())
                .truncate()
                .selectable(false)
                .sense(egui::Sense::click_and_drag()),
        );
        self.cache.start_drag(&heading, entry, self.clip);
        if entry.kind == Kind::Project && heading.double_clicked() {
            self.open_source = Some(entry.path.clone());
        }
        heading.on_hover_cursor(egui::CursorIcon::Grab);
        if let Some(message) = self.cache.error(&key) {
            ui.colored_label(ui.visuals().warn_fg_color, message);
        }
        if entry.kind != Kind::Project {
            if thumbnail.is_some() {
                let response = tiles::strip(ui, &[thumbnail], 76.);
                self.cache.start_drag(&response, entry, None);
            }
            ui.weak("Drag onto the canvas to place.");
            return;
        }

        let clips = self.cache.clips(&key);
        let mut chosen = self
            .clip
            .and_then(|id| clips.iter().find(|c| c.id == id))
            .or_else(|| clips.first());
        if clips.len() == 1 {
            ui.add(egui::Label::new(&clips[0].name).truncate());
        } else if clips.len() > 1 {
            if clips.len() <= 3 && clips.iter().all(|c| c.name.chars().count() <= 10) {
                ui.horizontal_wrapped(|ui| {
                    for c in clips {
                        if ui
                            .selectable_label(chosen.is_some_and(|s| s.id == c.id), &c.name)
                            .clicked()
                        {
                            chosen = Some(c);
                        }
                    }
                });
            } else {
                egui::ComboBox::from_id_salt("asset-clip")
                    .selected_text(chosen.map_or("", |c| &c.name))
                    .truncate()
                    .show_ui(ui, |ui| {
                        let height = ui.spacing().interact_size.y;
                        egui::ScrollArea::vertical().max_height(180.).show_rows(
                            ui,
                            height,
                            clips.len(),
                            |ui, rows| {
                                for i in rows {
                                    let c = &clips[i];
                                    if ui
                                        .selectable_label(
                                            chosen.is_some_and(|s| s.id == c.id),
                                            &c.name,
                                        )
                                        .clicked()
                                    {
                                        chosen = Some(c);
                                        ui.close_menu();
                                    }
                                }
                            },
                        );
                    });
            }
        }
        let Some(clip) = chosen else {
            if self.cache.error(&key).is_none() {
                ui.weak("Loading clips…");
            }
            return;
        };
        let (id, frames, fps) = (clip.id, clip.frames.max(1), clip.fps.max(1));
        if Some(id) != self.clip {
            self.playback = None;
            self.clip = Some(id);
        }
        let now = ui.input(|i| i.time);
        ui.horizontal(|ui| {
            let playing = self.playback.is_some();
            let label = if playing { "Pause" } else { "Play" };
            if ui
                .add_enabled(frames > 1, egui::Button::new(label))
                .on_hover_text("Preview this clip here. Short loops play at up to 24 fps and longer clips sample up to 12 fps.")
                .clicked()
            {
                self.playback = (!playing).then_some(Playback {
                    sample: 0,
                    last_ready: 0,
                    next_at: now,
                });
            }
            ui.label(egui::RichText::new(format!("{frames} frames · {fps} fps")).size(11.));
        });

        let last = frames - 1;
        let times = [0, last / 3, last.saturating_mul(2) / 3, last];
        let count = (frames as usize).min(4);
        let mut images = [None; 4];
        for (i, image) in images[..count].iter_mut().enumerate() {
            let frame = if frames < 4 { i as u32 } else { times[i] };
            *image = self
                .cache
                .texture(PreviewKey::frame(entry, id, frame), true);
        }
        let mut playing_image = None;
        if let Some(play) = &mut self.playback {
            // Sample in ascending order then restart at frame zero. Stepping by
            // a fixed stride modulo the frame count can freeze short loops.
            let (samples, max_rate) = if frames <= 12 {
                (frames, 24.)
            } else {
                ((frames * 12).div_ceil(fps).clamp(2, frames), 12.)
            };
            let frame = play.sample * frames / samples;
            let frame_key = PreviewKey::frame(entry, id, frame);
            if let Some(message) = self.cache.error(&frame_key) {
                ui.colored_label(ui.visuals().warn_fg_color, message);
                self.playback = None;
            } else if let Some(image) = self.cache.texture(frame_key, true) {
                playing_image = Some(image);
                play.last_ready = frame;
                if !self.busy {
                    if now >= play.next_at {
                        play.sample = (play.sample + 1) % samples;
                        let interval = f64::from(frames) / f64::from(samples * fps);
                        play.next_at = now + interval.max(1. / max_rate);
                    }
                    ui.ctx().request_repaint_after(Duration::from_secs_f64(
                        (play.next_at - now).max(0.),
                    ));
                }
            } else {
                playing_image = self
                    .cache
                    .texture(PreviewKey::frame(entry, id, play.last_ready), true);
            }
        }
        let response = if self.playback.is_some() {
            tiles::strip(ui, &[playing_image.or(images[0])], 64.)
        } else {
            tiles::strip(ui, &images[..count], 46.)
        };
        self.cache.start_drag(&response, entry, Some(id));
        ui.weak("Double-click to open in a tab · Drag to place");
    }

    #[cfg(test)]
    pub(super) fn entry_count(&self) -> usize {
        self.entries.len()
    }

    #[cfg(test)]
    pub(super) fn resident_count(&self) -> usize {
        self.cache.len()
    }

    #[cfg(test)]
    pub(super) fn playing_frame(&self) -> Option<u32> {
        self.playback.as_ref().map(|p| p.last_ready)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_visible_rows_and_the_selected_file_request_previews() {
        let mut panel = FolderPanel {
            directory: Some(PathBuf::from("Characters")),
            busy: true, // inspect UI requests without waking a worker
            entries: (0..10_000)
                .map(|i| {
                    let name = format!("hero-{i:05}.png");
                    Entry {
                        path: PathBuf::from("Characters").join(&name),
                        name,
                        kind: Kind::Image,
                    }
                })
                .collect(),
            selected: Some(9_999),
            ..Default::default()
        };
        let ctx = egui::Context::default();
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(256., 360.),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| panel.show(ui));
            },
        );
        let requests = panel.cache.requests();
        assert!(requests.len() > 1 && requests.len() < 25);
        assert_eq!(requests[0].path(), panel.entries[9_999].path);
        assert!(requests[1..]
            .iter()
            .all(|k| k.path() < panel.entries[24].path.as_path()));
        assert!(panel.backend.is_none());
    }
}
