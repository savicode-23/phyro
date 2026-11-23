//! The editor submits just the visible cells. A single worker reads directory
//! entries, checks visible files for changes and renders one thumbnail at a time.
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Condvar, Mutex,
    },
    time::{Duration, Instant, SystemTime},
};

mod assets;
#[cfg(test)]
mod tests;

const MAX_REQUESTS: usize = 96;
const MAX_READY: usize = 8;
const CHECK_INTERVAL: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub(super) enum Kind {
    Folder,
    Project,
    Svg,
    Image,
}

impl Kind {
    fn file(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?;
        if ext.eq_ignore_ascii_case("vanim") {
            Some(Self::Project)
        } else if ext.eq_ignore_ascii_case("svg") {
            Some(Self::Svg)
        } else if ["png", "jpg", "jpeg"]
            .iter()
            .any(|s| ext.eq_ignore_ascii_case(s))
        {
            Some(Self::Image)
        } else {
            None
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub kind: Kind,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct PreviewKey {
    path: PathBuf,
    kind: Kind,
    frame: Option<(u64, u32)>,
}

impl PreviewKey {
    pub(super) fn thumbnail(entry: &Entry) -> Self {
        Self {
            path: entry.path.clone(),
            kind: entry.kind,
            frame: None,
        }
    }
    pub(super) fn frame(entry: &Entry, clip: u64, frame: u32) -> Self {
        Self {
            frame: Some((clip, frame)),
            ..Self::thumbnail(entry)
        }
    }
    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct ClipInfo {
    pub id: u64,
    pub name: String,
    pub frames: u32,
    pub fps: u32,
}

pub(super) struct Preview {
    pub image: egui::ColorImage,
    pub clips: Vec<ClipInfo>,
}

#[derive(Clone, Copy, Eq, PartialEq, Hash)]
struct Stamp {
    bytes: u64,
    modified: Option<SystemTime>,
}

fn stamp(path: &Path) -> Option<Stamp> {
    let m = fs::metadata(path).ok()?;
    m.is_file().then_some(Stamp {
        bytes: m.len(),
        modified: m.modified().ok(),
    })
}

#[derive(Default)]
struct State {
    quit: bool,
    paused: bool,
    scan: Option<PathBuf>,
    retired: Vec<Vec<Entry>>,
    wanted: Arc<[PreviewKey]>,
    started: HashSet<PreviewKey>,
    watch: Arc<[PathBuf]>,
    listing: Option<Result<Vec<Entry>, String>>,
    ready: VecDeque<(PreviewKey, Result<Preview, String>)>,
    changed: Vec<PathBuf>,
}

impl State {
    fn source_changed(&mut self, path: &Path) {
        self.started.retain(|k| k.path != path);
        self.ready.retain(|(k, _)| k.path != path);
        if !self.changed.iter().any(|p| p == path) && self.changed.len() < MAX_REQUESTS {
            self.changed.push(path.to_owned());
        }
    }
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    wake: Condvar,
    generation: AtomicU64,
}

pub(super) struct Backend {
    shared: Arc<Shared>,
    paused: bool,
    wanted: Arc<[PreviewKey]>,
    watch: Arc<[PathBuf]>,
    started: bool,
}

impl Backend {
    pub(super) fn new(ctx: &egui::Context) -> Self {
        Self::start(ctx, assets::cache_dir())
    }
    #[cfg(test)]
    pub(super) fn with_cache(ctx: &egui::Context, cache: PathBuf) -> Self {
        Self::start(ctx, Some(cache))
    }
    fn start(ctx: &egui::Context, cache: Option<PathBuf>) -> Self {
        let shared = Arc::<Shared>::default();
        let thread_shared = Arc::clone(&shared);
        let ctx = ctx.clone();
        let started = std::thread::Builder::new()
            .name("asset previews".into())
            .spawn(move || worker(&thread_shared, &ctx, cache))
            .is_ok();
        Self {
            shared,
            paused: false,
            wanted: Default::default(),
            watch: Default::default(),
            started,
        }
    }
    pub(super) fn navigate(&mut self, folder: PathBuf, old_entries: Vec<Entry>) {
        let mut state = self.shared.state.lock().unwrap();
        self.shared.generation.fetch_add(1, Ordering::Relaxed);
        // Releasing an index frees one path and one name per file. Hand that
        // work to the scanner, including results superseded before poll().
        let mut retired = std::mem::take(&mut state.retired);
        if let Some(Ok(index)) = state.listing.take() {
            retired.push(index);
        }
        if !old_entries.is_empty() {
            retired.push(old_entries);
        }
        *state = State {
            retired,
            ..State::default()
        };
        if self.started {
            state.scan = Some(folder);
        } else {
            state.listing = Some(Err("Could not start the asset browser worker.".into()));
        }
        self.wanted = Default::default();
        self.watch = Default::default();
        self.paused = false;
        self.shared.wake.notify_one();
    }
    pub(super) fn poll(&mut self) -> Option<Result<Vec<Entry>, String>> {
        self.shared.state.lock().unwrap().listing.take()
    }
    pub(super) fn take_preview(&mut self) -> Option<(PreviewKey, Result<Preview, String>)> {
        let mut state = self.shared.state.lock().unwrap();
        let item = state.ready.pop_front();
        if item.is_some() {
            self.shared.wake.notify_one();
        }
        item
    }
    pub(super) fn take_changed(&mut self) -> Option<PathBuf> {
        self.shared.state.lock().unwrap().changed.pop()
    }
    pub(super) fn watch(&mut self, paths: &[PathBuf]) {
        let paths = &paths[..paths.len().min(MAX_REQUESTS)];
        if self.watch.as_ref() == paths {
            return;
        }
        self.watch = Arc::from(paths);
        self.shared.state.lock().unwrap().watch = Arc::clone(&self.watch);
        self.shared.wake.notify_one();
    }
    pub(super) fn set_requests(&mut self, keys: &[PreviewKey], paused: bool) {
        let keys = &keys[..keys.len().min(MAX_REQUESTS)];
        if self.paused == paused && self.wanted.as_ref() == keys {
            return;
        }
        self.paused = paused;
        let mut state = self.shared.state.lock().unwrap();
        state.paused = paused;
        if self.wanted.as_ref() != keys {
            self.wanted = Arc::from(keys);
            state.wanted = Arc::clone(&self.wanted);
            state.started.retain(|k| keys.contains(k));
            state.ready.retain(|(k, _)| keys.contains(k));
        }
        self.shared.wake.notify_one();
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        let mut state = self.shared.state.lock().unwrap();
        state.quit = true;
        self.shared.generation.fetch_add(1, Ordering::Relaxed);
        self.shared.wake.notify_one();
        // Leave an in-flight filesystem read on its worker; closing a window
        // must never join a thread waiting for a slow or disconnected volume.
    }
}

enum Job {
    Scan(PathBuf, Vec<Vec<Entry>>),
    Preview(PreviewKey),
    Check(Arc<[PathBuf]>),
    ReleaseSource,
}

fn worker(shared: &Shared, ctx: &egui::Context, cache: Option<PathBuf>) {
    let mut previews = assets::Renderer::new(cache);
    let mut stamps = HashMap::new();
    let mut checked = Instant::now();
    loop {
        let mut state = shared.state.lock().unwrap();
        let job = loop {
            if state.quit {
                return;
            }
            if let Some(dir) = state.scan.take() {
                break Job::Scan(dir, std::mem::take(&mut state.retired));
            }
            if previews
                .selected_path()
                .is_some_and(|p| state.paused || !state.watch.iter().any(|w| w == p))
            {
                break Job::ReleaseSource;
            }
            if !state.paused {
                if !state.watch.is_empty() && checked.elapsed() >= CHECK_INTERVAL {
                    checked = Instant::now();
                    break Job::Check(Arc::clone(&state.watch));
                }
                if state.ready.len() < MAX_READY {
                    if let Some(key) = state
                        .wanted
                        .iter()
                        .find(|k| !state.started.contains(k))
                        .cloned()
                    {
                        state.started.insert(key.clone());
                        break Job::Preview(key);
                    }
                }
            }
            state = if state.paused || state.watch.is_empty() {
                shared.wake.wait(state).unwrap()
            } else {
                shared
                    .wake
                    .wait_timeout(state, CHECK_INTERVAL.saturating_sub(checked.elapsed()))
                    .unwrap()
                    .0
            };
        };
        let generation = shared.generation.load(Ordering::Relaxed);
        drop(state);
        let current = || shared.generation.load(Ordering::Relaxed) == generation;
        let resume = || {
            let guard = shared.state.lock().unwrap();
            let _guard = shared
                .wake
                .wait_while(guard, |s| s.paused && current())
                .unwrap();
            current()
        };
        match job {
            Job::ReleaseSource => previews.release(),
            Job::Scan(path, retired) => {
                drop(retired);
                previews.release();
                stamps.clear();
                let Some(result) = scan(&path, resume) else {
                    continue;
                };
                let mut state = shared.state.lock().unwrap();
                if current() {
                    state.listing = Some(result);
                    drop(state);
                    ctx.request_repaint();
                }
            }
            Job::Preview(key) => {
                let version = stamp(&key.path);
                let previous = stamps.get(&key.path).copied();
                if stamps.len() >= MAX_REQUESTS * 2 {
                    let visible = Arc::clone(&shared.state.lock().unwrap().watch);
                    stamps.retain(|p, _| visible.contains(p));
                }
                stamps.insert(key.path.clone(), version);
                // Playback may see a replacement before the periodic check.
                // Invalidate the poster, clip list and all poses together before
                // publishing anything decoded from the newer source.
                if previous.map_or(key.frame.is_some(), |last| last != version) {
                    let mut state = shared.state.lock().unwrap();
                    if current() {
                        state.source_changed(&key.path);
                        drop(state);
                        ctx.request_repaint();
                    }
                    continue;
                }
                let result = version
                    .ok_or_else(|| "File is unavailable.".to_owned())
                    .and_then(|s| previews.render(&key, s));
                let mut state = shared.state.lock().unwrap();
                if current() && state.wanted.contains(&key) {
                    state.ready.push_back((key, result));
                    let repaint = !state.paused;
                    drop(state);
                    if repaint {
                        ctx.request_repaint();
                    }
                }
            }
            Job::Check(paths) => {
                // Keep history bounded, including files visited during rapid scrolling.
                // Unknown stamps invalidate once when a cached tile comes back on screen.
                stamps.retain(|path, _| paths.contains(path));
                let mut changed = Vec::new();
                for path in paths.iter() {
                    if !resume() {
                        break;
                    }
                    let version = stamp(path);
                    if stamps.insert(path.clone(), version) != Some(version) {
                        changed.push(path.clone());
                    }
                }
                let mut state = shared.state.lock().unwrap();
                if current() && !changed.is_empty() {
                    for path in changed {
                        state.source_changed(&path);
                    }
                    drop(state);
                    ctx.request_repaint();
                }
            }
        }
    }
}

fn scan(path: &Path, current: impl Fn() -> bool) -> Option<Result<Vec<Entry>, String>> {
    let iter = match fs::read_dir(path) {
        Ok(d) => d,
        Err(e) => return Some(Err(e.to_string())),
    };
    let mut entries = Vec::new();
    for (i, item) in iter.enumerate() {
        if i % 32 == 0 && !current() {
            return None;
        }
        let Ok(item) = item else {
            continue;
        };
        let Ok(mut ty) = item.file_type() else {
            continue;
        };
        let path = item.path();
        if ty.is_symlink() {
            let Ok(target) = fs::metadata(&path) else {
                continue;
            };
            ty = target.file_type();
        }
        let kind = if ty.is_dir() {
            Kind::Folder
        } else if ty.is_file() {
            let Some(kind) = Kind::file(&path) else {
                continue;
            };
            kind
        } else {
            continue;
        };
        entries.push(Entry {
            kind,
            path,
            name: item.file_name().to_string_lossy().into_owned(),
        });
        if entries.len() > 200_000 {
            return Some(Err(
                "This folder has more than 200,000 assets. Choose a subfolder.".into(),
            ));
        }
    }
    if !current() {
        return None;
    }
    entries.sort_by_cached_key(|e| (e.kind != Kind::Folder, e.name.to_lowercase()));
    Some(Ok(entries))
}
