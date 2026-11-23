use super::*;
use egui::{Event, FullOutput, Key, Modifiers, PointerButton, Pos2};

struct Window {
    ctx: egui::Context,
    dialog: RecoveryDialog,
}

impl Window {
    fn new() -> Self {
        let ctx = egui::Context::default();
        ctx.style_mut(|style| style.animation_time = 0.);
        Self {
            ctx,
            dialog: RecoveryDialog::default(),
        }
    }

    fn frame(&mut self, events: Vec<Event>) -> (FullOutput, Option<AnimationProject>) {
        let mut loaded = None;
        let output = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    Pos2::ZERO,
                    egui::vec2(800., 500.),
                )),
                events,
                ..Default::default()
            },
            |ctx| loaded = self.dialog.show(ctx),
        );
        (output, loaded)
    }

    fn control(&mut self, label: &str) -> Pos2 {
        self.frame(vec![]);
        let (output, _) = self.frame(vec![]);
        output
            .shapes
            .iter()
            .rev()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == label => {
                    let rect = text.galley.rect.translate(text.pos.to_vec2());
                    shape.clip_rect.contains_rect(rect).then_some(rect.center())
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("Missing visible control {label:?}"))
    }

    fn click(&mut self, label: &str) -> Option<AnimationProject> {
        let pos = self.control(label);
        self.frame(vec![Event::PointerMoved(pos)]);
        let button = |pressed| Event::PointerButton {
            pos,
            pressed,
            button: PointerButton::Primary,
            modifiers: Modifiers::NONE,
        };
        self.frame(vec![button(true)]);
        self.frame(vec![button(false)]).1
    }
}

fn draft(root: &Path, source: Option<&Path>, name: &str) -> AnimationProject {
    let mut project = AnimationProject::default();
    project.clips[0].name = name.into();
    let mut writer = recovery::Writer::new(root.to_owned());
    writer
        .checkpoint(source.map(Path::to_owned), project.clone())
        .unwrap();
    writer.wait_written().unwrap();
    project
}

#[test]
fn later_preserves_all_drafts_and_recover_claims_only_the_selected_copy() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("Lantern.vanim");
    let original = draft(root.path(), Some(&path), "Lantern exposure");
    draft(root.path(), None, "Untitled exposure");
    let mut window = Window::new();
    window.dialog.scan(Some(root.path()), true);
    assert!(window.dialog.is_open());
    let mut second = RecoveryDialog::default();
    second.scan(Some(root.path()), true);
    assert!(!second.is_open(), "only one window offers a claimed draft");
    window.control("Untitled");
    window.control("Lantern.vanim");
    let checkpoint_time = window.dialog.drafts[0].modified.clone();
    window.control(&checkpoint_time);
    window.click("Later");
    assert_eq!(recovery::discover(root.path()).unwrap().len(), 2);
    window.dialog.scan(Some(root.path()), false);
    window.click("Lantern.vanim");
    assert_eq!(window.dialog.selected_source(), Some(path.as_path()));
    assert_eq!(window.click("Recover a copy").unwrap(), original);
    // The editor can decline to replace its current drawing after this load.
    assert!(recovery::discover(root.path()).unwrap().is_empty());
    assert_eq!(window.click("Recover a copy").unwrap(), original);
    let recovered = window.dialog.take_recovered().unwrap();
    assert!(!window.dialog.is_open());
    let remaining = recovery::discover(root.path()).unwrap();
    assert_eq!(remaining.len(), 1);
    assert!(remaining[0].source.is_none());
    drop(recovered);
    assert_eq!(recovery::discover(root.path()).unwrap().len(), 2);
}

#[test]
fn discard_requires_confirmation_and_removes_only_one_draft() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("Original.vanim");
    let project = draft(root.path(), Some(&source), "Recovered pose");
    project_io::save_animation(&source, &project, false).unwrap();
    draft(root.path(), None, "Second pose");
    let mut window = Window::new();
    window.dialog.scan(Some(root.path()), true);
    window.click("Original.vanim");
    window.click("Discard…");
    window.control("Permanently discard this recovery copy?");
    window.frame(vec![Event::Key {
        key: Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }]);
    assert!(window.dialog.is_open());
    assert!(!window.dialog.confirm_discard);
    window.click("Discard…");
    window.click("Keep draft");
    window.click("Discard…");
    window.click("Discard draft");
    window.control("Untitled");
    window.click("Later");
    let remaining = recovery::discover(root.path()).unwrap();
    assert_eq!(remaining.len(), 1);
    assert!(remaining[0].source.is_none());
    assert_eq!(project_io::load_animation(&source).unwrap(), project);
}

#[test]
fn empty_and_unavailable_storage_only_open_on_request() {
    let root = tempfile::tempdir().unwrap();
    let mut window = Window::new();
    for directory in [Some(root.path()), None] {
        window.dialog.scan(directory, true);
        assert!(!window.dialog.is_open());
        window.dialog.scan(directory, false);
        window.control(if directory.is_some() {
            "No recoverable drafts are available."
        } else {
            "Recovery storage is unavailable on this device."
        });
        window.click("Close");
        assert!(!window.dialog.is_open());
    }
}

#[test]
fn broken_draft_keeps_its_claim_and_can_be_retried() {
    let root = tempfile::tempdir().unwrap();
    let original = draft(root.path(), None, "Restored drawing");
    let archive = std::fs::read_dir(root.path())
        .unwrap()
        .map(|entry| entry.unwrap().path().join("draft.vanim"))
        .find(|path| path.is_file())
        .unwrap();
    let bytes = std::fs::read(&archive).unwrap();
    let mut window = Window::new();
    window.dialog.scan(Some(root.path()), true);
    std::fs::write(&archive, b"partial archive").unwrap();
    assert!(window.click("Recover a copy").is_none());
    assert!(window.dialog.is_open());
    let error = window.dialog.error.clone().unwrap();
    assert!(error.starts_with("Could not open this draft."));
    window.control(&error);
    assert!(recovery::discover(root.path()).unwrap().is_empty());
    std::fs::write(&archive, bytes).unwrap();
    assert_eq!(window.click("Recover a copy").unwrap(), original);
}
