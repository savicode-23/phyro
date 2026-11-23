/// Sorted, disjoint inclusive runs of ticks. Adjacent runs are always merged.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct FrameSelection {
    runs: Vec<(u32, u32)>,
    count: u32,
}

impl FrameSelection {
    pub(crate) fn ranges(
        &self,
    ) -> impl DoubleEndedIterator<Item = (u32, u32)> + ExactSizeIterator + '_ {
        self.runs.iter().copied()
    }

    pub(super) fn visible_ranges(
        &self,
        first: u32,
        last: u32,
    ) -> impl Iterator<Item = (u32, u32)> + '_ {
        let start = self.runs.partition_point(|&(_, b)| b < first);
        self.runs[start..]
            .iter()
            .copied()
            .take_while(move |&(a, _)| a <= last)
    }

    pub(crate) fn count(&self) -> u32 {
        self.count
    }

    pub(crate) fn contains(&self, tick: u32) -> bool {
        self.visible_ranges(tick, tick).next().is_some()
    }

    fn copy_from(&mut self, other: &Self) {
        self.runs.clone_from(&other.runs);
        self.count = other.count;
    }

    fn single(&mut self, tick: u32) {
        self.runs.clear();
        self.runs.push((tick, tick));
        self.count = 1;
    }

    fn add(&mut self, mut first: u32, mut last: u32) {
        let start = self.runs.partition_point(|&(_, b)| b + 1 < first);
        let end = self.runs.partition_point(|&(a, _)| a <= last + 1);
        for &(a, b) in &self.runs[start..end] {
            first = first.min(a);
            last = last.max(b);
            self.count -= b - a + 1;
        }
        self.runs.splice(start..end, [(first, last)]);
        self.count += last - first + 1;
    }

    fn remove(&mut self, tick: u32) {
        let i = self.runs.partition_point(|&(_, b)| b < tick);
        let (a, b) = self.runs[i];
        if a == b {
            self.runs.remove(i);
        } else if a == tick {
            self.runs[i].0 += 1;
        } else if b == tick {
            self.runs[i].1 -= 1;
        } else {
            self.runs[i].1 = tick - 1;
            self.runs.insert(i + 1, (tick + 1, b));
        }
        self.count -= 1;
    }
}

#[derive(Default)]
pub(super) struct FrameFocus {
    pub(super) clip: u64,
    pub(super) anchor: u32,
    pub(super) selection: FrameSelection,
    // A modified drag rebuilds its live range from the selection at pointer-down.
    // Reversing direction therefore restores any gaps the pointer passed over.
    base: FrameSelection,
}

impl FrameFocus {
    pub(super) fn new(clip: u64, anchor: u32) -> Self {
        Self {
            clip,
            anchor,
            ..Self::default()
        }
    }

    pub(super) fn single(&mut self, tick: u32) {
        self.anchor = tick;
        self.selection.single(tick);
    }

    pub(super) fn start(&mut self, tick: u32, extend: bool, additive: bool) {
        if !extend {
            self.anchor = tick;
        }
        if additive {
            self.base.copy_from(&self.selection);
        } else {
            self.base.runs.clear();
            self.base.count = 0;
        }
        if additive && !extend && self.selection.contains(tick) {
            self.selection.remove(tick);
        } else {
            self.extend_to(tick);
        }
    }

    pub(super) fn extend_to(&mut self, tick: u32) {
        self.selection.copy_from(&self.base);
        self.selection
            .add(self.anchor.min(tick), self.anchor.max(tick));
    }
}
