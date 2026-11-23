//! Private, independently locked native recovery drafts for desktop editor windows.
use document_core::AnimationProject;
use std::{
    ffi::OsString,
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::SystemTime,
};

mod writer;
pub(crate) use writer::Writer;
#[cfg(test)]
mod tests;

const PREFIX: &str = "draft-";
const ARCHIVE: &str = "draft.vanim";
const STAGED: &str = "pending.vanim";
const LOCK: &str = "owner";
const LABEL: &str = "source.json";
const SIGNATURE: &[u8] = b"Phyro recovery draft 1\n";
const LEGACY_SIGNATURE: &[u8] = b"Loopline recovery draft 1\n";

pub(crate) fn directory() -> Option<PathBuf> {
    let env = |key| {
        std::env::var_os(key)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
    };
    #[cfg(windows)]
    let base = env("LOCALAPPDATA").or_else(|| env("APPDATA"))?;
    #[cfg(target_os = "macos")]
    let base = env("HOME")?.join("Library/Application Support");
    #[cfg(not(any(windows, target_os = "macos")))]
    let base = env("XDG_STATE_HOME").or_else(|| env("HOME").map(|p| p.join(".local/state")))?;
    let (brand, component) = if cfg!(any(windows, target_os = "macos")) {
        ("Phyro", "Recovery")
    } else {
        ("phyro", "recovery")
    };
    Some(crate::profile::data_path(&base, brand, component))
}

#[derive(Clone, Debug)]
pub(crate) struct Draft {
    path: PathBuf,
    pub(crate) source: Option<PathBuf>,
    pub(crate) modified: SystemTime,
}

impl Draft {
    pub(crate) fn claim(&self) -> io::Result<Claim> {
        let lease = Lease::acquire(&self.path)?;
        lease.archive_modified()?;
        Ok(Claim {
            lease: Arc::new(lease),
            source: self.source.clone(),
        })
    }
}

/// Enumerating does not load any project data or keep other windows from
/// recovering a draft. Claim again before displaying or acting on a candidate.
pub(crate) fn discover(root: &Path) -> io::Result<Vec<Draft>> {
    let entries = match fs::read_dir(root) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(vec![]),
        entries => entries?,
    };
    let mut drafts = Vec::new();
    let mut first_error = None;
    for entry in entries {
        match entry.and_then(candidate) {
            Ok(Some(draft)) => drafts.push(draft),
            Ok(None) => {}
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    if drafts.is_empty() {
        if let Some(error) = first_error {
            return Err(error);
        }
    }
    drafts.sort_by(|a, b| {
        b.modified
            .cmp(&a.modified)
            .then_with(|| a.path.cmp(&b.path))
    });
    Ok(drafts)
}

fn candidate(entry: fs::DirEntry) -> io::Result<Option<Draft>> {
    if !entry.file_type()?.is_dir() || !entry.file_name().to_string_lossy().starts_with(PREFIX) {
        return Ok(None);
    }
    let path = entry.path();
    let lease = match Lease::acquire(&path) {
        Ok(lease) => lease,
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::WouldBlock | io::ErrorKind::InvalidData
            ) =>
        {
            return Ok(None)
        }
        Err(e) => return Err(e),
    };
    match lease.archive_modified() {
        Ok(modified) => {
            // A missing or damaged display label must not hide a valid archive.
            let label = path.join(LABEL);
            let source = fs::symlink_metadata(&label)
                .ok()
                .filter(|meta| meta.is_file())
                .and_then(|_| File::open(label).ok())
                .and_then(|f| {
                    serde_json::from_reader::<_, Option<OsString>>(f.take(256 * 1024)).ok()
                })
                .flatten()
                .map(PathBuf::from);
            Ok(Some(Draft {
                path,
                source,
                modified,
            }))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            // A dead writer may have stopped before its first publication.
            lease.cleanup.store(true, Ordering::Relaxed);
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

pub(crate) struct Claim {
    lease: Arc<Lease>,
    source: Option<PathBuf>,
}

impl Claim {
    pub(crate) fn source(&self) -> Option<&Path> {
        self.source.as_deref()
    }

    pub(crate) fn load(&self) -> io::Result<AnimationProject> {
        project_io::load_animation(&self.lease.path.join(ARCHIVE))
    }

    pub(crate) fn discard(&mut self) -> io::Result<()> {
        self.lease.retire()
    }
}

struct Lease {
    path: PathBuf,
    // Held until after the archive has been removed, including while a canceled
    // worker finishes writing its staging file in this directory.
    _owner: File,
    cleanup: AtomicBool,
}

impl Lease {
    fn create(root: &Path, source: Option<&Path>) -> io::Result<Self> {
        fs::create_dir_all(root)?;
        let mut builder = tempfile::Builder::new();
        builder.prefix(PREFIX);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            builder.permissions(fs::Permissions::from_mode(0o700));
        }
        let folder = builder.tempdir_in(root)?;
        // Publish the lock name only after acquiring it. A simultaneous
        // startup must never claim a newly created, still-empty directory.
        let initializing = folder.path().join("initializing");
        let mut lock = File::options()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&initializing)?;
        lock.try_lock()?;
        lock.write_all(SIGNATURE)?;
        lock.sync_all()?;
        fs::rename(initializing, folder.path().join(LOCK))?;
        let mut label = File::create(folder.path().join(LABEL))?;
        label.write_all(&serde_json::to_vec(&source.map(Path::as_os_str))?)?;
        label.sync_all()?;
        sync_directory(root)?;
        Ok(Self {
            path: folder.keep(),
            _owner: lock,
            cleanup: AtomicBool::new(true),
        })
    }

    fn acquire(path: &Path) -> io::Result<Self> {
        let marker = path.join(LOCK);
        if !fs::symlink_metadata(&marker)?.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid recovery owner marker",
            ));
        }
        let lock = File::options().read(true).write(true).open(marker)?;
        lock.try_lock()?;
        let mut signature = Vec::new();
        (&lock)
            .take(SIGNATURE.len().max(LEGACY_SIGNATURE.len()) as u64 + 1)
            .read_to_end(&mut signature)?;
        if signature != SIGNATURE && signature != LEGACY_SIGNATURE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Unrecognized recovery directory",
            ));
        }
        Ok(Self {
            path: path.to_owned(),
            _owner: lock,
            cleanup: AtomicBool::new(false),
        })
    }

    fn archive_modified(&self) -> io::Result<SystemTime> {
        let meta = fs::symlink_metadata(self.path.join(ARCHIVE))?;
        if !meta.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid recovery archive file",
            ));
        }
        meta.modified()
    }

    fn stage(&self, project: &AnimationProject) -> io::Result<()> {
        project_io::save_animation(&self.path.join(STAGED), project, true)
    }

    fn publish(&self) -> io::Result<()> {
        fs::rename(self.path.join(STAGED), self.path.join(ARCHIVE))?;
        self.cleanup.store(false, Ordering::Relaxed);
        Ok(())
    }

    fn retire(&self) -> io::Result<()> {
        match fs::remove_file(self.path.join(ARCHIVE)) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            result => result?,
        }
        self.cleanup.store(true, Ordering::Relaxed);
        sync_directory(&self.path)
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        if self.cleanup.load(Ordering::Relaxed) {
            // The lock and signature identify our folder. Still leave any
            // unfamiliar file in place. tempfile uses .tmp + six randomized
            // alphanumerics for its same-directory native archive staging.
            if let Ok(entries) = fs::read_dir(&self.path) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    let temporary = name.strip_prefix(".tmp").is_some_and(|suffix| {
                        suffix.len() == 6 && suffix.bytes().all(|c| c.is_ascii_alphanumeric())
                    });
                    if name == STAGED || name == LABEL || temporary {
                        let _ = fs::remove_file(entry.path());
                    }
                }
            }
            let _ = fs::remove_file(self.path.join(LOCK));
            let _ = fs::remove_dir(&self.path);
        }
    }
}

fn sync_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}
