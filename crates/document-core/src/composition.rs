//! Project references form a bounded directed acyclic graph. Hydrated drawings can be
//! evaluated by the renderer, SVG writer and picking without an editor or project handle.
use crate::{AnimationProject, Clip, Document, Shape, VectorObject};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

#[cfg(test)]
mod drawing_tests;
mod import;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LoopMode {
    #[default]
    Loop,
    Clamp,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClipTiming {
    /// Phase in child ticks at parent frame zero.
    pub offset: i32,
    pub rate_numerator: u16,
    pub rate_denominator: u16,
    pub mode: LoopMode,
}
impl Default for ClipTiming {
    fn default() -> Self {
        Self {
            offset: 0,
            rate_numerator: 1,
            rate_denominator: 1,
            mode: LoopMode::Loop,
        }
    }
}
impl ClipTiming {
    fn valid(self) -> bool {
        self.rate_numerator > 0 && self.rate_denominator > 0
    }
    pub fn sample(self, parent_tick: u32, parent_fps: u32, child: &Clip) -> u32 {
        let denom = i64::from(parent_fps.max(1)) * i64::from(self.rate_denominator.max(1));
        let scaled =
            i64::from(parent_tick) * i64::from(child.fps) * i64::from(self.rate_numerator) / denom;
        let tick = scaled + i64::from(self.offset);
        let length = i64::from(child.frames.max(1));
        (match self.mode {
            LoopMode::Loop => tick.rem_euclid(length),
            LoopMode::Clamp => tick.clamp(0, length - 1),
        }) as u32
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClipInstance {
    pub clip_id: u64,
    pub timing: ClipTiming,
    /// Original artboard dimensions when a project resize changes the child viewport.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_size: Option<[u32; 2]>,
    /// Parent tick to held child tick. An empty track uses normal clip playback.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub drawing_keys: BTreeMap<u32, u32>,
    #[serde(skip)]
    cached: Option<Arc<Clip>>,
    #[serde(skip)]
    parent_fps: u32,
}
impl PartialEq for ClipInstance {
    fn eq(&self, o: &Self) -> bool {
        self.clip_id == o.clip_id
            && self.timing == o.timing
            && self.source_size == o.source_size
            && self.drawing_keys == o.drawing_keys
            && self.parent_fps == o.parent_fps
            && match (&self.cached, &o.cached) {
                (Some(a), Some(b)) => {
                    Arc::ptr_eq(a, b)
                        || (a.id == b.id
                            && a.frames == b.frames
                            && a.fps == b.fps
                            && a.cels.len() == b.cels.len()
                            && a.cels.iter().zip(&b.cels).all(|((i, x), (j, y))| {
                                i == j && crate::animation::same_drawing(x, y)
                            }))
                }
                (None, None) => true,
                _ => false,
            }
    }
}
impl ClipInstance {
    /// A shortened source clip holds its last surviving frame. Keeping the
    /// authored tick allows extending the child to restore the original pose.
    pub fn sample(&self, frame: u32) -> Option<(&Document, u32)> {
        let clip = self.cached.as_ref()?;
        let t = if self.drawing_keys.is_empty() {
            self.timing.sample(frame, self.parent_fps, clip)
        } else {
            self.held_drawing(frame).min(clip.frames.saturating_sub(1))
        };
        Some((clip.drawing_at(t), t))
    }

    pub(crate) fn held_drawing(&self, tick: u32) -> u32 {
        self.drawing_keys
            .range(..=tick)
            .next_back()
            .map_or(0, |(_, &source)| source)
    }

    pub fn validate_timing(&self, parent_frames: u32) -> Result<(), String> {
        if self
            .source_size
            .is_some_and(|size| size.iter().any(|n| !(1..=crate::MAX_DIMENSION).contains(n)))
        {
            return Err("Invalid original artboard size for a nested clip.".into());
        }
        if !self.timing.valid() {
            return Err("Clip playback speed must be positive.".into());
        }
        if self
            .drawing_keys
            .last_key_value()
            .is_some_and(|(&f, _)| f >= parent_frames)
            || self
                .drawing_keys
                .values()
                .any(|&tick| tick >= crate::MAX_FRAMES)
        {
            return Err("A drawing key contains an invalid parent or source frame.".into());
        }
        Ok(())
    }

    pub fn shared_clip(&self) -> Option<&Arc<Clip>> {
        self.cached.as_ref()
    }
}

impl Document {
    fn editable_clip_instance(&mut self, object: u64) -> Result<&mut ClipInstance, String> {
        let index = self.editable_object_index(object)?;
        self.objects[index]
            .clip_instance
            .as_mut()
            .ok_or_else(|| "Select a nested clip placement.".into())
    }

    pub fn set_drawing_key(
        &mut self,
        object: u64,
        frame: u32,
        source_frame: u32,
    ) -> Result<(), String> {
        if frame >= self.frames {
            return Err("Choose a frame inside the parent clip.".into());
        }
        let instance = self.editable_clip_instance(object)?;
        let child = instance
            .shared_clip()
            .ok_or("Resolve the nested clip before editing its drawings.")?;
        if source_frame >= child.frames {
            return Err("Choose an existing frame in the source clip.".into());
        }
        instance.drawing_keys.insert(frame, source_frame);
        Ok(())
    }

    pub fn move_drawing_key(&mut self, object: u64, from: u32, to: u32) -> Result<(), String> {
        if from >= self.frames || to >= self.frames {
            return Err("Choose frames inside the parent clip.".into());
        }
        let keys = &mut self.editable_clip_instance(object)?.drawing_keys;
        let source = *keys
            .get(&from)
            .ok_or("The source drawing key no longer exists.")?;
        if from != to {
            if keys.contains_key(&to) {
                return Err("There is already a drawing key at this frame.".into());
            }
            keys.remove(&from);
            keys.insert(to, source);
        }
        Ok(())
    }

    pub fn remove_drawing_key(&mut self, object: u64, frame: u32) -> Result<(), String> {
        self.editable_clip_instance(object)?
            .drawing_keys
            .remove(&frame)
            .ok_or("There is no drawing key at this frame.")?;
        Ok(())
    }

    pub fn clear_drawing_keys(&mut self, object: u64) -> Result<(), String> {
        self.editable_clip_instance(object)?.drawing_keys.clear();
        Ok(())
    }
}
impl VectorObject {
    pub(crate) fn placement_source_size(&self, art: &Document) -> [f32; 2] {
        self.clip_instance
            .as_ref()
            .and_then(|instance| instance.source_size)
            .unwrap_or([art.width, art.height])
            .map(|n| n as f32)
    }

    pub fn conflicting_placements(&self) -> bool {
        (self.symbol.is_some() && self.embedded_artwork.is_some())
            || (self.clip_instance.is_some()
                && (self.symbol.is_some() || self.embedded_artwork.is_some()))
    }
    pub fn is_instance(&self) -> bool {
        self.symbol.is_some() || self.clip_instance.is_some() || self.embedded_artwork.is_some()
    }
    pub fn static_artwork(&self) -> Option<&Arc<Document>> {
        self.embedded_artwork
            .as_deref()
            .or_else(|| self.symbol.as_ref()?.shared_artwork())
    }
    pub fn instance_at(&self, frame: u32) -> Option<(&Document, u32)> {
        if let Some(instance) = &self.clip_instance {
            instance.sample(frame)
        } else {
            self.static_artwork().map(|a| (a.as_ref(), 0))
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Ref {
    Symbol(u64),
    Clip(u64),
}
fn reference(o: &VectorObject) -> Option<Ref> {
    o.symbol
        .as_ref()
        .map(|s| Ref::Symbol(s.definition))
        .or_else(|| o.clip_instance.as_ref().map(|s| Ref::Clip(s.clip_id)))
}
#[derive(Default)]
struct Resolver {
    symbols: BTreeMap<u64, Arc<Document>>,
    clips: BTreeMap<u64, Arc<Clip>>,
    embedded: HashMap<*const Document, (Arc<Document>, (usize, usize))>,
    embedded_depth: usize,
    active: HashSet<Ref>,
    // Maximum expanded object count at a tick and longest path below each node.
    costs: HashMap<Ref, (usize, usize)>,
}
impl Resolver {
    fn resolve(&mut self, r: Ref, p: &AnimationProject) -> Result<(usize, usize), String> {
        if let Some(cost) = self.costs.get(&r) {
            return Ok(*cost);
        }
        if self.active.len() + self.embedded_depth >= 16 {
            return Err("Composition exceeds 16 nesting levels.".into());
        }
        if !self.active.insert(r) {
            return Err("This placement would create a composition cycle.".into());
        }
        let mut max_cost = 0;
        let mut max_depth = 1;
        match r {
            Ref::Symbol(id) => {
                let mut doc = p
                    .symbols
                    .get(&id)
                    .ok_or("A symbol definition is missing.")?
                    .artwork
                    .clone();
                if doc.objects.iter().any(|o| o.clip_instance.is_some()) {
                    return Err("Static symbol definitions can contain other symbols. Place animated clips on a timeline.".into());
                }
                (max_cost, max_depth) = self.resolve_drawing(&mut doc, p)?;
                self.symbols.insert(id, doc);
            }
            Ref::Clip(id) => {
                let index = p
                    .clip_index(id)
                    .ok_or("A nested clip reference is missing.")?;
                let mut clip = p.clips[index].clone();
                // The source project owns these immutable drawings throughout this
                // resolution. Held copies share their refreshed placements and rig.
                let mut drawings = (clip.cels.len() > 1).then(HashMap::new);
                for doc in clip.cels.values_mut() {
                    let source = Arc::as_ptr(doc);
                    if let Some(resolved) = drawings.as_ref().and_then(|seen| seen.get(&source)) {
                        *doc = Arc::clone(resolved);
                        continue;
                    }
                    let (cost, depth) = self.resolve_drawing(doc, p)?;
                    max_cost = max_cost.max(cost);
                    max_depth = max_depth.max(depth);
                    if let Some(seen) = &mut drawings {
                        seen.insert(source, Arc::clone(doc));
                    }
                }
                self.clips.insert(id, Arc::new(clip));
            }
        }
        self.active.remove(&r);
        self.costs.insert(r, (max_cost, max_depth));
        Ok((max_cost, max_depth))
    }
    fn resolve_embedded(
        &mut self,
        source: &Arc<Document>,
        p: &AnimationProject,
    ) -> Result<(Arc<Document>, (usize, usize)), String> {
        let key = Arc::as_ptr(source);
        if let Some(cached) = self.embedded.get(&key) {
            return Ok(cached.clone());
        }
        if self.active.len() + self.embedded_depth >= 16 {
            return Err("Composition exceeds 16 nesting levels.".into());
        }
        self.embedded_depth += 1;
        let mut artwork = source.clone();
        let cost = self.resolve_drawing(&mut artwork, p)?;
        self.embedded_depth -= 1;
        crate::SymbolDefinition::validate_artwork(&artwork)?;
        self.embedded.insert(key, (artwork.clone(), cost));
        Ok((artwork, cost))
    }
    fn resolve_drawing(
        &mut self,
        doc: &mut Arc<Document>,
        p: &AnimationProject,
    ) -> Result<(usize, usize), String> {
        if doc.uses_rig() && !doc.rig_is_current()? {
            Arc::make_mut(doc).refresh_rig()?;
        }
        let mut count = doc.objects.len();
        let mut depth = 1;
        let mut stale = false;
        let mut embedded_updates = Vec::new();
        for (index, o) in doc.objects.iter().enumerate() {
            if o.conflicting_placements() {
                return Err("A placement must have a single artwork source.".into());
            }
            let (cost, child_depth) = if let Some(source) = &o.embedded_artwork {
                let (art, cost) = self.resolve_embedded(source, p)?;
                if !Arc::ptr_eq(source, &art) {
                    embedded_updates.push((index, art));
                }
                cost
            } else if let Some(r) = reference(o) {
                let cost = self.resolve(r, p)?;
                match r {
                    Ref::Symbol(id) => {
                        stale |= o
                            .symbol
                            .as_ref()
                            .unwrap()
                            .shared_artwork()
                            .is_none_or(|a| !Arc::ptr_eq(a, &self.symbols[&id]))
                    }
                    Ref::Clip(id) => {
                        let place = o.clip_instance.as_ref().unwrap();
                        place.validate_timing(doc.frames)?;
                        stale |= place.parent_fps != doc.fps
                            || place
                                .cached
                                .as_ref()
                                .is_none_or(|c| !same_clip(c, &self.clips[&id]));
                    }
                }
                cost
            } else {
                continue;
            };
            count = count.saturating_add(cost);
            depth = depth.max(1 + child_depth);
            if depth > 16 || count > 262144 {
                return Err("Composition exceeds the nested artwork budget.".into());
            }
        }
        if !embedded_updates.is_empty() {
            let doc = Arc::make_mut(doc);
            for (index, art) in embedded_updates {
                doc.objects[index].embedded_artwork = Some(art.into());
            }
        }
        if stale {
            let doc = Arc::make_mut(doc);
            for o in &mut doc.objects {
                if let Some(s) = &mut o.symbol {
                    s.bind(self.symbols[&s.definition].clone());
                }
                if let Some(c) = &mut o.clip_instance {
                    c.cached = Some(self.clips[&c.clip_id].clone());
                    c.parent_fps = doc.fps;
                }
            }
        }
        Ok((count, depth))
    }
}
// Only fields that affect sampled artwork need to invalidate a nested placement cache.
fn same_clip(a: &Clip, b: &Clip) -> bool {
    a.id == b.id
        && a.fps == b.fps
        && a.frames == b.frames
        && a.cels.len() == b.cels.len()
        && a.cels
            .iter()
            .zip(&b.cels)
            .all(|((a, x), (b, y))| a == b && Arc::ptr_eq(x, y))
}
impl AnimationProject {
    pub fn has_clip_instances(&self) -> bool {
        self.clips
            .iter()
            .flat_map(|c| c.cels.values())
            .any(|d| d.objects.iter().any(|o| o.clip_instance.is_some()))
    }
    pub fn hydrate_composition(&mut self) -> Result<(), String> {
        let mut r = Resolver::default();
        for id in self.symbols.keys() {
            r.resolve(Ref::Symbol(*id), self)?;
        }
        for c in &self.clips {
            r.resolve(Ref::Clip(c.id), self)?;
        }
        for (id, def) in &mut self.symbols {
            def.artwork = r.symbols.remove(id).unwrap();
        }
        for clip in &mut self.clips {
            clip.cels = r.clips[&clip.id].cels.clone();
        }
        Ok(())
    }
    pub fn resolved_snapshot(&self) -> Result<Self, String> {
        let mut snapshot = self.clone();
        snapshot.hydrate_composition()?;
        Ok(snapshot)
    }
    pub fn clip_usage(&self, id: u64) -> usize {
        self.clips
            .iter()
            .flat_map(|c| c.cels.values())
            .flat_map(|d| &d.objects)
            .filter(|o| o.clip_instance.as_ref().is_some_and(|i| i.clip_id == id))
            .count()
    }
    pub fn place_clip(
        &mut self,
        source: u64,
        parent: usize,
        tick: u32,
        layer: u64,
        position: [f32; 2],
    ) -> Result<u64, String> {
        let child = &self.clips[self
            .clip_index(source)
            .ok_or("Choose an existing child clip.")?];
        let scene = child.drawing_at(0);
        let size = [scene.width as f32, scene.height as f32];
        let mut staged = self.clone();
        let clip = staged
            .clips
            .get_mut(parent)
            .ok_or("Choose a parent clip.")?;
        if tick >= clip.frames || !position.iter().all(|x| x.is_finite()) {
            return Err("Invalid clip placement.".into());
        }
        let doc = clip.drawing_at_mut(tick);
        if !doc
            .layers
            .iter()
            .any(|l| l.id == layer && l.visible && !l.locked)
        {
            return Err("Choose an unlocked visible layer.".into());
        }
        let id = doc.add_shape(position, size, [0; 4], Shape::Rectangle, layer);
        let placed = doc.objects.last_mut().unwrap();
        placed.name.clone_from(&child.name);
        placed.clip_instance = Some(ClipInstance {
            clip_id: source,
            timing: ClipTiming::default(),
            source_size: None,
            drawing_keys: BTreeMap::new(),
            cached: None,
            parent_fps: doc.fps,
        });
        clip.sync_layer_edits(&self.clips[parent], tick)?;
        staged.hydrate_composition()?;
        *self = staged;
        Ok(id)
    }
    pub fn nest_symbol(&mut self, parent: u64, child: u64) -> Result<u64, String> {
        let art = self
            .symbols
            .get(&child)
            .ok_or("Unknown child symbol.")?
            .artwork
            .clone();
        let mut draft = self
            .symbols
            .get(&parent)
            .ok_or("Unknown symbol.")?
            .artwork
            .as_ref()
            .clone();
        let layer = draft
            .layers
            .iter()
            .find(|l| l.visible && !l.locked)
            .ok_or("Choose an unlocked visible layer.")?
            .id;
        let id = draft.add_shape(
            [0., 0.],
            [art.width as f32, art.height as f32],
            [0; 4],
            Shape::Rectangle,
            layer,
        );
        draft.objects.last_mut().unwrap().symbol = Some(crate::SymbolInstance::new(child, art));
        self.replace_symbol_artwork(parent, draft)?;
        Ok(id)
    }
}
