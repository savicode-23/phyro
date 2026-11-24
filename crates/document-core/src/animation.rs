//! Named clips, held vector drawings and nondestructive timeline edits.
use crate::{Document, MAX_FPS, MAX_FRAMES};
mod events;
mod foot_contacts;
mod frame_removal;
use frame_removal::FrameRemoval;
mod layer_blank;
mod layer_exposures;
mod sets;
use serde::{Deserialize, Serialize};
pub use sets::{AnimationSet, DEFAULT_ANIMATION_SET_ID};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

impl Document {
    /// Initial drawing for a new project. Further drawings inherit its artboard.
    pub fn for_new_project() -> Self {
        Self {
            width: 1000,
            height: 1000,
            ..Self::default()
        }
    }

    /// A new drawing on the same artboard keeps layers, guides and the rig.
    fn blank_drawing(&self) -> Self {
        Self {
            objects: Vec::new(),
            next_id: 1,
            skeleton: self.skeleton.clone(),
            layers: self.layers.clone(),
            guides_x: self.guides_x.clone(),
            guides_y: self.guides_y.clone(),
            ..*self
        }
    }

    /// Whether every valid tick paints this drawing identically. A nested clip
    /// can change exposures without placement keys. Symbols display tick zero.
    pub fn frame_independent(&self) -> bool {
        !self.skeleton.has_animation()
            && self
                .objects
                .iter()
                .all(crate::VectorObject::frame_independent)
    }
}

impl crate::Skeleton {
    pub(crate) fn has_animation(&self) -> bool {
        !self.foot_pins.is_empty() || self.bones.iter().any(|b| !b.rotation_keys.is_empty())
    }
}

impl crate::VectorObject {
    /// Whether the evaluated artwork stays identical throughout its drawing exposure.
    pub fn frame_independent(&self) -> bool {
        self.last_keyframe().is_none()
            && !self.has_animated_rig()
            && self.clip_instance.as_ref().is_none_or(|placement| {
                placement.shared_clip().is_some_and(|clip| {
                    clip.cels.len() == 1
                        && clip.cels.get(&0).is_some_and(|doc| doc.frame_independent())
                })
            })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnimationProject {
    #[serde(default, skip_serializing_if = "crate::GridSettings::is_default")]
    pub grid: crate::GridSettings,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub symbols: BTreeMap<u64, crate::SymbolDefinition>,
    pub clips: Vec<Clip>,
    #[serde(
        default = "sets::default_sets",
        skip_serializing_if = "sets::is_default_collection"
    )]
    pub animation_sets: Vec<AnimationSet>,
    #[serde(
        default = "sets::first_unused_id",
        skip_serializing_if = "sets::is_first_unused_id"
    )]
    pub next_set_id: u64,
    /// Allocation cursor for persistent references, including IDs of deleted clips.
    #[serde(default)]
    pub next_clip_id: u64,
    pub preset: ExportPreset,
    #[serde(default)]
    pub presets: BTreeMap<String, ExportPreset>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Clip {
    /// Immutable within the owning project, including across rename and reorder.
    #[serde(default)]
    pub id: u64,
    #[serde(
        default = "sets::default_id",
        skip_serializing_if = "sets::is_default_id"
    )]
    pub set_id: u64,
    /// Write this clip as a named animation when exporting its set. Placements always render.
    #[serde(default = "default_in_set_export", skip_serializing_if = "is_true")]
    pub include_in_set_export: bool,
    /// An editor-session identity shared by undo and worker snapshots.
    #[serde(skip, default = "fresh_clip_id")]
    session_id: u64,
    pub name: String,
    pub frames: u32,
    pub fps: u32,
    pub looping: bool,
    /// Pixel coordinates relative to the top-left of the untrimmed artboard.
    pub origin: [f32; 2],
    /// Each entry begins an exposure. Until the next entry all ticks refer to this drawing.
    pub cels: BTreeMap<u32, Arc<Document>>,
    /// At these composite drawing boundaries only the listed layers begin a new drawing.
    /// Owner zero delimits independent skeletons. Other boundaries start independent
    /// drawings for every layer and the rig, including in older files.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub layer_starts: BTreeMap<u32, BTreeSet<u64>>,
    #[serde(skip)]
    layer_index: std::sync::OnceLock<Arc<layer_exposures::LayerIndex>>,
    /// Names in trigger order at each zero-based tick.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub events: BTreeMap<u32, Vec<String>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportPreset {
    pub columns: u32,
    pub padding: u32,
    pub margin: u32,
    pub scale: u32,
    /// Denominator of the PNG scale. Older presets store whole-number multiples.
    #[serde(default = "unit_divisor", skip_serializing_if = "is_unit_divisor")]
    pub scale_divisor: u32,
    #[serde(default)]
    pub packed: PackedOptions,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PackedOptions {
    pub enabled: bool,
    pub max_size: u32,
    pub trim: bool,
    pub deduplicate: bool,
    pub allow_rotation: bool,
    pub extrude: u32,
}

impl Default for PackedOptions {
    fn default() -> Self {
        Self {
            enabled: false,
            max_size: 2048,
            trim: true,
            deduplicate: true,
            allow_rotation: false,
            extrude: 1,
        }
    }
}

/// Compare unchanged snapshots without walking the shared artwork. Documents
/// loaded independently still compare by their authored values.
pub(crate) fn same_drawing(a: &Arc<Document>, b: &Arc<Document>) -> bool {
    Arc::ptr_eq(a, b) || a.as_ref() == b.as_ref()
}

fn fresh_clip_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}
fn default_in_set_export() -> bool {
    true
}
fn is_true(value: &bool) -> bool {
    *value
}
impl PartialEq for Clip {
    fn eq(&self, other: &Self) -> bool {
        let Self {
            id,
            set_id,
            include_in_set_export,
            session_id: _,
            name,
            frames,
            fps,
            looping,
            origin,
            cels,
            layer_starts,
            layer_index: _,
            events,
        } = self;
        *id == other.id
            && *set_id == other.set_id
            && *include_in_set_export == other.include_in_set_export
            && name == &other.name
            && *frames == other.frames
            && *fps == other.fps
            && *looping == other.looping
            && *origin == other.origin
            && events == &other.events
            && layer_starts == &other.layer_starts
            && cels.len() == other.cels.len()
            && cels
                .iter()
                .zip(&other.cels)
                .all(|((a, x), (b, y))| a == b && same_drawing(x, y))
    }
}

impl Default for ExportPreset {
    fn default() -> Self {
        Self {
            columns: 4,
            padding: 0,
            margin: 0,
            scale: 1,
            scale_divisor: 1,
            packed: PackedOptions::default(),
        }
    }
}

fn unit_divisor() -> u32 {
    1
}
fn is_unit_divisor(divisor: &u32) -> bool {
    *divisor == 1
}

impl ExportPreset {
    pub fn raster_scale(&self) -> f64 {
        f64::from(self.scale) / f64::from(self.scale_divisor)
    }

    /// Preserve the exact requested ratio, including when it comes from a pixel width.
    pub fn set_raster_ratio(&mut self, pixels: u32, units: u32) {
        let (mut a, mut b) = (pixels, units);
        while b != 0 {
            (a, b) = (b, a % b);
        }
        self.scale = pixels / a.max(1);
        self.scale_divisor = units / a.max(1);
    }

    pub fn raster_size(&self, native: [u32; 2]) -> Result<[u32; 2], String> {
        Self::scaled_size(native, self.raster_scale())
    }

    pub fn scaled_size(native: [u32; 2], scale: f64) -> Result<[u32; 2], String> {
        if !(scale > 0. && scale <= 32.) || native.contains(&0) {
            return Err("Choose a positive PNG size up to 32× the original.".into());
        }
        let size = native.map(|n| (f64::from(n) * scale).round().max(1.) as u32);
        if u64::from(size[0]) * u64::from(size[1]) > crate::MAX_RASTER_PIXELS {
            return Err(format!(
                "{} × {} px exceeds the 16 megapixel PNG limit. Reduce the resolution.",
                size[0], size[1]
            ));
        }
        Ok(size)
    }
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=4096).contains(&self.packed.max_size) || self.packed.extrude > 32 {
            return Err(
                "Use a maximum page size of 1–4096 pixels and 0–32 pixels of edge extrusion."
                    .into(),
            );
        }
        if !(1..=MAX_FRAMES).contains(&self.columns)
            || self.scale == 0
            || self.scale_divisor == 0
            || self.raster_scale() > 32.
            || self.padding > 4096
            || self.margin > 4096
        {
            return Err(
                "Use 1–4096 columns, a positive scale up to 32× and 0–4096 px of padding or margin.".into(),
            );
        }
        Ok(())
    }
}

impl Default for AnimationProject {
    fn default() -> Self {
        Self::from_document(Document::for_new_project())
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.trim() == name
        && name.chars().count() <= 80
        && !name.chars().any(char::is_control)
}

impl AnimationProject {
    /// Resize the shared project artboard without transforming placed artwork.
    pub fn resize_artboard(&mut self, size: [u32; 2]) -> Result<(), String> {
        if size.iter().any(|n| !(1..=crate::MAX_DIMENSION).contains(n)) {
            return Err(format!(
                "Use 1–{} pixels for each artboard dimension.",
                crate::MAX_DIMENSION
            ));
        }
        let first = self
            .clips
            .first()
            .ok_or("The project has no artboard.")?
            .drawing_at(0);
        let old_size = [first.width, first.height];
        if size == old_size {
            return Ok(());
        }
        let mut staged = self.clone();
        // A duplicated clip can share every drawing with the original clip.
        let mut resized = HashMap::new();
        for cel in staged
            .clips
            .iter_mut()
            .flat_map(|clip| clip.cels.values_mut())
        {
            *cel = Arc::clone(resized.entry(Arc::as_ptr(cel)).or_insert_with(|| {
                let mut doc = cel.as_ref().clone();
                [doc.width, doc.height] = size;
                for placement in doc
                    .objects
                    .iter_mut()
                    .filter_map(|o| o.clip_instance.as_mut())
                {
                    let original = placement.source_size.unwrap_or(old_size);
                    placement.source_size = (original != size).then_some(original);
                }
                Arc::new(doc)
            }));
        }
        staged.hydrate_composition()?;
        *self = staged;
        Ok(())
    }

    pub fn from_document(doc: Document) -> Self {
        let clip = Clip {
            id: 1,
            set_id: DEFAULT_ANIMATION_SET_ID,
            include_in_set_export: true,
            session_id: fresh_clip_id(),
            name: "Animation".into(),
            frames: doc.frames,
            fps: doc.fps,
            looping: true,
            origin: [doc.width as f32 / 2., doc.height as f32],
            cels: BTreeMap::from([(0, Arc::new(doc))]),
            layer_starts: BTreeMap::new(),
            layer_index: Default::default(),
            events: BTreeMap::new(),
        };
        Self {
            clips: vec![clip],
            grid: Default::default(),
            animation_sets: sets::default_sets(),
            next_set_id: sets::first_unused_id(),
            next_clip_id: 2,
            symbols: BTreeMap::new(),
            preset: ExportPreset::default(),
            presets: BTreeMap::new(),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        self.grid.validate()?;
        self.validate_symbols()?;
        self.validate_animation_sets()?;
        if self.clips.is_empty() || self.clips.len() > 256 {
            return Err("A project needs 1–256 animation clips.".into());
        }
        self.preset.validate()?;
        if self.presets.len() > 128 {
            return Err("A project can have up to 128 named export presets.".into());
        }
        for (name, preset) in &self.presets {
            if !valid_name(name) {
                return Err("Invalid export preset name.".into());
            }
            preset.validate()?;
        }
        let mut ids = HashSet::new();
        let mut names = HashSet::new();
        let mut dimensions = None;
        for clip in &self.clips {
            if clip.id == 0 || clip.id >= self.next_clip_id || !ids.insert(clip.id) {
                return Err("A clip has an invalid or duplicated persistent ID.".into());
            }
            if !valid_name(&clip.name) || !names.insert((clip.set_id, clip.name.to_lowercase())) {
                return Err(
                    "Clip names must be unique within each animation set, with 1–80 characters."
                        .into(),
                );
            }
            clip.validate()?;
            let scene = clip.drawing_at(0);
            let size = (scene.width, scene.height);
            if dimensions
                .replace(size)
                .is_some_and(|previous| previous != size)
            {
                return Err("All clips must use the project artboard dimensions.".into());
            }
        }
        Ok(())
    }

    /// Resolve a persistent ID after changes to the clip list.
    pub fn clip_index(&self, id: u64) -> Option<usize> {
        self.clips.iter().position(|clip| clip.id == id)
    }

    fn allocate_clip_id(&mut self) -> Result<u64, String> {
        let id = self.next_clip_id;
        self.next_clip_id = id.checked_add(1).ok_or("Clip IDs have been exhausted.")?;
        Ok(id)
    }

    fn check_name(&self, set_id: u64, name: &str, skip: Option<usize>) -> Result<(), String> {
        if self.animation_set(set_id).is_none() {
            return Err("Unknown animation set.".into());
        }
        if !valid_name(name) {
            return Err("Enter a clip name of 1–80 characters, without surrounding spaces.".into());
        }
        let normalized = name.to_lowercase();
        if self.clips.iter().enumerate().any(|(i, c)| {
            Some(i) != skip && c.set_id == set_id && c.name.to_lowercase() == normalized
        }) {
            return Err(format!("A clip named '{name}' already exists."));
        }
        Ok(())
    }

    pub fn add_clip(&mut self, name: &str) -> Result<usize, String> {
        self.add_clip_in_set(DEFAULT_ANIMATION_SET_ID, name)
    }

    pub fn add_clip_in_set(&mut self, set_id: u64, name: &str) -> Result<usize, String> {
        self.check_name(set_id, name, None)?;
        if self.clips.len() >= 256 {
            return Err("The project already has 256 clips.".into());
        }
        let (mut doc, origin) =
            if let Some(source) = self.clips.iter().find(|clip| clip.set_id == set_id) {
                (source.drawing_at(0).blank_drawing(), source.origin)
            } else {
                let canvas = self
                    .clips
                    .first()
                    .ok_or("Project contains no artboard.")?
                    .drawing_at(0);
                let doc = Document {
                    width: canvas.width,
                    height: canvas.height,
                    fps: canvas.fps,
                    ..Document::default()
                };
                (doc, [canvas.width as f32 / 2., canvas.height as f32])
            };
        let pose = doc.skeleton.posed_rotations_at(0)?;
        for bone in &mut doc.skeleton.bones {
            bone.rotation = pose[&bone.id];
            bone.rotation_keys.clear();
        }
        doc.skeleton.foot_pins.clear();
        doc.frames = 1;
        let id = self.allocate_clip_id()?;
        self.clips.push(Clip {
            id,
            set_id,
            include_in_set_export: true,
            session_id: fresh_clip_id(),
            name: name.into(),
            fps: doc.fps,
            frames: 1,
            origin,
            looping: true,
            cels: BTreeMap::from([(0, Arc::new(doc))]),
            layer_starts: BTreeMap::new(),
            layer_index: Default::default(),
            events: BTreeMap::new(),
        });
        Ok(self.clips.len() - 1)
    }

    /// Keep the imported clip's authored data and assign identities in this project.
    pub(crate) fn append_imported_clip(&mut self, mut clip: Clip) -> Result<usize, String> {
        self.check_name(clip.set_id, &clip.name, None)?;
        if self.clips.len() >= 256 {
            return Err("The project already has 256 clips.".into());
        }
        clip.id = self.allocate_clip_id()?;
        clip.session_id = fresh_clip_id();
        self.clips.push(clip);
        Ok(self.clips.len() - 1)
    }

    pub fn duplicate_clip(&mut self, index: usize, name: &str) -> Result<usize, String> {
        let source = self.clips.get(index).ok_or("Unknown clip.")?;
        self.check_name(source.set_id, name, None)?;
        if self.clips.len() >= 256 {
            return Err("The project already has 256 clips.".into());
        }
        let mut clip = source.clone();
        clip.name = name.into();
        clip.id = self.allocate_clip_id()?;
        clip.session_id = fresh_clip_id();
        self.clips.insert(index + 1, clip);
        Ok(index + 1)
    }

    pub fn rename_clip(&mut self, index: usize, name: &str) -> Result<(), String> {
        let clip = self.clips.get(index).ok_or("Unknown clip.")?;
        self.check_name(clip.set_id, name, Some(index))?;
        self.clips[index].name = name.into();
        Ok(())
    }

    pub fn remove_clip(&mut self, index: usize) -> Result<(), String> {
        if self.clips.len() <= 1 {
            return Err("Keep at least one clip in the project.".into());
        }
        if index >= self.clips.len() {
            return Err("Unknown clip.".into());
        }
        if self.clip_usage(self.clips[index].id) > 0 {
            return Err("Remove placements of this clip before deleting it.".into());
        }
        self.clips.remove(index);
        Ok(())
    }
}

impl Clip {
    pub fn session_id(&self) -> u64 {
        self.session_id
    }
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=MAX_FRAMES).contains(&self.frames) || !(1..=MAX_FPS).contains(&self.fps) {
            return Err(format!("Invalid timing for clip '{}'.", self.name));
        }
        self.validate_events()?;
        self.validate_layer_starts()?;
        if !self.origin.iter().all(|p| p.is_finite()) {
            return Err("Sprite origin must be finite.".into());
        }
        let first = self
            .cels
            .get(&0)
            .ok_or("Each clip needs a drawing at its first frame.")?;
        let mut validated = HashSet::new();
        for (&start, scene) in &self.cels {
            if start >= self.frames
                || scene.frames != self.frames
                || scene.fps != self.fps
                || (scene.width, scene.height) != (first.width, first.height)
            {
                return Err(format!(
                    "Invalid drawing size or timing at frame {} in '{}'.",
                    u64::from(start) + 1,
                    self.name
                ));
            }
            if validated.insert(Arc::as_ptr(scene)) {
                scene.validate()?;
            }
        }
        Ok(())
    }

    pub fn drawing_at(&self, frame: u32) -> &Document {
        self.cels
            .range(..=frame)
            .next_back()
            .expect("validated clip starts at zero")
            .1
    }

    pub fn drawing_at_mut(&mut self, frame: u32) -> &mut Document {
        Arc::make_mut(self.drawing_snapshot_mut(frame))
    }

    /// Access the held drawing so a finished edit can also supply a renderer snapshot.
    pub fn drawing_snapshot_mut(&mut self, frame: u32) -> &mut Arc<Document> {
        self.cels
            .range_mut(..=frame)
            .next_back()
            .expect("validated clip starts at zero")
            .1
    }

    pub fn exposure(&self, frame: u32) -> (u32, u32) {
        let start = *self
            .cels
            .range(..=frame)
            .next_back()
            .expect("validated clip starts at zero")
            .0;
        let end = self
            .cels
            .range(start + 1..)
            .next()
            .map_or(self.frames, |(&f, _)| f);
        (start, end - start)
    }

    pub fn set_fps(&mut self, fps: u32) -> Result<(), String> {
        if !(1..=MAX_FPS).contains(&fps) {
            return Err(format!("Use 1–{MAX_FPS} fps."));
        }
        if self.fps != fps {
            update_shared_drawings(self.cels.values_mut(), |doc| {
                doc.fps = fps;
                Ok(())
            })?;
            self.fps = fps;
        }
        Ok(())
    }

    pub fn set_length(&mut self, frames: u32) -> Result<(), String> {
        if !(1..=MAX_FRAMES).contains(&frames) {
            return Err(format!("Use 1–{MAX_FRAMES} frames."));
        }
        if frames < self.frames
            && (self.events.range(frames..).next().is_some()
                || self.cels.range(frames..).next().is_some()
                || self.cels.values().any(|doc| {
                    doc.skeleton
                        .foot_pins
                        .values()
                        .any(|track| track.range(frames..).next().is_some())
                        || doc
                            .skeleton
                            .bones
                            .iter()
                            .any(|bone| bone.rotation_keys.range(frames..).next().is_some())
                        || doc
                            .objects
                            .iter()
                            .any(|o| o.last_keyframe().is_some_and(|f| f >= frames))
                }))
        {
            return Err("Shortening the clip would discard drawings, keys or events. Delete those frames explicitly.".into());
        }
        if frames != self.frames {
            update_shared_drawings(
                self.cels.values_mut().filter(|doc| doc.frames != frames),
                |doc| {
                    doc.frames = frames;
                    Ok(())
                },
            )?;
            self.frames = frames;
        }
        Ok(())
    }

    pub fn set_exposure(&mut self, frame: u32, duration: u32) -> Result<(), String> {
        if frame >= self.frames || !(1..=MAX_FRAMES).contains(&duration) {
            return Err("Exposure duration or playhead is out of range.".into());
        }
        let (start, old) = self.exposure(frame);
        if duration > old {
            self.insert_ticks(start + old, duration - old)?;
        } else if duration < old {
            self.delete_range(start + duration, start + old - 1)?;
        }
        Ok(())
    }

    pub fn duplicate_frame(&mut self, frame: u32) -> Result<u32, String> {
        self.duplicate_range(frame, frame)
    }

    pub fn duplicate_range(&mut self, first: u32, last: u32) -> Result<u32, String> {
        if first > last || last >= self.frames {
            return Err("Invalid range to duplicate.".into());
        }
        let count = last - first + 1;
        if self.frames + count > MAX_FRAMES {
            return Err(format!("A clip can contain up to {MAX_FRAMES} frames."));
        }
        let at = last + 1;
        let mut copies = Vec::new();
        let layer_starts: Vec<_> = self
            .layer_starts
            .range(first + 1..at)
            .map(|(&tick, layers)| (tick + count, layers.clone()))
            .collect();
        // Preserve exposure runs, including a range that starts partway into a hold.
        let initial = self
            .cels
            .range(..=first)
            .next_back()
            .expect("validated clip starts at zero")
            .1;
        copies.push((first + count, Arc::clone(initial)));
        copies.extend(
            self.cels
                .range(first + 1..at)
                .map(|(&tick, doc)| (tick + count, Arc::clone(doc))),
        );
        update_shared_drawings(copies.iter_mut().map(|(_, doc)| doc), |doc| {
            retime(
                doc,
                Edit::Copy {
                    first,
                    last,
                    offset: count,
                },
                self.frames + count,
            )
        })?;
        let events: Vec<_> = self
            .events
            .range(first..=last)
            .map(|(&tick, names)| (tick + count, names.clone()))
            .collect();
        self.insert_drawings(at, count, copies)?;
        self.layer_starts.extend(layer_starts);
        self.events.extend(events);
        Ok(at)
    }

    pub fn blank_frame(&mut self, frame: u32) -> Result<u32, String> {
        if frame >= self.frames {
            return Err("Invalid playhead.".into());
        }
        let mut doc = self.drawing_at(frame).blank_drawing();
        let new_frames = doc.frames + 1;
        retime(
            &mut doc,
            Edit::Insert {
                at: frame + 1,
                count: 1,
            },
            new_frames,
        )?;
        self.insert_drawings(frame + 1, 1, vec![(frame + 1, Arc::new(doc))])?;
        Ok(frame + 1)
    }

    pub fn delete_frame(&mut self, frame: u32) -> Result<u32, String> {
        self.delete_range(frame, frame)
    }

    pub fn delete_range(&mut self, first: u32, last: u32) -> Result<u32, String> {
        self.delete_frame_ranges(&[(first, last)])
    }

    fn insert_drawings(
        &mut self,
        at: u32,
        count: u32,
        copies: Vec<(u32, Arc<Document>)>,
    ) -> Result<(), String> {
        let resume = at < self.frames && !self.cels.contains_key(&at);
        self.insert_ticks(at, count)?;
        if resume {
            let previous = self
                .cels
                .range(..at)
                .next_back()
                .expect("an inserted drawing follows an existing frame")
                .1
                .clone();
            self.cels.insert(at + count, previous);
        }
        // The copied interval starts and ends with a whole drawing, even when
        // the following original frame only had a boundary on one layer.
        self.layer_starts.remove(&(at + count));
        self.cels.extend(copies);
        Ok(())
    }

    fn insert_ticks(&mut self, at: u32, count: u32) -> Result<(), String> {
        if self.frames + count > MAX_FRAMES {
            return Err(format!("A clip can contain up to {MAX_FRAMES} frames."));
        }
        let mut cels = self.cels.clone();
        update_shared_drawings(cels.values_mut(), |doc| {
            retime(doc, Edit::Insert { at, count }, self.frames + count)
        })?;
        let tail = self.events.split_off(&at);
        self.events
            .extend(tail.into_iter().map(|(tick, names)| (tick + count, names)));
        let tail = self.layer_starts.split_off(&at);
        self.layer_index.take();
        self.layer_starts.extend(
            tail.into_iter()
                .map(|(tick, owners)| (tick + count, owners)),
        );
        self.frames += count;
        self.cels = cels
            .into_iter()
            .map(|(start, doc)| (if start >= at { start + count } else { start }, doc))
            .collect();
        Ok(())
    }
}

/// Apply a drawing-wide change once per shared snapshot. Each exposure still
/// owns its boundary, and older snapshots retain the original drawing.
fn update_shared_drawings<'a>(
    drawings: impl Iterator<Item = &'a mut Arc<Document>>,
    mut update: impl FnMut(&mut Document) -> Result<(), String>,
) -> Result<(), String> {
    let mut updated = HashMap::new();
    for doc in drawings {
        let source = Arc::as_ptr(doc);
        if let Some(previous) = updated.get(&source) {
            *doc = Arc::clone(previous);
        } else {
            let shared = Arc::strong_count(doc) > 1;
            update(Arc::make_mut(doc))?;
            if shared {
                updated.insert(source, Arc::clone(doc));
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum Edit<'a> {
    Insert { at: u32, count: u32 },
    Remove(&'a FrameRemoval),
    Copy { first: u32, last: u32, offset: u32 },
}

impl Edit<'_> {
    /// Inserting strictly after a track's final key leaves the entire track,
    /// including its outgoing easing, in exactly the same place.
    fn affects_keys(self, last: Option<u32>) -> bool {
        last.is_some_and(|last| !matches!(self, Self::Insert { at, .. } if at > last))
    }
}

/// Preserve sampled poses on both sides of a splice. Shifting key indices alone
/// would stretch a linear segment that crosses the edit (even on earlier frames).
fn retime(doc: &mut Document, edit: Edit<'_>, new_frames: u32) -> Result<(), String> {
    let old_frames = doc.frames;
    for o in &mut doc.objects {
        if let Some(instance) = &mut o.clip_instance {
            if edit.affects_keys(instance.drawing_keys.last_key_value().map(|(&t, _)| t)) {
                retime_drawing_keys(instance, edit, old_frames);
            }
        }
        if !crate::Property::ALL
            .into_iter()
            .any(|p| edit.affects_keys(p.last(o)))
        {
            continue;
        }
        let mut source = o.clone();
        source.animated.muted.clear();
        for channel in crate::Property::ALL {
            if !edit.affects_keys(channel.last(&source)) {
                continue;
            }
            let old = channel.frames(&source);
            let keys = old.iter().map(|&f| (f, f)).collect();
            let moved = remapped_track(&keys, |t| t, edit, old_frames);
            if channel == crate::Property::Geometry && source.animated.guided_geometry.is_some() {
                o.retime_guided_geometry(&source, &moved)?;
            } else {
                for t in old.iter().copied() {
                    channel.remove(o, t);
                }
                for (&dest, &src) in &moved {
                    channel.set(o, dest, channel.sample(&source, src));
                }
            }
            // Reparameterize each surviving portion of a nonlinear segment.
            for (&dest, &src) in &moved {
                let mut curve = channel.easing_at(&source, src);
                if let Some((&next_dest, &end)) = moved.range(dest + 1..).next() {
                    if end == src {
                        curve = crate::Easing::Hold;
                    } else if end.wrapping_sub(src) == next_dest - dest {
                        if let (Some((&lo, _)), Some((&hi, _))) =
                            (keys.range(..=src).next_back(), keys.range(src + 1..).next())
                        {
                            if end <= hi {
                                curve = channel.easing_at(&source, lo).portion(
                                    f64::from(src - lo) / f64::from(hi - lo),
                                    f64::from(end - lo) / f64::from(hi - lo),
                                );
                            }
                        }
                    }
                }
                channel.set_easing(o, dest, curve);
            }
        }
    }
    doc.frames = new_frames;
    let mut changed_rig = !doc.skeleton.foot_pins.is_empty();
    doc.skeleton.foot_pins.retain(|_, track| {
        retime_foot_pins(track, edit, old_frames);
        !track.is_empty()
    });
    for bone in &mut doc.skeleton.bones {
        if !bone.rotation_keys.is_empty() {
            bone.rotation_keys = remapped_track(
                &bone.rotation_keys,
                |t| bone.rotation_at(t),
                edit,
                old_frames,
            );
            changed_rig = true;
        }
    }
    if changed_rig {
        doc.refresh_rig()?;
    }
    Ok(())
}

fn remap_key_indices<T: Copy>(keys: &BTreeMap<u32, T>, edit: Edit<'_>) -> BTreeMap<u32, T> {
    let mut moved = BTreeMap::new();
    for (&frame, &value) in keys {
        let destination = match edit {
            Edit::Insert { at, count } => {
                if frame >= at {
                    frame + count
                } else {
                    frame
                }
            }
            Edit::Remove(removal) => {
                let (dest, kept) = removal.map(frame);
                if !kept {
                    continue;
                }
                dest
            }
            Edit::Copy {
                first,
                last,
                offset,
            } => {
                if (first..=last).contains(&frame) {
                    frame + offset
                } else {
                    continue;
                }
            }
        };
        moved.insert(destination, value);
    }
    moved
}

/// Drawing choices are stepped. Only create a boundary key if the surviving
/// exposure would otherwise show another drawing or revert to timed playback.
fn retime_drawing_keys(instance: &mut crate::ClipInstance, edit: Edit<'_>, old_frames: u32) {
    if instance.drawing_keys.is_empty() {
        return;
    }
    let mut moved = remap_key_indices(&instance.drawing_keys, edit);
    match edit {
        Edit::Copy { first, offset, .. } => {
            let value = instance.held_drawing(first);
            if value != 0 || moved.is_empty() {
                moved.entry(first + offset).or_insert(value);
            }
        }
        Edit::Remove(removal) => {
            for cut in removal.cuts.iter().filter(|c| c.end < old_frames) {
                let successor = instance.held_drawing(cut.end);
                let prior = moved.range(..=cut.dest).next_back().map_or(0, |(_, &v)| v);
                if prior != successor {
                    moved.insert(cut.dest, successor);
                }
            }
            if moved.is_empty() {
                moved.insert(0, 0);
            }
        }
        Edit::Insert { .. } => {}
    }
    instance.drawing_keys = moved;
}

/// An interval can start before the copied range, or end inside a deleted range.
/// Carry the contact at the surviving boundary without creating a key on every tick.
fn retime_foot_pins(
    track: &mut BTreeMap<u32, Option<crate::FootPin>>,
    edit: Edit<'_>,
    old_frames: u32,
) {
    if !edit.affects_keys(track.last_key_value().map(|(&t, _)| t)) {
        return;
    }
    let mut moved = remap_key_indices(track, edit);
    let mut preserve_contact = |source, dest| {
        let contact = track.range(..=source).next_back().and_then(|(_, pin)| *pin);
        moved.insert(dest, contact);
    };
    match edit {
        Edit::Copy { first, offset, .. } => preserve_contact(first, first + offset),
        Edit::Remove(removal) => {
            for cut in removal.cuts.iter().filter(|c| c.end < old_frames) {
                preserve_contact(cut.end, cut.dest);
            }
        }
        Edit::Insert { .. } => {}
    }
    let mut previous = None;
    moved.retain(|_, &mut contact| {
        let changed = contact != previous;
        previous = contact;
        changed
    });
    *track = moved;
}

fn remapped_track<T: Copy>(
    keys: &BTreeMap<u32, T>,
    evaluate: impl Fn(u32) -> T,
    edit: Edit<'_>,
    old_frames: u32,
) -> BTreeMap<u32, T> {
    if keys.is_empty() {
        return BTreeMap::new();
    }
    let mut moved = remap_key_indices(keys, edit);
    match edit {
        Edit::Copy {
            first,
            last,
            offset,
        } => {
            moved.insert(first + offset, evaluate(first));
            moved.insert(last + offset, evaluate(last));
        }
        Edit::Insert { at, count }
            if keys.range(..at).next().is_some() && keys.range(at..).next().is_some() =>
        {
            let value = evaluate(at - 1);
            moved.insert(at - 1, value);
            moved.insert(at + count - 1, value);
        }
        Edit::Remove(removal) => {
            for cut in &removal.cuts {
                let cuts_keys = keys.range(cut.start..cut.end).next().is_some();
                let cuts_segment = keys.range(..cut.start).next().is_some()
                    && keys.range(cut.end..).next().is_some();
                if cuts_keys || cuts_segment {
                    if cut.start > 0 {
                        moved.insert(cut.dest - 1, evaluate(cut.start - 1));
                    }
                    if cut.end < old_frames {
                        moved.insert(cut.dest, evaluate(cut.end));
                    }
                }
            }
        }
        _ => {}
    }
    moved
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Interpolation;

    fn animated() -> AnimationProject {
        let mut drawing = Document::default();
        drawing.add_rectangle([0.; 2], [8.; 2], [255; 4]);
        let o = &mut drawing.objects[0];
        o.keys.extend([(0, [0., 4.]), (4, [8., 0.])]);
        o.rotation_keys.extend([(0, 0.), (4, 180.)]);
        o.scale_keys.extend([(0, [1., 0.5]), (4, [2., 2.5])]);
        o.opacity_keys.extend([(0, 0.), (4, 1.)]);
        AnimationProject::from_document(drawing)
    }

    fn poses(clip: &Clip) -> Vec<[f32; 6]> {
        (0..clip.frames)
            .map(|f| {
                let obj = &clip.drawing_at(f).objects[0];
                let [x, y] = obj.position_at(f);
                let [sx, sy] = obj.scale_at(f);
                [x, y, sx, sy, obj.rotation_at(f), obj.opacity_at(f)]
            })
            .collect()
    }

    #[test]
    fn held_drawings_are_shared_until_split_and_duplicates_edit_independently() {
        let mut project = animated();
        project.duplicate_clip(0, "Variation").unwrap();
        let source = project.clips[0].drawing_at(0).clone();
        let clip = &mut project.clips[1];
        clip.drawing_at_mut(1).objects[0].fill = [1, 2, 3, 255];
        assert_eq!(clip.drawing_at(4).objects[0].fill, [1, 2, 3, 255]);
        let inserted = clip.duplicate_frame(1).unwrap();
        assert_eq!(inserted, 2);
        assert_eq!(clip.exposure(0), (0, 2));
        assert_eq!(clip.exposure(2), (2, 1));
        assert_eq!(clip.exposure(5), (3, 3));
        clip.drawing_at_mut(inserted).objects[0].fill = [77; 4];
        for f in [0, 1, 3, 4, 5] {
            assert_eq!(clip.drawing_at(f).objects[0].fill, [1, 2, 3, 255]);
        }
        assert_eq!(&source, project.clips[0].drawing_at(0));
        project.validate().unwrap();
    }

    #[test]
    fn duplicating_and_deleting_any_subrange_preserves_the_other_sampled_poses() {
        for interpolation in [Interpolation::Linear, Interpolation::Hold] {
            for start in 0..5 {
                for last in start..5 {
                    let mut clip = animated().clips.remove(0);
                    clip.drawing_at_mut(0).objects[0].interpolation = interpolation;
                    let original = poses(&clip);
                    clip.duplicate_range(start, last).unwrap();
                    let expected: Vec<_> = original[..=last as usize]
                        .iter()
                        .chain(&original[start as usize..=last as usize])
                        .chain(&original[last as usize + 1..])
                        .copied()
                        .collect();
                    assert_eq!(poses(&clip), expected);
                    clip.validate().unwrap();
                    clip.delete_range(last + 1, last + 1 + last - start)
                        .unwrap();
                    assert_eq!(poses(&clip), original);
                    clip.validate().unwrap();
                }
            }
        }
    }

    #[test]
    fn editing_frames_preserves_planted_contacts_and_releases() {
        let mut project = AnimationProject::default();
        let clip = &mut project.clips[0];
        clip.set_length(8).unwrap();
        let doc = clip.drawing_at_mut(0);
        let hip = doc.add_bone("Hips", None, [20., 20.], [23., 24.]).unwrap();
        let thigh = doc
            .add_bone("Thigh", Some(hip), [23., 24.], [32., 38.])
            .unwrap();
        let shin = doc
            .add_bone("Shin", Some(thigh), [32., 38.], [24., 52.])
            .unwrap();
        doc.set_bone_rotation_key(hip, 0, -10.).unwrap();
        doc.set_bone_rotation_key(hip, 7, 20.).unwrap();
        doc.pin_foot(shin, 2).unwrap();
        doc.release_foot_pin(shin, 6).unwrap();
        let snapshot = |clip: &Clip, tick| {
            let doc = clip.drawing_at(tick);
            (
                doc.foot_pin_at(shin, tick),
                doc.bone_segment_at(shin, true, tick).unwrap().1,
            )
        };
        let original: Vec<_> = (0..clip.frames).map(|t| snapshot(clip, t)).collect();
        let check = |clip: &Clip, ticks: &[usize]| {
            clip.validate().unwrap();
            assert_eq!(clip.frames as usize, ticks.len());
            for (tick, &source) in ticks.iter().enumerate() {
                let (contact, tip) = snapshot(clip, tick as u32);
                assert_eq!(contact, original[source].0);
                for (a, b) in tip.into_iter().zip(original[source].1) {
                    assert!((a - b).abs() < 0.001);
                }
            }
        };
        for first in 0..8 {
            for last in first..8 {
                let mut copy = clip.clone();
                copy.duplicate_range(first, last).unwrap();
                let frames: Vec<_> = (0..=last)
                    .chain(first..=last)
                    .chain(last + 1..8)
                    .map(|t| t as usize)
                    .collect();
                check(&copy, &frames);
                if first != 0 || last != 7 {
                    let mut trimmed = clip.clone();
                    trimmed.delete_range(first, last).unwrap();
                    let frames: Vec<_> =
                        (0..first).chain(last + 1..8).map(|t| t as usize).collect();
                    check(&trimmed, &frames);
                }
            }
        }
    }

    #[test]
    fn extending_a_planted_hold_and_starting_a_new_clip_preserve_the_pose() {
        let mut project = AnimationProject::default();
        let clip = &mut project.clips[0];
        clip.set_length(8).unwrap();
        let doc = clip.drawing_at_mut(0);
        let leg = doc.add_bone("Thigh", None, [20., 20.], [22., 32.]).unwrap();
        let shin = doc
            .add_bone("Shin", Some(leg), [22., 32.], [20., 45.])
            .unwrap();
        doc.pin_foot(shin, 5).unwrap();
        assert!(
            clip.set_length(5).is_err(),
            "shortening would lose the only pin event"
        );
        clip.set_length(1000).unwrap();
        let doc = clip.drawing_at_mut(0);
        assert_eq!(doc.foot_pin_at(shin, 999), doc.foot_pin_at(shin, 5));
        doc.pin_foot(shin, 0).unwrap();
        // Change the authored angle behind the pin. A new clip must start
        // with the evaluated leg pose that was visible on the first frame.
        doc.set_bone_rotation_key(shin, 0, 35.).unwrap();
        let pose = doc.bone_segment_at(shin, true, 0).unwrap();
        let new = project.add_clip("Fresh pose").unwrap();
        let doc = project.clips[new].drawing_at(0);
        assert!(doc.skeleton.foot_pins.is_empty());
        assert_eq!(doc.bone_segment_at(shin, true, 0).unwrap(), pose);
    }

    #[test]
    fn exposure_edits_keep_the_following_drawing_and_its_timing() {
        let mut clip = animated().clips.remove(0);
        let blank = clip.blank_frame(1).unwrap();
        assert_eq!(blank, 2);
        assert!(clip.drawing_at(blank).objects.is_empty());
        assert_eq!(clip.exposure(3), (3, 3));
        let original = clip.drawing_at(3).objects[0].position_at(3);
        clip.set_exposure(blank, 4).unwrap();
        assert_eq!(clip.exposure(5), (2, 4));
        assert_eq!(clip.drawing_at(6).objects[0].position_at(6), original);
        clip.set_exposure(4, 2).unwrap();
        assert_eq!(clip.drawing_at(4).objects[0].position_at(4), original);
        // Delete from the previous drawing into a held blank. Its last tick survives.
        clip.delete_range(0, 2).unwrap();
        assert!(clip.drawing_at(0).objects.is_empty());
        assert_eq!(clip.drawing_at(1).objects[0].position_at(1), original);
        clip.validate().unwrap();
    }

    #[test]
    fn blank_splice_keeps_finished_tracks_and_artboard_settings() {
        let mut clip = animated().clips.remove(0);
        let doc = clip.drawing_at_mut(0);
        doc.guides_x.push(4.);
        doc.guides_y.push(7.);
        doc.add_bone("Arm", None, [4., 4.], [10., 4.]).unwrap();
        let o = &mut doc.objects[0];
        o.keys = [(0, [1., 1.]), (1, [5., 2.])].into();
        // An explicit final curve remains authored even if it currently
        // matches the object's default interpolation.
        o.animated
            .easing
            .entry(crate::Property::Position)
            .or_default()
            .insert(1, crate::Easing::Linear);
        let keys = o.keys.clone();
        let easing = o.animated.easing[&crate::Property::Position].clone();
        let old = poses(&clip);
        let settings = clip.drawing_at(0).clone();
        let blank = clip.blank_frame(2).unwrap();
        let new = clip.drawing_at(blank);
        assert!(new.objects.is_empty());
        assert_eq!(new.objects.capacity(), 0);
        assert_eq!(new.next_id, 1);
        assert_eq!(new.layers, settings.layers);
        assert_eq!(new.skeleton, settings.skeleton);
        assert_eq!(new.guides_x, settings.guides_x);
        assert_eq!(new.guides_y, settings.guides_y);
        for (tick, expected) in old.into_iter().enumerate() {
            let tick = tick as u32 + u32::from(tick >= blank as usize);
            let o = &clip.drawing_at(tick).objects[0];
            assert_eq!(o.keys, keys);
            assert_eq!(o.animated.easing[&crate::Property::Position], easing);
            let [x, y] = o.position_at(tick);
            let [sx, sy] = o.scale_at(tick);
            assert_eq!(
                [x, y, sx, sy, o.rotation_at(tick), o.opacity_at(tick)],
                expected
            );
        }
        clip.validate().unwrap();
    }

    #[test]
    fn invalid_requests_leave_the_project_unchanged() {
        let mut p = animated();
        let original = p.clone();
        assert!(p.add_clip("animation").is_err());
        assert!(p.remove_clip(0).is_err());
        let c = &mut p.clips[0];
        assert!(c.set_length(3).is_err());
        assert!(c.duplicate_frame(5).is_err());
        assert!(c.set_exposure(0, MAX_FRAMES + 1).is_err());
        assert!(c.delete_range(0, 4).is_err());
        assert_eq!(p, original);
    }

    #[test]
    fn snapshot_comparison_covers_shared_and_independently_loaded_art() {
        let mut project = AnimationProject::default();
        project.clips[0]
            .drawing_at_mut(0)
            .add_rectangle([1., 2.], [5., 6.], [255; 4]);
        let original = project.clips[0].cels[&0].clone();
        let mut independent = project.clone();
        independent.clips[0]
            .cels
            .insert(0, Arc::new((*original).clone()));
        assert_eq!(project, independent);
        independent.clips[0].drawing_at_mut(0).objects[0].size[0] += 1.;
        assert_ne!(project, independent);
        independent = project.clone();
        independent.clips[0].cels.insert(1, original.clone());
        assert_ne!(project, independent);
        let definition = crate::SymbolDefinition {
            name: "Bird".into(),
            artwork: original.clone(),
        };
        let other = crate::SymbolDefinition {
            name: "Bird".into(),
            artwork: Arc::new((*original).clone()),
        };
        assert_eq!(definition, other);
        let a = crate::SymbolInstance::new(1, original.clone());
        let b = crate::SymbolInstance::new(1, other.artwork);
        assert_eq!(a, b);
        assert_ne!(a, crate::SymbolInstance::new(2, original));
    }
}
