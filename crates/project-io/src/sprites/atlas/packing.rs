//! Pack trimmed output at integer pixel coordinates in their chosen orientation. A shelf is
//! shared by cells of similar height. Try several shelf widths and keep the fewest pages,
//! then the smallest stored area. Stable tie breaking makes repeated exports identical.
use super::{invalid, io};
use document_core::ExportPreset;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Region {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Slot {
    pub rotated: bool,
    pub page: usize,
    pub x: u32,
    pub y: u32,
}

#[derive(Default)]
pub(super) struct Page {
    pub width: u32,
    pub height: u32,
    pub entries: Vec<usize>,
    shelves: Vec<Shelf>,
}

struct Shelf {
    y: u32,
    height: u32,
    right: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum PageLimit {
    Single,
    Multiple,
}

fn single_sheet_full(limit: u32) -> io::Error {
    invalid(format!(
        "The artwork cannot fit on one {limit} × {limit} px sprite sheet. Reduce the frame resolution or spacing, enable trimming or frame sharing, or increase the sheet limit up to 4096 px."
    ))
}

impl PageLimit {
    /// Check each distinct stored frame during sampling. A large clip that
    /// already exceeds the sheet budget needs no further rasterization.
    pub(super) fn reserve(
        self,
        region: Region,
        preset: &ExportPreset,
        occupied: &mut u64,
    ) -> io::Result<()> {
        let limit = preset.packed.max_size;
        let usable = limit.saturating_sub(2 * preset.margin);
        let w = region.w + 2 * preset.packed.extrude;
        let h = region.h + 2 * preset.packed.extrude;
        let too_large = w > usable || h > usable;
        if self == Self::Single {
            *occupied += u64::from(w) * u64::from(h);
            if too_large || *occupied > u64::from(usable) * u64::from(usable) {
                return Err(single_sheet_full(limit));
            }
        } else if too_large {
            return Err(invalid(format!(
                "Frame artwork of {} × {} exceeds a {limit} px page including its margin and extrusion.",
                region.w, region.h
            )));
        }
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct Packing {
    pub pages: Vec<Page>,
    pub slots: Vec<Slot>,
}

impl Packing {
    pub fn plan(regions: &[Region], preset: &ExportPreset, pages: PageLimit) -> io::Result<Self> {
        let limit = preset.packed.max_size;
        let usable = limit.saturating_sub(2 * preset.margin);
        let bleed = 2 * preset.packed.extrude;
        let mut items: Vec<_> = regions
            .iter()
            .enumerate()
            .map(|(index, region)| (index, region.w + bleed, region.h + bleed))
            .collect();
        let mut best = plan_rows(&mut items, usable, preset, pages);
        if preset.packed.allow_rotation {
            for landscape in [true, false] {
                let mut rotated = vec![false; items.len()];
                let mut oriented = items.clone();
                for (i, w, h) in &mut oriented {
                    if *w != *h && (*w < *h) == landscape {
                        std::mem::swap(w, h);
                        rotated[*i] = true;
                    }
                }
                let Some(mut alternative) = plan_rows(&mut oriented, usable, preset, pages) else {
                    continue;
                };
                for (slot, turn) in alternative.slots.iter_mut().zip(rotated) {
                    slot.rotated = turn;
                }
                if best
                    .as_ref()
                    .is_none_or(|best| alternative.cost() < best.cost())
                {
                    best = Some(alternative);
                }
            }
        }
        best.ok_or_else(|| single_sheet_full(limit))
    }

    fn cost(&self) -> (usize, u64, usize) {
        let pixels = self
            .pages
            .iter()
            .map(|p| u64::from(p.width) * u64::from(p.height))
            .sum();
        (
            self.pages.len(),
            pixels,
            self.slots.iter().filter(|s| s.rotated).count(),
        )
    }
}

fn plan_rows(
    items: &mut [(usize, u32, u32)],
    usable: u32,
    preset: &ExportPreset,
    pages: PageLimit,
) -> Option<Packing> {
    items.sort_by_key(|&(i, w, h)| (std::cmp::Reverse(h), std::cmp::Reverse(w), i));
    let widest = items.iter().map(|&(_, w, _)| w).max().unwrap_or(1);
    let area = items
        .iter()
        .map(|&(_, w, h)| u64::from(w) * u64::from(h))
        .sum::<u64>();
    let side = (area as f64).sqrt().ceil() as u32;
    let mut widths = vec![
        widest,
        side * 3 / 4,
        side,
        side * 5 / 4,
        side * 3 / 2,
        usable,
    ];
    for w in &mut widths {
        *w = (*w).clamp(widest, usable);
    }
    widths.sort_unstable();
    widths.dedup();
    widths
        .into_iter()
        .filter_map(|w| pack(items, w, usable, preset, pages))
        .min_by_key(|plan| {
            (
                plan.pages.len(),
                plan.pages
                    .iter()
                    .map(|p| u64::from(p.width) * u64::from(p.height))
                    .sum::<u64>(),
            )
        })
}

fn pack(
    items: &[(usize, u32, u32)],
    width: u32,
    height: u32,
    preset: &ExportPreset,
    pages: PageLimit,
) -> Option<Packing> {
    let mut plan = Packing {
        pages: Vec::new(),
        slots: vec![Slot::default(); items.len()],
    };
    for &(index, w, h) in items {
        let mut target = None;
        let mut new_row = None;
        for (page_i, page) in plan.pages.iter().enumerate() {
            for (row_i, row) in page.shelves.iter().enumerate() {
                let x = row.right + preset.padding;
                if h <= row.height && x + w <= width {
                    let score = (row.height - h, width - x - w, page_i, row_i);
                    if target.is_none_or(|old| score < old) {
                        target = Some(score);
                    }
                }
            }
            let next = page
                .shelves
                .last()
                .map_or(0, |r| r.y + r.height + preset.padding);
            if next + h <= height && new_row.is_none() {
                new_row = Some((page_i, next));
            }
        }
        let (page_i, x, y) = if let Some((_, _, page_i, row_i)) = target {
            let row = &mut plan.pages[page_i].shelves[row_i];
            let x = row.right + preset.padding;
            row.right = x + w;
            (page_i, x, row.y)
        } else {
            if new_row.is_none() && pages == PageLimit::Single && !plan.pages.is_empty() {
                return None;
            }
            let (page_i, y) = new_row.unwrap_or_else(|| {
                let index = plan.pages.len();
                plan.pages.push(Page::default());
                (index, 0)
            });
            plan.pages[page_i].shelves.push(Shelf {
                y,
                height: h,
                right: w,
            });
            (page_i, 0, y)
        };
        let page = &mut plan.pages[page_i];
        page.width = page.width.max(x + w + 2 * preset.margin);
        page.height = page.height.max(y + h + 2 * preset.margin);
        page.entries.push(index);
        plan.slots[index] = Slot {
            page: page_i,
            rotated: false,
            x: preset.margin + x + preset.packed.extrude,
            y: preset.margin + y + preset.packed.extrude,
        };
    }
    Some(plan)
}
