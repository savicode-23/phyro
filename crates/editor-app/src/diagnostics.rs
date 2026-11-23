//! Local, user-reviewable crash reports. The interactive path only updates memory.
mod abnormal;
mod platform;
mod progress;
#[cfg(windows)]
pub(crate) mod windows_hang;
use platform::{module_offsets, os_version, Graphics};
use progress::{Checkpoint, UiProgress};

use std::backtrace::Backtrace;
use std::ffi::OsStr;
use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const HISTORY: usize = 20;
const MAX_REPORTS: usize = 32;
const MAX_BYTES: usize = 32 * 1024;
static NEXT_FILE: AtomicU64 = AtomicU64::new(1);

// Only these fixed labels can appear in breadcrumbs, including recovered session markers.
macro_rules! categories {
    ($name:ident { $($variant:ident),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        #[repr(u8)]
        pub(crate) enum $name { $($variant),+ }
        impl $name {
            const ALL: &'static [Self] = &[$(Self::$variant),+];
            fn from_number(n: u64) -> Option<Self> {
                Self::ALL.get(usize::try_from(n).ok()?).copied()
            }
        }
    };
}
categories!(Action {
    Launch,
    Open,
    Save,
    Draw,
    Select,
    Transform,
    EditPath,
    EditText,
    Paint,
    Timeline,
    Playback,
    Rig,
    Library,
    Undo,
    Redo,
    Export,
    View,
    Move,
    Resize,
    Rotate,
    Pan,
    Zoom,
    Align,
    Duplicate,
    Union,
    Difference,
    Intersection,
    Exclusion,
    Division,
    CutPath,
    Stack,
    SamplePaint
});
categories!(Tool {
    Select,
    Node,
    Rectangle,
    Ellipse,
    Pen,
    Text,
    Eyedropper,
    Pencil,
    Gradient
});
categories!(WindowState {
    Focused,
    Background,
    Minimized
});
categories!(UpdatePhase {
    Commands,
    Timeline,
    Panels,
    Canvas,
    Dialogs,
    History,
    NativeDialog
});

// Every native chooser in this editor runs on its one GUI thread. Each
// editor process has its own counter, including if several editors are open.
static OPEN_NATIVE_DIALOGS: AtomicU64 = AtomicU64::new(0);

pub(crate) fn native_dialog<T>(show: impl FnOnce() -> T) -> T {
    struct OpenDialog;
    impl Drop for OpenDialog {
        fn drop(&mut self) {
            OPEN_NATIVE_DIALOGS.fetch_sub(1, Relaxed);
        }
    }
    OPEN_NATIVE_DIALOGS.fetch_add(1, Relaxed);
    let _dialog = OpenDialog;
    show()
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Context {
    pub clip_index: usize,
    pub tick: u32,
    pub tool: Tool,
    pub layer_count: usize,
    pub object_count: usize,
    pub selection_count: usize,
    pub page_size: [u32; 2],
    pub pixels_per_point: f32,
    pub window: Option<WindowState>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Report {
    pub filename: String,
    pub timestamp: SystemTime,
}

impl Report {
    pub fn timestamp_utc(&self) -> String {
        utc(self.timestamp)
    }
    pub fn summary(&self) -> &'static str {
        match self.filename.rsplit('-').next() {
            Some("panic.txt") => "Main thread panic",
            Some("worker.txt") => "Background thread panic",
            Some("startup.txt") => "Startup failure",
            _ => "Unexpected exit",
        }
    }
}

struct State {
    directory: PathBuf,
    build: &'static str,
    main: std::thread::ThreadId,
    started: Instant,
    actions: [AtomicU64; HISTORY],
    sequence: AtomicU64,
    worker_reports: AtomicU64,
    context: Mutex<Option<Context>>,
    generation: AtomicU64,
    ui: UiProgress,
    graphics: OnceLock<String>,
    session: OnceLock<abnormal::Session>,
}

#[derive(Clone)]
pub(crate) struct Diagnostics(Arc<State>, Option<Arc<SessionOwner>>);

pub(crate) struct EditorUpdate(Arc<State>);

impl Drop for EditorUpdate {
    fn drop(&mut self) {
        self.0.ui.finish(self.0.started.elapsed());
    }
}

struct SessionOwner(Weak<State>);
impl Drop for SessionOwner {
    fn drop(&mut self) {
        if let Some(state) = self.0.upgrade() {
            if let Some(session) = state.session.get() {
                session.close();
            }
        }
    }
}

impl Diagnostics {
    pub fn in_dir(directory: PathBuf, build: &'static str) -> io::Result<Self> {
        fs::create_dir_all(&directory)?;
        let this = Self(
            Arc::new(State {
                directory,
                build,
                main: std::thread::current().id(),
                started: Instant::now(),
                actions: std::array::from_fn(|_| AtomicU64::new(0)),
                sequence: AtomicU64::new(0),
                worker_reports: AtomicU64::new(0),
                context: Mutex::new(None),
                generation: AtomicU64::new(1),
                ui: UiProgress::default(),
                graphics: OnceLock::new(),
                session: OnceLock::new(),
            }),
            None,
        );
        this.prune();
        Ok(this)
    }

    pub fn install(build: &'static str) -> Option<Self> {
        let mut this = directories().into_iter().find_map(|dir| {
            let diagnostics = Self::in_dir(dir, build).ok()?;
            // Creating an existing directory succeeds even when it is read-only.
            // Check that we can actually publish here before choosing this location.
            let probe = diagnostics.directory().join(format!(
                ".write-{}-{}",
                std::process::id(),
                NEXT_FILE.fetch_add(1, Relaxed)
            ));
            let probe_file = new_file(&probe).ok()?;
            drop(probe_file);
            fs::remove_file(probe).ok()?;
            Some(diagnostics)
        })?;
        if let Ok(session) = abnormal::Session::start(
            this.directory(),
            build,
            |key, when, build, context, graphics| this.recover(key, when, build, context, graphics),
        ) {
            let _ = this.0.session.set(session);
            this.1 = Some(Arc::new(SessionOwner(Arc::downgrade(&this.0))));
            let state = Arc::downgrade(&this.0);
            let _ = std::thread::Builder::new()
                .name("phyro-diagnostics".into())
                .spawn(move || {
                    let mut saved = 0;
                    let mut previous = None;
                    loop {
                        std::thread::sleep(Duration::from_secs(3));
                        let Some(state) = state.upgrade() else { break };
                        let generation = state.generation.load(Relaxed);
                        let checkpoint = state.ui.sample(state.started.elapsed());
                        if (generation != saved || checkpoint.stall_changed(previous))
                            && state.session.get().is_some_and(|s| {
                                s.update_context(&state.snapshot(checkpoint).encode())
                                    .is_ok()
                            })
                        {
                            saved = generation;
                            previous = Some(checkpoint);
                        }
                    }
                });
        }
        let state = Arc::downgrade(&this.0);
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if let Some(state) = state.upgrade() {
                let diagnostics = Self(state, None);
                let _ = diagnostics.capture(info);
            }
            previous(info);
        }));
        Some(this)
    }

    pub fn directory(&self) -> &Path {
        &self.0.directory
    }

    pub fn worker_reports(&self) -> u64 {
        self.0.worker_reports.load(Relaxed)
    }

    pub fn end_session(&self) {
        if let Some(session) = self.0.session.get() {
            session.close();
        }
    }

    pub fn editor_update(&self) -> EditorUpdate {
        self.0.ui.begin(self.0.started.elapsed());
        EditorUpdate(Arc::clone(&self.0))
    }

    pub fn phase(&self, phase: UpdatePhase) {
        self.0.ui.phase(phase);
    }

    pub fn note(&self, action: Action) {
        self.note_at(action, self.0.started.elapsed().as_secs());
    }

    fn note_at(&self, action: Action, seconds: u64) {
        let code = action as u64;
        let mut prior = self.0.sequence.load(Relaxed);
        let seq = loop {
            if prior != 0 && prior & 255 == code {
                break prior >> 8;
            }
            let next = (prior >> 8) + 1;
            match self
                .0
                .sequence
                .compare_exchange_weak(prior, (next << 8) | code, Relaxed, Relaxed)
            {
                Ok(_) => break next,
                Err(current) => prior = current,
            }
        };
        let seconds = seconds.min(0xff_ffff);
        let item = (seq << 32) | (seconds << 8) | code;
        let slot = &self.0.actions[(seq as usize) % HISTORY];
        if item > slot.load(Relaxed) && slot.fetch_max(item, Relaxed) < item {
            self.0.generation.fetch_add(1, Relaxed);
        }
    }

    pub fn context(&self, context: Context) -> bool {
        if let Ok(mut current) = self.0.context.try_lock() {
            if *current != Some(context) {
                *current = Some(context);
                self.0.generation.fetch_add(1, Relaxed);
            }
            true
        } else {
            false
        }
    }

    pub fn graphics(&self, vendor: &str, renderer: &str, version: &str) {
        self.0.graphics.get_or_init(|| {
            if let Some(session) = self.0.session.get() {
                let _ = session
                    .set_graphics(&Graphics::from_driver(vendor, renderer, version).encode());
            }
            format!(
                "GPU vendor: {}\nGPU renderer: {}\nOpenGL: {}\n",
                driver_field(vendor),
                driver_field(
                    renderer
                        .split_once("/PCIe/")
                        .map_or(renderer, |(model, _)| model)
                ),
                driver_field(version)
            )
        });
    }

    fn capture(&self, info: &std::panic::PanicHookInfo<'_>) -> io::Result<Report> {
        let thread = std::thread::current();
        let main = thread.id() == self.0.main;
        let code = if main { "panic" } else { "worker" };
        let role = match thread.name() {
            _ if main => "main",
            Some("phyro-zoom") => "canvas refinement",
            Some("phyro-pasteboard") => "pasteboard rendering",
            Some("sprite-export") => "sprite export",
            Some("sheet-preview") => "sprite sheet preview",
            _ => "background",
        };
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| info.payload().downcast_ref::<String>().map(String::as_str));
        let mut detail = format!("Thread: {role}\nFailure: {}\n", panic_class(message));
        if let Some(loc) = info.location() {
            let _ = writeln!(
                detail,
                "Source: {}:{}:{}",
                source_name(loc.file()),
                loc.line(),
                loc.column()
            );
        }
        detail.push_str(&module_offsets());
        detail.push_str("Stack:\n");
        detail.push_str(&safe_trace(&Backtrace::force_capture().to_string()));
        let report = self.publish_new(code, &detail)?;
        if main {
            if let Some(session) = self.0.session.get() {
                session.mark_panic();
            }
        } else {
            self.0.worker_reports.fetch_add(1, Relaxed);
        }
        Ok(report)
    }

    pub fn record_startup_failure(
        &self,
        class: &'static str,
        native_code: Option<i64>,
    ) -> io::Result<Report> {
        let class = match class {
            "renderer" | "window" | "graphics" | "graphics-config" | "event-loop" => class,
            _ => "startup",
        };
        let mut detail = format!("Failure: {class} initialization failed\n");
        if let Some(code) = native_code {
            let _ = writeln!(detail, "Native error code: {code}");
        }
        self.publish_new("startup", &detail)
    }

    fn publish_new(&self, kind: &str, detail: &str) -> io::Result<Report> {
        let now = SystemTime::now();
        let sequence = NEXT_FILE.fetch_add(1, Relaxed);
        let name = format!(
            "crash-{}-{}-{sequence}-{kind}.txt",
            millis(now),
            std::process::id()
        );
        let report = report_named(&name).ok_or_else(invalid_report)?;
        let mut body = header(&report, self.0.build);
        let _ = writeln!(body, "OS version: {}", os_version());
        if let Some(graphics) = self.0.graphics.get() {
            body.push_str(graphics);
        }
        let checkpoint = self.0.ui.sample(self.0.started.elapsed());
        body.push_str(&self.0.snapshot(checkpoint).describe());
        body.push_str(detail);
        self.publish(&report, &body)?;
        Ok(report)
    }

    fn recover(
        &self,
        key: &str,
        when: SystemTime,
        build: &str,
        context: &str,
        graphics: &str,
    ) -> io::Result<()> {
        let filename = format!("crash-{}-{key}-exit.txt", millis(when));
        let report = report_named(&filename).ok_or_else(invalid_report)?;
        let mut body = format!(
            "Phyro crash report\n{}\nSession start (UTC): {}\nPrevious build: {}\nRecovered by Phyro {} (build {})\nRecovery platform: {} {} ({})\n",
            report.summary(), report.timestamp_utc(), driver_field(build), env!("CARGO_PKG_VERSION"),
            driver_field(self.0.build), std::env::consts::OS, os_version(), env!("PHYRO_TARGET")
        );
        body.push_str("The previous session ended without a clean shutdown. The timestamp is the session start. A cause or stack was unavailable.\n");
        if let Some(snapshot) = Snapshot::decode(context) {
            if let Some(at) = snapshot
                .checkpoint
                .and_then(|sample| when.checked_add(Duration::from_secs(u64::from(sample.elapsed))))
            {
                let _ = writeln!(body, "Last diagnostic checkpoint (UTC): {}", utc(at));
            }
            body.push_str(&snapshot.describe());
        }
        if let Some(gpu) = Graphics::decode(graphics) {
            body.push_str(&gpu.describe());
        }
        self.publish(&report, &body)
    }

    fn publish(&self, report: &Report, body: &str) -> io::Result<()> {
        let final_path = self.directory().join(&report.filename);
        if final_path.is_file() {
            return Ok(());
        }
        let part = self.directory().join(format!(
            ".report-{}-{}.tmp",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Relaxed)
        ));
        let mut file = new_file(&part)?;
        let result = (|| {
            file.write_all(body.as_bytes())?;
            file.sync_all()?;
            drop(file);
            fs::rename(&part, final_path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(part);
        }
        result?;
        self.prune();
        Ok(())
    }

    pub fn reports(&self) -> io::Result<Vec<Report>> {
        let mut found = Vec::new();
        for entry in fs::read_dir(self.directory())? {
            let entry = entry?;
            if let Some(r) = entry.file_name().to_str().and_then(report_named) {
                match entry.file_type() {
                    Ok(kind) if kind.is_file() => found.push(r),
                    Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                    _ => {}
                }
            }
        }
        found.sort_unstable_by(|a, b| {
            b.timestamp
                .cmp(&a.timestamp)
                .then_with(|| b.filename.cmp(&a.filename))
        });
        Ok(found)
    }

    pub fn pending(&self) -> io::Result<Vec<Report>> {
        let mut reports = self.reports()?;
        reports.retain(|r| !self.seen(r).is_file());
        Ok(reports)
    }

    fn seen(&self, report: &Report) -> PathBuf {
        self.directory().join(format!("{}.seen", report.filename))
    }

    fn checked(&self, report: &Report) -> io::Result<PathBuf> {
        if report_named(&report.filename).as_ref() != Some(report) {
            return Err(invalid_report());
        }
        let path = self.directory().join(&report.filename);
        if !fs::symlink_metadata(&path)?.file_type().is_file() {
            return Err(invalid_report());
        }
        Ok(path)
    }

    pub fn read(&self, report: &Report) -> io::Result<String> {
        let mut body = String::new();
        File::open(self.checked(report)?)?
            .take(MAX_BYTES as u64 + 1)
            .read_to_string(&mut body)?;
        if body.len() > MAX_BYTES || report_body(&body).is_none() {
            return Err(invalid_report());
        }
        Ok(body)
    }

    pub fn acknowledge(&self, report: &Report) -> io::Result<()> {
        self.checked(report)?;
        match new_file(&self.seen(report)) {
            Ok(_) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(()),
            Err(e) => Err(e),
        }
    }

    fn prune(&self) {
        if let Ok(reports) = self.reports() {
            for r in reports.into_iter().skip(MAX_REPORTS) {
                let _ = fs::remove_file(self.directory().join(&r.filename));
                let _ = fs::remove_file(self.seen(&r));
            }
        }
    }
}

impl State {
    fn snapshot(&self, checkpoint: Checkpoint) -> Snapshot {
        let mut actions: Vec<_> = self
            .actions
            .iter()
            .map(|s| s.load(Relaxed))
            .filter(|&n| n != 0)
            .collect();
        actions.sort_unstable();
        Snapshot {
            checkpoint: Some(checkpoint),
            context: self.context.try_lock().ok().and_then(|c| *c),
            actions: actions
                .into_iter()
                .filter_map(|n| {
                    Some((Action::from_number(n & 255)?, ((n >> 8) & 0xff_ffff) as u32))
                })
                .collect(),
        }
    }
}

struct Snapshot {
    checkpoint: Option<Checkpoint>,
    context: Option<Context>,
    actions: Vec<(Action, u32)>,
}
impl Snapshot {
    // The separate session marker accepts numeric data only. Decode it back
    // into typed fields before including anything in a shareable report.
    fn encode(&self) -> String {
        let mut out = String::new();
        if let Some(c) = self.context {
            let _ = write!(
                out,
                "{} {} {} {} {} {} {} {} {}",
                c.clip_index,
                c.tick,
                c.tool as u8,
                c.layer_count,
                c.object_count,
                c.selection_count,
                c.page_size[0],
                c.page_size[1],
                (c.pixels_per_point.clamp(0., 100.) * 100.).round() as u32
            );
            if let Some(window) = c.window {
                let _ = write!(out, " {}", window as u8);
            }
        }
        out.push(':');
        for (a, seconds) in &self.actions {
            let _ = write!(out, "{},{seconds} ", *a as u8);
        }
        if let Some(checkpoint) = self.checkpoint {
            checkpoint.encode(&mut out);
        }
        out
    }
    fn decode(text: &str) -> Option<Self> {
        let (context, actions) = text.split_once(':')?;
        let (actions, checkpoint) = match actions.split_once(':') {
            Some((actions, fields)) => (actions, Some(Checkpoint::decode(fields)?)),
            None => (actions, None),
        };
        let context = if context.is_empty() {
            None
        } else {
            let values: Vec<u64> = context
                .split_ascii_whitespace()
                .map(str::parse)
                .collect::<Result<_, _>>()
                .ok()?;
            let [clip, tick, tool, layers, objects, selected, w, h, dpi, extra @ ..] =
                values.as_slice()
            else {
                return None;
            };
            let window = match extra {
                [] => None,
                [w] => Some(WindowState::from_number(*w)?),
                _ => return None,
            };
            Some(Context {
                clip_index: (*clip).try_into().ok()?,
                tick: (*tick).try_into().ok()?,
                tool: Tool::from_number(*tool)?,
                layer_count: (*layers).try_into().ok()?,
                object_count: (*objects).try_into().ok()?,
                selection_count: (*selected).try_into().ok()?,
                page_size: [(*w).try_into().ok()?, (*h).try_into().ok()?],
                pixels_per_point: (*dpi).min(10_000) as f32 / 100.,
                window,
            })
        };
        let actions: Vec<_> = actions
            .split_ascii_whitespace()
            .map(|s| {
                let (code, time) = s.split_once(',')?;
                Some((
                    Action::from_number(code.parse().ok()?)?,
                    time.parse::<u32>().ok()?,
                ))
            })
            .collect::<Option<_>>()?;
        (actions.len() <= HISTORY).then_some(Self {
            context,
            actions,
            checkpoint,
        })
    }
    fn describe(&self) -> String {
        let mut out = String::new();
        if let Some(c) = self.context {
            let _ = writeln!(out,"Clip index: {}\nTick (zero-based): {}\nTool: {:?}\nLayers: {}\nObjects: {}\nSelected objects: {}\nPage size: {} x {} px\nDisplay pixels per point: {:.2}",
                c.clip_index,c.tick,c.tool,c.layer_count,c.object_count,c.selection_count,c.page_size[0],c.page_size[1],c.pixels_per_point);
            if let Some(window) = c.window {
                let _ = writeln!(out, "Window at last UI observation: {window:?}");
            }
        }
        if let Some(checkpoint) = self.checkpoint {
            checkpoint.describe(&mut out);
        }
        out.push_str("Recent actions (seconds since launch):");
        for (a, seconds) in &self.actions {
            let _ = write!(out, " {a:?}(+{seconds}s)");
        }
        out.push('\n');
        out
    }
}

fn header(report: &Report, build: &str) -> String {
    format!("Phyro crash report\n{}\nTime (UTC): {}\nPhyro: {}\nBuild: {}\nTarget: {}\nPlatform: {} {} ({}-bit)\n",
        report.summary(),report.timestamp_utc(),env!("CARGO_PKG_VERSION"),driver_field(build),
        env!("PHYRO_TARGET"),std::env::consts::OS,std::env::consts::ARCH,usize::BITS)
}

fn new_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

fn invalid_report() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "Invalid crash report.")
}

fn report_body(body: &str) -> Option<&str> {
    body.strip_prefix("Phyro crash report\n")
        .or_else(|| body.strip_prefix("Loopline crash report\n"))
}

fn report_named(name: &str) -> Option<Report> {
    if name.len() > 200 {
        return None;
    }
    let body = name.strip_prefix("crash-")?.strip_suffix(".txt")?;
    let (number, rest) = body.split_once('-')?;
    let (identity, kind) = rest.rsplit_once('-')?;
    if !matches!(kind, "panic" | "worker" | "startup" | "exit")
        || !identity
            .split('-')
            .all(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    if !number.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let timestamp = UNIX_EPOCH.checked_add(Duration::from_millis(number.parse().ok()?))?;
    Some(Report {
        filename: name.into(),
        timestamp,
    })
}

fn directories() -> Vec<PathBuf> {
    use crate::profile::data_path;
    let env = |key| {
        std::env::var_os(key)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
    };
    let mut paths = Vec::new();
    if cfg!(windows) {
        for name in ["LOCALAPPDATA", "APPDATA"] {
            paths.extend(env(name).map(|p| data_path(&p, "Phyro", "CrashReports")));
        }
        paths.extend(
            env("USERPROFILE")
                .map(|p| data_path(&p.join("AppData/Local"), "Phyro", "CrashReports")),
        );
        // Windows TEMP is normally private to the account.
        paths.extend(env("TEMP").map(|p| data_path(&p, "Phyro", "CrashReports")));
    } else {
        paths.extend(env("XDG_STATE_HOME").map(|p| data_path(&p, "phyro", "crash-reports")));
        paths.extend(env("HOME").map(|p| {
            if cfg!(target_os = "macos") {
                data_path(
                    &p.join("Library/Application Support"),
                    "Phyro",
                    "CrashReports",
                )
            } else {
                data_path(&p.join(".local/state"), "phyro", "crash-reports")
            }
        }));
        paths.extend(env("XDG_RUNTIME_DIR").map(|p| data_path(&p, "phyro", "crash-reports")));
    }
    paths
}

fn millis(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

pub(crate) fn utc(t: SystemTime) -> String {
    let seconds = millis(t) / 1000;
    // Gregorian date from elapsed days, using March as the start of each year.
    let days = seconds / 86400 + 719468;
    let era = days / 146097;
    let day = days % 146097;
    let year = (day - day / 1460 + day / 36524 - day / 146096) / 365;
    let doy = day - (365 * year + year / 4 - year / 100);
    let month = (5 * doy + 2) / 153;
    let d = doy - (153 * month + 2) / 5 + 1;
    let m = if month < 10 { month + 3 } else { month - 9 };
    let y = year + era * 400 + u64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02} UTC",
        seconds / 3600 % 24,
        seconds / 60 % 60,
        seconds % 60
    )
}

fn driver_field(s: &str) -> &str {
    let value = s.trim();
    if value.is_empty()
        || value.len() > 256
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b" .,_-+()[]".contains(&b))
    {
        "unavailable"
    } else {
        value
    }
}

fn source_name(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let repo = normalized.split("crates/").last().filter(|p| {
        p.starts_with("editor-app/src/")
            || p.starts_with("renderer/src/")
            || p.starts_with("document-core/src/")
            || p.starts_with("project-io/src/")
    });
    let name = repo
        .or_else(|| Path::new(&normalized).file_name().and_then(OsStr::to_str))
        .unwrap_or("unknown");
    if name.len() <= 180
        && name.ends_with(".rs")
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/._-".contains(&b))
    {
        name.into()
    } else {
        "unknown".into()
    }
}

fn panic_class(value: Option<&str>) -> String {
    let value = value.unwrap_or("");
    const BOUNDS: &str = "index out of bounds: the len is ";
    if let Some((len, index)) = value
        .strip_prefix(BOUNDS)
        .and_then(|s| s.split_once(" but the index is "))
    {
        if let (Ok(len), Ok(index)) = (len.parse::<usize>(), index.parse::<usize>()) {
            return format!("Index out of bounds (length {len}, index {index})");
        }
    }
    for (prefix, category) in [
        (
            "called `Option::unwrap()` on a `None` value",
            "Missing optional value",
        ),
        (
            "called `Result::unwrap()` on an `Err` value",
            "Unexpected error result",
        ),
        ("assertion", "Assertion failed"),
        ("attempt to divide by zero", "Division by zero"),
        (
            "attempt to calculate the remainder with a divisor of zero",
            "Division by zero",
        ),
        ("attempt to add with overflow", "Arithmetic overflow"),
        ("attempt to subtract with overflow", "Arithmetic overflow"),
        ("attempt to multiply with overflow", "Arithmetic overflow"),
        ("range end index", "Invalid slice range"),
        ("range start index", "Invalid slice range"),
    ] {
        if value.starts_with(prefix) {
            return category.into();
        }
    }
    "Rust panic (message omitted for privacy)".into()
}

fn safe_trace(raw: &str) -> String {
    let mut out = String::new();
    let mut include_source = false;
    for line in raw.lines() {
        if out.len() >= 10_000 {
            break;
        }
        if let Some(at) = line.trim().strip_prefix("at ") {
            if include_source {
                if let Some((file, line_col)) = at.rsplit_once(".rs:") {
                    if line_col.len() <= 24
                        && line_col.bytes().all(|b| b.is_ascii_digit() || b == b':')
                    {
                        let source = source_name(&format!("{file}.rs"));
                        let _ = writeln!(out, "      at {source}:{line_col}");
                    }
                }
            }
            continue;
        }
        include_source = false;
        let Some((index, symbol)) = line.trim().split_once(':') else {
            continue;
        };
        let symbol = symbol.trim();
        if index.parse::<u16>().is_err()
            || symbol.is_empty()
            || symbol.len() > 280
            || symbol.contains(['/', '\\', '@'])
            || symbol.chars().any(char::is_control)
        {
            continue;
        }
        if ![
            "phyro::",
            "renderer::",
            "project_io::",
            "document_core::",
            "egui::",
            "eframe::",
            "epaint::",
            "tiny_skia::",
            "std::",
            "core::",
            "alloc::",
            "__rustc",
            "rust_begin_unwind",
        ]
        .iter()
        .any(|s| symbol.contains(s))
        {
            continue;
        }
        let _ = writeln!(out, "  {index}: {symbol}");
        include_source = true;
    }
    if out.is_empty() {
        out.push_str("  Symbol names unavailable in this build.\n");
    }
    out
}

#[cfg(test)]
mod tests;
