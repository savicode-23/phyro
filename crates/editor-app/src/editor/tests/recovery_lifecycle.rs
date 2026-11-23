use super::*;
use crate::recovery;
use std::{
    fs,
    io::BufRead,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

mod lantern_walk;

const PROFILE: &str = "PHYRO_TEST_DRAFT_PROFILE";
const ROLE: &str = "PHYRO_TEST_DRAFT_ROLE";

fn edited(mut project: AnimationProject, label: &str) -> AnimationProject {
    let clip = &mut project.clips[0];
    let drawing = clip.drawing_at_mut(0);
    let mark = drawing.add_rectangle([24., 16.], [13., 21.], [225, 55, 35, 255]);
    let mark = drawing.objects.iter_mut().find(|o| o.id == mark).unwrap();
    mark.name = label.to_owned();
    mark.keys.extend([(0, [24., 16.]), (2, [28., 19.])]);
    project
}

fn editor(root: &Path) -> EditorWindow {
    let mut w = EditorWindow::new(egui::vec2(1100., 740.));
    w.ctx.style_mut(|style| style.animation_time = 0.);
    w.editor.enable_draft_recovery(root.join("drafts"));
    w.frame(vec![]);
    w
}

fn checkpoint(w: &mut EditorWindow, project: AnimationProject) {
    w.editor.project = project;
    w.editor.finish_edit();
    w.editor
        .poll_draft_recovery(&w.ctx, Instant::now() + Duration::from_secs(30));
}

#[test]
fn switching_tabs_before_autosave_keeps_independent_recovery_copies() {
    let folder = tempfile::tempdir().unwrap();
    let paths = ["Hero.vanim", "Forest.vanim"].map(|name| folder.path().join(name));
    let mut source = AnimationProject::default();
    source.clips[0].set_length(3).unwrap();
    let edits = ["Character edit", "Scene edit"].map(|name| edited(source.clone(), name));
    let mut window = editor(folder.path());
    for (path, project) in paths.iter().zip(&edits) {
        project_io::save_animation(path, &source, false).unwrap();
        window.editor.open_project(path.clone());
        window.editor.project = project.clone();
        window.editor.finish_edit();
    }
    window
        .editor
        .poll_draft_recovery(&window.ctx, Instant::now() + Duration::from_secs(20));
    window
        .editor
        .drafts
        .as_ref()
        .unwrap()
        .writer
        .wait_written()
        .unwrap();
    for tab in window.editor.documents.inactive_mut() {
        tab.drafts.as_ref().unwrap().writer.wait_written().unwrap();
    }
    drop(window);
    let drafts = recovery::discover(&folder.path().join("drafts")).unwrap();
    assert_eq!(drafts.len(), 2);
    for (path, expected) in paths.iter().zip(&edits) {
        let draft = drafts
            .iter()
            .find(|d| d.source.as_deref() == Some(path.as_path()))
            .unwrap();
        assert_eq!(draft.claim().unwrap().load().unwrap(), *expected);
        assert_eq!(project_io::load_animation(path).unwrap(), source);
    }
}

#[test]
fn recovery_child() {
    let Some(root) = std::env::var_os(PROFILE).map(PathBuf::from) else {
        return;
    };
    let role = std::env::var(ROLE).unwrap();
    let source = root.join("Original.vanim");
    let mut w = editor(&root);
    let publication_guard = match role.as_str() {
        "First" | "Second" => {
            w.editor.open_project(source.clone());
            // Checkpoint two revisions. A retry or late worker completion must
            // never replace the final committed drawing with this earlier one.
            let original = w.editor.project.clone();
            for label in ["Earlier revision", role.as_str()] {
                checkpoint(&mut w, edited(original.clone(), label));
            }
            w.editor
                .drafts
                .as_ref()
                .unwrap()
                .writer
                .wait_written()
                .unwrap();
            None
        }
        "Staging" => {
            w.editor.open_project(source);
            let original = w.editor.project.clone();
            checkpoint(&mut w, lantern_walk::edited(original.clone(), "Published"));
            let writer = &w.editor.drafts.as_ref().unwrap().writer;
            writer.wait_written().unwrap();
            let (staged, release) = writer.pause_next_publication();
            checkpoint(&mut w, lantern_walk::edited(original, "Replacement"));
            staged.recv_timeout(Duration::from_secs(30)).unwrap();
            Some(release)
        }
        "Restored" => {
            w.click("Recover a copy");
            assert!(w.editor.path.is_none());
            assert_ne!(w.editor.saved.as_ref(), Some(&w.editor.project));
            assert_eq!(
                w.editor.project,
                edited(project_io::load_animation(&source).unwrap(), "First")
            );
            None
        }
        _ => panic!("Unexpected child role {role}"),
    };
    fs::write(root.join(format!("{role}.ready")), []).unwrap();
    // The parent terminates an actual live process, bypassing Rust destructors
    // and eframe shutdown. EOF also releases the child if its parent exits.
    let mut line = String::new();
    let _ = std::io::stdin().lock().read_line(&mut line);
    drop(publication_guard);
}

struct RunningEditor(Child);

impl RunningEditor {
    fn launch(root: &Path, role: &str) -> Self {
        let mut child = Self(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "editor::tests::recovery_lifecycle::recovery_child",
                    "--nocapture",
                ])
                .env(PROFILE, root)
                .env(ROLE, role)
                .stdin(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        let ready = root.join(format!("{role}.ready"));
        let deadline = Instant::now() + Duration::from_secs(30);
        while !ready.is_file() {
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "{role} editor exited before publishing readiness"
            );
            assert!(
                Instant::now() < deadline,
                "{role} editor never became ready"
            );
            thread::sleep(Duration::from_millis(10));
        }
        child
    }
    fn kill(mut self) {
        self.0.kill().unwrap();
        assert!(!self.0.wait().unwrap().success());
    }
}

impl Drop for RunningEditor {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn killing_a_replacement_checkpoint_preserves_the_previous_thousand_frame_walk() {
    verify_interrupted_replacement(false);
}

#[test]
#[ignore = "exports and verifies all 1,000 rendered frames of the recovered walk"]
fn all_thousand_recovered_walk_frames_match_the_checkpoint() {
    verify_interrupted_replacement(true);
}

fn verify_interrupted_replacement(all_pixels: bool) {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("Original.vanim");
    let original = lantern_walk::manual_source(&source);
    let original_bytes = fs::read(&source).unwrap();
    let expected = lantern_walk::edited(original.clone(), "Published");
    assert_ne!(expected, lantern_walk::edited(original, "Replacement"));
    let writer = RunningEditor::launch(dir.path(), "Staging");
    writer.kill();

    let drafts = recovery::discover(&dir.path().join("drafts")).unwrap();
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].source.as_deref(), Some(source.as_path()));
    let mut w = editor(dir.path());
    w.click("Recover a copy");
    assert!(w.editor.path.is_none());
    lantern_walk::assert_checkpoint(dir.path(), &expected, &w.editor.project, all_pixels);
    assert_eq!(fs::read(source).unwrap(), original_bytes);
}

#[test]
fn killed_windows_recover_independently_and_a_restored_copy_survives_a_second_kill() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let drafts = root.join("drafts");
    let source = root.join("Original.vanim");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lantern-walk.vanim");
    let original_bytes = fs::read(fixture).unwrap();
    fs::write(&source, &original_bytes).unwrap();
    let original = project_io::load_animation(&source).unwrap();
    let first = RunningEditor::launch(root, "First");
    let second = RunningEditor::launch(root, "Second");
    assert!(recovery::discover(&drafts).unwrap().is_empty());
    first.kill();

    let verify = |name| {
        let candidates = recovery::discover(&drafts).unwrap();
        assert_eq!(
            candidates.len(),
            1,
            "only the terminated window is available"
        );
        assert_eq!(candidates[0].source.as_deref(), Some(source.as_path()));
        assert_eq!(
            candidates[0].claim().unwrap().load().unwrap(),
            edited(original.clone(), name)
        );
        assert_eq!(fs::read(&source).unwrap(), original_bytes);
    };
    verify("First");
    let restored = RunningEditor::launch(root, "Restored");
    assert!(recovery::discover(&drafts).unwrap().is_empty());
    restored.kill();
    verify("First");
    {
        let mut w = editor(root);
        w.click("Later");
        verify("First");
        w.click("File");
        w.click("Recover drafts…");
        w.click("Recover a copy");
        assert!(w.editor.path.is_none());
        assert_ne!(w.editor.saved.as_ref(), Some(&w.editor.project));
        assert_eq!(w.editor.project, edited(original.clone(), "First"));
        let failed_path = root.join("missing-dir").join("copy.vanim");
        assert!(!w.editor.save_to(failed_path, false));
        assert!(w.editor.path.is_none());
        let output = root.join("Recovered copy.vanim");
        assert!(w.editor.save_to(output.clone(), false));
        assert_eq!(
            project_io::load_animation(&output).unwrap(),
            edited(original.clone(), "First")
        );
    }
    assert!(recovery::discover(&drafts).unwrap().is_empty());
    second.kill();
    verify("Second");
    let mut next = editor(root);
    next.click("Discard…");
    next.frame(vec![]); // The shorter confirmation recenters on its next layout pass.
    next.click("Keep draft");
    next.frame(vec![]);
    next.click("Discard…");
    next.frame(vec![]);
    next.click("Discard draft");
    assert!(recovery::discover(&drafts).unwrap().is_empty());
    assert_eq!(fs::read(source).unwrap(), original_bytes);
}

#[test]
fn a_checkpoint_uses_the_committed_drawing_and_undoing_to_clean_retires_it() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut w = editor(root);
    let clean = w.editor.project.clone();
    let committed = edited(clean.clone(), "Committed");
    w.editor.project = committed.clone();
    w.editor.finish_edit();
    // A gesture can start while an older edit is waiting for its idle write.
    // The recovery archive should stop at the previous transaction boundary.
    w.editor.project.clips[0].drawing_at_mut(0).objects[0].position = [407., 301.];
    w.editor
        .poll_draft_recovery(&w.ctx, Instant::now() + Duration::from_secs(30));
    w.editor
        .drafts
        .as_ref()
        .unwrap()
        .writer
        .wait_written()
        .unwrap();
    w.editor.project = committed.clone(); // cancel the uncommitted gesture
    let drafts = root.join("drafts");
    let archives: Vec<_> = fs::read_dir(&drafts)
        .unwrap()
        .map(|e| e.unwrap().path().join("draft.vanim"))
        .filter(|p| p.is_file())
        .collect();
    assert_eq!(archives.len(), 1);
    assert_eq!(project_io::load_animation(&archives[0]).unwrap(), committed);
    w.editor.restore(false);
    assert_eq!(w.editor.project, clean);
    assert!(!archives[0].exists());
    drop(w);
    assert!(recovery::discover(&drafts).unwrap().is_empty());
}
