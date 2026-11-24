use super::{Claim, Lease};
use document_core::AnimationProject;
#[cfg(test)]
use std::sync::mpsc;
use std::{
    io,
    path::PathBuf,
    sync::{Arc, Condvar, Mutex},
    thread,
    time::{Duration, Instant},
};

pub(crate) struct Writer {
    shared: Arc<Shared>,
    started: bool,
}

struct Shared {
    root: PathBuf,
    state: Mutex<State>,
    // Reading an error never waits for the filesystem work under the state lock.
    notice: Mutex<Option<String>>,
    wake: Condvar,
    #[cfg(test)]
    pause_publication: Mutex<Option<PublicationPause>>,
}

#[cfg(test)]
struct PublicationPause {
    staged: mpsc::Sender<()>,
    resume: mpsc::Receiver<()>,
}

#[derive(Default)]
struct State {
    generation: u64,
    active: Option<Arc<Lease>>,
    pending: Option<Request>,
    busy: bool,
    closed: bool,
    retry: Option<Instant>,
    failures: u32,
}

struct Request {
    source: Option<PathBuf>,
    project: AnimationProject,
}

impl Writer {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self {
            shared: Arc::new(Shared {
                root,
                state: Mutex::default(),
                notice: Mutex::default(),
                wake: Condvar::new(),
                #[cfg(test)]
                pause_publication: Mutex::default(),
            }),
            started: false,
        }
    }

    #[cfg(test)]
    pub(crate) fn checkpoint(
        &mut self,
        source: Option<PathBuf>,
        project: AnimationProject,
    ) -> io::Result<()> {
        self.checkpoint_notifying(source, project, || {})
    }

    pub(crate) fn checkpoint_notifying(
        &mut self,
        source: Option<PathBuf>,
        project: AnimationProject,
        notify_failure: impl Fn() + Send + 'static,
    ) -> io::Result<()> {
        if !self.started {
            let shared = self.shared.clone();
            thread::Builder::new()
                .name("recovery writer".into())
                .spawn(move || shared.run(notify_failure))?;
            self.started = true;
        }
        self.shared.state.lock().unwrap().pending = Some(Request { source, project });
        self.shared.wake.notify_all();
        Ok(())
    }

    /// The active generation changes before cleanup, including on a filesystem
    /// error. An obsolete serialization can leave only a private staging file.
    pub(crate) fn retire(&mut self) -> io::Result<()> {
        let lease = {
            let mut state = self.shared.state.lock().unwrap();
            state.generation = state.generation.wrapping_add(1);
            state.pending = None;
            state.retry = None;
            state.failures = 0;
            *self.shared.notice.lock().unwrap() = None;
            state.active.take()
        };
        self.shared.wake.notify_all();
        lease.map_or(Ok(()), |lease| lease.retire())
    }

    pub(crate) fn adopt(&mut self, claim: Claim) -> io::Result<()> {
        let cleanup = self.retire();
        self.shared.state.lock().unwrap().active = Some(claim.lease);
        cleanup
    }

    pub(crate) fn take_error(&self) -> Option<String> {
        self.shared.notice.lock().unwrap().take()
    }

    #[cfg(test)]
    pub(crate) fn pause_next_publication(&self) -> (mpsc::Receiver<()>, mpsc::Sender<()>) {
        let (finished, staged) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        *self.shared.pause_publication.lock().unwrap() = Some(PublicationPause {
            staged: finished,
            resume,
        });
        (staged, release)
    }

    #[cfg(test)]
    pub(crate) fn writing(&self) -> bool {
        self.shared.state.lock().unwrap().busy
    }

    #[cfg(test)]
    pub(crate) fn wait_written(&self) -> io::Result<()> {
        let state = self.shared.state.lock().unwrap();
        let (state, timeout) = self
            .shared
            .wake
            .wait_timeout_while(state, Duration::from_secs(30), |s| {
                (s.busy || s.pending.is_some()) && s.retry.is_none()
            })
            .unwrap();
        if state.retry.is_some() {
            Err(io::Error::other(
                self.shared
                    .notice
                    .lock()
                    .unwrap()
                    .clone()
                    .unwrap_or_else(|| "Checkpoint failed".into()),
            ))
        } else if timeout.timed_out() {
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Checkpoint worker did not finish",
            ))
        } else {
            Ok(())
        }
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        // Only an explicit retirement discards a valid checkpoint. In
        // particular, a panic unwinding the editor must preserve its draft.
        let mut state = self.shared.state.lock().unwrap();
        state.closed = true;
        state.active = None;
        state.pending = None;
        self.shared.wake.notify_all();
    }
}

impl Shared {
    fn run(&self, notify_failure: impl Fn()) {
        loop {
            let (generation, request) = {
                let mut state = self.state.lock().unwrap();
                loop {
                    if state.closed {
                        return;
                    }
                    if let Some(delay) = state
                        .retry
                        .and_then(|t| t.checked_duration_since(Instant::now()))
                    {
                        state = self.wake.wait_timeout(state, delay).unwrap().0;
                    } else if let Some(request) = state.pending.take() {
                        state.busy = true;
                        break (state.generation, request);
                    } else {
                        state = self.wake.wait(state).unwrap();
                    }
                }
            };
            let result = self.write(generation, &request);
            let mut state = self.state.lock().unwrap();
            state.busy = false;
            let mut new_failure = false;
            if !state.closed && state.generation == generation {
                match result {
                    Ok(()) => {
                        state.failures = 0;
                        state.retry = None;
                    }
                    Err(error) => {
                        if state.failures == 0 {
                            *self.notice.lock().unwrap() =
                                Some(format!("Could not checkpoint the recovery draft. {error}"));
                            new_failure = true;
                        }
                        state.failures = state.failures.saturating_add(1);
                        let seconds = 1 << state.failures.min(6);
                        state.retry = Some(Instant::now() + Duration::from_secs(seconds));
                        state.pending.get_or_insert(request);
                    }
                }
            }
            self.wake.notify_all();
            drop(state);
            if new_failure {
                notify_failure();
            }
        }
    }

    fn write(&self, generation: u64, request: &Request) -> io::Result<()> {
        let active = {
            let state = self.state.lock().unwrap();
            if state.closed || state.generation != generation {
                return Ok(());
            }
            state.active.clone()
        };
        let lease = match active {
            Some(lease) => lease,
            None => {
                let created = Arc::new(Lease::create(&self.root, request.source.as_deref())?);
                let mut state = self.state.lock().unwrap();
                if state.closed || state.generation != generation {
                    return Ok(());
                }
                state.active = Some(created.clone());
                created
            }
        };
        lease.stage(&request.project)?;
        #[cfg(test)]
        if let Some(pause) = self.pause_publication.lock().unwrap().take() {
            let _ = pause.staged.send(());
            let _ = pause.resume.recv_timeout(Duration::from_secs(60));
        }
        self.publish_if_current(generation, &lease)
    }

    fn publish_if_current(&self, generation: u64, lease: &Lease) -> io::Result<()> {
        let state = self.state.lock().unwrap();
        if state.closed || state.generation != generation {
            return Ok(());
        }
        lease.publish()?;
        drop(state);
        super::sync_directory(&lease.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retiring_a_generation_while_serializing_never_republishes_it() {
        let root = tempfile::tempdir().unwrap();
        let mut writer = Writer::new(root.path().to_owned());
        writer
            .checkpoint(None, AnimationProject::default())
            .unwrap();
        writer.wait_written().unwrap();
        let (staged, release) = writer.pause_next_publication();
        writer
            .checkpoint(None, AnimationProject::default())
            .unwrap();
        staged.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(writer.writing());
        writer.retire().unwrap();
        release.send(()).unwrap();
        writer.wait_written().unwrap();
        drop(writer);
        assert!(super::super::discover(root.path()).unwrap().is_empty());
    }
}
