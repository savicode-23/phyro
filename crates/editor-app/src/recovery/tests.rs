use super::*;

fn drawing(name: &str) -> AnimationProject {
    let mut project = AnimationProject::default();
    project.clips[0].name = name.into();
    project
}

#[test]
fn independent_windows_same_path_and_untitled_work_survive_and_claim_exclusively() {
    let folder = tempfile::tempdir().unwrap();
    let root = folder.path().join("recovery");
    let original = folder.path().join("Artwork.vanim");
    fs::write(&original, "original stays intact").unwrap();
    let mut writers: Vec<_> = ["Blue", "Green", "Untitled strokes"]
        .into_iter()
        .enumerate()
        .map(|(index, name)| {
            let mut writer = Writer::new(root.clone());
            writer
                .checkpoint((index < 2).then(|| original.clone()), drawing(name))
                .unwrap();
            writer.wait_written().unwrap();
            writer
        })
        .collect();
    assert!(discover(&root).unwrap().is_empty());
    drop(writers.remove(0));
    let first = discover(&root).unwrap().pop().unwrap();
    fs::write(first.path.join(LOCK), LEGACY_SIGNATURE).unwrap();
    let first = discover(&root).unwrap().pop().unwrap();
    assert_eq!(first.source.as_ref(), Some(&original));
    let claim = first.claim().unwrap();
    assert_eq!(claim.load().unwrap(), drawing("Blue"));
    assert!(first
        .claim()
        .is_err_and(|e| e.kind() == io::ErrorKind::WouldBlock));
    drop(writers);
    assert_eq!(discover(&root).unwrap().len(), 2);
    let mut restored = Writer::new(root.clone());
    restored.adopt(claim).unwrap();
    restored.checkpoint(None, drawing("Reopened blue")).unwrap();
    restored.wait_written().unwrap();
    drop(restored);
    let all = discover(&root).unwrap();
    assert_eq!(all.len(), 3);
    let mut names = Vec::new();
    for draft in all {
        let mut claim = draft.claim().unwrap();
        let name = claim.load().unwrap().clips.remove(0).name;
        assert_eq!(claim.source().is_some(), name != "Untitled strokes");
        names.push(name);
        claim.discard().unwrap();
    }
    names.sort();
    assert_eq!(names, ["Green", "Reopened blue", "Untitled strokes"]);
    assert!(discover(&root).unwrap().is_empty());
    assert_eq!(
        fs::read_to_string(original).unwrap(),
        "original stays intact"
    );
}

#[test]
fn failed_checkpoint_and_interrupted_publish_leave_previous_valid_archive() {
    let root = tempfile::tempdir().unwrap();
    let mut writer = Writer::new(root.path().to_owned());
    writer.checkpoint(None, drawing("Earlier")).unwrap();
    writer.wait_written().unwrap();
    let mut invalid = drawing("Invalid");
    invalid.clips[0].fps = 0;
    writer.checkpoint(None, invalid).unwrap();
    assert!(writer.wait_written().is_err());
    assert!(writer.take_error().unwrap().contains("checkpoint"));
    drop(writer);
    let draft = discover(root.path()).unwrap().pop().unwrap();
    // Kill at either point before publication: while writing the native temp,
    // or after it is a complete staged archive but before the guarded rename.
    fs::write(draft.path.join(".tmpAb3Z6r"), b"incomplete").unwrap();
    let claim = draft.claim().unwrap();
    claim.lease.stage(&drawing("Not published")).unwrap();
    drop(claim);
    let mut claim = discover(root.path())
        .unwrap()
        .pop()
        .unwrap()
        .claim()
        .unwrap();
    assert_eq!(claim.load().unwrap(), drawing("Earlier"));
    claim.discard().unwrap();
    drop(claim);
    assert!(!draft.path.exists());
}

#[test]
fn failed_first_write_retries_without_another_edit() {
    let folder = tempfile::tempdir().unwrap();
    let root = folder.path().join("recovery");
    fs::write(&root, b"blocks directory creation").unwrap();
    let mut writer = Writer::new(root.clone());
    writer.checkpoint(None, drawing("Retry")).unwrap();
    assert!(writer.wait_written().is_err());
    fs::remove_file(&root).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while writer.wait_written().is_err() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    drop(writer);
    let draft = discover(&root).unwrap().pop().unwrap();
    assert_eq!(draft.claim().unwrap().load().unwrap(), drawing("Retry"));
}

#[test]
fn discovery_skips_foreign_folders_and_does_not_remove_unrelated_files() {
    let root = tempfile::tempdir().unwrap();
    let foreign = root.path().join("draft-personal");
    fs::create_dir(&foreign).unwrap();
    fs::write(foreign.join(LOCK), b"another application").unwrap();
    fs::write(foreign.join("notes"), b"unrelated").unwrap();
    assert!(discover(root.path()).unwrap().is_empty());
    assert!(foreign.join("notes").exists());
    let mut writer = Writer::new(root.path().to_owned());
    writer.checkpoint(None, drawing("Test")).unwrap();
    writer.wait_written().unwrap();
    drop(writer);
    let draft = discover(root.path()).unwrap().pop().unwrap();
    fs::write(draft.path.join("my notes.txt"), b"keep").unwrap();
    let mut claim = draft.claim().unwrap();
    claim.discard().unwrap();
    drop(claim);
    assert_eq!(fs::read(draft.path.join("my notes.txt")).unwrap(), b"keep");
    assert!(discover(root.path()).unwrap().is_empty());
}

#[test]
fn inaccessible_or_malformed_siblings_do_not_hide_valid_drafts() {
    let root = tempfile::tempdir().unwrap();
    let mut writer = Writer::new(root.path().to_owned());
    writer.checkpoint(None, drawing("Recover me")).unwrap();
    writer.wait_written().unwrap();
    drop(writer);
    // A recognized but damaged entry whose archive path is not even a file.
    let broken = root.path().join("draft-damaged");
    fs::create_dir(&broken).unwrap();
    fs::write(broken.join(LOCK), SIGNATURE).unwrap();
    fs::create_dir(broken.join(ARCHIVE)).unwrap();
    let drafts = discover(root.path()).unwrap();
    assert_eq!(drafts.len(), 1);
    let mut claim = drafts[0].claim().unwrap();
    assert_eq!(claim.load().unwrap(), drawing("Recover me"));
    claim.discard().unwrap();
    drop(claim);
    assert!(discover(root.path()).is_err_and(|e| e.kind() == io::ErrorKind::InvalidData));
    assert!(broken.join(ARCHIVE).is_dir());
}
