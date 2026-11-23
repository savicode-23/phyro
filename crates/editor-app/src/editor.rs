pub(crate) mod arrange;
mod artboard_size;
mod artwork_menu;
mod assistant;
mod clipboard_image;
mod document_properties;
mod document_tab_bar;
mod documents;
mod drafts;
mod drop_files;
pub(crate) mod edit_commands;
mod folder_assets;
mod guided_morph;
mod keyboard_settings;
mod notices;
mod panel_layout;
mod preferences;
mod recovery_dialog;
mod reference_image;
mod welcome;
mod welcome_preview;
mod workspace;

use std::path::{Path, PathBuf};

use document_core::{AnimationProject, Document};
use eframe::egui::{self, Key};
use rfd::{FileDialog, MessageButtons, MessageDialog, MessageDialogResult};

use crate::canvas::{Canvas, CanvasStyle, Tool};
use crate::commands::{self, Clipboard, History};
use crate::diagnostics::{self, Action, Diagnostics, UpdatePhase};
use crate::exports::Exports;
use crate::inspector::Tab;
use crate::keymap::{self, Action as Shortcut};
use crate::timeline::Timeline;

#[derive(PartialEq, Eq)]
enum OpenDecision {
    Proceed,
    Saved,
    Cancel,
}

pub struct Editor {
    assistant: assistant::Assistant,
    assistant_enabled: bool,
    document_id: u64,
    documents: documents::Documents,
    file_batch: Option<drop_files::FileBatch>,
    welcome_preview: Option<welcome_preview::Preview>,
    canonical_path: Option<PathBuf>,
    canvas_filter: Option<std::sync::Arc<crate::canvas::display_filter::Filter>>,
    preferences_file: Option<preferences::PreferencesFile>,
    keyboard_settings: keyboard_settings::KeyboardSettings,
    drafts: Option<drafts::DraftSession>,
    inactive_draft_due: Option<std::time::Instant>,
    recovery_dialog: recovery_dialog::RecoveryDialog,
    recovered_name: Option<String>,
    recent_projects: Vec<PathBuf>,
    artboard_size: Option<[u32; 2]>,
    guided_morph: Option<guided_morph::MorphDialog>,
    diagnostics: Option<Diagnostics>,
    diagnostic_context: Option<diagnostics::Context>,
    shown_worker_reports: u64,
    crash_reports: crate::crash_report_ui::CrashReportUi,
    project: AnimationProject,
    path: Option<PathBuf>,
    saved: Option<AnimationProject>,
    // A baseline for the in-progress gesture. History stores only its differences.
    baseline: AnimationProject,
    // Inspectors and canvas take mutable access even on read-only paints. Keep
    // the current exposure writable and return earlier unchanged cels to the baseline.
    writable_cel: Option<(u64, u32)>,
    // Re-evaluated when a transaction ends or the saved snapshot changes.
    baseline_is_saved: bool,
    // The previous paint's uncommitted edit can enable Undo during an active drag.
    pending_edit: bool,
    history: History,
    clipboard: Clipboard,
    image_clipboard: clipboard_image::ImageClipboard,
    text_menu: edit_commands::TextMenu,
    menu_bar_open: bool,
    artwork_menu_open: bool,
    path_shortcuts: crate::path_ops::Shortcuts,
    tool: Tool,
    pencil_smoothing: u8,
    pencil_pressure: bool,
    style: CanvasStyle,
    path_paints: crate::canvas::PathPaintMemory,
    inspector_tab: crate::inspector::Tab,
    panel_layout: panel_layout::PanelLayout,
    visible_panels: [bool; Tab::COUNT],
    selection: Vec<u64>,
    canvas: Canvas,
    show_inspector: bool,
    show_timeline: bool,
    timeline: Timeline,
    exports: Exports,
    library: crate::library::Library,
    folder_assets: folder_assets::Placements,
    text_dialog: Option<crate::type_editor::TypeEditor>,
    status: String,
    notices: notices::Notices,
    window_title: String,
}

impl Editor {
    pub fn new(ctx: &egui::Context) -> Self {
        workspace::apply_theme(ctx);
        let project = AnimationProject::default();
        Self {
            assistant: Default::default(),
            assistant_enabled: false,
            document_id: crate::editor::assistant::document_id(),
            documents: Default::default(),
            file_batch: None,
            welcome_preview: None,
            canonical_path: None,
            canvas_filter: None,
            preferences_file: None,
            keyboard_settings: Default::default(),
            drafts: None,
            inactive_draft_due: None,
            recovery_dialog: Default::default(),
            recovered_name: None,
            recent_projects: Vec::new(),
            artboard_size: None,
            guided_morph: None,
            diagnostics: None,
            diagnostic_context: None,
            shown_worker_reports: 0,
            crash_reports: Default::default(),
            path: None,
            saved: Some(project.clone()),
            baseline: project.clone(),
            project,
            writable_cel: None,
            baseline_is_saved: true,
            pending_edit: false,
            history: History::default(),
            clipboard: Clipboard::default(),
            image_clipboard: Default::default(),
            text_menu: Default::default(),
            menu_bar_open: false,
            artwork_menu_open: false,
            path_shortcuts: Default::default(),
            tool: Tool::Rectangle,
            pencil_smoothing: crate::tool_keys::DEFAULT_PENCIL_SMOOTHING,
            pencil_pressure: false,
            style: CanvasStyle {
                edit_motion: false,
                key_mode: Default::default(),
                fill: [88, 209, 167, 255],
                gradient: None,
                stroke_gradient: None,
                no_fill: false,
                no_stroke: false,
                gradient_channel: crate::canvas::PaintChannel::Fill,
                cap: Default::default(),
                join: Default::default(),
                stroke: [35, 40, 48, 255],
                stroke_width: 1.5,
                layer: 1,
                snap: Default::default(),
            },
            path_paints: Default::default(),
            inspector_tab: Default::default(),
            panel_layout: Default::default(),
            visible_panels: Tab::DEFAULT_VISIBLE,
            selection: Vec::new(),
            canvas: Canvas::default(),
            show_inspector: true,
            show_timeline: true,
            timeline: Timeline::default(),
            exports: Exports::default(),
            library: Default::default(),
            folder_assets: Default::default(),
            text_dialog: None,
            status: String::new(),
            notices: Default::default(),
            window_title: String::new(),
        }
    }

    pub(crate) fn install_canvas_filter(
        &mut self,
        ctx: &egui::Context,
        gl: &eframe::glow::Context,
    ) {
        self.canvas_filter = crate::canvas::display_filter::Filter::install(ctx, gl);
    }

    pub fn attach_diagnostics(&mut self, store: Option<Diagnostics>) {
        self.crash_reports.initialize(store.as_ref());
        self.diagnostic_context = None;
        self.shown_worker_reports = store.as_ref().map_or(0, Diagnostics::worker_reports);
        self.notices.worker_reports_seen = self.shown_worker_reports;
        self.canvas.set_diagnostics(store.clone());
        self.diagnostics = store;
    }

    fn note(&self, action: Action) {
        if let Some(store) = &self.diagnostics {
            store.note(action);
        }
    }

    fn open_crash_reports(&mut self) {
        self.shown_worker_reports = self
            .diagnostics
            .as_ref()
            .map_or(0, Diagnostics::worker_reports);
        self.crash_reports.open(self.diagnostics.as_ref());
    }

    fn update_diagnostic_context(&mut self, ctx: &egui::Context) {
        let Some(store) = &self.diagnostics else {
            return;
        };
        let doc = self.scene();
        let context = diagnostics::Context {
            clip_index: self.timeline.clip,
            tick: self.timeline.frame,
            tool: self.tool.into(),
            layer_count: doc.layers.len(),
            object_count: doc.objects.len(),
            selection_count: self.selection.len(),
            page_size: [doc.width, doc.height],
            pixels_per_point: ctx.pixels_per_point(),
            window: ctx.input(|i| match (i.viewport().minimized, i.viewport().focused) {
                (Some(true), _) => Some(diagnostics::WindowState::Minimized),
                (_, Some(true)) => Some(diagnostics::WindowState::Focused),
                (_, Some(false)) => Some(diagnostics::WindowState::Background),
                _ => None,
            }),
        };
        if self.diagnostic_context != Some(context) && store.context(context) {
            self.diagnostic_context = Some(context);
        }
    }

    fn save(&mut self, choose_path: bool) -> bool {
        self.note(Action::Save);
        self.timeline.stop();
        self.finish_edit();
        let (path, replace) = if choose_path || self.path.is_none() {
            let name = self.name();
            let default = Path::new(&name).with_extension("vanim");
            let Some(mut path) = diagnostics::native_dialog(|| {
                FileDialog::new()
                    .add_filter("Phyro project", &["vanim"])
                    .set_file_name(default.to_string_lossy().as_ref())
                    .save_file()
            }) else {
                return false;
            };
            if path.extension().is_none() {
                path.set_extension("vanim");
            }
            if self.path_open_elsewhere(&path) {
                self.report_warning(
                    "Choose a different save location",
                    format!("{} is already open in another tab.", path.display()),
                );
                return false;
            }
            // Check the final path, including an extension added after the dialog.
            let replace = path.exists();
            if replace && self.path.as_ref() != Some(&path) {
                let answer = diagnostics::native_dialog(|| {
                    MessageDialog::new()
                        .set_title("Replace project?")
                        .set_description(format!("Replace {}?", path.display()))
                        .set_buttons(MessageButtons::YesNo)
                        .show()
                });
                if answer != MessageDialogResult::Yes {
                    return false;
                }
            }
            (path, replace)
        } else if let Some(path) = &self.path {
            (path.clone(), true)
        } else {
            return false;
        };
        self.save_to(path, replace)
    }

    fn save_to(&mut self, path: PathBuf, replace: bool) -> bool {
        if self.path_open_elsewhere(&path) {
            self.report_warning(
                "Choose a different save location",
                format!("{} is already open in another tab.", path.display()),
            );
            return false;
        }
        let result = project_io::save_animation(&path, &self.project, replace);
        match result {
            Ok(()) => {
                self.status = format!("Saved {}", path.display());
                self.update_recent(&path, true);
                self.canonical_path = Some(documents::path_key(&path));
                self.path = Some(path);
                self.saved = Some(self.project.clone());
                self.baseline_is_saved = true;
                self.recovered_name = None;
                self.retire_draft();
                true
            }
            Err(err) => {
                self.report_error(
                    "Could not save project",
                    format!(
                        "Use File > Save as to try another location.\nYour changes are still open.\n{}\n{err}",
                        path.display()
                    ),
                );
                false
            }
        }
    }

    fn confirm_switch(&mut self) -> OpenDecision {
        if self.saved.as_ref() == Some(&self.project) {
            return OpenDecision::Proceed;
        }
        self.timeline.stop();
        match diagnostics::native_dialog(|| {
            MessageDialog::new()
                .set_title("Unsaved changes")
                .set_description(format!(
                    "Save changes to {} before continuing?",
                    self.name()
                ))
                .set_buttons(MessageButtons::YesNoCancel)
                .show()
        }) {
            MessageDialogResult::Yes if self.save(false) => OpenDecision::Saved,
            MessageDialogResult::No => OpenDecision::Proceed,
            _ => OpenDecision::Cancel,
        }
    }

    fn open(&mut self, ctx: &egui::Context) {
        self.timeline.stop();
        let Some(paths) = diagnostics::native_dialog(|| {
            FileDialog::new()
                .add_filter("Phyro project", &["vanim"])
                .pick_files()
        }) else {
            return;
        };
        self.open_paths(ctx, paths);
    }

    fn open_failure_details(&mut self, path: &Path, error: std::io::Error) -> String {
        if error.kind() == std::io::ErrorKind::NotFound {
            self.update_recent(path, false);
        }
        format!("Could not open {}: {error}", path.display())
    }

    pub fn open_project(&mut self, path: PathBuf) {
        if self.focus_open_path(&path) {
            return;
        }
        self.note(Action::Open);
        match project_io::load_animation(&path) {
            Ok(document) => self.open_loaded(path, document),
            Err(error) => {
                let details = self.open_failure_details(&path, error);
                self.report_error("Could not open project", details);
            }
        }
    }

    fn open_loaded(&mut self, path: PathBuf, document: AnimationProject) {
        let key = documents::path_key(&path);
        self.open_loaded_with_key(path, key, document);
    }

    fn open_loaded_with_key(&mut self, path: PathBuf, key: PathBuf, document: AnimationProject) {
        if let Some(index) = self.document_index(&path, Some(&key)) {
            self.focus_file_tab(index);
            return;
        }
        self.update_recent(&path, true);
        self.open_document_with_key(document, Some(path), Some(key));
        self.status = format!("Opened {}", self.name());
    }

    fn scene(&self) -> &Document {
        self.project.clips[self.timeline.clip].drawing_at(self.timeline.frame)
    }

    fn initial_layer(doc: &Document) -> u64 {
        doc.layers
            .iter()
            .find(|layer| layer.visible && !layer.locked)
            .unwrap_or(&doc.layers[0])
            .id
    }

    fn replace_project(&mut self, project: AnimationProject, path: Option<PathBuf>) {
        let key = path.as_deref().map(documents::path_key);
        self.replace_project_with_key(project, path, key);
    }

    fn replace_project_with_key(
        &mut self,
        project: AnimationProject,
        path: Option<PathBuf>,
        key: Option<PathBuf>,
    ) {
        self.revoke_shared_document();
        self.document_id = assistant::document_id();
        self.documents.reusable_blank = false;
        self.retire_draft();
        self.recovered_name = None;
        self.saved = Some(project.clone());
        self.baseline = project.clone();
        self.writable_cel = None;
        self.baseline_is_saved = true;
        self.pending_edit = false;
        self.project = project;
        self.canonical_path = key;
        self.path = path;
        self.selection.clear();
        self.history = History::default();
        self.timeline.reset_for_document();
        self.style.layer = Self::initial_layer(self.scene());
        self.tool = if self.scene().objects.is_empty() {
            Tool::Rectangle
        } else {
            Tool::Select
        };
        self.path_paints.restore(self.tool, &mut self.style);
        let checkerboard = self.canvas.show_checkerboard();
        self.canvas = Canvas::default();
        self.canvas.set_checkerboard(checkerboard);
        self.canvas.set_diagnostics(self.diagnostics.clone());
        self.exports.reset_destination();
        self.library.reset_project();
        self.folder_assets = Default::default();
        self.text_dialog = None;
        self.artboard_size = None;
        self.guided_morph = None;
    }

    fn finish_edit(&mut self) {
        self.pending_edit = false;
        // Export controls have a separate preview key. An artwork edit invalidates the snapshot.
        let artwork_changed = !self.exports.take_animation_option_edit();
        if self.baseline != self.project {
            let clip = &mut self.project.clips[self.timeline.clip];
            let synced = self
                .baseline
                .clips
                .get(self.timeline.clip)
                .map_or(Ok(()), |before| {
                    clip.sync_layer_edits(before, self.timeline.frame)
                });
            self.record_project_edit(synced, artwork_changed);
        }
    }

    fn record_synced_edit(&mut self, synced: Result<(), String>) {
        self.record_project_edit(synced, true);
    }

    fn record_project_edit(&mut self, synced: Result<(), String>, artwork_changed: bool) {
        if let Err(error) = synced.and_then(|()| self.project.hydrate_symbols()) {
            self.project.clone_from(&self.baseline);
            self.timeline.clip = self.timeline.clip.min(self.project.clips.len() - 1);
            self.timeline.frame = self
                .timeline
                .frame
                .min(self.project.clips[self.timeline.clip].frames - 1);
            self.tidy_selection();
            self.report_error("Could not apply edit", error);
            return;
        }
        self.history.record(
            &self.baseline,
            &self.project,
            self.timeline.clip,
            self.timeline.frame,
        );
        if artwork_changed {
            self.capture_baseline();
        } else {
            self.update_baseline();
        }
    }

    fn enter_cel(&mut self, clip_session: u64, start: u32) {
        let active = (clip_session, start);
        let Some((previous, frame)) = self.writable_cel.replace(active) else {
            return;
        };
        if active == (previous, frame) {
            return;
        }
        let original = self
            .baseline
            .clips
            .iter()
            .find(|clip| clip.session_id() == previous)
            .and_then(|clip| clip.cels.get(&frame));
        let edited = self
            .project
            .clips
            .iter_mut()
            .find(|clip| clip.session_id() == previous)
            .and_then(|clip| clip.cels.get_mut(&frame));
        if let (Some(original), Some(edited)) = (original, edited) {
            if !std::sync::Arc::ptr_eq(original, edited) && original.as_ref() == edited.as_ref() {
                *edited = std::sync::Arc::clone(original);
            }
        }
    }

    fn capture_baseline(&mut self) {
        self.exports.artwork_changed();
        self.update_baseline();
    }

    fn update_baseline(&mut self) {
        self.baseline.clone_from(&self.project);
        self.baseline_is_saved = self.saved.as_ref() == Some(&self.baseline);
        self.record_draft_change();
    }

    fn can_undo(&self) -> bool {
        self.history.can_undo() || self.pending_edit
    }

    fn drawing_location(&self) -> (usize, u32) {
        let clip = &self.project.clips[self.timeline.clip];
        (self.timeline.clip, clip.exposure(self.timeline.frame).0)
    }

    /// Other layers can switch drawings while selected artwork is still on its hold.
    fn selection_crossed_drawing(&self, previous: (usize, u32)) -> bool {
        let current = self.drawing_location();
        if previous == current {
            return false;
        }
        if previous.0 != current.0 || self.selection.is_empty() {
            return true;
        }
        let clip = &self.project.clips[current.0];
        let Some(before) = clip.cels.get(&previous.1) else {
            return true;
        };
        // A legacy whole-drawing cut starts a new exposure on every layer.
        if clip.layer_starts.is_empty() {
            return true;
        }
        if let [id] = self.selection.as_slice() {
            return before.objects.iter().find(|o| o.id == *id).is_none_or(|o| {
                clip.layer_exposure(previous.1, o.layer).0
                    != clip.layer_exposure(current.1, o.layer).0
            });
        }
        let ids: std::collections::HashSet<_> = self.selection.iter().copied().collect();
        let layers: std::collections::HashSet<_> = before
            .objects
            .iter()
            .filter(|o| ids.contains(&o.id))
            .map(|o| o.layer)
            .collect();
        layers.is_empty()
            || layers.into_iter().any(|layer| {
                clip.layer_exposure(previous.1, layer).0 != clip.layer_exposure(current.1, layer).0
            })
    }

    fn tidy_selection(&mut self) {
        let doc = self.project.clips[self.timeline.clip].drawing_at(self.timeline.frame);
        if !doc.layers.iter().any(|l| l.id == self.style.layer) {
            self.style.layer = Self::initial_layer(doc);
        }
        commands::retain_editable_selection(doc, &mut self.selection, self.style.layer);
    }

    /// An explicit artwork selection takes over keyboard editing from the timeline.
    fn select_artwork(&mut self, ids: Vec<u64>) {
        self.timeline.leave_grid_selection();
        self.selection = ids;
    }

    fn sync_paint(&mut self) {
        let doc = self.project.clips[self.timeline.clip].drawing_at(self.timeline.frame);
        if let Some(obj) = self
            .selection
            .last()
            .and_then(|id| doc.objects.iter().find(|o| o.id == *id))
        {
            if crate::canvas::PathPaintMemory::is_drawing_tool(self.tool) {
                self.style.layer = obj.layer;
            } else {
                self.style.read_paint(obj, self.timeline.frame);
            }
        } else {
            self.path_paints.restore(self.tool, &mut self.style);
        }
    }

    fn restore(&mut self, redo: bool) {
        self.note(if redo { Action::Redo } else { Action::Undo });
        self.timeline.stop();
        self.canvas.cancel_drag();
        self.finish_edit();
        let old_view = self.drawing_location();
        let restored = if redo {
            self.history.redo(&mut self.project)
        } else {
            self.history.undo(&mut self.project)
        };
        if restored {
            if let Some((clip, frame)) = self.history.location() {
                let clip = clip.min(self.project.clips.len() - 1);
                self.timeline
                    .restore_playhead(clip, frame.min(self.project.clips[clip].frames - 1));
            }
            if self.selection_crossed_drawing(old_view) {
                self.selection.clear();
            }
        }
        self.capture_baseline();
        self.tidy_selection();
        if self.tool == Tool::Select {
            self.selection = commands::editable_selection(self.scene(), &self.selection);
        }
        self.sync_paint();
    }

    fn paste(&mut self) {
        if self.clipboard.is_empty() || !commands::is_editable(self.scene(), self.style.layer) {
            return;
        }
        self.timeline.stop();
        self.canvas.cancel_drag();
        match self.clipboard.paste_project(
            &mut self.project,
            self.timeline.clip,
            self.timeline.frame,
            self.style.layer,
        ) {
            Ok(ids) => self.select_artwork(ids),
            Err(error) => {
                self.report_error("Could not paste artwork", error);
                return;
            }
        }
        self.tool = Tool::Select;
        self.sync_paint();
        self.finish_edit();
    }
    fn duplicate_selection(&mut self) {
        self.note(Action::Duplicate);
        if self.selection.is_empty() {
            return;
        }
        self.finish_edit();
        let doc = self.project.clips[self.timeline.clip].drawing_at_mut(self.timeline.frame);
        let copies = commands::duplicate(doc, &self.selection);
        if !copies.is_empty() {
            self.selection = copies;
            self.set_tool(Tool::Select);
            self.finish_edit();
            if let [id] = self.selection.as_slice() {
                let frame = self.timeline.frame;
                let drawing = self.project.clips[self.timeline.clip].drawing_snapshot_mut(frame);
                self.canvas.show_duplicated_placement(drawing, frame, *id);
            }
        }
    }

    fn stack_selection(&mut self, order: commands::Arrange) {
        self.note(Action::Stack);
        self.finish_edit();
        let doc = self.project.clips[self.timeline.clip].drawing_at_mut(self.timeline.frame);
        commands::arrange(doc, &self.selection, self.timeline.frame, order);
        self.finish_edit();
    }

    fn combine_nodes(&mut self, ctx: &egui::Context) {
        let frame = self.timeline.frame;
        if self.pending_edit {
            self.finish_edit();
        }
        let doc = self.project.clips[self.timeline.clip].drawing_at_mut(frame);
        if crate::path_editor::combine_nodes(
            ctx,
            doc,
            &self.selection,
            frame,
            self.timeline.key_mode,
        ) {
            self.note(Action::EditPath);
            self.finish_edit();
        }
    }

    fn flip_selection(&mut self, horizontal: bool) {
        self.finish_edit();
        let frame = self.timeline.frame;
        let drawing = self.project.clips[self.timeline.clip].drawing_at_mut(frame);
        if crate::canvas::flip_selection(
            drawing,
            &self.selection,
            frame,
            horizontal,
            self.timeline.key_mode,
        ) {
            self.status.clear();
            self.finish_edit();
        } else {
            self.report_warning(
                "Could not flip selection",
                "The selection cannot be flipped at this frame.",
            );
        }
    }

    fn set_tool(&mut self, tool: Tool) {
        self.timeline.leave_grid_selection();
        self.timeline.stop();
        self.canvas.rig.cancel_placement();
        self.canvas.cancel_drag();
        self.path_paints.switch(self.tool, tool, &mut self.style);
        self.tool = tool;
        if tool == Tool::Text && self.visible_panels[crate::inspector::Tab::Properties as usize] {
            self.inspector_tab = crate::inspector::Tab::Properties;
        }
        if tool == Tool::Select {
            self.selection = commands::editable_selection(self.scene(), &self.selection);
            self.sync_paint();
        }
        self.status.clear();
    }
    fn import_svg(&mut self) {
        self.note(Action::Open);
        self.timeline.stop();
        let Some(path) = diagnostics::native_dialog(|| {
            FileDialog::new()
                .add_filter("SVG artwork", &["svg"])
                .pick_file()
        }) else {
            return;
        };
        match project_io::import_svg(&path) {
            Ok(doc) if self.allow_document_change() => {
                self.open_document_tab(AnimationProject::from_document(doc), None);
                self.saved = None;
                self.baseline_is_saved = false;
                self.record_draft_change();
                self.status = format!(
                    "Imported {}. Save as a Phyro project to keep editing.",
                    path.display()
                );
            }
            Ok(_) => {}
            Err(error) => self.report_error(
                "Could not import SVG",
                format!("{}\n{error}", path.display()),
            ),
        }
    }
    fn edit_typography(&mut self, id: u64) {
        self.note(Action::EditText);
        let Some(style) = self
            .scene()
            .objects
            .iter()
            .find(|o| o.id == id && commands::is_editable(self.scene(), o.layer))
            .and_then(|o| o.text.clone())
        else {
            return;
        };
        self.timeline.stop();
        self.canvas.cancel_drag();
        self.text_dialog = Some(crate::type_editor::TypeEditor::new(id, style));
    }

    fn text_window(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.text_dialog.as_mut() else {
            return;
        };
        let doc = self.project.clips[self.timeline.clip].drawing_at_mut(self.timeline.frame);
        match dialog.show(ctx, doc, self.timeline.frame) {
            crate::type_editor::Outcome::Editing => {}
            crate::type_editor::Outcome::Cancelled => self.text_dialog = None,
            crate::type_editor::Outcome::Applied(id) => {
                self.text_dialog = None;
                self.select_artwork(vec![id]);
                self.tool = Tool::Select;
                self.sync_paint();
                self.finish_edit();
            }
        }
    }

    fn apply_path_operation(&mut self, op: document_core::BooleanOp) {
        use document_core::BooleanOp;
        self.note(match op {
            BooleanOp::Union => Action::Union,
            BooleanOp::Difference => Action::Difference,
            BooleanOp::Intersection => Action::Intersection,
            BooleanOp::Exclusion => Action::Exclusion,
            BooleanOp::Division => Action::Division,
            BooleanOp::CutPath => Action::CutPath,
        });
        self.finish_edit();
        self.canvas.cancel_drag();
        let frame = self.timeline.frame;
        let drawing = self.project.clips[self.timeline.clip].drawing_at_mut(frame);
        match drawing.boolean(&self.selection, frame, op) {
            Ok(ids) => {
                self.select_artwork(ids);
                self.tool = Tool::Select;
                self.sync_paint();
                self.status.clear();
                self.finish_edit();
            }
            Err(error) => self.report_error("Could not apply path operation", error),
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let keyboard_owned = self.menu_bar_open
            || self.artwork_menu_open
            || self.crash_reports.is_open()
            || self.canvas.text_active()
            || keymap::widget_owns_shortcuts(ctx)
            || ctx.memory(|m| m.any_popup_open() || m.top_modal_layer().is_some())
            || self.text_dialog.is_some()
            || self.artboard_size.is_some()
            || self.library.is_editing();
        if !keyboard_owned && !self.exports.is_open() && self.canvas.cancel_placement(ctx) {
            return;
        }
        let allow_canvas_edit = !keyboard_owned
            && !self.exports.is_open()
            && !self.timeline.is_playing()
            && !self.canvas.rig.placing()
            && !self.canvas.path_in_progress()
            && !ctx.input(|i| i.pointer.any_down());
        let edits = self.path_shortcuts.take(
            ctx,
            if allow_canvas_edit {
                self.selection.len()
            } else {
                0
            },
        );
        for op in edits {
            self.apply_path_operation(op);
        }
        if keyboard_owned {
            return;
        }
        if !self.exports.is_open()
            && !ctx.input(|i| i.pointer.any_down())
            && self.document_tab_shortcuts(ctx)
        {
            return;
        }
        if allow_canvas_edit && self.conversion_shortcuts(ctx) {
            return;
        }
        if allow_canvas_edit
            && keymap::pressed(ctx, Shortcut::LockObjects)
            && !self.selection.is_empty()
        {
            self.finish_edit();
            let doc = self.project.clips[self.timeline.clip].drawing_at_mut(self.timeline.frame);
            document_core::arrange::set_object_locks(doc, &self.selection, true);
            self.selection.clear();
            self.finish_edit();
            return;
        }
        if allow_canvas_edit && self.tool == Tool::Node {
            if keymap::pressed(ctx, Shortcut::CombineNodes) {
                self.combine_nodes(ctx);
                return;
            }
            let styles = crate::path_editor::styles::shortcuts(ctx);
            if !styles.is_empty() {
                for style in styles {
                    self.style_nodes(ctx, style);
                }
                return;
            }
        }
        if allow_canvas_edit && !self.selection.is_empty() {
            use crate::tool_keys::{selection_shortcuts, FLIPS, STACKING};
            let flips = if self.canvas.moving_selection() {
                Vec::new()
            } else {
                selection_shortcuts(ctx, &FLIPS, false)
            };
            if !flips.is_empty() {
                for horizontal in flips {
                    self.flip_selection(horizontal);
                }
                return;
            }
            let orders = selection_shortcuts(ctx, &STACKING, true);
            if !orders.is_empty() {
                for order in orders {
                    self.stack_selection(order);
                }
                return;
            }
        }
        if self.canvas.rig.placing() || self.canvas.rig.dragging_handle() {
            if ctx.input(|i| i.key_pressed(Key::Escape)) {
                self.canvas.rig.cancel_placement();
            }
            return;
        }
        if self.edit_shortcuts(ctx) {
            return;
        }
        if keymap::pressed(ctx, Shortcut::Save) {
            self.save(false);
            return;
        }
        if keymap::pressed(ctx, Shortcut::SaveAs) {
            self.save(true);
            return;
        }
        if keymap::pressed(ctx, Shortcut::Open) {
            self.open(ctx);
            return;
        }
        if !self.exports.is_open()
            && !self.canvas.path_in_progress()
            && !ctx.memory(|memory| memory.any_popup_open() || memory.top_modal_layer().is_some())
        {
            let actions = crate::timeline::AnimationAction::from_input(ctx);
            if !actions.is_empty() {
                for action in actions {
                    self.animation_action(action);
                }
                return;
            }
        }
        if keymap::pressed(ctx, Shortcut::Play) {
            self.note(Action::Playback);
            self.canvas.cancel_drag();
            self.timeline
                .toggle_clip(&self.project.clips[self.timeline.clip]);
        } else if let Some(tool) = crate::tool_keys::pressed(ctx) {
            self.set_tool(tool);
        }
    }

    fn style_nodes(&mut self, ctx: &egui::Context, style: document_core::path_edit::HandleStyle) {
        if self.pending_edit {
            self.finish_edit();
        }
        let frame = self.timeline.frame;
        let doc = self.project.clips[self.timeline.clip].drawing_at_mut(frame);
        if crate::path_editor::styles::apply(
            ctx,
            doc,
            &self.selection,
            frame,
            self.timeline.key_mode,
            style,
        ) {
            self.note(Action::EditPath);
            self.status.clear();
            self.finish_edit();
        }
    }

    fn animation_action(&mut self, action: crate::timeline::AnimationAction) {
        self.note(Action::Timeline);
        if self.pending_edit {
            self.finish_edit();
        }
        let drawing = self.drawing_location();
        let frame = self.timeline.frame;
        let clip = self.timeline.clip;
        let selected = self.single();
        let changed = self.timeline.run_action(
            action,
            &mut self.project.clips[clip],
            selected,
            self.style.layer,
        );
        if changed || self.timeline.frame != frame {
            self.canvas.cancel_drag();
            if self.selection_crossed_drawing(drawing) {
                self.selection.clear();
            }
            self.tidy_selection();
            self.sync_paint();
        }
        if changed {
            self.finish_edit();
        }
    }

    fn inspector_body(
        &mut self,
        ui: &mut egui::Ui,
        tab: crate::inspector::Tab,
        reveal_align: bool,
    ) {
        use crate::inspector::{self, Tab};
        let index = self.timeline.clip;
        let frame = self.timeline.frame;
        match tab {
            Tab::Properties => {
                if self.tool == Tool::Text {
                    self.canvas.text_controls(ui);
                    if self.canvas.text_active() {
                        return;
                    }
                    ui.separator();
                }
                let edit = matches!(
                    self.tool,
                    Tool::Select | Tool::Node | Tool::Eyedropper | Tool::Gradient
                );
                let snapshot = self
                    .baseline
                    .clips
                    .get(index)
                    .and_then(|c| c.cels.range(..=frame).next_back().map(|(_, d)| d));
                inspector::selection_title(ui, self.scene(), snapshot, &self.selection, frame);
                if let Some(object) = self.single().and_then(|id| {
                    self.scene().objects.iter().find(|object| {
                        object.id == id
                            && object.can_guided_morph()
                            && commands::is_editable(self.scene(), object.layer)
                    })
                }) {
                    let label = if object.guided_morph_span(frame).is_some() {
                        "Edit morph…"
                    } else {
                        "Guided morph…"
                    };
                    if ui
                        .button(label)
                        .on_hover_text("Preview the shape animation and adjust matching points.")
                        .clicked()
                    {
                        self.open_guided_morph(object.id);
                    }
                }
                if let Some(message) = crate::composition_panel::playback(
                    ui,
                    &mut self.project,
                    index,
                    frame,
                    &self.selection,
                ) {
                    self.report_error("Could not change clip playback", message);
                }
                self.library
                    .edit_copy_button(ui, &self.project, index, frame, &self.selection);
                if let Some(id) = self.single().filter(|id| {
                    self.scene().objects.iter().any(|o| {
                        o.id == *id
                            && o.text.is_some()
                            && commands::is_editable(self.scene(), o.layer)
                    })
                }) {
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("Edit text").clicked() {
                            self.edit_typography(id);
                        }
                        self.convert_to_path_button(ui, true);
                    });
                }
                let previous_paint = crate::canvas::DrawingPaint::read(&self.style);
                let doc = self.project.clips[index].drawing_at_mut(frame);
                if let Some(channel) = inspector::properties(
                    ui,
                    doc,
                    &mut self.selection,
                    frame,
                    &mut self.style,
                    edit,
                    reveal_align,
                ) {
                    self.canvas.property_pick = Some(channel);
                }
                inspector::snapping(ui, doc, &mut self.style, false);
                let paint = crate::canvas::DrawingPaint::read(&self.style);
                if paint != previous_paint {
                    self.path_paints.record_edit(&previous_paint, &paint);
                    self.note(Action::Paint);
                }
            }
            Tab::Layers => {
                let clip = &mut self.project.clips[index];
                ui.push_id(clip.session_id(), |ui| {
                    inspector::layers(ui, clip, &mut self.selection, frame, &mut self.style);
                });
            }
            Tab::Library => {
                if let Some(message) = self.library.panel(
                    ui,
                    &mut self.project,
                    index,
                    frame,
                    self.style.layer,
                    &mut self.selection,
                ) {
                    self.status = message;
                }
            }
            Tab::Document => self.document_properties(ui),
            Tab::Rig => {
                self.canvas.rig.show(
                    ui,
                    &mut self.project.clips[index],
                    &self.selection,
                    frame,
                    self.timeline.key_mode,
                );
            }
        }
    }

    fn single(&self) -> Option<u64> {
        match self.selection.as_slice() {
            [id] => Some(*id),
            _ => None,
        }
    }

    fn morph_action(&mut self, ui: &mut egui::Ui, enabled: bool) -> bool {
        match *self.selection.as_slice() {
            [source, target] => {
                let eligible = guided_morph::pair_eligible(self.scene(), source, target);
                if ui
                    .add_enabled(eligible && enabled, egui::Button::new("Morph"))
                    .on_hover_text("Animate the first selected shape into the second.")
                    .on_disabled_hover_text(
                        "Choose two editable single outlines, both open or both closed.",
                    )
                    .clicked()
                {
                    self.open_selection_morph(source, target);
                    return true;
                }
            }
            [id] => {
                let has_morph = self
                    .scene()
                    .objects
                    .iter()
                    .find(|o| o.id == id)
                    .is_some_and(|o| {
                        commands::is_editable(self.scene(), o.layer)
                            && o.guided_morph_span(self.timeline.frame).is_some()
                    });
                if !has_morph {
                    return false;
                }
                if ui
                    .add_enabled(enabled, egui::Button::new("Edit morph…"))
                    .on_hover_text("Preview this morph and adjust its matching points.")
                    .clicked()
                {
                    self.open_guided_morph(id);
                    return true;
                }
            }
            _ => {}
        }
        false
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        egui::menu::bar(ui, |ui| {
            ui.set_min_height(28.);
            ui.menu_button("File", |ui| {
                let keys = keymap::active(ui.ctx());
                if ui.button("New").clicked() {
                    ui.close_menu();
                    self.new_document_tab();
                }
                if ui
                    .add(
                        egui::Button::new("Open project…")
                            .shortcut_text(keys.label(ui.ctx(), Shortcut::Open)),
                    )
                    .clicked()
                {
                    ui.close_menu();
                    self.open(ui.ctx());
                }
                self.open_recent_menu(ui);
                if ui.button("Share this tab with an assistant…").clicked() {
                    self.assistant.open = true;
                    ui.close_menu();
                }
                if ui.button("Recover drafts…").clicked() {
                    ui.close_menu();
                    self.timeline.stop();
                    self.recovery_dialog
                        .scan(self.drafts.as_ref().map(|d| d.root.as_path()), false);
                }
                ui.separator();
                if ui
                    .add(
                        egui::Button::new("Save")
                            .shortcut_text(keys.label(ui.ctx(), Shortcut::Save)),
                    )
                    .clicked()
                {
                    ui.close_menu();
                    self.save(false);
                }
                if ui
                    .add(
                        egui::Button::new("Save as…")
                            .shortcut_text(keys.label(ui.ctx(), Shortcut::SaveAs)),
                    )
                    .clicked()
                {
                    ui.close_menu();
                    self.save(true);
                }
                self.document_tab_menu(ui, !self.exports.is_open() && !self.exports.busy());
                ui.separator();
                if ui.button("Import SVG as project…").clicked() {
                    ui.close_menu();
                    self.import_svg();
                }
                if ui
                    .button("Import Reference Image…")
                    .on_hover_text("Place an image behind the current drawing as a tracing guide")
                    .clicked()
                {
                    ui.close_menu();
                    self.choose_reference_image();
                }
                ui.separator();
                if ui.button("Artboard size…").clicked() {
                    ui.close_menu();
                    self.open_artboard_size();
                }
            });
            self.text_menu.popup = egui::Rect::NOTHING;
            let edit_menu = ui.menu_button("Edit", |ui| {
                self.edit_menu(ui);
                ui.separator();
                if ui.button("Keyboard shortcuts…").clicked() {
                    self.timeline.stop();
                    self.keyboard_settings.open();
                    ui.close_menu();
                }
                self.text_menu.popup = ui.min_rect();
            });
            self.text_menu.button = edit_menu.response.rect;
            ui.menu_button("Object", |ui| self.object_menu(ui));
            if self.assistant.sharing() && ui.button("Stop sharing").clicked() {
                self.assistant = Default::default();
                self.status = "Assistant sharing stopped.".into();
            }
            ui.menu_button("Path", |ui| {
                self.path_conversion_commands(ui);
                ui.separator();
                let chosen = ui
                    .add_enabled_ui(!self.timeline.is_playing(), |ui| {
                        crate::path_ops::choose(ui, self.selection.len(), true)
                    })
                    .inner;
                if let Some(op) = chosen {
                    ui.close_menu();
                    self.apply_path_operation(op);
                }
            });
            ui.menu_button("View", |ui| {
                if ui.button("Fit artboard").clicked() {
                    self.note(Action::View);
                    self.canvas.fit();
                    ui.close_menu();
                }
                ui.separator();
                if ui.checkbox(&mut self.show_inspector, "Inspector").changed() {
                    if !self.show_inspector {
                        self.canvas.rig.leave_inspector();
                    }
                    ui.close_menu();
                }
                if ui.checkbox(&mut self.show_timeline, "Timeline").changed() {
                    self.timeline.leave_grid_selection();
                    ui.close_menu();
                }
                self.panel_menu(ui);
                ui.separator();
                let mut checkerboard = self.canvas.show_checkerboard();
                if ui
                    .checkbox(&mut checkerboard, "Transparency checkerboard")
                    .changed()
                {
                    self.canvas.set_checkerboard(checkerboard);
                    ui.close_menu();
                }
            });
            ui.menu_button("Export", |ui| {
                self.note(Action::Export);
                ui.weak("Current frame");
                if let Some(message) = self.exports.menu(
                    ui,
                    &self.project,
                    self.timeline.clip,
                    self.timeline.frame,
                    &self.selection,
                ) {
                    self.report_error("Could not start export", message);
                }
                ui.separator();
                if ui
                    .add_enabled(
                        self.exports.can_reexport(&self.project, self.timeline.clip),
                        egui::Button::new("Re-export"),
                    )
                    .clicked()
                {
                    if let Some(message) =
                        self.exports
                            .reexport(&self.project, self.timeline.clip, ui.ctx())
                    {
                        self.report_error("Could not re-export animation", message);
                    }
                    ui.close_menu();
                }
            });
            ui.menu_button("Help", |ui| {
                if ui
                    .add_enabled(
                        self.notices.has_history(),
                        egui::Button::new("Recent issues"),
                    )
                    .clicked()
                {
                    ui.close_menu();
                    self.notices.open_history();
                }
                if ui.button("Crash reports…").clicked() {
                    ui.close_menu();
                    self.open_crash_reports();
                }
            });
            // egui's menu bar keeps its own state outside the popup stack. Give
            // Escape to the open menu before the canvas sees the same event.
            if self.menu_bar_open {
                ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape));
            }
            self.menu_bar_open = egui::menu::BarState::load(ui.ctx(), ui.id()).is_some();
            if self.tool == Tool::Gradient {
                self.style.gradient_controls(ui);
            }
            if self.tool == Tool::Select {
                let enabled = !self.timeline.is_playing() && !self.canvas.rig.placing();
                self.morph_action(ui, enabled);
                if self.canvas.pivot_controls(
                    ui,
                    &mut self.project.clips[self.timeline.clip],
                    self.timeline.frame,
                    &self.selection,
                    enabled,
                ) {
                    self.note(Action::Transform);
                    self.finish_edit();
                }
            }
            if self.tool == Tool::Pencil {
                ui.separator();
                crate::tool_keys::pencil_controls(
                    ui,
                    &mut self.pencil_smoothing,
                    &mut self.pencil_pressure,
                );
            }
            if self.tool == Tool::Node {
                ui.separator();
                let enabled = !self.timeline.is_playing() && !self.canvas.rig.placing();
                if let Some(style) = crate::path_editor::styles::toolbar(
                    ui,
                    self.scene(),
                    &self.selection,
                    self.timeline.frame,
                    self.timeline.key_mode,
                    enabled,
                ) {
                    self.style_nodes(ui.ctx(), style);
                }
                if crate::path_editor::combine_button(
                    ui,
                    self.scene(),
                    &self.selection,
                    self.timeline.frame,
                    self.timeline.key_mode,
                    enabled,
                ) {
                    self.combine_nodes(ui.ctx());
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.checkbox(&mut self.style.snap.enabled, "Snap")
                    .on_hover_text("Snap to the enabled grid, guides, bounds and anchors");
                if ui
                    .button("Fit")
                    .on_hover_text("Fit the artboard in the canvas")
                    .clicked()
                {
                    self.note(Action::View);
                    self.canvas.fit();
                }
            });
        });
    }

    fn artboard(&mut self, ui: &mut egui::Ui) {
        self.timeline.leave_if_pressed(ui);
        let menu_available = !self.canvas.path_in_progress()
            && !self.canvas.text_active()
            && !self.canvas.rig.placing()
            && !self.canvas.rig.dragging_handle()
            && !self.timeline.is_playing()
            && !self.exports.is_open()
            && !self.library.is_editing()
            && self.artboard_size.is_none()
            && self.text_dialog.is_none()
            && !self.crash_reports.is_open();
        self.canvas
            .set_pencil_options(self.pencil_smoothing, self.pencil_pressure);
        self.style.snap.grid_settings = self.project.grid;
        self.path_paints.remember(self.tool, &self.style);
        let previous = self.selection.clone();
        let t = &self.timeline;
        self.canvas.set_animation_overlay(
            &self.project.clips[t.clip],
            t.frame,
            [
                (if t.onion { t.previous_onions } else { 0 }, t.previous_tint),
                (if t.onion { t.next_onions } else { 0 }, t.next_tint),
            ],
            !t.is_playing(),
        );
        let result = self.canvas.show(
            ui,
            self.project.clips[self.timeline.clip].drawing_snapshot_mut(self.timeline.frame),
            self.timeline.frame,
            self.tool,
            &mut self.selection,
            &self.style,
            !self.timeline.is_playing(),
        );
        self.notices
            .observe_canvas_error(result.as_ref().err().map(String::as_str));
        match result {
            Ok(true) if self.tool == Tool::Pencil => {
                self.status.clear();
                self.finish_edit();
                for stroke in self.canvas.take_extra_pencil_strokes() {
                    let drawing =
                        self.project.clips[self.timeline.clip].drawing_at_mut(self.timeline.frame);
                    if stroke.apply(drawing, &mut self.selection) {
                        self.finish_edit();
                    }
                }
            }
            Ok(true) if matches!(self.tool, Tool::Text | Tool::Pen) => self.status.clear(),
            Ok(true) if self.tool == Tool::Gradient => {
                let before = crate::canvas::DrawingPaint::read(&self.style);
                self.sync_paint();
                if self.selection == previous {
                    self.path_paints
                        .record_edit(&before, &crate::canvas::DrawingPaint::read(&self.style));
                }
            }
            Ok(true) if matches!(self.tool, Tool::Rectangle | Tool::Ellipse) => {
                self.tool = Tool::Select;
                self.status.clear();
            }
            _ => {}
        }
        self.artwork_menu_open = false;
        let loading = self.folder_assets.loading_point(
            ui.ctx(),
            self.timeline.clip,
            self.timeline.frame,
            self.style.layer,
        );
        self.canvas.paint_folder_drop(ui, loading, || {
            if self.folder_assets.is_pending() {
                Some("Waiting for previous asset")
            } else if self.timeline.is_playing() {
                Some("Pause playback to place")
            } else if !menu_available {
                Some("Finish the current edit to place")
            } else if !commands::is_editable(self.scene(), self.style.layer) {
                Some("Choose a visible, unlocked layer")
            } else {
                None
            }
        });
        if menu_available && ui.input(|i| i.pointer.any_released()) {
            if let Some((asset, at)) = self.canvas.take_folder_asset_drop() {
                self.place_asset(ui.ctx(), asset.source.clone(), asset.label.clone(), at);
            }
        }
        if menu_available {
            self.artwork_context_menu();
        }
        if self.selection != previous {
            self.sync_paint();
        }
        if let Some(color) = self
            .canvas
            .update_sampled_paint(&mut self.style, &mut self.path_paints)
        {
            crate::color_picker::remember(ui.ctx(), color);
        }
    }
}

impl eframe::App for Editor {
    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        if let (Some(filter), Some(gl)) = (self.canvas_filter.take(), gl) {
            filter.destroy(gl);
        }
        self.save_preferences();
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let _update = self.diagnostics.as_ref().map(Diagnostics::editor_update);
        self.show(ctx);
    }
}

impl Editor {
    fn update_phase(&self, phase: UpdatePhase) {
        if let Some(store) = &self.diagnostics {
            store.phase(phase);
        }
    }

    fn show(&mut self, ctx: &egui::Context) {
        if let Some(store) = &self.diagnostics {
            self.notices.observe_worker_reports(store.worker_reports());
        }
        if self.welcome_preview.is_some() {
            self.show_startup(ctx);
            self.notices.show(ctx, false);
            return;
        }
        // Give drawing, zooming and scrubbing first use of the frame. The folder
        // browser schedules the visible previews again when the input settles.
        let (pointer_down, resume_after_input) = ctx.input(|i| {
            (
                i.pointer.any_down(),
                i.pointer.any_released()
                    || i.smooth_scroll_delta != egui::Vec2::ZERO
                    || i.zoom_delta() != 1.,
            )
        });
        let foreground_busy = pointer_down
            || resume_after_input
            || self.timeline.is_playing()
            || self.guided_morph.is_some()
            || self.keyboard_settings.is_open()
            || self.recovery_dialog.is_open()
            || self.notices.open;
        self.library.begin_frame(foreground_busy);
        // Switching panels can remove a combo box or color swatch before its
        // popup handles the outside click. egui retains the popup ID even
        // after its foreground area disappears, blocking editor shortcuts.
        ctx.memory_mut(|memory| {
            if memory.any_popup_open()
                && !memory.layer_ids().any(|layer| {
                    layer.order == egui::Order::Foreground
                        && memory.is_popup_open(layer.id)
                        && memory.areas().visible_last_frame(&layer)
                })
            {
                memory.close_popup();
            }
        });
        self.image_clipboard.observe(ctx);
        edit_commands::guard_paste_reply(ctx);
        if self.recovery_dialog.is_open() || self.keyboard_settings.is_open() || self.notices.open {
            self.show_front_dialog(ctx);
            self.handle_close_request(ctx);
            self.show_passive_workspace(ctx);
        } else if self.guided_morph.is_some() {
            self.update_diagnostic_context(ctx);
            self.handle_close_request(ctx);
            self.show_morph_background(ctx);
            self.show_guided_morph(ctx);
            if self.guided_morph.is_none() {
                ctx.request_repaint();
            }
        } else {
            self.show_workspace(ctx, false);
            if let Some(path) = self.library.take_open_asset() {
                self.open_paths(ctx, vec![path]);
            }
            if self.folder_assets.is_pending() && !foreground_busy && self.folder_assets_idle(ctx) {
                self.poll_folder_placement(ctx);
            }
            if self.file_batch.is_some() && !foreground_busy && self.folder_assets_idle(ctx) {
                self.poll_file_batch(ctx);
            }
            if self.guided_morph.is_some() {
                self.show_guided_morph(ctx);
                ctx.request_repaint();
            } else {
                self.show_front_dialog(ctx);
            }
        }
        self.assistant_ui(ctx);
        self.library.end_frame(ctx, resume_after_input);
        self.poll_draft_recovery(ctx, std::time::Instant::now());
        crate::color_picker::recent::finish(ctx);
        self.notices
            .observe_workspace(self.timeline.error(), self.canvas.refinement_warning());
        self.notices
            .show(ctx, self.menu_bar_open || self.artwork_menu_open);
    }

    fn show_startup(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) {
            self.handle_close_request(ctx);
            return;
        }
        if self.recovery_dialog.is_open() || self.crash_reports.is_open() {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE.fill(crate::theme::SURFACE))
                .show(ctx, |_| {});
            if self.recovery_dialog.is_open() {
                self.show_recover_drafts(ctx);
            } else {
                self.crash_reports.show(ctx, self.diagnostics.as_ref());
            }
            return;
        }
        let preview = self
            .welcome_preview
            .as_mut()
            .expect("welcome screen is active");
        preview.set_opening(self.file_batch.is_some());
        let action = welcome::show(
            ctx,
            &self.recent_projects,
            &self.status,
            self.notices.has_history(),
            preview,
        );
        if self.notices.open {
            return;
        }
        match action {
            Some(welcome::Action::New) => {
                // The first, untouched tab already owns a fresh document.
                self.file_batch = None;
                self.welcome_preview = None;
                self.status.clear();
                ctx.request_repaint();
                return;
            }
            Some(welcome::Action::Open) => self.open(ctx),
            Some(welcome::Action::Recent(path)) => self.open_paths(ctx, vec![path]),
            Some(welcome::Action::RecentIssues) => self.notices.open_history(),
            None if keymap::pressed(ctx, Shortcut::Open) => self.open(ctx),
            None => self.dropped_projects(ctx),
        }
        if self.file_batch.is_some() {
            self.poll_file_batch(ctx);
        }
    }

    fn folder_assets_idle(&self, ctx: &egui::Context) -> bool {
        !self.timeline.is_playing()
            && !self.canvas.path_in_progress()
            && !self.canvas.text_active()
            && !self.canvas.rig.placing()
            && !self.exports.is_open()
            && !self.exports.busy()
            && self.guided_morph.is_none()
            && self.text_dialog.is_none()
            && self.artboard_size.is_none()
            && !self.recovery_dialog.is_open()
            && !self.keyboard_settings.is_open()
            && !self.crash_reports.is_open()
            && !self.library.is_editing()
            && !self.notices.open
            && !ctx.wants_keyboard_input()
            && !ctx.memory(|m| m.any_popup_open() || m.top_modal_layer().is_some())
    }

    fn show_passive_workspace(&mut self, ctx: &egui::Context) {
        // The canvas and timeline read raw scroll, touch, and keyboard gestures
        // as well as widget responses. Keep layout metrics for this paint only.
        let active = ctx.input_mut(|input| {
            let active = std::mem::take(input);
            input.raw.viewport_id = active.raw.viewport_id;
            input.raw.viewports.clone_from(&active.raw.viewports);
            input.screen_rect = active.screen_rect;
            input.pixels_per_point = active.pixels_per_point;
            input.max_texture_side = active.max_texture_side;
            input.time = active.time;
            input.focused = active.focused;
            active
        });
        self.show_workspace(ctx, true);
        ctx.input_mut(|input| *input = active);
    }

    fn show_front_dialog(&mut self, ctx: &egui::Context) {
        if self.recovery_dialog.is_open() {
            self.show_recover_drafts(ctx);
        } else if self.keyboard_settings.is_open() {
            self.show_keyboard_settings(ctx);
        }
    }

    fn handle_close_request(&mut self, ctx: &egui::Context) {
        if !ctx.input(|i| i.viewport().close_requested()) {
            return;
        }
        if self.exports.busy() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.exports.cancel();
            self.status = "Stopping the export. Close again when it finishes.".into();
        } else if self.canvas.text_needs_correction()
            || self.canvas.path_in_progress()
            || self.guided_morph.is_some()
            || self.library.is_editing()
            || self.text_dialog.is_some()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.report_warning(
                "Could not close Phyro",
                "Finish or cancel the current edit before closing Phyro.",
            );
        } else {
            if self.canvas.text_active() {
                let doc =
                    self.project.clips[self.timeline.clip].drawing_at_mut(self.timeline.frame);
                if let Err(error) = self.canvas.finish_text(ctx, doc, &mut self.selection) {
                    self.report_error("Could not finish text", error);
                    ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                    return;
                }
            }
            if self.confirm_close_all_with(Self::confirm_switch) {
                self.drafts = None;
                self.file_batch = None;
                self.folder_assets = Default::default();
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            }
        }
    }

    fn show_workspace(&mut self, ctx: &egui::Context, behind_dialog: bool) {
        let menu_pointer = self.text_menu.prepare(ctx);
        if !behind_dialog
            && !menu_pointer
            && crate::inspector::finish_rename_on_blur(
                ctx,
                &mut self.project.clips[self.timeline.clip],
                self.timeline.frame,
            )
        {
            self.finish_edit();
        }
        if !behind_dialog {
            self.dropped_projects(ctx);
        }
        let initial_session = self.project.clips[self.timeline.clip].session_id();
        crate::path_editor::use_canvas(ctx, initial_session);
        self.update_diagnostic_context(ctx);
        if !menu_pointer && self.canvas.text_should_finish(ctx) {
            let doc = self.project.clips[self.timeline.clip].drawing_at_mut(self.timeline.frame);
            // The inline editor keeps its buffer and displays any layout error.
            if let Ok(true) = self.canvas.finish_text(ctx, doc, &mut self.selection) {
                self.sync_paint();
                self.status.clear();
                self.finish_edit();
            }
        }
        // End the previous inspector edit before another field or toolbar action
        // takes focus. A continuous pointer drag remains a single transaction.
        if !menu_pointer
            && self.pending_edit
            && ctx.input(|i| {
                i.pointer.primary_pressed() || i.key_pressed(Key::Tab) || i.key_pressed(Key::Enter)
            })
        {
            self.finish_edit();
        }
        if !behind_dialog {
            self.handle_close_request(ctx);
            self.shortcuts(ctx);
        }
        if !self.show_inspector
            && !keymap::widget_owns_shortcuts(ctx)
            && !self.canvas.text_active()
            && !self.exports.is_open()
            && !self.library.is_editing()
            && self.artboard_size.is_none()
            && !self.crash_reports.is_open()
            && !ctx.input(|i| i.pointer.any_down())
            && keymap::pressed(ctx, Shortcut::Align)
        {
            self.show_inspector = true;
        }
        let old_drawing = self.drawing_location();
        self.timeline
            .advance_clip(ctx, &self.project.clips[self.timeline.clip]);
        if self.selection_crossed_drawing(old_drawing) {
            self.selection.clear();
            self.tidy_selection();
        }
        let could_undo = self.can_undo();
        let correct_text = self.canvas.text_needs_correction();
        let reveal_align = egui::TopBottomPanel::top("toolbar")
            .show(ctx, |ui| {
                if correct_text {
                    ui.disable();
                }
                self.toolbar(ui);
                self.show_inspector && crate::inspector::align_shortcut(ui)
            })
            .inner;
        self.document_tab_bar(
            ctx,
            !behind_dialog && !correct_text && !self.exports.busy() && !self.exports.is_open(),
        );
        let session = self.project.clips[self.timeline.clip].session_id();
        if session != initial_session {
            crate::path_editor::use_canvas(ctx, session);
        }
        // Editing menus borrow the pointer without transferring text focus to the
        // canvas or inspector. The real field still receives its clipboard and keys.
        let menu_pointer = menu_pointer.then(|| ctx.input_mut(|i| std::mem::take(&mut i.pointer)));
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.set_min_height(ui.spacing().interact_size.y);
            match self.exports.status(ui) {
                Some(Ok(message)) => self.status = message,
                Some(Err(error)) => self.report_error("Export failed", error),
                None => {}
            }
            if self.exports.busy() {
                return;
            }
            if self.diagnostics.as_ref().is_some_and(|store| store.worker_reports() > self.shown_worker_reports) {
                ui.horizontal(|ui| {
                    ui.label("A background task stopped.");
                    if ui.small_button("View crash report").clicked() {
                        self.open_crash_reports();
                    }
                });
            } else if let Some(error) = self.timeline.error() {
                ui.add(egui::Label::new(egui::RichText::new(error).color(ui.visuals().warn_fg_color)).truncate()).on_hover_text(error);
            } else if let Some(warning) = self.canvas.refinement_warning() {
                ui.colored_label(ui.visuals().warn_fg_color, warning);
            } else if self.canvas.rig.placing() {
                ui.small("Click the artboard to place. Press Esc to cancel.");
            } else if self.canvas.editing_pivot() {
                workspace::status_line(ui, "Click or drag to place the pivot. Press Esc to scale or rotate around it.");
            } else if matches!(self.tool, Tool::Rectangle | Tool::Ellipse | Tool::Pen | Tool::Pencil | Tool::Text)
                && !commands::is_editable(self.scene(), self.style.layer)
            {
                ui.small("Choose a visible, unlocked layer to draw on.");
            } else if self.status.is_empty() {
                let help = match self.tool {
                    Tool::Select => "Shift-click to extend the selection. Drag empty space to select an area.",
                    Tool::Rectangle | Tool::Ellipse => "Drag in the drawing area to draw on the active layer.",
                    Tool::Pen => "Click for corners, drag for curves. Shift splits handles. Click a marked end to continue or the start ring to close. Right-click or Enter finishes open.",
                    Tool::Pencil => "Drag to draw. Lift at the start ring to close the path, or lift elsewhere to finish open.",
                    Tool::Node => "Drag a segment to bend it. Ctrl-drag a corner to pull out one handle. Double-click a segment to add a node.",
                    Tool::Text => "Click to start writing or edit lettering. Enter starts a line. Ctrl+Enter finishes. Esc cancels.",
                    Tool::Eyedropper => "Click a color to set the fill. Shift-click to set the stroke.",
                    Tool::Gradient => match self.style.gradient_channel {
                        crate::canvas::PaintChannel::Fill => "Drag across an object to draw a fill gradient. Drag its handles to move or resize the fade.",
                        crate::canvas::PaintChannel::Stroke => "Drag across an object to draw a stroke gradient. Drag its handles to move or resize the fade.",
                    },
                };
                workspace::status_line(ui, help);
            } else {
                workspace::status_line(ui, &self.status);
            }
        });
        if self.show_timeline {
            self.update_phase(UpdatePhase::Timeline);
            Timeline::panel(ctx, |ui| {
                if correct_text {
                    ui.disable();
                }
                if ui.rect_contains_pointer(ui.max_rect())
                    && ui.input(|i| i.pointer.primary_pressed())
                {
                    self.note(Action::Timeline);
                }
                let previous = (self.timeline.clip, self.timeline.frame);
                let drawing = self.drawing_location();
                let clip_count = self.project.clips.len();
                let selected = self.single();
                let changed = self.timeline.show(
                    ui,
                    &mut self.project,
                    selected,
                    self.style.layer,
                    self.canvas.rig.selected_bone(),
                );
                if let Some(target) = self.timeline.export_request.take() {
                    if let Some(message) = self.exports.open_animation_export(target) {
                        self.report_error("Could not start export", message);
                    }
                }
                if let Some(layer) = self.timeline.layer_request.take() {
                    if self.scene().layers.iter().any(|l| l.id == layer)
                        && self.style.layer != layer
                    {
                        self.canvas.cancel_drag();
                        self.selection.clear();
                        self.style.layer = layer;
                        self.sync_paint();
                    }
                }
                if let Some(id) = self.timeline.guided_morph_request.take() {
                    self.open_guided_morph(id);
                }
                if changed || previous != (self.timeline.clip, self.timeline.frame) {
                    self.note(if self.timeline.is_playing() {
                        Action::Playback
                    } else {
                        Action::Timeline
                    });
                    self.canvas.cancel_drag();
                    if self.selection_crossed_drawing(drawing)
                        || self.project.clips.len() != clip_count
                    {
                        self.selection.clear();
                    }
                    self.tidy_selection();
                    self.sync_paint();
                }
                if self.timeline.is_playing() {
                    self.canvas.cancel_drag();
                }
            });
        }
        self.update_phase(UpdatePhase::Panels);
        let active = &self.project.clips[self.timeline.clip];
        let session = active.session_id();
        let exposure = active.exposure(self.timeline.frame).0;
        let rig_exposure = active.rig_exposure(self.timeline.frame).0;
        self.enter_cel(session, exposure);
        crate::path_editor::use_canvas(ctx, session);
        self.canvas.rig.set_drawing(session, rig_exposure);
        if self.timeline.is_playing() {
            self.canvas.rig.cancel_placement();
        }
        self.style.key_mode = self.timeline.key_mode;
        egui::SidePanel::left("drawing-tools")
            .exact_width(58.)
            .resizable(false)
            .frame(
                egui::Frame::side_top_panel(&ctx.style())
                    .inner_margin(egui::Margin::symmetric(3, 6)),
            )
            .show(ctx, |ui| {
                if correct_text {
                    ui.disable();
                }
                egui::ScrollArea::vertical().show(ui, |ui| {
                    if let Some(tool) = workspace::tools(ui, self.tool) {
                        self.set_tool(tool);
                    }
                });
            });
        if self.show_inspector {
            self.dock_panels(ctx, correct_text, reveal_align);
        }
        self.update_phase(UpdatePhase::Canvas);
        egui::CentralPanel::default().show(ctx, |ui| self.artboard(ui));
        self.update_phase(UpdatePhase::Dialogs);
        if let Some(message) = self
            .exports
            .window(ctx, &mut self.project, self.timeline.clip)
        {
            self.report_error("Could not start export", message);
        }
        self.text_window(ctx);
        self.artboard_size_dialog(ctx);
        if let Some(result) = self.library.dialogs(
            ctx,
            &mut self.project,
            self.canvas.show_checkerboard(),
            &mut self.pencil_smoothing,
            &mut self.pencil_pressure,
        ) {
            match result {
                Ok(message) => self.status = message,
                Err(error) => self.report_error("Could not remove symbol", error),
            }
        }
        if !behind_dialog && !self.recovery_dialog.is_open() {
            self.crash_reports.show(ctx, self.diagnostics.as_ref());
        }
        if let Some(pointer) = menu_pointer {
            ctx.input_mut(|i| i.pointer = pointer);
        }
        // Controls above can edit the project even when the previous frame was clean.
        self.update_phase(UpdatePhase::History);
        if !ctx.input(|i| i.pointer.primary_down()) && !ctx.wants_keyboard_input() {
            self.finish_edit();
        } else {
            self.pending_edit = self.baseline != self.project;
        }
        if self.can_undo() != could_undo {
            ctx.request_repaint();
        }
        let is_saved = if self.pending_edit {
            self.saved.as_ref() == Some(&self.project)
        } else {
            self.baseline_is_saved
        };
        let mark = if is_saved { "" } else { " *" };
        let title = format!("{}{} · Phyro", self.name(), mark);
        if title != self.window_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    mod animation_sets;
    mod animation_shortcuts;
    mod arrange_ui;
    mod assistant;
    mod cel_sharing;
    mod clipping;
    mod context_menu;
    mod crash_reporting;
    mod document_sessions;
    mod document_tabs_ui;
    mod edit_menu;
    mod file_menu;
    mod folder_library;
    mod gradient_handles;
    mod guided_morph;
    mod inspector_tabs;
    mod keyboard_settings;
    mod node_click;
    mod notices;
    mod panels;
    mod pasteboard;
    mod path_ops;
    mod pen;
    mod pencil;
    mod pivot;
    mod point_text;
    mod recovery_lifecycle;
    mod responsiveness;
    mod rig_handles;
    mod selection_export;
    mod welcome;

    struct EditorWindow {
        editor: Editor,
        ctx: egui::Context,
        size: egui::Vec2,
    }

    impl EditorWindow {
        fn new(size: egui::Vec2) -> Self {
            let ctx = egui::Context::default();
            Self {
                editor: Editor::new(&ctx),
                ctx,
                size,
            }
        }
        fn with_artboard(size: egui::Vec2, [width, height]: [u32; 2]) -> Self {
            let mut window = Self::new(size);
            let drawing = Document {
                width,
                height,
                ..Document::default()
            };
            window
                .editor
                .replace_project(AnimationProject::from_document(drawing), None);
            window
        }
        fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
            self.frame_with_input(egui::RawInput {
                events,
                ..Default::default()
            })
        }
        fn frame_with_input(&mut self, mut input: egui::RawInput) -> egui::FullOutput {
            input.screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.size));
            self.ctx.run(input, |ctx| self.editor.show(ctx))
        }
        fn board(&mut self) -> egui::Rect {
            Self::board_rect(&self.frame(Vec::new()))
        }
        fn board_rect(output: &egui::FullOutput) -> egui::Rect {
            output
                .shapes
                .iter()
                .find_map(|s| crate::canvas::painted_artboard(&s.shape))
                .expect("visible artboard")
        }
        fn control(&mut self, label: &str) -> egui::Pos2 {
            let output = self.frame(Vec::new());
            output
                .shapes
                .iter()
                .rev()
                .find_map(|s| match &s.shape {
                    egui::Shape::Text(t) if t.galley.text() == label => {
                        let bounds = t.galley.rect.translate(t.pos.to_vec2());
                        s.clip_rect.contains_rect(bounds).then_some(bounds.center())
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("Missing visible control {label:?} at {:?}", self.size))
        }
        fn click(&mut self, label: &str) {
            let center = self.control(label);
            self.click_at(center);
        }
        fn click_edit(&mut self, label: &str) {
            self.click("Edit");
            self.click(label);
        }
        fn timeline_icon(&mut self, label: &str) -> egui::Pos2 {
            self.frame(vec![]);
            let id = egui::Id::new(("timeline-action", label));
            let button = self.ctx.read_response(id).unwrap();
            assert!(self.ctx.screen_rect().contains_rect(button.rect));
            button.rect.center()
        }
        fn field_on_row(&mut self, label: &str, suffix: &str) -> egui::Pos2 {
            let row = self.control(label);
            self.frame(Vec::new())
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::Shape::Text(t) if t.galley.text().ends_with(suffix) => {
                        let bounds = t.galley.rect.translate(t.pos.to_vec2());
                        (s.clip_rect.contains_rect(bounds)
                            && (bounds.center().y - row.y).abs() < 5.)
                            .then_some(bounds.center())
                    }
                    _ => None,
                })
                .unwrap_or_else(|| {
                    panic!("Missing {suffix} field on {label} row at {:?}", self.size)
                })
        }
        fn click_at(&mut self, center: egui::Pos2) {
            self.frame(vec![egui::Event::PointerMoved(center)]);
            for pressed in [true, false] {
                self.frame(vec![egui::Event::PointerButton {
                    pos: center,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }]);
            }
        }
        fn drag(&mut self, start: egui::Pos2, delta: egui::Vec2) {
            let button = |pos, pressed| egui::Event::PointerButton {
                pos,
                pressed,
                button: egui::PointerButton::Primary,
                modifiers: egui::Modifiers::NONE,
            };
            self.frame(vec![egui::Event::PointerMoved(start)]);
            self.frame(vec![button(start, true)]);
            self.frame(vec![egui::Event::PointerMoved(start + delta * 0.25)]);
            let end = start + delta;
            self.frame(vec![egui::Event::PointerMoved(end)]);
            self.frame(vec![button(end, false)]);
        }
    }

    #[test]
    fn window_title_updates_when_the_name_or_unsaved_marker_changes() {
        let mut w = EditorWindow::new(egui::vec2(800., 500.));
        let titles = |w: &mut EditorWindow| {
            w.frame(Vec::new()).viewport_output[&egui::ViewportId::ROOT]
                .commands
                .iter()
                .filter_map(|cmd| match cmd {
                    egui::ViewportCommand::Title(title) => Some(title.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(titles(&mut w), ["Untitled · Phyro"]);
        assert!(titles(&mut w).is_empty());
        w.editor
            .replace_project(AnimationProject::default(), Some("Eyes.vanim".into()));
        assert_eq!(titles(&mut w), ["Eyes.vanim · Phyro"]);
        w.editor.project.clips[0]
            .drawing_at_mut(0)
            .add_rectangle([1.; 2], [2.; 2], [255; 4]);
        assert_eq!(titles(&mut w), ["Eyes.vanim * · Phyro"]);
        assert!(titles(&mut w).is_empty());
        w.editor.restore(false);
        assert_eq!(titles(&mut w), ["Eyes.vanim · Phyro"]);
    }

    #[test]
    fn selecting_artwork_finishes_the_focused_timeline_edit_once() {
        let mut doc = Document::default();
        let id = doc.add_rectangle([20., 20.], [30.; 2], [255; 4]);
        let project = AnimationProject::from_document(doc);
        let mut w = EditorWindow::new(egui::vec2(1100., 740.));
        w.editor.replace_project(project.clone(), None);
        let board = w.board();
        let field = w.field_on_row("Hold", " fr");
        w.click_at(field);
        w.frame(vec![egui::Event::Text("7".into())]);
        assert!(w.ctx.wants_keyboard_input());
        assert_eq!(w.editor.project.clips[0].frames, 7);
        assert!(w.editor.pending_edit && !w.editor.history.can_undo());
        let at = board.min + board.size() * (35. / 128.);
        w.click_at(at);
        assert_eq!(w.editor.selection, [id]);
        assert!(!w.editor.pending_edit && w.editor.history.can_undo());
        w.click_edit("Undo");
        assert_eq!(w.editor.project, project);
        assert!(!w.editor.can_undo());
        w.click_edit("Redo");
        assert_eq!(w.editor.project.clips[0].frames, 7);
    }

    #[test]
    fn pending_edit_updates_the_title_and_commits_once_on_release() {
        let mut w = EditorWindow::new(egui::vec2(800., 500.));
        let dir = tempfile::tempdir().unwrap();
        w.editor.path = Some(dir.path().join("Pose.vanim"));
        w.frame(Vec::new());
        let button = |pressed| egui::Event::PointerButton {
            pos: egui::pos2(1., 1.),
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        w.frame(vec![button(true)]);
        w.editor.project.clips[0].origin = [10., 10.];
        w.frame(Vec::new());
        assert!(w.editor.can_undo());
        assert!(!w.editor.history.can_undo());
        assert_eq!(w.editor.window_title, "Pose.vanim * · Phyro");
        w.editor.project.clips[0].origin = w.editor.baseline.clips[0].origin;
        w.frame(Vec::new());
        assert!(!w.editor.can_undo());
        assert_eq!(w.editor.window_title, "Pose.vanim · Phyro");
        w.editor.project.clips[0].origin = [20., 20.];
        w.frame(vec![button(false)]);
        assert!(w.editor.history.can_undo());
        w.click_edit("Undo");
        assert_eq!(w.editor.project, AnimationProject::default());
        assert_eq!(w.editor.window_title, "Pose.vanim · Phyro");
        assert!(!w.editor.can_undo());
        w.click_edit("Redo");
        assert_eq!(w.editor.project.clips[0].origin, [20., 20.]);
        assert!(w.editor.save(false));
        w.frame(Vec::new());
        assert_eq!(w.editor.window_title, "Pose.vanim · Phyro");
        w.click_edit("Undo");
        assert_eq!(w.editor.window_title, "Pose.vanim * · Phyro");
        w.click_edit("Redo");
        assert_eq!(w.editor.window_title, "Pose.vanim · Phyro");
    }

    #[test]
    fn skin_panel_binds_edits_a_node_and_undoes_at_both_window_sizes() {
        use document_core::{PathNode, Shape};
        for size in [egui::vec2(800., 500.), egui::vec2(1100., 740.)] {
            let mut w = EditorWindow::new(size);
            w.ctx.style_mut(|style| style.animation_time = 0.);
            let doc = w.editor.project.clips[0].drawing_at_mut(0);
            let upper = doc
                .add_bone("Sleeve", None, [24., 64.], [56., 64.])
                .unwrap();
            doc.add_bone("Cuff", Some(upper), [56., 64.], [88., 64.])
                .unwrap();
            let path = Shape::Path {
                nodes: [[0., 0.], [0.5, 1.], [1., 0.]]
                    .map(PathNode::corner)
                    .to_vec(),
                closed: true,
            };
            let art = doc.add_shape([24., 60.], [64., 8.], [255; 4], path.clone(), 1);
            w.editor.selection = vec![art];
            w.editor.capture_baseline();
            w.frame(Vec::new());
            w.click("Rig");
            w.click("Sleeve");
            w.click("Cuff");
            w.click("Skin path");
            let angle = w.field_on_row("Pose angle", "°");
            w.drag(angle, egui::vec2(70., 0.));
            assert!(
                w.editor.scene().skeleton.bones[1].rotation.abs() > 1.,
                "pose angle at {size:?}"
            );
            let binding = w.editor.scene().objects[0].skin.clone().unwrap();
            let bent = renderer::render(w.editor.scene(), 0).unwrap();
            let node = w.control("Node 1");
            w.drag(node, egui::vec2(24., 0.));
            w.control("Node 3");
            let field = w.field_on_row("Child weight", "%");
            w.drag(field, egui::vec2(-60., 0.));
            let edited = w.editor.scene().objects[0].skin.clone().unwrap();
            assert!(edited.weights[0][2] < binding.weights[0][2]);
            assert_eq!(edited.weights[0][..2], binding.weights[0][..2]);
            assert_eq!(w.editor.scene().objects[0].shape, path);
            let adjusted = renderer::render(w.editor.scene(), 0).unwrap();
            assert!(adjusted.data() != bent.data(), "weight edit at {size:?}");
            w.click_edit("Undo");
            assert_eq!(w.editor.scene().objects[0].skin.as_ref(), Some(&binding));
            assert_eq!(
                renderer::render(w.editor.scene(), 0).unwrap().data(),
                bent.data()
            );
            w.click_edit("Redo");
            assert_eq!(w.editor.scene().objects[0].skin.as_ref(), Some(&edited));
            w.click("Detach");
            assert!(w.editor.scene().objects[0].skin.is_none());
            assert_eq!(
                renderer::render(w.editor.scene(), 0).unwrap().data(),
                adjusted.data()
            );
            w.click_edit("Undo");
            assert_eq!(w.editor.scene().objects[0].skin.as_ref(), Some(&edited));
        }
    }

    #[test]
    fn ik_canvas_target_keys_both_joints_as_one_undo_at_both_window_sizes() {
        for size in [egui::vec2(800., 500.), egui::vec2(1100., 740.)] {
            let mut w = EditorWindow::with_artboard(size, [128; 2]);
            w.ctx.style_mut(|style| style.animation_time = 0.);
            let doc = w.editor.project.clips[0].drawing_at_mut(0);
            let root = doc
                .add_bone("Shoulder", None, [32., 64.], [56., 64.])
                .unwrap();
            let tip = doc
                .add_bone("Wrist", Some(root), [56., 64.], [80., 64.])
                .unwrap();
            let art = doc.add_rectangle([70., 61.], [8., 5.], [255; 4]);
            doc.bind_rigid(art, Some(tip)).unwrap();
            let original = doc.clone();
            w.editor.capture_baseline();
            w.frame(Vec::new());
            w.click("Rig");
            w.click("Shoulder");
            w.click("Wrist");
            w.click("5");
            w.click("Place IK target");
            w.click("Cancel target");
            assert_eq!(w.editor.scene(), &original);
            w.click("Place IK target");
            let board = w.board();
            let target = [56., 84.];
            w.click_at(
                board.min
                    + egui::vec2(
                        board.width() * target[0] / 128.,
                        board.height() * target[1] / 128.,
                    ),
            );
            let doc = w.editor.scene();
            let end = doc.bone_segment_at(tip, true, 4).unwrap().1;
            assert!((end[0] - target[0]).hypot(end[1] - target[1]) < 0.01);
            assert!(doc
                .skeleton
                .bones
                .iter()
                .all(|bone| bone.rotation_keys.contains_key(&4)));
            assert_eq!(
                renderer::render(doc, 0).unwrap().data(),
                renderer::render(&original, 0).unwrap().data()
            );
            assert_ne!(
                renderer::render(doc, 4).unwrap().data(),
                renderer::render(&original, 4).unwrap().data()
            );
            w.click_edit("Undo");
            assert_eq!(w.editor.scene(), &original);
            w.click_edit("Redo");
            assert_eq!(
                w.editor.scene().bone_segment_at(tip, true, 4).unwrap().1,
                end
            );
        }
    }

    #[test]
    fn rig_controls_place_a_chain_pose_artwork_and_undo_detach_at_both_window_sizes() {
        for size in [egui::vec2(800., 500.), egui::vec2(1100., 740.)] {
            let mut w = EditorWindow::with_artboard(size, [128; 2]);
            w.ctx.style_mut(|style| style.animation_time = 0.);
            let id = w.editor.project.clips[0].drawing_at_mut(0).add_rectangle(
                [81., 60.],
                [8., 8.],
                [255; 4],
            );
            w.editor.selection = vec![id];
            w.editor.capture_baseline();
            w.frame(Vec::new());
            w.click("Rig");
            w.click("Add root bone");
            let board = w.board();
            let at = |x: f32, y: f32| {
                board.min + egui::vec2(x * board.width() / 128., y * board.height() / 128.)
            };
            w.click_at(at(36., 64.));
            assert!(w.editor.scene().skeleton.bones.is_empty());
            w.click_at(at(64., 64.));
            let root = w.editor.scene().skeleton.bones[0].clone();
            assert!(root.parent.is_none());
            w.click("Add child bone");
            w.click_at(at(90., 64.));
            let arm = w.editor.scene().skeleton.bones[1].clone();
            assert_eq!(arm.parent, Some(root.id));
            assert_eq!(arm.head, root.tail);
            assert!((arm.tail[0] - 90.).abs() < 0.01);
            w.click("Bind selection");
            assert_eq!(w.editor.scene().objects[0].rigid_bone, Some(arm.id));
            let point = w.field_on_row("Pose angle", "°");
            w.drag(point, egui::vec2(40., 0.));
            let doc = w.editor.scene();
            assert!(
                doc.skeleton.bones[1].rotation.abs() > 1.,
                "pose angle after drag, {}; root {}",
                w.editor.status,
                doc.skeleton.bones[0].rotation
            );
            assert_ne!(doc.objects[0].position_at(0), [81., 60.]);
            let posed = renderer::render(doc, 0).unwrap();
            w.click("Detach");
            assert_eq!(w.editor.scene().objects[0].rigid_bone, None);
            assert_eq!(
                renderer::render(w.editor.scene(), 0).unwrap().data(),
                posed.data()
            );
            w.click_edit("Undo");
            assert_eq!(w.editor.scene().objects[0].rigid_bone, Some(arm.id));
            w.click("Reset");
            assert_eq!(w.editor.scene().objects[0].position_at(0), [81., 60.]);
        }
    }

    #[test]
    fn bone_keys_follow_the_playhead_and_undo_at_both_window_sizes() {
        for size in [egui::vec2(800., 500.), egui::vec2(1100., 740.)] {
            let mut w = EditorWindow::new(size);
            w.ctx.style_mut(|style| style.animation_time = 0.);
            let doc = w.editor.project.clips[0].drawing_at_mut(0);
            let arm = doc.add_bone("Arm", None, [32., 32.], [64., 32.]).unwrap();
            doc.set_bone_rotation(arm, 30.).unwrap();
            w.editor.capture_baseline();
            w.frame(Vec::new());
            w.click("Rig");
            w.click("Key pose");
            w.click("5");
            assert_eq!(w.editor.timeline.frame, 4);
            w.click("Reset");
            assert_eq!(w.editor.scene().skeleton.bones[0].rotation_keys[&4], 0.);
            w.click("3");
            assert_eq!(w.editor.timeline.frame, 2);
            w.control("15.0°");
            let bone = &w.editor.scene().skeleton.bones[0];
            assert_eq!(bone.rotation_keys.len(), 2);
            assert_eq!(bone.rotation, 30.);
            w.click("Key pose");
            assert_eq!(w.editor.scene().skeleton.bones[0].rotation_keys[&2], 15.);
            w.click("Remove pose key");
            assert!(!w.editor.scene().skeleton.bones[0]
                .rotation_keys
                .contains_key(&2));
            w.click_edit("Undo");
            assert_eq!(w.editor.scene().skeleton.bones[0].rotation_keys[&2], 15.);
            w.click_edit("Redo");
            assert!(!w.editor.scene().skeleton.bones[0]
                .rotation_keys
                .contains_key(&2));
            w.click("Linear");
            w.click("Hold");
            w.control("30.0°");
        }
    }

    #[test]
    fn placing_a_child_on_a_posed_arm_tracks_the_visible_tip_and_stores_rest_coordinates() {
        let mut w = EditorWindow::with_artboard(egui::vec2(800., 500.), [128; 2]);
        w.ctx.style_mut(|style| style.animation_time = 0.);
        let doc = w.editor.project.clips[0].drawing_at_mut(0);
        let shoulder = doc
            .add_bone("Shoulder", None, [40., 40.], [65., 40.])
            .unwrap();
        doc.set_bone_rotation(shoulder, 90.).unwrap();
        w.editor.capture_baseline();
        w.frame(Vec::new());
        w.click("Rig");
        w.click("Add child bone");
        let board = w.board();
        let at = |x: f32, y: f32| {
            board.min + egui::vec2(x * board.width() / 128., y * board.height() / 128.)
        };
        let shoulder_tip = at(40., 65.);
        let cursor = at(45., 90.);
        let output = w.frame(vec![egui::Event::PointerMoved(cursor)]);
        assert!(output.shapes.iter().any(|s| matches!(&s.shape,
            egui::Shape::LineSegment {points, stroke} if stroke.color==egui::Color32::from_rgb(255,210,135) &&
            points[0].distance(shoulder_tip)<0.01 && points[1].distance(cursor)<0.01
        )));
        w.click_at(cursor);
        let doc = w.editor.scene();
        let child = &doc.skeleton.bones[1];
        assert_eq!(child.parent, Some(shoulder));
        assert_eq!(child.head, [65., 40.]);
        assert!(egui::Pos2::from(child.tail).distance(egui::pos2(90., 35.)) < 0.01);
        let (head, tail) = doc.bone_segment(child.id, true).unwrap();
        assert!(egui::Pos2::from(head).distance(egui::pos2(40., 65.)) < 0.01);
        assert!(egui::Pos2::from(tail).distance(egui::pos2(45., 90.)) < 0.01);
    }

    #[test]
    fn nested_drawing_keys_can_be_chosen_moved_removed_and_undone() {
        for size in [egui::vec2(800., 500.), egui::vec2(1100., 740.)] {
            let mut w = EditorWindow::new(size);
            w.ctx.style_mut(|style| style.animation_time = 0.);
            let project = &mut w.editor.project;
            let child = project.add_clip("Eyes").unwrap();
            let source = &mut project.clips[child];
            source
                .drawing_at_mut(0)
                .add_rectangle([12., 12.], [8., 6.], [255; 4]);
            source.duplicate_frame(0).unwrap();
            source.drawing_at_mut(1).objects[0].size[1] = 1.;
            let source_id = source.id;
            let placed = project.place_clip(source_id, 0, 0, 1, [0.; 2]).unwrap();
            w.editor.selection = vec![placed];
            w.editor.capture_baseline();
            let keys = |w: &EditorWindow| {
                w.editor.scene().objects[0]
                    .clip_instance
                    .as_ref()
                    .unwrap()
                    .drawing_keys
                    .clone()
            };
            let pixels =
                |w: &EditorWindow, frame| renderer::render(w.editor.scene(), frame).unwrap();
            w.frame(Vec::new());
            w.click("Properties");
            w.click("Hold child frame");
            assert_eq!(keys(&w), [(0, 0)].into());
            w.click("3");
            let open = pixels(&w, 2);
            let field = w.field_on_row("Source frame", " / 2");
            w.drag(field, egui::vec2(20., 0.));
            assert_eq!(keys(&w), [(0, 0), (2, 1)].into());
            let closed = pixels(&w, 2);
            assert!(open.data() != closed.data());
            assert!(pixels(&w, 4).data() == closed.data());
            let row = w.control("Child frame").y;
            let start = egui::pos2(w.control("3").x, row);
            let end = egui::pos2(w.control("4").x, row);
            w.drag(start, end - start);
            assert_eq!(keys(&w), [(0, 0), (3, 1)].into());
            assert!(pixels(&w, 2).data() == open.data());
            w.click("Remove key");
            assert_eq!(keys(&w), [(0, 0)].into());
            w.click_edit("Undo");
            assert_eq!(keys(&w), [(0, 0), (3, 1)].into());
            w.click("5");
            w.click("Key drawing");
            assert_eq!(keys(&w).get(&4), Some(&1));
            w.click("Clear keys and play");
            assert!(keys(&w).is_empty());
            assert!(pixels(&w, 4).data() == open.data());
            w.click_edit("Undo");
            assert!(pixels(&w, 4).data() == closed.data());
        }
    }

    #[test]
    fn timeline_drags_a_linked_clip_onto_the_active_layer_and_properties_edits_its_playback() {
        for (width, from_other_set) in [(800., false), (1100., false), (800., true), (1100., true)]
        {
            let mut w = EditorWindow::with_artboard(egui::vec2(width, 740.), [128, 128]);
            w.ctx.style_mut(|style| style.animation_time = 0.);
            let p = &mut w.editor.project;
            p.rename_clip(0, "Host").unwrap();
            let set = if from_other_set {
                p.add_animation_set("Face").unwrap()
            } else {
                p.clips[0].set_id
            };
            let i = p.add_clip_in_set(set, "Blink").unwrap();
            w.editor.project.clips[i]
                .drawing_at_mut(0)
                .add_rectangle([2.; 2], [3.; 2], [255; 4]);
            let layer = crate::commands::add_layer(w.editor.project.clips[0].drawing_at_mut(0));
            w.editor.style.layer = layer;
            w.editor.capture_baseline();
            let board = w.board();
            let drop = board.center() + board.size() * 0.05;
            if from_other_set {
                w.click("Animations");
                let heading = w.control("Face");
                let toggle = egui::Id::new((
                    "browse-set-clips",
                    w.editor.project.clips[0].session_id(),
                    set,
                ));
                let arrow_x = w.ctx.read_response(toggle).unwrap().rect.center().x;
                w.click_at(egui::pos2(arrow_x, heading.y));
            } else if width < 940. {
                w.click("Host");
            }
            let source = w.control("Blink");
            w.drag(source, drop - source);
            folder_library::wait_for(&mut w, |w| !w.editor.folder_assets.is_pending());
            assert_eq!(w.editor.timeline.clip, 0);
            assert_eq!(w.editor.scene().objects.len(), 1, "{}", w.editor.status);
            let placed = &w.editor.scene().objects[0];
            assert_eq!(placed.name, "Blink");
            assert!(
                placed.position.iter().all(|axis| (axis - 6.4).abs() < 0.25),
                "{:?}",
                placed.position
            );
            assert_eq!(w.editor.selection, [placed.id]);
            assert_eq!(placed.layer, layer);
            assert_eq!(
                placed.clip_instance.as_ref().unwrap().clip_id,
                w.editor.project.clips[i].id
            );
            w.click("Properties");
            w.click("Hold at ends");
            assert_eq!(
                w.editor.scene().objects[0]
                    .clip_instance
                    .as_ref()
                    .unwrap()
                    .timing
                    .mode,
                document_core::LoopMode::Clamp
            );
            w.click_edit("Undo");
            assert_eq!(
                w.editor.scene().objects[0]
                    .clip_instance
                    .as_ref()
                    .unwrap()
                    .timing
                    .mode,
                document_core::LoopMode::Loop
            );
            w.click_edit("Undo");
            assert!(w.editor.scene().objects.is_empty());
        }
    }

    #[test]
    fn switch_keying_and_add_and_mute_a_track_at_both_window_sizes() {
        for size in [egui::vec2(800., 500.), egui::vec2(1100., 740.)] {
            let mut window = EditorWindow::new(size);
            let id = window.editor.project.clips[0]
                .drawing_at_mut(0)
                .add_rectangle([20.; 2], [15.; 2], [255; 4]);
            window.editor.selection = vec![id];
            window.editor.set_tool(Tool::Select);
            window.frame(Vec::new());
            window.click("Track: Position");
            window.click("Keying: Tracked");
            window.click("Auto");
            assert!(window.editor.timeline.key_mode == crate::timeline::KeyMode::Auto);
            window.click("Track: Position");
            window.click("Opacity");
            window.click("Add key");
            assert!(window.editor.scene().objects[0]
                .opacity_keys
                .contains_key(&0));
            window.click("Track: Opacity");
            window.click("Mute track");
            assert!(document_core::Property::Opacity.is_muted(&window.editor.scene().objects[0]));
        }
    }

    #[test]
    fn visible_controls_create_convert_and_undo_text_at_both_window_sizes() {
        for size in [egui::vec2(800., 500.), egui::vec2(1100., 740.)] {
            let mut window = EditorWindow::new(size);
            window.frame(Vec::new());
            window.click("Text");
            let at = window.board().center();
            window.click_at(at);
            window.frame(vec![egui::Event::Text("Text".into())]);
            window.click("Select");
            assert_eq!(window.editor.window_title, "Untitled * · Phyro");
            let obj = &window.editor.scene().objects[0];
            assert_eq!(obj.text.as_ref().unwrap().content, "Text");
            let outlines = obj.shape.clone();
            window.click("Edit text");
            window.click("Cancel");
            window.click("Convert to path");
            assert!(window.editor.scene().objects[0].text.is_none());
            assert_eq!(window.editor.scene().objects[0].shape, outlines);
            window.click_edit("Undo");
            assert!(window.editor.scene().objects[0].text.is_some());
            window.click("Onion skin");
            assert!(window.editor.timeline.onion);
            window.click("Library");
            assert_eq!(window.editor.inspector_tab, crate::inspector::Tab::Library);
        }
    }

    #[test]
    fn clip_actions_duplicate_and_delete_without_discarding_saved_events() {
        for size in [egui::vec2(800., 500.), egui::vec2(1200., 740.)] {
            let mut w = EditorWindow::new(size);
            w.editor.project.clips[0].add_event(0, "footstep").unwrap();
            w.editor.capture_baseline();
            let duplicate = w.timeline_icon("Duplicate clip");
            w.click_at(duplicate);
            assert_eq!(w.editor.timeline.clip, 1);
            assert_eq!(w.editor.project.clips[1].events[&0], ["footstep"]);
            let delete = w.timeline_icon("Delete clip");
            w.click_at(delete);
            w.click("Delete clip");
            assert_eq!(w.editor.timeline.clip, 0);
            assert_eq!(w.editor.project.clips.len(), 1);
            w.click_edit("Undo");
            assert_eq!(w.editor.project.clips.len(), 2);
            assert_eq!(w.editor.project.clips[1].events[&0], ["footstep"]);
        }
    }

    #[test]
    fn edits_to_a_nested_clip_reach_parent_render_after_undo_and_redo() {
        let mut doc = Document::default();
        doc.add_rectangle([9., 10.], [10., 12.], [10, 20, 30, 255]);
        let mut project = AnimationProject::from_document(doc);
        project.rename_clip(0, "Child").unwrap();
        let parent = project.add_clip("Parent").unwrap();
        let child_id = project.clips[0].id;
        project
            .place_clip(child_id, parent, 0, 1, [0., 0.])
            .unwrap();
        let sample = |p: &AnimationProject| {
            renderer::render(p.clips[parent].drawing_at(0), 0)
                .unwrap()
                .pixel(10, 11)
                .unwrap()
                .red()
        };
        assert_eq!(sample(&project), 10);
        let mut editor = Editor::new(&egui::Context::default());
        editor.replace_project(project, None);
        editor.project.clips[0].drawing_at_mut(0).objects[0].fill[0] = 150;
        editor.finish_edit();
        assert_eq!(sample(&editor.project), 150);
        editor.restore(false);
        assert_eq!(sample(&editor.project), 10);
        editor.restore(true);
        assert_eq!(sample(&editor.project), 150);
    }

    #[test]
    fn undoing_a_paste_from_a_longer_clip_restores_a_renderable_frame() {
        let mut editor = Editor::new(&egui::Context::default());
        let original = editor.scene().clone();
        let mut longer = Document {
            frames: 12,
            ..Document::default()
        };
        let id = longer.add_rectangle([2.; 2], [10.; 2], [255; 4]);
        longer.objects[0].keys.insert(11, [3., 4.]);
        editor.clipboard.copy(&longer, &[id], 0);
        editor.paste();
        editor.timeline.frame = 11;
        editor.restore(false);
        assert_eq!(editor.scene(), &original);
        assert_eq!(editor.timeline.frame, 0);
        assert!(renderer::render(editor.scene(), editor.timeline.frame).is_ok());
        editor.restore(true);
        assert_eq!(editor.project.clips[0].frames, 12);
    }

    #[test]
    fn direct_sheet_dialog_preflights_the_layout_and_keeps_presets_in_context() {
        for size in [egui::vec2(800., 500.), egui::vec2(1100., 740.)] {
            let mut window = EditorWindow::with_artboard(size, [16, 16]);
            window.ctx.style_mut(|style| style.animation_time = 0.);
            let p = &mut window.editor.project;
            let clip = &mut p.clips[0];
            let doc = clip.drawing_at_mut(0);
            doc.add_rectangle([1., 1.], [6., 6.], [255, 10, 10, 255]);
            let mut second = doc.clone();
            second.objects[0].fill = [10, 255, 10, 255];
            clip.cels.insert(2, std::sync::Arc::new(second));
            p.preset.packed.max_size = 8;
            window.frame(Vec::new());
            window.click("Export");
            window.click("Export clip…");
            window.click("Packed atlas");
            window.control("Maximum side");
            window.click("Allow 90° rotation");
            assert!(window.editor.project.preset.packed.allow_rotation);
            let wait_text = |window: &mut EditorWindow, predicate: fn(&str) -> bool| {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                while std::time::Instant::now() < deadline {
                    if window.frame(Vec::new()).shapes.iter().any(
                        |s| matches!(&s.shape,egui::Shape::Text(t) if predicate(t.galley.text())),
                    ) {
                        return;
                    }
                    std::thread::yield_now();
                }
                panic!("Missing sheet feedback at {:?}", window.size);
            };
            wait_text(&mut window, |text| {
                text.to_lowercase().contains("fit") && text.contains("8")
            });
            window.editor.project.preset.packed.max_size = 64;
            wait_text(&mut window, |text| text.starts_with("Sheet size"));
            window.click("Presets…");
            window.click("Four columns");
            window.control("Columns");
            assert!(!window.editor.project.preset.packed.enabled);
            assert_eq!(window.editor.project.preset.columns, 4);
            window.click("PNG sequence per clip");
            window.control("Frame resolution");
            assert!(window.frame(Vec::new()).shapes.iter().all(|shape| !matches!(&shape.shape, egui::Shape::Text(t) if t.galley.text() == "Sheet layout")));
            window.click("Close");
        }
    }

    #[test]
    fn library_controls_create_place_and_apply_shared_art_at_both_window_sizes() {
        for size in [egui::vec2(800., 500.), egui::vec2(1100., 740.)] {
            let mut window = EditorWindow::new(size);
            let id = window.editor.project.clips[0]
                .drawing_at_mut(0)
                .add_rectangle([2., 3.], [12., 10.], [200; 4]);
            window.editor.selection = vec![id];
            window.editor.tool = Tool::Select;
            window.frame(Vec::new());
            window.click("Library");
            window.frame(Vec::new());
            window.click("Symbol name");
            window.frame(vec![egui::Event::Text("Bell".into())]);
            window.click("Convert selection");
            assert_eq!(window.editor.project.symbols.len(), 1);
            window.click("Place instance");
            assert_eq!(window.editor.project.symbol_usage(1), 2);
            let before_edit = window.editor.project.clone();
            window.click("Edit artwork…");
            window.frame(Vec::new());
            assert!(window.editor.library.is_editing());
            for (key, modifiers) in [
                (egui::Key::A, egui::Modifiers::COMMAND),
                (egui::Key::Delete, egui::Modifiers::NONE),
            ] {
                for pressed in [true, false] {
                    let event = egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed,
                        repeat: false,
                        modifiers,
                    };
                    let input = egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        events: vec![event],
                        modifiers,
                        ..Default::default()
                    };
                    let _ = window.ctx.run(input, |ctx| window.editor.show(ctx));
                }
            }
            window.click("Apply to instances");
            assert!(!window.editor.library.is_editing());
            assert!(window.editor.project.symbols[&1].artwork.objects.is_empty());
            assert_eq!(window.editor.project.symbol_usage(1), 2);
            let applied = window.editor.project.clone();
            window.editor.restore(false);
            assert_eq!(window.editor.project, before_edit);
            window.editor.restore(true);
            assert_eq!(window.editor.project, applied);
            window.click("Remove symbol…");
            window.click("Cancel");
            assert_eq!(window.editor.project, applied);
            window.click("Remove symbol…");
            window.click("Remove and keep artwork");
            assert!(window.editor.project.symbols.is_empty());
            assert!(window
                .editor
                .scene()
                .objects
                .iter()
                .all(|o| o.embedded_artwork.is_some()));
            let independent = window.editor.project.clone();
            window.editor.restore(false);
            assert_eq!(window.editor.project, applied);
            window.editor.restore(true);
            assert_eq!(window.editor.project, independent);
        }
    }

    #[test]
    fn switching_from_node_to_select_restores_the_whole_group() {
        let mut editor = Editor::new(&egui::Context::default());
        let a = editor.project.clips[0]
            .drawing_at_mut(0)
            .add_rectangle([2.; 2], [4.; 2], [255; 4]);
        let b = editor.project.clips[0]
            .drawing_at_mut(0)
            .add_rectangle([9.; 2], [4.; 2], [255; 4]);
        commands::group(editor.project.clips[0].drawing_at_mut(0), &[a, b]);
        editor.set_tool(Tool::Node);
        editor.selection = vec![a];
        editor.set_tool(Tool::Select);
        assert_eq!(editor.selection, vec![a, b]);
    }
}
