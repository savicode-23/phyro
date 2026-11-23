use super::{potentially_animated, Document, VectorObject};
use std::sync::{Arc, Weak};

#[cfg(test)]
mod tests;

#[derive(Default)]
pub(super) struct HeldRanges {
    source: Weak<Document>,
    intervals: Vec<[u32; 2]>,
    // A search of at least MIN_TICKS makes verification worthwhile on the next paint.
    long_scan: bool,
    animated_indices: Vec<usize>,
    // None defers verification until the first nested-pose search in this paint.
    verified: Option<bool>,
}

impl HeldRanges {
    // Short holds scan cheaply. Bound the cost of checking an already known hold
    // and the number of intervals retained when an artist jumps between poses.
    const MIN_TICKS: u32 = 16;
    const MAX_ANIMATED_OBJECTS: usize = 64;
    const MAX_INTERVALS: usize = 32;

    pub(super) fn begin(&mut self, painted: Option<&Arc<Document>>) {
        self.verified = Some(false);
        if !std::mem::take(&mut self.long_scan) {
            return;
        }
        let Some(painted) = painted else { return };
        let source = Arc::downgrade(painted);
        if !self.source.ptr_eq(&source) {
            self.intervals.clear();
            self.source = source;
            self.animated_indices.clear();
            self.animated_indices.extend(
                painted
                    .objects
                    .iter()
                    .enumerate()
                    .filter(|(_, o)| potentially_animated(painted, o))
                    .map(|(index, _)| index)
                    .take(Self::MAX_ANIMATED_OBJECTS + 1),
            );
        }
        if self.animated_indices.len() <= Self::MAX_ANIMATED_OBJECTS {
            self.verified = None;
        }
    }

    pub(super) fn skip(&mut self, from: u32, edge: u32, live: &[&VectorObject]) -> u32 {
        // The exact search compares only these potentially animated objects.
        // Nested cel identity matters even when two cels contain equal artwork.
        let verified = *self.verified.get_or_insert_with(|| {
            let child = |o: &VectorObject| {
                o.clip_instance
                    .as_ref()
                    .and_then(|i| i.shared_clip())
                    .map(Arc::as_ptr)
            };
            self.animated_indices.len() == live.len()
                && self.source.upgrade().is_some_and(|old| {
                    self.animated_indices
                        .iter()
                        .zip(live)
                        .all(|(&index, &current)| {
                            let previous = &old.objects[index];
                            child(previous) == child(current) && previous == current
                        })
                })
        });
        if !verified {
            return from;
        }
        match self
            .intervals
            .iter()
            .find(|[lo, hi]| (*lo..=*hi).contains(&from))
        {
            Some([lo, hi]) => edge.clamp(*lo, *hi),
            None => from,
        }
    }

    pub(super) fn remember(&mut self, from: u32, last_equal: u32) {
        if from.abs_diff(last_equal) < Self::MIN_TICKS {
            return;
        }
        self.long_scan = true;
        if self.verified != Some(true) {
            return;
        }
        let (mut lo, mut hi) = (from.min(last_equal), from.max(last_equal));
        // Overlapping intervals have a shared sample proving their poses match.
        // Adjacent intervals can belong to different poses.
        self.intervals.retain(|&[a, b]| {
            if b < lo || a > hi {
                return true;
            }
            lo = lo.min(a);
            hi = hi.max(b);
            false
        });
        if self.intervals.len() == Self::MAX_INTERVALS {
            self.intervals.remove(0);
        }
        self.intervals.push([lo, hi]);
    }
}
