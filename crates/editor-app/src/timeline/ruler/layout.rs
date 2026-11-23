use super::{BoneTrack, Channel};
use document_core::{Clip, VectorObject};
use eframe::egui::{self, Pos2, Rect};
use std::ops::Range;

pub(super) const LABEL_WIDTH: f32 = 132.;
pub(super) const HEADER: f32 = 22.;
const LAYER: f32 = 30.;
const TRACK: f32 = 19.;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum Row {
    Layer(usize),
    Object,
    Property(Channel),
    Bone,
    Child,
}

#[derive(Clone, Copy)]
pub(super) struct Band {
    pub row: Row,
    pub top: f32,
    pub height: f32,
}

pub(super) struct Tracks {
    pub scope: super::Scope,
    layer_count: usize,
    layers_before_detail: usize,
    detail: Vec<Row>,
    pub drawing_start: u32,
    pub height: f32,
    pub selected: Option<u64>,
    pub object_name: String,
    object_index: Option<usize>,
    pub bone: Option<BoneTrack>,
    pub bone_name: String,
    pub exposure: Range<u32>,
}

impl Tracks {
    pub fn new(
        clip: &Clip,
        frame: u32,
        selected: Option<u64>,
        selected_bone: Option<(u64, u32, u64)>,
        active: Channel,
        all_keyed: bool,
        collapsed: bool,
    ) -> Self {
        let (start, length) = clip.exposure(frame);
        let drawing = &clip.cels[&start];
        let index = selected.and_then(|id| drawing.objects.iter().position(|o| o.id == id));
        let object = index.map(|i| &drawing.objects[i]);
        let bone = BoneTrack::selected(clip, frame, selected_bone);
        let layer_count = drawing.layers.len();
        let layers_before_detail = object
            .and_then(|o| drawing.layers.iter().rev().position(|l| l.id == o.layer))
            .map_or(layer_count, |i| i + 1);
        let (object_start, object_length) =
            object.map_or((start, length), |o| clip.layer_exposure(frame, o.layer));
        let mut tracks = Self {
            scope: super::Scope::new(clip, frame, selected, selected_bone),
            layer_count,
            layers_before_detail,
            detail: Vec::with_capacity(Channel::ALL.len() + 2),
            drawing_start: start,
            height: HEADER,
            selected: object.map(|o| o.id),
            object_name: object.map(VectorObject::display_name).unwrap_or_default(),
            object_index: index,
            bone: bone.map(|(b, _)| b),
            bone_name: bone.map_or_else(String::new, |(_, name)| name.to_owned()),
            exposure: object_start..object_start + object_length,
        };
        if let Some(object) = object {
            tracks.detail.push(Row::Object);
            if !collapsed {
                if object
                    .clip_instance
                    .as_ref()
                    .is_some_and(|i| !i.drawing_keys.is_empty())
                {
                    tracks.detail.push(Row::Child);
                }
                for channel in Channel::ALL {
                    if channel == active || (all_keyed && channel.last(object).is_some()) {
                        tracks.detail.push(Row::Property(channel));
                    }
                }
            }
        }
        tracks.height = HEADER
            + layer_count as f32 * LAYER
            + tracks.detail.len() as f32 * TRACK
            + if bone.is_some() { TRACK } else { 0. };
        tracks
    }

    fn count(&self) -> usize {
        self.layer_count + self.detail.len() + usize::from(self.bone.is_some())
    }

    fn band(&self, index: usize) -> Band {
        let split = self.layers_before_detail;
        let detail_end = split + self.detail.len();
        if (split..detail_end).contains(&index) {
            return Band {
                row: self.detail[index - split],
                top: HEADER + split as f32 * LAYER + (index - split) as f32 * TRACK,
                height: TRACK,
            };
        }
        let layer = if index < split {
            index
        } else {
            index - self.detail.len()
        };
        let top = HEADER
            + layer.min(self.layer_count) as f32 * LAYER
            + if index < split {
                0.
            } else {
                self.detail.len() as f32 * TRACK
            };
        if layer < self.layer_count {
            Band {
                row: Row::Layer(self.layer_count - 1 - layer),
                top,
                height: LAYER,
            }
        } else {
            debug_assert!(self.bone.is_some() && layer == self.layer_count);
            Band {
                row: Row::Bone,
                top,
                height: TRACK,
            }
        }
    }

    /// Find the first row extending below y without allocating or scanning the layer list.
    pub fn bands(&self, top: f32, bottom: f32) -> impl Iterator<Item = Band> + '_ {
        let (mut lo, mut hi) = (0, self.count());
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let row = self.band(mid);
            if row.top + row.height <= top {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        (lo..self.count())
            .map(|i| self.band(i))
            .take_while(move |b| b.top < bottom)
    }

    pub fn selected_rows(&self) -> Option<(f32, f32)> {
        if self.detail.is_empty() || self.layers_before_detail == 0 {
            return None;
        }
        let layer = self.band(self.layers_before_detail - 1);
        let detail = self.band(self.layers_before_detail + self.detail.len().min(2) - 1);
        Some((layer.top, detail.top + detail.height))
    }

    pub fn object<'a>(&self, clip: &'a Clip) -> Option<&'a VectorObject> {
        let obj = clip
            .cels
            .get(&self.drawing_start)?
            .objects
            .get(self.object_index?)?;
        (Some(obj.id) == self.selected).then_some(obj)
    }
}

pub(super) struct Grid {
    pub content: Rect,
    pub view: Rect,
    pub cell: f32,
}

impl Grid {
    pub fn x(&self, tick: u32) -> f32 {
        self.content.left() + LABEL_WIDTH + self.cell * tick as f32
    }
    pub fn tick(&self, x: f32, frames: u32) -> u32 {
        (((x - self.x(0)) / self.cell).floor() as u32).min(frames - 1)
    }
    pub fn rows_top(&self) -> f32 {
        self.view.top() + HEADER
    }
    pub fn time_view(&self) -> Rect {
        Rect::from_min_max(
            egui::pos2(self.view.left() + LABEL_WIDTH, self.view.top()),
            self.view.max,
        )
    }
    /// Valid time columns include the space beneath the last track.
    pub fn frame_area(&self) -> Rect {
        self.time_view()
            .with_max_x(self.content.right().min(self.view.right()))
    }
    pub fn band(&self, band: &Band) -> Rect {
        let y = self.content.top() + band.top;
        Rect::from_min_max(
            egui::pos2(self.view.left(), y),
            egui::pos2(self.view.right(), y + band.height),
        )
    }
    pub fn body_clip(&self) -> Rect {
        Rect::from_min_max(egui::pos2(self.view.left(), self.rows_top()), self.view.max)
    }
    pub fn row_at(&self, pos: Pos2, tracks: &Tracks) -> Option<Row> {
        self.band_at(pos, tracks).map(|band| band.row)
    }
    pub fn band_at(&self, pos: Pos2, tracks: &Tracks) -> Option<Band> {
        if pos.y < self.rows_top() {
            return None;
        }
        let y = pos.y - self.content.top();
        tracks.bands(y, y + 1.).next()
    }
}
