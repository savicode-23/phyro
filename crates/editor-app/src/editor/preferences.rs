use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, BufReader, Write};
use std::path::{Path, PathBuf};

use eframe::egui;
use serde::{Deserialize, Serialize};

use super::Editor;
use crate::inspector::Tab;
use crate::keymap::{self, Keymap};
use crate::timeline::DEFAULT_FRAME_WIDTH;
const MAX_RECENT_PROJECTS: usize = 10;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(default)]
struct Preferences {
    assistant_enabled: bool,
    visible_panels: Vec<Tab>,
    active_panel: Tab,
    // Older profiles saved only the active tab and visible panels in a single dock.
    #[serde(default)]
    panel_layout: Option<super::panel_layout::PanelLayout>,
    show_inspector: bool,
    frame_width: f32,
    pencil_smoothing: u8,
    pencil_pressure: bool,
    drawing_paint: crate::canvas::PathPaintMemory,
    recent_colors: Vec<[u8; 4]>,
    show_checkerboard: bool,
    keyboard_shortcuts: Keymap,
    // OsString also round-trips native paths that aren't valid Unicode.
    #[serde(with = "native_paths")]
    recent_projects: Vec<PathBuf>,
    library_directory: Option<OsString>,
    library_folders_active: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            assistant_enabled: false,
            visible_panels: Tab::ALL
                .into_iter()
                .filter(|tab| Tab::DEFAULT_VISIBLE[*tab as usize])
                .collect(),
            active_panel: Tab::default(),
            panel_layout: Some(super::panel_layout::PanelLayout::stacked()),
            show_inspector: true,
            frame_width: DEFAULT_FRAME_WIDTH,
            pencil_smoothing: crate::tool_keys::DEFAULT_PENCIL_SMOOTHING,
            pencil_pressure: false,
            drawing_paint: Default::default(),
            recent_colors: Vec::new(),
            show_checkerboard: false,
            keyboard_shortcuts: Keymap::default(),
            recent_projects: Vec::new(),
            library_directory: None,
            library_folders_active: false,
        }
    }
}

impl Preferences {
    fn read(path: &Path) -> Self {
        File::open(path)
            .ok()
            .and_then(|file| serde_json::from_reader(BufReader::new(file)).ok())
            .unwrap_or_default()
    }

    fn update(path: &Path, change: impl FnOnce(&mut Self) -> io::Result<()>) -> io::Result<()> {
        let directory = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
        fs::create_dir_all(directory)?;
        // The JSON is replaced atomically, so lock a stable sibling across read/merge/write.
        let lock = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path.with_extension("lock"))?;
        lock.lock()?;
        let previous = Self::read(path);
        let mut next = previous.clone();
        change(&mut next)?;
        if next == previous {
            return Ok(());
        }
        let mut file = tempfile::NamedTempFile::new_in(directory)?;
        file.write_all(&serde_json::to_vec_pretty(&next)?)?;
        file.as_file().sync_all()?;
        file.persist(path)?;
        Ok(())
    }
}

pub(super) struct PreferencesFile {
    path: PathBuf,
    initial: Preferences,
    ctx: egui::Context,
}

fn preferences_path() -> Option<PathBuf> {
    let absolute_env = |key| {
        std::env::var_os(key)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    #[cfg(windows)]
    let base = absolute_env("APPDATA").or_else(|| absolute_env("LOCALAPPDATA"))?;
    #[cfg(target_os = "macos")]
    let base = absolute_env("HOME")?.join("Library/Application Support");
    #[cfg(not(any(windows, target_os = "macos")))]
    let base = absolute_env("XDG_CONFIG_HOME")
        .or_else(|| absolute_env("HOME").map(|home| home.join(".config")))?;
    let brand = if cfg!(any(windows, target_os = "macos")) {
        "Phyro"
    } else {
        "phyro"
    };
    Some(crate::profile::data_path(&base, brand, "preferences.json"))
}

impl Editor {
    pub(crate) fn new_native(ctx: &egui::Context) -> Self {
        let mut editor = Self::new(ctx);
        editor.panel_layout = super::panel_layout::PanelLayout::stacked();
        editor.image_clipboard = super::clipboard_image::ImageClipboard::native();
        if let Some(path) = preferences_path() {
            editor.load_preferences(ctx, path);
        }
        if let Some(root) = crate::recovery::directory() {
            editor.enable_draft_recovery(root);
        }
        editor
    }

    fn load_preferences(&mut self, ctx: &egui::Context, path: PathBuf) {
        let loaded = Preferences::read(&path);
        self.assistant_enabled = loaded.assistant_enabled;
        self.panel_layout = loaded.panel_layout.unwrap_or_default();
        self.panel_layout.normalize();
        self.show_inspector = loaded.show_inspector;
        self.visible_panels = Tab::ALL.map(|tab| loaded.visible_panels.contains(&tab));
        self.inspector_tab = if self.visible_panels[loaded.active_panel as usize] {
            loaded.active_panel
        } else {
            Tab::ALL
                .into_iter()
                .find(|tab| self.visible_panels[*tab as usize])
                .unwrap_or_default()
        };
        self.timeline.set_frame_width(loaded.frame_width);
        self.pencil_smoothing = loaded.pencil_smoothing.min(200);
        self.pencil_pressure = loaded.pencil_pressure;
        self.path_paints = loaded.drawing_paint;
        self.path_paints.sanitize();
        self.path_paints.restore(self.tool, &mut self.style);
        crate::color_picker::recent::restore(ctx, &loaded.recent_colors);
        self.canvas.set_checkerboard(loaded.show_checkerboard);
        self.recent_projects = loaded.recent_projects;
        if let Some(path) = loaded
            .library_directory
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
        {
            self.library.set_directory(path);
        }
        self.library
            .set_folders_active(loaded.library_folders_active);
        self.install_keyboard_shortcuts(ctx, loaded.keyboard_shortcuts);
        self.preferences_file = Some(PreferencesFile {
            path,
            initial: Preferences {
                recent_colors: crate::color_picker::recent::colors(ctx),
                ..self.preferences()
            },
            ctx: ctx.clone(),
        });
    }

    fn preferences(&self) -> Preferences {
        Preferences {
            assistant_enabled: self.assistant_enabled,
            visible_panels: Tab::ALL
                .into_iter()
                .filter(|tab| self.visible_panels[*tab as usize])
                .collect(),
            active_panel: self.inspector_tab,
            panel_layout: Some(self.panel_layout.clone()),
            show_inspector: self.show_inspector,
            frame_width: self.timeline.frame_width(),
            pencil_smoothing: self.pencil_smoothing,
            pencil_pressure: self.pencil_pressure,
            drawing_paint: self.path_paints.clone(),
            recent_colors: self
                .preferences_file
                .as_ref()
                .map_or_else(Vec::new, |file| {
                    crate::color_picker::recent::colors(&file.ctx)
                }),
            show_checkerboard: self.canvas.show_checkerboard(),
            keyboard_shortcuts: self.keyboard_settings.current.clone(),
            recent_projects: self.recent_projects.clone(),
            library_directory: self.library.directory().map(|p| p.as_os_str().to_owned()),
            library_folders_active: self.library.folders_active(),
        }
    }

    fn install_keyboard_shortcuts(&mut self, ctx: &egui::Context, map: Keymap) {
        self.keyboard_settings.current = map.clone();
        keymap::install(ctx, std::sync::Arc::new(map));
    }

    pub(super) fn refresh_keyboard_shortcuts(&mut self, ctx: &egui::Context) {
        if let Some(file) = &mut self.preferences_file {
            let map = Preferences::read(&file.path).keyboard_shortcuts;
            file.initial.keyboard_shortcuts = map.clone();
            self.install_keyboard_shortcuts(ctx, map);
        }
    }

    pub(super) fn apply_keyboard_shortcuts(
        &mut self,
        ctx: &egui::Context,
        edited: Keymap,
    ) -> io::Result<()> {
        let mut effective = edited.clone();
        if let Some(file) = &mut self.preferences_file {
            Preferences::update(&file.path, |saved| {
                saved
                    .keyboard_shortcuts
                    .merge_changes(&self.keyboard_settings.current, &edited)
                    .map_err(io::Error::other)?;
                effective.clone_from(&saved.keyboard_shortcuts);
                Ok(())
            })?;
            file.initial.keyboard_shortcuts = effective.clone();
        }
        self.install_keyboard_shortcuts(ctx, effective);
        Ok(())
    }

    pub(super) fn save_preferences(&self) {
        let Some(file) = &self.preferences_file else {
            return;
        };
        let current = self.preferences();
        if current == file.initial {
            return;
        }
        // A second editor window may have changed an unrelated preference.
        let result = Preferences::update(&file.path, |saved| {
            saved
                .drawing_paint
                .merge(&file.initial.drawing_paint, &current.drawing_paint);
            crate::color_picker::recent::merge(&file.ctx, &mut saved.recent_colors);
            if current.panel_layout != file.initial.panel_layout {
                saved.panel_layout.clone_from(&current.panel_layout);
            }
            if current.assistant_enabled != file.initial.assistant_enabled {
                saved.assistant_enabled = current.assistant_enabled;
            }
            if current.show_inspector != file.initial.show_inspector {
                saved.show_inspector = current.show_inspector;
            }
            if (current.active_panel, &current.visible_panels)
                != (file.initial.active_panel, &file.initial.visible_panels)
            {
                saved.active_panel = current.active_panel;
                saved.visible_panels = current.visible_panels;
            }
            if current.frame_width != file.initial.frame_width {
                saved.frame_width = current.frame_width;
            }
            if current.pencil_pressure != file.initial.pencil_pressure {
                saved.pencil_pressure = current.pencil_pressure;
            }
            if current.pencil_smoothing != file.initial.pencil_smoothing {
                saved.pencil_smoothing = current.pencil_smoothing;
            }
            if current.show_checkerboard != file.initial.show_checkerboard {
                saved.show_checkerboard = current.show_checkerboard;
            }
            if current.recent_projects != file.initial.recent_projects {
                saved.recent_projects = current.recent_projects;
            }
            if current.library_directory != file.initial.library_directory {
                saved.library_directory = current.library_directory;
            }
            if current.library_folders_active != file.initial.library_folders_active {
                saved.library_folders_active = current.library_folders_active;
            }
            Ok(())
        });
        if let Err(error) = result {
            eprintln!("Could not save Phyro preferences: {error}");
        }
    }

    pub(super) fn update_recent(&mut self, path: &Path, opened: bool) {
        let Ok(path) = std::path::absolute(path) else {
            return;
        };
        if !opened && !self.recent_projects.contains(&path) {
            return;
        }
        let update = |recent: &mut Vec<PathBuf>| {
            recent.retain(|p| p != &path);
            if opened {
                recent.insert(0, path.clone());
                recent.truncate(MAX_RECENT_PROJECTS);
            }
        };
        if let Some(file) = &mut self.preferences_file {
            let result = Preferences::update(&file.path, |saved| {
                update(&mut saved.recent_projects);
                self.recent_projects.clone_from(&saved.recent_projects);
                Ok(())
            });
            match result {
                Ok(()) => {
                    file.initial
                        .recent_projects
                        .clone_from(&self.recent_projects);
                    return;
                }
                Err(error) => eprintln!("Could not save recent Phyro projects: {error}"),
            }
        }
        update(&mut self.recent_projects);
    }

    pub(super) fn open_recent_menu(&mut self, ui: &mut egui::Ui) {
        let response = ui.menu_button("Open Recent", |ui| {
            if self.recent_projects.is_empty() {
                ui.weak("No recent projects");
            }
            ui.set_max_width(330.);
            for path in &self.recent_projects {
                let name = path.file_name().unwrap_or_default();
                let repeated = self
                    .recent_projects
                    .iter()
                    .any(|other| other != path && other.file_name() == path.file_name());
                let label = if repeated {
                    format!(
                        "{} — {}",
                        name.to_string_lossy(),
                        path.parent().unwrap_or(path).display()
                    )
                } else {
                    name.to_string_lossy().into_owned()
                };
                if ui
                    .add(egui::Button::new(label).truncate())
                    .on_hover_text(path.to_string_lossy())
                    .clicked()
                {
                    ui.close_menu();
                    return Some(path.clone());
                }
            }
            None
        });
        if let Some(Some(path)) = response.inner {
            self.timeline.stop();
            self.open_paths(ui.ctx(), vec![path]);
        }
    }
}

mod native_paths {
    use super::*;
    use serde::{Deserializer, Serializer};

    pub(super) fn serialize<S: Serializer>(
        paths: &[PathBuf],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(paths.iter().map(|path| path.as_os_str()))
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<PathBuf>, D::Error> {
        let paths = Vec::<OsString>::deserialize(deserializer)?;
        let mut recent = Vec::new();
        for path in paths
            .into_iter()
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
        {
            if !recent.contains(&path) {
                recent.push(path);
                if recent.len() == MAX_RECENT_PROJECTS {
                    break;
                }
            }
        }
        Ok(recent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::App;

    fn launch(path: &Path) -> Editor {
        let ctx = egui::Context::default();
        let mut editor = Editor::new(&ctx);
        editor.load_preferences(&ctx, path.to_owned());
        editor
    }

    #[test]
    fn asset_folder_survives_restarts_and_independent_window_edits() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preferences.json");
        let assets = directory.path().join("Game Art");
        let mut browser = launch(&path);
        let mut other = launch(&path);
        browser.library.set_directory(assets.clone());
        browser.library.set_folders_active(true);
        browser.on_exit(None);
        other.pencil_smoothing = 85;
        other.on_exit(None);
        let next = launch(&path);
        assert_eq!(next.library.directory(), Some(assets.as_path()));
        assert!(next.library.folders_active());
        assert_eq!(next.pencil_smoothing, 85);
    }

    #[test]
    fn drawing_tools_and_recent_colors_survive_restart_and_merge_separate_window_edits() {
        use crate::canvas::Tool;
        use crate::color_picker::recent;
        use document_core::{Gradient, GradientKind, LineCap, LineJoin};
        use std::sync::Arc;

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preferences.json");
        fs::write(&path, r#"{"frame_width":7}"#).unwrap();
        let mut first = launch(&path);
        let mut second = launch(&path);
        first.panel_layout = super::super::panel_layout::PanelLayout::stacked();
        let clear = [99, 27, 43, 0];
        let teal = [15, 151, 138, 192];
        let fade = Arc::new(Gradient::from_color(
            GradientKind::Radial {
                center: [0.3, 0.4],
                radius: 0.7,
            },
            teal,
        ));
        first.set_tool(Tool::Pen);
        first.style.fill = clear;
        first.style.no_fill = false;
        first.set_tool(Tool::Pencil);
        first.style.no_fill = true;
        first.style.no_stroke = true;
        first.style.stroke = [80, 60, 40, 255];
        first.style.stroke_width = 9.;
        first.style.join = LineJoin::Bevel;
        first.set_tool(Tool::Select);
        let first_ctx = &first.preferences_file.as_ref().unwrap().ctx;
        recent::remember(first_ctx, clear);
        recent::finish(first_ctx);

        second.set_tool(Tool::Pen);
        second.style.stroke = teal;
        second.style.stroke_gradient = Some(fade.clone());
        second.style.stroke_width = 3.5;
        second.style.cap = LineCap::Square;
        second.set_tool(Tool::Select);
        let second_ctx = &second.preferences_file.as_ref().unwrap().ctx;
        recent::remember(second_ctx, [25; 4]); // intermediate color in the same picker gesture
        recent::remember(second_ctx, teal);
        second.on_exit(None);
        first.on_exit(None);

        let mut next = launch(&path);
        let ctx = &next.preferences_file.as_ref().unwrap().ctx;
        assert_eq!(next.panel_layout, first.panel_layout);
        assert_eq!(recent::colors(ctx), [clear, teal]);
        assert_eq!(next.timeline.frame_width(), 7.);
        next.set_tool(Tool::Pen);
        assert_eq!(next.style.join, LineJoin::Miter);
        assert_eq!(next.style.fill, clear);
        assert!(!next.style.no_fill);
        assert_eq!(next.style.stroke, teal);
        assert_eq!(next.style.stroke_gradient.as_ref(), Some(&fade));
        assert_eq!(next.style.stroke_width, 3.5);
        assert_eq!(next.style.cap, LineCap::Square);
        next.replace_project(Default::default(), None);
        next.set_tool(Tool::Pencil);
        assert_eq!(next.style.stroke, [80, 60, 40, 255]);
        assert!(next.style.no_fill && next.style.no_stroke);
        assert_eq!(next.style.stroke_width, 9.);
        assert_eq!(next.style.join, LineJoin::Bevel);
    }

    #[test]
    fn shortcut_edits_persist_on_apply_and_merge_across_windows() {
        use crate::keymap::Action;
        use egui::{Key, Modifiers};

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preferences.json");
        fs::write(&path, r#"{"frame_width":7}"#).unwrap();
        let contexts = [egui::Context::default(), egui::Context::default()];
        let [mut first, mut second] = contexts.each_ref().map(|ctx| {
            let mut editor = Editor::new(ctx);
            editor.load_preferences(ctx, path.clone());
            editor
        });
        let mut pen = first.keyboard_settings.current.clone();
        pen.set(Action::PenTool, Key::F12, Modifiers::NONE).unwrap();
        first
            .apply_keyboard_shortcuts(&contexts[0], pen.clone())
            .unwrap();
        assert_eq!(launch(&path).keyboard_settings.current, pen);

        let mut conflicting = second.keyboard_settings.current.clone();
        conflicting.clear(Action::PenTool);
        assert!(second
            .apply_keyboard_shortcuts(&contexts[1], conflicting)
            .is_err());
        assert_eq!(*keymap::active(&contexts[1]), Keymap::default());

        let mut frames = second.keyboard_settings.current.clone();
        frames.clear(Action::NextFrame);
        second
            .apply_keyboard_shortcuts(&contexts[1], frames)
            .unwrap();
        first.on_exit(None);
        let expected = {
            pen.clear(Action::NextFrame);
            pen
        };
        assert_eq!(*keymap::active(&contexts[1]), expected);
        let reopened_ctx = egui::Context::default();
        let mut restored = Editor::new(&reopened_ctx);
        restored.load_preferences(&reopened_ctx, path);
        assert_eq!(*keymap::active(&reopened_ctx), expected);
        assert_eq!(restored.timeline.frame_width(), 7.);
    }

    #[test]
    fn recent_projects_persist_only_successful_opens_and_saves_once_in_recency_order() {
        let directory = tempfile::tempdir().unwrap();
        let settings = directory.path().join("prefs.json");
        let mut editor = launch(&settings);
        let a = directory.path().join("first.vanim");
        let b = directory.path().join("second.vanim");
        project_io::save_animation(&a, &editor.project, false).unwrap();
        editor.open_project(a.clone());
        editor.path = Some(b.clone());
        assert!(editor.save(false));
        editor.open_project(a.clone());
        let broken = directory.path().join("broken.vanim");
        fs::write(&broken, "invalid").unwrap();
        editor.open_project(broken);
        editor.path = Some(directory.path().join("missing/fails.vanim"));
        assert!(!editor.save(false));
        assert_eq!(launch(&settings).recent_projects, [a.clone(), b]);
        for i in 0..MAX_RECENT_PROJECTS + 3 {
            let path = directory.path().join(format!("pose-{i}.vanim"));
            editor.path = Some(path);
            assert!(editor.save(false));
        }
        let reopened = launch(&settings);
        assert_eq!(reopened.recent_projects.len(), MAX_RECENT_PROJECTS);
        assert_eq!(
            reopened.recent_projects[0].file_name().unwrap(),
            "pose-12.vanim"
        );
        assert!(!reopened.recent_projects.contains(&a));
    }

    #[test]
    fn concurrent_project_windows_keep_each_others_recent_entries() {
        let directory = tempfile::tempdir().unwrap();
        let settings = directory.path().join("preferences.json");
        let paths: Vec<_> = (0..4)
            .map(|i| directory.path().join(format!("pose-{i}.vanim")))
            .collect();
        for path in &paths {
            project_io::save_animation(path, &Default::default(), false).unwrap();
        }
        let start = std::sync::Barrier::new(paths.len());
        std::thread::scope(|scope| {
            for path in &paths {
                scope.spawn(|| {
                    let mut editor = launch(&settings);
                    start.wait();
                    editor.open_project(path.clone());
                    editor.on_exit(None);
                });
            }
        });
        let mut recent = launch(&settings).recent_projects;
        recent.sort();
        assert_eq!(recent, paths);
    }

    #[test]
    fn missing_and_invalid_preferences_open_with_defaults() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preferences.json");
        let mut editor = launch(&path);
        assert_eq!(
            editor.panel_layout,
            crate::editor::panel_layout::PanelLayout::stacked()
        );
        assert_eq!(editor.preferences(), Preferences::default());
        editor.on_exit(None);
        assert!(!path.exists());
        fs::write(&path, "{broken").unwrap();
        assert_eq!(launch(&path).preferences(), Preferences::default());
        fs::write(&path, r#"{"visible_panels":["rig"],"frame_width":0}"#).unwrap();
        let editor = launch(&path);
        assert_eq!(editor.panel_layout, Default::default());
        assert_eq!(editor.inspector_tab, Tab::Rig);
        assert_eq!(editor.visible_panels, [false, false, true, false, false]);
        assert_eq!(editor.timeline.frame_width(), 3.);
    }

    #[test]
    fn restart_restores_panel_layout_width_and_background() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("Phyro/preferences.json");
        let mut editor = launch(&path);
        let mut other = launch(&path);
        editor.panel_layout = Default::default();
        let tabbed = editor.panel_layout.clone();
        editor.visible_panels = [true, false, false, true, true];
        editor.inspector_tab = Tab::Library;
        editor.timeline.set_frame_width(7.);
        editor.pencil_smoothing = 178;
        editor.pencil_pressure = true;
        editor.assistant_enabled = true;
        editor.on_exit(None);
        other.canvas.set_checkerboard(true);
        other.on_exit(None);

        let mut restored = launch(&path);
        assert_eq!(restored.visible_panels, [true, false, false, true, true]);
        assert_eq!(restored.inspector_tab, Tab::Library);
        assert_eq!(restored.panel_layout, tabbed);
        assert_eq!(restored.timeline.frame_width(), 7.);
        assert_eq!(restored.pencil_smoothing, 178);
        assert!(restored.pencil_pressure);
        assert!(restored.assistant_enabled);
        assert!(!restored.assistant.sharing());
        assert!(restored.canvas.show_checkerboard());
        restored.replace_project(Default::default(), None);
        assert!(restored.canvas.show_checkerboard());
        assert_eq!(restored.timeline.frame_width(), 7.);
        restored.visible_panels = [false; Tab::COUNT];
        restored.inspector_tab = Tab::default();
        restored.on_exit(None);
        assert_eq!(launch(&path).visible_panels, [false; Tab::COUNT]);
    }
}
