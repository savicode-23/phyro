use super::{
    drafts::DraftSession, AnimationProject, Canvas, CanvasStyle, Editor, Exports, History,
    OpenDecision, Path, PathBuf, Timeline, Tool,
};
use std::mem;

/// The active slot is empty: its document is borrowed directly by the editor's
/// drawing, timeline and inspector code. Switching swaps ownership with one slot.
struct Tab {
    id: u64,
    document: Option<Box<StoredDocument>>,
}

pub(super) struct Documents {
    tabs: Vec<Tab>,
    active: usize,
    next_id: u64,
    pub(super) reusable_blank: bool,
}

impl Default for Documents {
    fn default() -> Self {
        Self {
            tabs: vec![Tab {
                id: 1,
                document: None,
            }],
            active: 0,
            next_id: 2,
            reusable_blank: true,
        }
    }
}

impl Documents {
    pub(super) fn inactive_mut(&mut self) -> impl Iterator<Item = &mut StoredDocument> {
        self.tabs
            .iter_mut()
            .filter_map(|tab| tab.document.as_deref_mut())
    }
}

pub(super) struct StoredDocument {
    document_id: u64,
    project: AnimationProject,
    pub(super) baseline: AnimationProject,
    saved: Option<AnimationProject>,
    pub(super) path: Option<PathBuf>,
    canonical_path: Option<PathBuf>,
    recovered_name: Option<String>,
    baseline_is_saved: bool,
    pending_edit: bool,
    writable_cel: Option<(u64, u32)>,
    history: History,
    canvas: Canvas,
    timeline: Timeline,
    style: CanvasStyle,
    tool: Tool,
    selection: Vec<u64>,
    exports: Exports,
    pub(super) drafts: Option<DraftSession>,
    pub(super) status: String,
}

impl StoredDocument {
    fn new(
        editor: &mut Editor,
        project: AnimationProject,
        path: Option<PathBuf>,
        canonical_path: Option<PathBuf>,
    ) -> Self {
        let mut timeline = Timeline::default();
        timeline.set_frame_width(editor.timeline.frame_width());
        let mut canvas = Canvas::default();
        canvas.set_checkerboard(editor.canvas.show_checkerboard());
        canvas.set_diagnostics(editor.diagnostics.clone());
        let drawing = project.clips[0].drawing_at(0);
        let tool = if drawing.objects.is_empty() {
            Tool::Rectangle
        } else {
            Tool::Select
        };
        let mut style = editor.style.clone();
        style.layer = Editor::initial_layer(drawing);
        style.edit_motion = false;
        style.key_mode = Default::default();
        editor.path_paints.restore(tool, &mut style);
        Self {
            saved: Some(project.clone()),
            document_id: crate::editor::assistant::document_id(),
            baseline: project.clone(),
            project,
            canonical_path,
            path,
            recovered_name: None,
            baseline_is_saved: true,
            pending_edit: false,
            writable_cel: None,
            history: Default::default(),
            canvas,
            timeline,
            style,
            tool,
            selection: Vec::new(),
            exports: Default::default(),
            drafts: editor
                .drafts
                .as_ref()
                .map(|d| DraftSession::new(d.root.clone())),
            status: String::new(),
        }
    }

    fn swap(&mut self, editor: &mut Editor) {
        // Move the whole document, including its view, history and recovery
        // writer. No model, textures or histories are duplicated on a switch.
        macro_rules! exchange {
            ($($f:ident),+ $(,)?) => { $(mem::swap(&mut self.$f, &mut editor.$f);)+ };
        }
        exchange!(
            document_id,
            project,
            baseline,
            saved,
            path,
            canonical_path,
            recovered_name,
            baseline_is_saved,
            pending_edit,
            writable_cel,
            history,
            canvas,
            timeline,
            style,
            tool,
            selection,
            exports,
            drafts,
            status
        );
    }
}

pub(super) fn path_key(path: &Path) -> PathBuf {
    // Keep lexical aliases working after an open file has been moved or deleted.
    path.canonicalize()
        .or_else(|_| {
            Ok::<_, std::io::Error>(
                path.parent()
                    .unwrap_or(Path::new("."))
                    .canonicalize()?
                    .join(path.file_name().ok_or(std::io::ErrorKind::InvalidInput)?),
            )
        })
        .or_else(|_| std::path::absolute(path))
        .unwrap_or_else(|_| path.into())
}

fn document_name(path: Option<&Path>, recovered: Option<&str>, id: u64) -> String {
    path.and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
        .or_else(|| recovered.map(str::to_owned))
        .unwrap_or_else(|| {
            if id == 1 {
                "Untitled".into()
            } else {
                format!("Untitled {id}")
            }
        })
}

impl Editor {
    pub(super) fn name(&self) -> String {
        document_name(
            self.path.as_deref(),
            self.recovered_name.as_deref(),
            self.documents.tabs[self.documents.active].id,
        )
    }

    pub(super) fn document_tab_count(&self) -> usize {
        self.documents.tabs.len()
    }
    pub(super) fn active_document_tab(&self) -> usize {
        self.documents.active
    }

    pub(super) fn document_tab_info(&self, index: usize) -> (u64, String, bool, Option<&Path>) {
        let tab = &self.documents.tabs[index];
        if let Some(doc) = &tab.document {
            (
                tab.id,
                document_name(doc.path.as_deref(), doc.recovered_name.as_deref(), tab.id),
                !doc.baseline_is_saved || doc.pending_edit,
                doc.path.as_deref(),
            )
        } else {
            (
                tab.id,
                self.name(),
                !self.baseline_is_saved || self.pending_edit,
                self.path.as_deref(),
            )
        }
    }

    fn document_at_path(&self, path: &Path) -> Option<usize> {
        self.document_index(path, None)
            .or_else(|| self.document_index(path, Some(&path_key(path))))
    }

    pub(super) fn document_index(&self, path: &Path, key: Option<&Path>) -> Option<usize> {
        self.documents.tabs.iter().position(|tab| {
            let (file, canonical) = match &tab.document {
                Some(doc) => (&doc.path, &doc.canonical_path),
                None => (&self.path, &self.canonical_path),
            };
            file.as_deref() == Some(path) || (key.is_some() && canonical.as_deref() == key)
        })
    }

    pub(super) fn path_open_elsewhere(&self, path: &Path) -> bool {
        self.document_at_path(path)
            .is_some_and(|index| index != self.documents.active)
    }

    pub(super) fn focus_open_path(&mut self, path: &Path) -> bool {
        let Some(index) = self.document_at_path(path) else {
            return false;
        };
        self.focus_file_tab(index);
        true
    }

    pub(super) fn focus_file_tab(&mut self, index: usize) {
        self.select_document_tab(index);
        if index == self.documents.active {
            if let Some(path) = self.path.clone() {
                self.welcome_preview = None;
                if self.recent_projects.first() != Some(&path) {
                    self.update_recent(&path, true);
                }
                self.status = format!("Editing {}", self.name());
            }
        }
    }

    pub(super) fn allow_document_change(&mut self) -> bool {
        if self.exports.busy()
            || self.exports.is_open()
            || self.text_dialog.is_some()
            || self.guided_morph.is_some()
            || self.artboard_size.is_some()
            || self.library.is_editing()
            || self.recovery_dialog.is_open()
            || self.keyboard_settings.is_open()
            || self.crash_reports.is_open()
        {
            self.report_warning(
                "Could not switch documents",
                "Finish the current dialog before switching documents.",
            );
            return false;
        }
        if self.canvas.path_in_progress() || self.canvas.text_active() || self.canvas.rig.placing()
        {
            self.report_warning(
                "Could not switch documents",
                "Finish or cancel the current drawing gesture before switching documents.",
            );
            return false;
        }
        true
    }

    fn park_document(&mut self) {
        self.timeline.park();
        self.canvas.cancel_drag();
        self.finish_edit();
        self.park_draft_deadline();
        self.canvas.park();
        self.folder_assets = Default::default();
        self.library.reset_project();
        self.path_shortcuts = Default::default();
        self.text_menu = Default::default();
        self.menu_bar_open = false;
        self.artwork_menu_open = false;
    }

    fn activate_document(&mut self, index: usize) {
        if index == self.documents.active {
            return;
        }
        self.park_document();
        let mut incoming = self.documents.tabs[index]
            .document
            .take()
            .expect("inactive document");
        incoming.swap(self);
        self.documents.tabs[self.documents.active].document = Some(incoming);
        self.documents.active = index;
    }

    pub(super) fn select_document_tab(&mut self, index: usize) {
        if index < self.document_tab_count()
            && index != self.documents.active
            && self.allow_document_change()
        {
            self.activate_document(index);
        }
    }

    pub(super) fn new_document_tab(&mut self) {
        if self.allow_document_change() {
            self.documents.reusable_blank = false;
            self.open_document_tab(AnimationProject::default(), None);
        }
    }

    pub(super) fn open_document_tab(&mut self, project: AnimationProject, path: Option<PathBuf>) {
        let canonical = path.as_deref().map(path_key);
        self.open_document_with_key(project, path, canonical);
    }

    pub(super) fn open_document_with_key(
        &mut self,
        project: AnimationProject,
        path: Option<PathBuf>,
        canonical: Option<PathBuf>,
    ) {
        self.welcome_preview = None;
        self.finish_edit();
        if self.documents.reusable_blank
            && self.document_tab_count() == 1
            && self.path.is_none()
            && self.baseline_is_saved
            && !self.history.can_undo()
            && !self.history.can_redo()
        {
            self.replace_project_with_key(project, path, canonical);
            return;
        }
        self.documents.reusable_blank = false;
        let slot = self.document_tab_count();
        let document = Some(Box::new(StoredDocument::new(
            self, project, path, canonical,
        )));
        self.documents.tabs.push(Tab {
            id: self.documents.next_id,
            document,
        });
        self.documents.next_id += 1;
        self.activate_document(slot);
    }

    pub(super) fn close_document_tab(&mut self, index: usize) {
        self.close_document_tab_with(index, Self::confirm_switch);
    }

    pub(super) fn close_document_tab_with(
        &mut self,
        index: usize,
        confirm: impl FnOnce(&mut Self) -> OpenDecision,
    ) -> bool {
        if index >= self.document_tab_count() || !self.allow_document_change() {
            return false;
        }
        let previous = self.documents.active;
        self.activate_document(index);
        self.finish_edit();
        if !self.baseline_is_saved && confirm(self) == OpenDecision::Cancel {
            return false;
        }
        if self.document_tab_count() == 1 {
            self.status.clear();
            self.replace_project(AnimationProject::default(), None);
            self.documents.tabs[0].id = self.documents.next_id;
            self.documents.next_id += 1;
            self.documents.reusable_blank = true;
            return true;
        }
        self.revoke_shared_document();
        self.retire_draft();
        let next = if previous != index {
            previous
        } else if index + 1 < self.document_tab_count() {
            index + 1
        } else {
            index - 1
        };
        self.activate_document(next);
        self.documents.tabs.remove(index);
        if next > index {
            self.documents.active -= 1;
        }
        true
    }

    pub(super) fn confirm_close_all_with(
        &mut self,
        mut confirm: impl FnMut(&mut Self) -> OpenDecision,
    ) -> bool {
        self.finish_edit();
        let active = self.documents.active;
        for index in
            std::iter::once(active).chain((0..self.document_tab_count()).filter(|i| *i != active))
        {
            if !self.document_tab_info(index).2 {
                continue;
            }
            self.activate_document(index);
            if confirm(self) == OpenDecision::Cancel {
                return false;
            }
        }
        // A later Cancel leaves all unsaved documents and their drafts available.
        // Only retire discarded checkpoints after every tab has been confirmed.
        self.retire_draft();
        for doc in self.documents.inactive_mut() {
            if let Some(mut session) = doc.drafts.take() {
                if let Err(error) = session.writer.retire() {
                    self.status = format!("Could not remove a recovery draft. {error}");
                }
            }
        }
        true
    }
}
