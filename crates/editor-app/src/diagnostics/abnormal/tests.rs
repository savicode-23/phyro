use super::*;
use std::{
    io::BufRead,
    process::{Child, Command, Stdio},
    thread,
    time::Instant,
};

const CHILD_DIR: &str = "PHYRO_TEST_ABNORMAL_CHILD_DIR";
const BUILD: &str = "0123456789abcdef";

#[test]
fn session_child() {
    let Some(dir) = std::env::var_os(CHILD_DIR) else {
        return;
    };
    let dir = PathBuf::from(dir);
    let session =
        Session::start(&dir, BUILD, |_, _, _, _, _| panic!("unexpected prior run")).unwrap();
    session.update_context("1 2 3").unwrap();
    session.update_context("7 8 9").unwrap();
    session.set_graphics("2 4090 4 6").unwrap();
    assert!(session.set_graphics("driver path").is_err());
    assert!(session.update_context("private project name").is_err());
    fs::write(dir.join("ready"), []).unwrap();
    let mut disposition = String::new();
    std::io::stdin().lock().read_line(&mut disposition).unwrap();
    match disposition.trim() {
        "clean" => drop(session),
        "reported" => {
            session.mark_panic();
            session.update_context("99").unwrap();
            std::process::exit(73);
        }
        "abrupt" => std::process::exit(73),
        other => panic!("Unknown child disposition {other}"),
    }
}

fn child(dir: &Path) -> Child {
    let mut process = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "diagnostics::abnormal::tests::session_child",
            "--nocapture",
        ])
        .env(CHILD_DIR, dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !dir.join("ready").is_file() {
        assert!(
            process.try_wait().unwrap().is_none(),
            "child exited before locking its marker"
        );
        assert!(
            Instant::now() < deadline,
            "child did not publish its marker"
        );
        thread::sleep(Duration::from_millis(5));
    }
    process
}

fn finish(mut process: Child, command: &str) {
    writeln!(process.stdin.take().unwrap(), "{command}").unwrap();
    assert_eq!(
        process.wait().unwrap().code(),
        Some(if command == "clean" { 0 } else { 73 })
    );
}

fn assert_no_recovery(dir: &Path) {
    let _session = Session::start(dir, BUILD, |_, _, _, _, _| {
        panic!("a live or clean run was misclassified")
    })
    .unwrap();
}

#[test]
fn concurrent_editors_clean_shutdown_and_already_reported_panics_do_not_create_reports() {
    for disposition in ["clean", "reported"] {
        let dir = tempfile::tempdir().unwrap();
        let process = child(dir.path());
        assert_no_recovery(dir.path());
        finish(process, disposition);
        assert_no_recovery(dir.path());
        assert_eq!(
            fs::read_dir(dir.path().join("sessions")).unwrap().count(),
            0
        );
    }
}

#[test]
fn abrupt_exit_keeps_last_complete_activity_and_retries_an_unwritten_report_once() {
    let dir = tempfile::tempdir().unwrap();
    let process = child(dir.path());
    assert_no_recovery(dir.path());
    finish(process, "abrupt");

    let mut expected_key = String::new();
    drop(
        Session::start(
            dir.path(),
            BUILD,
            |key, started, build, context, graphics| {
                assert!(started <= SystemTime::now());
                assert_eq!(build, BUILD);
                assert_eq!(graphics, "2 4090 4 6");
                assert_eq!(context, "7 8 9");
                expected_key = key.into();
                Err(io::Error::other("Simulated report storage failure"))
            },
        )
        .unwrap(),
    );
    assert!(!expected_key.is_empty());

    // Concurrent launches can both enumerate the stale file. Only one of
    // them may publish it, even if the other already opened the same handle.
    let reports = std::sync::Mutex::new(Vec::new());
    let start = std::sync::Barrier::new(4);
    thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| {
                start.wait();
                let _session = Session::start(dir.path(), BUILD, |key, _, _, context, _| {
                    reports
                        .lock()
                        .unwrap()
                        .push((key.to_owned(), context.to_owned()));
                    Ok(())
                })
                .unwrap();
            });
        }
    });
    assert_eq!(
        reports.into_inner().unwrap(),
        [(expected_key, "7 8 9".into())]
    );
    assert_no_recovery(dir.path());
}

#[test]
fn unknown_activity_is_omitted_from_the_recovered_report() {
    for bad_context in [
        b"arbitrary untrusted contents".to_vec(),
        vec![b'0'; 513],
        vec![0xff],
    ] {
        let dir = tempfile::tempdir().unwrap();
        let process = child(dir.path());
        finish(process, "abrupt");
        let runs = dir.path().join("sessions");
        let context = fs::read_dir(&runs)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|e| e == "ctx"))
            .unwrap();
        fs::write(
            context.with_extension("run"),
            format!("{LEGACY_PREFIX}untrusted identifier"),
        )
        .unwrap();
        fs::write(context.with_extension("gpu"), &bad_context).unwrap();
        fs::write(context, bad_context).unwrap();
        let mut recovered = false;
        let _session = Session::start(dir.path(), BUILD, |_, _, build, context, graphics| {
            assert_eq!(build, "unknown");
            assert!(graphics.is_empty());
            assert!(context.is_empty());
            recovered = true;
            Ok(())
        })
        .unwrap();
        assert!(recovered);
    }
}

#[test]
fn killing_only_the_test_child_leaves_a_reportable_session() {
    let dir = tempfile::tempdir().unwrap();
    let mut process = child(dir.path());
    process.kill().unwrap();
    process.wait().unwrap();
    let mut recovered = 0;
    drop(
        Session::start(dir.path(), BUILD, |_, _, _, context, _| {
            assert_eq!(context, "7 8 9");
            recovered += 1;
            Ok(())
        })
        .unwrap(),
    );
    assert_eq!(recovered, 1);
    assert_no_recovery(dir.path());
}

#[test]
fn explicit_close_tolerates_a_writer_still_holding_the_session() {
    let dir = tempfile::tempdir().unwrap();
    let session = Session::start(
        dir.path(),
        "unrecognized identifier",
        |_, _, _, _, _| Ok(()),
    )
    .unwrap();
    let together = std::sync::Barrier::new(2);
    thread::scope(|scope| {
        scope.spawn(|| {
            together.wait();
            for _ in 0..8 {
                // Closing can remove a pending rename. Its failure must leave no sidecar.
                let _ = session.update_context("123");
            }
        });
        together.wait();
        session.close();
    });
    assert_eq!(
        fs::read_dir(dir.path().join("sessions")).unwrap().count(),
        0
    );
    assert_no_recovery(dir.path());
}
