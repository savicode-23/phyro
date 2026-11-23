use super::UpdatePhase;
use std::fmt::Write;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering::Relaxed};
use std::time::Duration;

const SLOW_SECONDS: u32 = 3;

#[derive(Default)]
pub(super) struct UiProgress {
    // Milliseconds since launch plus one. Zero means outside an update or
    // that no update has completed. Idle event loops need no heartbeat.
    active: AtomicU64,
    completed: AtomicU64,
    phase: AtomicU8,
}

fn millis(time: Duration) -> u64 {
    time.as_millis().min(u128::from(u64::MAX - 1)) as u64
}

fn seconds(ms: u64) -> u32 {
    (ms / 1000).min(u64::from(u32::MAX)) as u32
}

impl UiProgress {
    pub(super) fn begin(&self, now: Duration) {
        self.phase(UpdatePhase::Commands);
        self.active.store(millis(now) + 1, Relaxed);
    }

    pub(super) fn phase(&self, phase: UpdatePhase) {
        self.phase.store(phase as u8, Relaxed);
    }

    pub(super) fn finish(&self, now: Duration) {
        self.completed.store(millis(now) + 1, Relaxed);
        self.active.store(0, Relaxed);
    }

    pub(super) fn sample(&self, now: Duration) -> Checkpoint {
        let now = millis(now);
        let active = self.active.load(Relaxed).checked_sub(1);
        let completed = self.completed.load(Relaxed).checked_sub(1);
        let duration = active.map_or(0, |start| seconds(now.saturating_sub(start)));
        let phase = if duration < SLOW_SECONDS {
            None
        } else if super::OPEN_NATIVE_DIALOGS.load(Relaxed) != 0 {
            Some(UpdatePhase::NativeDialog)
        } else {
            UpdatePhase::from_number(u64::from(self.phase.load(Relaxed)))
        };
        // An update that returned while sampling is no longer outstanding.
        let phase = phase.filter(|_| self.active.load(Relaxed).checked_sub(1) == active);
        Checkpoint {
            phase,
            elapsed: seconds(now),
            completed: completed.map(|end| seconds(end.min(now))),
            stalled_for: if phase.is_some() { duration } else { 0 },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Checkpoint {
    pub(super) elapsed: u32,
    completed: Option<u32>,
    pub(super) stalled_for: u32,
    phase: Option<UpdatePhase>,
}

impl Checkpoint {
    // Save the first slow sample, recovery, and then every 30s during a long
    // update. A fully idle or minimized window never triggers these writes.
    pub(super) fn stall_changed(self, previous: Option<Self>) -> bool {
        let prior = previous.map_or(0, |c| c.stalled_for);
        self.phase != previous.and_then(|c| c.phase)
            || self.stalled_for < prior
            || (self.phase != Some(UpdatePhase::NativeDialog)
                && self.stalled_for / 30 != prior / 30)
    }

    pub(super) fn encode(self, out: &mut String) {
        let completed = self.completed.map_or(0, |s| u64::from(s) + 1);
        let phase = self.phase.map_or(0, |p| p as u8 + 1);
        let _ = write!(
            out,
            ":{} {completed} {} {phase}",
            self.elapsed, self.stalled_for
        );
    }

    pub(super) fn decode(fields: &str) -> Option<Self> {
        let mut parts = fields.split_ascii_whitespace();
        let elapsed: u32 = parts.next()?.parse().ok()?;
        let completed: u64 = parts.next()?.parse().ok()?;
        let stalled_for: u32 = parts.next()?.parse().ok()?;
        let phase = match parts.next()?.parse::<u64>().ok()? {
            0 => None,
            encoded => Some(UpdatePhase::from_number(encoded - 1)?),
        };
        if parts.next().is_some()
            || completed > u64::from(elapsed) + 1
            || stalled_for > elapsed
            || (stalled_for != 0 && stalled_for < SLOW_SECONDS)
            || (stalled_for == 0) != phase.is_none()
        {
            return None;
        }
        Some(Self {
            elapsed,
            completed: completed.checked_sub(1).map(|v| v as u32),
            stalled_for,
            phase,
        })
    }

    pub(super) fn describe(self, out: &mut String) {
        if let Some(end) = self.completed {
            let _ = writeln!(
                out,
                "Latest completed editor update at checkpoint: +{end}s since launch"
            );
        }
        if let Some(phase) = self.phase {
            let _ = writeln!(out, "Editor update in progress at checkpoint: at least {}s\nUI phase at that sample: {phase:?}\nAn update can include time waiting for a dialog.", self.stalled_for);
            if phase == UpdatePhase::NativeDialog {
                out.push_str("A native file or confirmation dialog was open when sampled.\n");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_windows_have_no_stall_and_running_updates_get_bounded_checkpoints() {
        let p = UiProgress::default();
        let time = Duration::from_secs;
        assert!(!p.sample(time(300)).stall_changed(None));
        p.begin(time(301));
        p.finish(time(302));
        assert!(!p.sample(time(500)).stall_changed(None));
        p.begin(time(500));
        assert!(!p.sample(time(502)).stall_changed(None));
        let first = p.sample(time(503));
        assert_eq!(first.completed, Some(302));
        assert_eq!(first.stalled_for, 3);
        assert!(first.stall_changed(None));
        assert!(!p.sample(time(529)).stall_changed(Some(first)));
        p.phase(UpdatePhase::Canvas);
        assert!(p.sample(time(529)).stall_changed(Some(first)));
        let long = p.sample(time(530));
        assert!(long.stall_changed(Some(first)));
        let mut encoded = String::new();
        long.encode(&mut encoded);
        assert_eq!(Checkpoint::decode(&encoded[1..]), Some(long));
        super::super::native_dialog(|| {
            let waiting = p.sample(time(540));
            assert_eq!(waiting.phase, Some(UpdatePhase::NativeDialog));
            assert!(waiting.stall_changed(Some(long)));
            assert!(!p.sample(time(640)).stall_changed(Some(waiting)));
        });
        p.finish(time(531));
        let idle = p.sample(time(6000));
        assert!(idle.stall_changed(Some(long)));
        assert_eq!(idle.completed, Some(531));
        assert!(!idle.stall_changed(Some(idle)));
    }
}
