use super::invalid;
use std::collections::{hash_map::DefaultHasher, BTreeMap};
use std::fs::{self, File};
use std::hash::Hasher;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

type Signature = (u64, u64);

/// The exact paths and contents previously written during this editor session.
#[derive(Clone, Debug)]
pub struct ExportReceipt {
    destination: PathBuf,
    files: BTreeMap<PathBuf, Signature>,
}

impl ExportReceipt {
    pub fn directory(&self) -> &Path {
        &self.destination
    }

    /// Files published by this export, relative to its destination directory.
    pub fn files(&self) -> impl Iterator<Item = &Path> {
        self.files.keys().map(PathBuf::as_path)
    }

    pub(super) fn new(directory: &Path) -> Self {
        Self {
            destination: directory.into(),
            files: BTreeMap::new(),
        }
    }
    pub(super) fn stage(
        &mut self,
        staging: &Path,
        relative: &Path,
        bytes: &[u8],
    ) -> io::Result<()> {
        let mut file = File::create_new(staging.join(relative))?;
        file.write_all(bytes)?;
        file.sync_all()?;
        let mut hash = DefaultHasher::new();
        hash.write(bytes);
        self.files
            .insert(relative.into(), (bytes.len() as u64, hash.finish()));
        Ok(())
    }

    pub(super) fn register_staged(&mut self, staging: &Path, relative: &Path) -> io::Result<()> {
        let path = staging.join(relative);
        self.files.insert(
            relative.into(),
            (fs::metadata(&path)?.len(), file_hash(&path)?),
        );
        Ok(())
    }

    pub(super) fn publish(&self, staging: &Path, previous: Option<&Self>) -> io::Result<()> {
        let destination = &self.destination;
        if let Some(old) = previous {
            if old.directory() != destination {
                return Err(invalid("The export destination has changed."));
            }
            require_directory(destination)?;
            for (file, signature) in &old.files {
                if file.parent().is_some_and(|p| !p.as_os_str().is_empty()) {
                    require_directory(&destination.join(file.parent().unwrap()))?;
                }
                let path = destination.join(file);
                if !unchanged(&path, *signature)? {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        format!(
                            "{} has changed since export. Choose a new destination.",
                            path.display()
                        ),
                    ));
                }
            }
        } else {
            fs::create_dir(destination)?;
        }
        let mut new_frames_dir = false;
        let mut published = Vec::new();
        let mut moved = Vec::new();
        let backup = match tempfile::tempdir_in(staging.parent().unwrap_or(staging)) {
            Ok(dir) => dir,
            Err(e) => {
                if previous.is_none() {
                    let _ = fs::remove_dir(destination);
                }
                return Err(e);
            }
        };
        let result = (|| {
            if self.files.keys().any(|p| p.starts_with("frames")) {
                let frames = destination.join("frames");
                match fs::create_dir(&frames) {
                    Ok(()) => new_frames_dir = true,
                    Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                        require_directory(&frames)?
                    }
                    Err(e) => return Err(e),
                }
            }
            if let Some(old) = previous {
                fs::create_dir(backup.path().join("frames"))?;
                for file in old.files.keys() {
                    fs::rename(destination.join(file), backup.path().join(file))?;
                    moved.push(file);
                }
            }
            for file in self.files.keys() {
                publish_new(&staging.join(file), &destination.join(file))?;
                published.push(file);
            }
            Ok(())
        })();
        if let Err(error) = result {
            for file in published.iter().rev() {
                let _ = fs::remove_file(destination.join(file));
            }
            for file in moved {
                if publish_new(&backup.path().join(file), &destination.join(file)).is_err() {
                    let saved = backup.keep();
                    return Err(io::Error::other(format!(
                        "Export failed: {error}. Recover previous outputs from {}.",
                        saved.display()
                    )));
                }
            }
            if new_frames_dir {
                let _ = fs::remove_dir(destination.join("frames"));
            }
            if previous.is_none() {
                let _ = fs::remove_dir(destination);
            }
            return Err(error);
        }
        Ok(())
    }
}

fn require_directory(path: &Path) -> io::Result<()> {
    if fs::symlink_metadata(path)?.file_type().is_dir() {
        Ok(())
    } else {
        Err(invalid(format!(
            "{} must be a regular directory.",
            path.display()
        )))
    }
}

// Verify bytes before moving any old outputs. Files unrelated to the export, such
// as Godot import settings, remain in place.
fn unchanged(path: &Path, expected: Signature) -> io::Result<bool> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    if !meta.is_file() || meta.len() != expected.0 {
        return Ok(false);
    }
    Ok(file_hash(path)? == expected.1)
}

fn file_hash(path: &Path) -> io::Result<u64> {
    let mut file = File::open(path)?;
    let mut hash = DefaultHasher::new();
    let mut buf = [0u8; 16384];
    loop {
        let count = file.read(&mut buf)?;
        if count == 0 {
            break;
        }
        hash.write(&buf[..count]);
    }
    Ok(hash.finish())
}

fn publish_new(source: &Path, target: &Path) -> io::Result<()> {
    // Both paths are on the same volume. A hard link publishes a complete file
    // without replacing an independently created destination.
    match fs::hard_link(source, target) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Err(e),
        Err(_) => {
            let mut input = File::open(source)?;
            let mut output = File::create_new(target)?;
            if let Err(e) = io::copy(&mut input, &mut output).and_then(|_| output.sync_all()) {
                drop(output);
                let _ = fs::remove_file(target);
                return Err(e);
            }
            Ok(())
        }
    }
}
