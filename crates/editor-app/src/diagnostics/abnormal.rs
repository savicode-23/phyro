//! A locked marker identifies a running editor independently of its PID. The OS
//! releases the lock if the process disappears before normal shutdown.
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub(super) struct Session {
    path: PathBuf,
    marker: File,
    closed: AtomicBool,
}

static NEXT: AtomicU64 = AtomicU64::new(0);
const PREFIX: &str = "Phyro session 1\n";
const LEGACY_PREFIX: &str = "Loopline session 1\n";
const BUILD_LIMIT: usize = 64;
const CONTEXT_LIMIT: usize = 512;
const GRAPHICS_LIMIT: usize = 64;

impl Session {
    pub(super) fn start(
        reports: &Path,
        build: &str,
        mut on_unclean: impl FnMut(&str, SystemTime, &str, &str, &str) -> io::Result<()>,
    ) -> io::Result<Self> {
        let build = revision(build);
        let runs = reports.join("sessions");
        fs::create_dir_all(&runs)?;
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let (path, mut marker) = loop {
            let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = runs.join(format!("{millis}-{}-{sequence}.run", std::process::id()));
            match create(&path) {
                Ok(file) => break (path, file),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        };
        // A concurrently starting reader may briefly lock the still-empty file.
        // An empty file is never a recoverable marker. Publish its contents only
        // after obtaining the lock and keep that same handle for our lifetime.
        let initialize = (|| -> io::Result<()> {
            marker.lock()?;
            marker.write_all(PREFIX.as_bytes())?;
            marker.write_all(build.as_bytes())?;
            marker.sync_data()
        })();
        if let Err(error) = initialize {
            let _ = fs::remove_file(&path);
            return Err(error);
        }
        let session = Self {
            path,
            marker,
            closed: AtomicBool::new(false),
        };
        if let Ok(entries) = fs::read_dir(runs) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path != session.path
                    && path.extension().is_some_and(|ext| ext == "run")
                    && entry.file_type().is_ok_and(|kind| kind.is_file())
                {
                    let _ = recover(&path, &mut on_unclean);
                }
            }
        }
        Ok(session)
    }

    /// The background writer supplies only encoded numeric activity. Losing the
    /// final update leaves the last complete snapshot available for recovery.
    pub(super) fn update_context(&self, summary: &str) -> io::Result<()> {
        if !valid_context(summary) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid diagnostic activity",
            ));
        }
        self.write_sidecar("ctx", summary)
    }

    pub(super) fn set_graphics(&self, encoded: &str) -> io::Result<()> {
        if !valid_graphics(encoded) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid graphics identifiers",
            ));
        }
        self.write_sidecar("gpu", encoded)
    }

    fn write_sidecar(&self, extension: &str, contents: &str) -> io::Result<()> {
        if self.closed.load(Ordering::Acquire) {
            return Ok(());
        }
        let pending = self.path.with_extension(format!("{extension}.tmp"));
        let mut file = create(&pending)?;
        let path = self.path.with_extension(extension);
        let saved = (|| {
            file.write_all(contents.as_bytes())?;
            drop(file);
            fs::rename(&pending, &path)
        })();
        if saved.is_err() {
            let _ = fs::remove_file(pending);
        }
        if self.closed.load(Ordering::Acquire) {
            let _ = fs::remove_file(path);
        }
        saved
    }

    /// Call only after a report for a terminal panic has been published. A
    /// caught worker panic can leave the editor running for many more edits.
    pub(super) fn mark_panic(&self) {
        self.close();
    }

    /// Main can finish while the activity writer briefly holds another reference.
    /// Close the marker on the orderly exit path before returning to the OS.
    pub(super) fn close(&self) {
        if !self.closed.swap(true, Ordering::AcqRel) {
            discard(&self.path, &self.marker);
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            self.close();
        }
    }
}

fn create(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

fn revision(build: &str) -> &str {
    if (7..=BUILD_LIMIT).contains(&build.len()) && build.bytes().all(|b| b.is_ascii_hexdigit()) {
        build
    } else {
        "unknown"
    }
}

fn valid_context(context: &str) -> bool {
    context.len() <= CONTEXT_LIMIT
        && context.bytes().all(|b| {
            b.is_ascii_digit() || matches!(b, b' ' | b',' | b':' | b'=' | b'.' | b'\n' | b'-')
        })
}

fn valid_graphics(encoded: &str) -> bool {
    encoded.len() <= GRAPHICS_LIMIT && encoded.bytes().all(|b| b.is_ascii_digit() || b == b' ')
}

fn session_start(key: &str) -> Option<SystemTime> {
    let mut parts = key.split('-');
    let millis = parts.next()?.parse::<u64>().ok()?;
    let _pid = parts.next()?.parse::<u32>().ok()?;
    let _sequence = parts.next()?.parse::<u64>().ok()?;
    if parts.next().is_some() || !key.bytes().all(|b| b.is_ascii_digit() || b == b'-') {
        return None;
    }
    UNIX_EPOCH.checked_add(Duration::from_millis(millis))
}

fn read_limited(file: &File, limit: usize) -> io::Result<String> {
    let mut text = String::new();
    file.take(limit as u64 + 1).read_to_string(&mut text)?;
    Ok(text)
}

fn discard(path: &Path, marker: &File) {
    // Another scanner may already have the same file open and acquire its
    // lock after ours drops. Invalidate the contents before unlinking it.
    let _ = marker.set_len(0);
    let _ = fs::remove_file(path);
    for extension in ["ctx", "ctx.tmp", "gpu", "gpu.tmp"] {
        let _ = fs::remove_file(path.with_extension(extension));
    }
}

fn read_sidecar(path: PathBuf, limit: usize, valid: fn(&str) -> bool) -> String {
    fs::symlink_metadata(&path)
        .ok()
        .filter(|meta| meta.file_type().is_file())
        .and_then(|_| read_limited(&File::open(path).ok()?, limit).ok())
        .filter(|text| valid(text))
        .unwrap_or_default()
}

fn recover(
    path: &Path,
    on_unclean: &mut impl FnMut(&str, SystemTime, &str, &str, &str) -> io::Result<()>,
) -> io::Result<()> {
    let Some(key) = path.file_stem().and_then(|s| s.to_str()) else {
        return Ok(());
    };
    let Some(started) = session_start(key) else {
        return Ok(());
    };
    let marker = OpenOptions::new().read(true).write(true).open(path)?;
    if marker.try_lock().is_err() {
        // In use, or liveness could not be established on this filesystem.
        return Ok(());
    }
    let record = read_limited(&marker, PREFIX.len().max(LEGACY_PREFIX.len()) + BUILD_LIMIT)?;
    if record.is_empty() {
        return Ok(());
    }
    let Some(build) = record
        .strip_prefix(PREFIX)
        .or_else(|| record.strip_prefix(LEGACY_PREFIX))
    else {
        return Ok(());
    };
    let context = read_sidecar(path.with_extension("ctx"), CONTEXT_LIMIT, valid_context);
    let graphics = read_sidecar(path.with_extension("gpu"), GRAPHICS_LIMIT, valid_graphics);
    // A failed report write leaves the original marker for the next launch.
    on_unclean(key, started, revision(build), &context, &graphics)?;
    discard(path, &marker);
    Ok(())
}

#[cfg(test)]
mod tests;
