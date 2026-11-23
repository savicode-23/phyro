use super::*;
use std::process::Command;
use tempfile::tempdir;

fn sample() -> Context {
    Context {
        clip_index: 1,
        tick: 6,
        tool: Tool::Node,
        layer_count: 3,
        object_count: 21,
        selection_count: 2,
        page_size: [128, 192],
        pixels_per_point: 1.25,
        window: Some(WindowState::Background),
    }
}

#[test]
fn reports_remain_readable_after_ack_and_never_accept_an_arbitrary_path() {
    let dir = tempdir().unwrap();
    let store = Diagnostics::in_dir(dir.path().into(), "0123456789abcdef").unwrap();
    store.context(sample());
    store.note(Action::Open);
    store.note(Action::Difference);
    for _ in 0..1000 {
        store.note(Action::Zoom);
    }
    store.graphics("Synthetic GPU", "Renderer 123", "4.6");
    let report = store.record_startup_failure("graphics", Some(-7)).unwrap();
    let expected = store.read(&report).unwrap();
    assert!(expected.contains("Difference(") && expected.matches("Zoom(").count() == 1);
    assert!(
        expected.contains("Page size: 128 x 192 px") && expected.contains("Native error code: -7")
    );
    assert!(expected.contains("Renderer 123") && expected.contains("0123456789abcdef"));
    assert_eq!(store.pending().unwrap(), std::slice::from_ref(&report));
    store.acknowledge(&report).unwrap();
    store.acknowledge(&report).unwrap();
    assert!(store.pending().unwrap().is_empty());
    assert_eq!(store.read(&report).unwrap(), expected);
    let original = expected.replacen("Phyro crash report", "Loopline crash report", 1);
    fs::write(store.directory().join(&report.filename), &original).unwrap();
    assert_eq!(store.read(&report).unwrap(), original);
    assert_eq!(store.reports().unwrap(), std::slice::from_ref(&report));
    let invalid = Report {
        filename: "../private.txt".into(),
        ..report
    };
    assert!(store.acknowledge(&invalid).is_err() && store.read(&invalid).is_err());
}

#[test]
fn hook_child() {
    let Ok(kind) = std::env::var("PHYRO_TEST_DIAGNOSTIC_PANIC") else {
        return;
    };
    let store = Diagnostics::install("abcdef012345").unwrap();
    if kind == "recover" {
        store.end_session();
        return;
    }
    store.context(sample());
    store.note(Action::Open);
    store.note(Action::Union);
    if kind == "abrupt" || kind == "idle_exit" {
        drop(store.editor_update());
        let unfinished = (kind == "abrupt").then(|| store.editor_update());
        if unfinished.is_some() {
            store.phase(UpdatePhase::Timeline);
        }
        store.graphics(
            "NVIDIA Corporation",
            "NVIDIA GeForce RTX 4070 Ti/PCIe/SSE2",
            "4.6.0",
        );
        let deadline = Instant::now() + Duration::from_secs(15);
        let sessions = store.directory().join("sessions");
        loop {
            if fs::read_dir(&sessions)
                .unwrap()
                .filter_map(Result::ok)
                .filter(|e| e.path().extension() == Some(OsStr::new("ctx")))
                .any(|e| {
                    fs::read_to_string(e.path())
                        .ok()
                        .and_then(|s| Snapshot::decode(&s))
                        .and_then(|s| s.checkpoint)
                        .is_some_and(|c| (c.stalled_for >= 3) == unfinished.is_some())
                })
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "activity snapshot was never published"
            );
            std::thread::sleep(Duration::from_millis(30));
        }
        // Exit only this isolated test process, bypassing Rust destructors.
        std::process::exit(71);
    }
    if kind == "worker" {
        let child = std::thread::Builder::new()
            .name("private-customer-thread".into())
            .spawn(|| panic!("private-customer-panic"))
            .unwrap();
        assert!(child.join().is_err());
        assert_eq!(store.worker_reports(), 1);
    } else {
        panic!("private-customer-panic");
    }
}

fn recovered_child_report(disposition: &str) -> String {
    let dir = tempdir().unwrap();
    for (phase, exit) in [(disposition, 71), ("recover", 0), ("recover", 0)] {
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "diagnostics::tests::hook_child", "--nocapture"])
            .env("PHYRO_TEST_DIAGNOSTIC_PANIC", phase)
            .env("LOCALAPPDATA", dir.path())
            .env("XDG_STATE_HOME", dir.path())
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(exit),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let folder = if cfg!(windows) {
        "Phyro/CrashReports"
    } else {
        "phyro/crash-reports"
    };
    let store = Diagnostics::in_dir(dir.path().join(folder), "abcdef012345").unwrap();
    let reports = store.pending().unwrap();
    assert_eq!(reports.len(), 1);
    store.read(&reports[0]).unwrap()
}

#[test]
fn restarting_after_an_abrupt_exit_recovers_saved_activity_and_hardware_once() {
    let detail = recovered_child_report("abrupt");
    assert!(
        detail.contains("Unexpected exit")
            && detail.contains("Union(")
            && detail.contains("Tool: Node")
            && detail.contains("Last diagnostic checkpoint (UTC):")
            && detail.contains("Latest completed editor update at checkpoint:")
            && detail.contains("Editor update in progress at checkpoint: at least")
            && detail.contains("UI phase at that sample: Timeline")
            && detail.contains("Window at last UI observation: Background")
    );
    assert!(
        detail.contains("RTX 4070")
            && detail.contains("OpenGL: 4.6")
            && detail.contains("abcdef012345")
    );
}

#[test]
fn an_idle_background_session_recovers_without_flagging_a_blocked_update() {
    let detail = recovered_child_report("idle_exit");
    assert!(detail.contains("Unexpected exit"));
    assert!(detail.contains("Window at last UI observation: Background"));
    assert!(detail.contains("Latest completed editor update at checkpoint:"));
    assert!(!detail.contains("Editor update in progress at checkpoint:"));
    assert!(!detail.contains("UI phase at that sample:"));
}

#[test]
fn real_hook_captures_ui_and_worker_panics_without_payload_or_personal_paths() {
    for kind in ["worker", "main"] {
        let dir = tempdir().unwrap();
        let mut cmd = Command::new(std::env::current_exe().unwrap());
        cmd.args(["--exact", "diagnostics::tests::hook_child", "--nocapture"])
            .env("PHYRO_TEST_DIAGNOSTIC_PANIC", kind)
            .env("LOCALAPPDATA", dir.path())
            .env("XDG_STATE_HOME", dir.path());
        let output = cmd.output().unwrap();
        assert_eq!(
            output.status.success(),
            kind == "worker",
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let folder = if cfg!(windows) {
            "Phyro/CrashReports"
        } else {
            "phyro/crash-reports"
        };
        let store = Diagnostics::in_dir(dir.path().join(folder), "abcdef012345").unwrap();
        let reports = store.pending().unwrap();
        assert_eq!(reports.len(), 1);
        let text = store.read(&reports[0]).unwrap();
        assert!(text.contains("Source: editor-app/src/diagnostics/tests.rs:"));
        assert!(text.contains("Union(") && text.contains("Stack:"));
        assert!(text.contains(if kind == "worker" {
            "Background thread panic"
        } else {
            "Main thread panic"
        }));
        assert!(
            !text.contains("private-customer")
                && !text.contains("C:\\Users\\")
                && !text.contains("/home/")
        );
        #[cfg(windows)]
        assert!(text.contains("Executable frame offsets (RVA): +0x"));
    }
}

#[test]
fn snapshots_round_trip_and_only_recognized_numeric_categories_survive_recovery() {
    let dir = tempdir().unwrap();
    let store = Diagnostics::in_dir(dir.path().into(), "abc1234").unwrap();
    store.context(sample());
    for _ in 0..40 {
        store.note(Action::Move);
        store.note(Action::Resize);
    }
    let snap = store.0.snapshot(store.0.ui.sample(Duration::from_secs(3)));
    assert_eq!(snap.actions.len(), HISTORY);
    let encoded = snap.encode();
    let decoded = Snapshot::decode(&encoded).unwrap();
    assert_eq!(decoded.context, Some(sample()));
    assert_eq!(decoded.actions, snap.actions);
    assert_eq!(decoded.checkpoint, snap.checkpoint);
    let mut old_context = sample();
    old_context.window = None;
    let history = encoded.split(':').nth(1).unwrap();
    let old = Snapshot::decode(&format!("1 6 1 3 21 2 128 192 125:{history}")).unwrap();
    assert_eq!(old.actions, snap.actions);
    assert_eq!(old.context, Some(old_context));
    assert_eq!(old.checkpoint, None);
    for invalid in [
        "",
        "password:",
        "0 0 900 1 2 3 4 5 6:",
        "0 0 4:",
        "private drawing",
        "0:2,2",
        "::5 7 0",
        "::5 1 20",
        "::5 1 0 9",
        "::5 1 3 999",
        "::5 1 3 0",
    ] {
        assert!(Snapshot::decode(invalid).is_none());
    }
    let session_time = UNIX_EPOCH + Duration::from_secs(1_582_978_203);
    assert_eq!(utc(session_time), "2020-02-29 12:10:03 UTC");
    let gpu = Graphics::from_driver(
        "NVIDIA Corporation",
        "NVIDIA GeForce RTX 4070 Ti/PCIe/SSE2",
        "4.6.0 NVIDIA 566.03",
    );
    for _ in 0..2 {
        store
            .recover("100-2-2", session_time, "abc1234", &encoded, &gpu.encode())
            .unwrap();
    }
    let reports = store.pending().unwrap();
    assert_eq!(reports.len(), 1);
    let text = store.read(&reports[0]).unwrap();
    assert!(
        text.contains("Unexpected exit") && text.contains("Tool: Node") && text.contains("Resize(")
    );
    assert!(text.contains("RTX 4070") && text.contains("OpenGL: 4.6"));
}

#[test]
fn consecutive_actions_retain_the_latest_seen_timestamp_without_filling_the_ring() {
    let dir = tempdir().unwrap();
    let store = Diagnostics::in_dir(dir.path().into(), "abc1234").unwrap();
    store.note_at(Action::Resize, 3);
    store.note_at(Action::Zoom, 5);
    store.note_at(Action::Zoom, 59);
    let generation = store.0.generation.load(Relaxed);
    store.note_at(Action::Zoom, 59);
    assert_eq!(store.0.generation.load(Relaxed), generation);
    let snap = store.0.snapshot(store.0.ui.sample(Duration::from_secs(60)));
    assert_eq!(snap.actions, [(Action::Resize, 3), (Action::Zoom, 59)]);
}

#[test]
fn concurrent_publication_and_retention_preserve_complete_independent_reports() {
    let dir = tempdir().unwrap();
    let store = Diagnostics::in_dir(dir.path().into(), "99aaff00").unwrap();
    std::thread::scope(|threads| {
        for i in 0..6 {
            let store = store.clone();
            threads.spawn(move || {
                for n in 0..8 {
                    store
                        .record_startup_failure("window", Some(i * 8 + n))
                        .unwrap();
                }
            });
        }
    });
    let reports = store.reports().unwrap();
    assert_eq!(reports.len(), MAX_REPORTS);
    for r in reports {
        assert!(store.read(&r).unwrap().contains("Native error code:"));
    }
}

#[test]
fn capture_keeps_numeric_failure_classes_and_strips_paths_and_untrusted_data() {
    let graphics = Graphics::from_driver("", "Radeon RX 6700", "3.3").describe();
    assert!(graphics.contains("GPU vendor: AMD") && graphics.contains("RX 6700"));
    assert_eq!(
        panic_class(Some(
            "index out of bounds: the len is 8 but the index is 12"
        )),
        "Index out of bounds (length 8, index 12)"
    );
    assert!(!panic_class(Some("assertion failed: private object A")).contains("private"));
    assert!(!panic_class(Some(
        "called `Result::unwrap()` on an `Err` value: C:\\private-file"
    ))
    .contains("private"));
    let trace=safe_trace("  0: phyro::paint\n       at C:\\Users\\private-name\\checkout\\crates\\editor-app\\src\\canvas.rs:99:2\n  1: C:\\private-name\\custom.dll\n  2: private-data\n");
    assert!(trace.contains("phyro::paint") && trace.contains("editor-app/src/canvas.rs:99:2"));
    assert!(!trace.contains("private"));
    assert_eq!(driver_field("GPU\nprivate output"), "unavailable");
}
