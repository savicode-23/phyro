//! Static shared artwork. Native instances persist only their definition ID. A hydrated
//! reference lets the existing drawing, hit-test and export APIs evaluate standalone cels.
use crate::{AnimationProject, Document, Shape, VectorObject, MAX_DIMENSION};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use tiny_skia_path::Transform;

mod detach;
pub use detach::EmbeddedArtwork;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SymbolDefinition {
    pub name: String,
    pub artwork: Arc<Document>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SymbolInstance {
    pub definition: u64,
    #[serde(skip)]
    cached: Option<Arc<Document>>,
}

impl PartialEq for SymbolDefinition {
    fn eq(&self, other: &Self) -> bool {
        let Self { name, artwork } = self;
        name == &other.name && crate::animation::same_drawing(artwork, &other.artwork)
    }
}

impl PartialEq for SymbolInstance {
    fn eq(&self, other: &Self) -> bool {
        let Self { definition, cached } = self;
        *definition == other.definition
            && match (cached, &other.cached) {
                (Some(a), Some(b)) => crate::animation::same_drawing(a, b),
                (None, None) => true,
                _ => false,
            }
    }
}

impl SymbolInstance {
    pub fn new(definition: u64, artwork: Arc<Document>) -> Self {
        Self {
            definition,
            cached: Some(artwork),
        }
    }
    pub fn artwork(&self) -> Option<&Document> {
        self.cached.as_deref()
    }
    pub(crate) fn bind(&mut self, art: Arc<Document>) {
        self.cached = Some(art);
    }
    pub fn shared_artwork(&self) -> Option<&Arc<Document>> {
        self.cached.as_ref()
    }
}

impl VectorObject {
    /// Maps artwork-local pixels into scene pixels, including the instance's animated transform.
    pub fn symbol_transform(&self, frame: u32) -> Option<Transform> {
        let (art, _) = self.instance_at(frame)?;
        let [w, h] = self.placement_source_size(art);
        let world = self.local_to_world_at(frame);
        let origin = world([0., 0.]);
        let right = world([1., 0.]);
        let bottom = world([0., 1.]);
        Some(Transform::from_row(
            (right[0] - origin[0]) / w,
            (right[1] - origin[1]) / w,
            (bottom[0] - origin[0]) / h,
            (bottom[1] - origin[1]) / h,
            origin[0],
            origin[1],
        ))
    }
}

impl SymbolDefinition {
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty()
            || self.name.trim() != self.name
            || self.name.chars().count() > 80
            || self.name.chars().any(char::is_control)
        {
            return Err("Give the symbol a name of 1–80 printable characters.".into());
        }
        Self::validate_artwork(&self.artwork)
    }

    pub(super) fn validate_artwork(artwork: &Document) -> Result<(), String> {
        if artwork.frames != 1
            || artwork.skeleton.has_animation()
            || artwork
                .objects
                .iter()
                .any(|o| o.clip_instance.is_some() || o.last_keyframe().is_some())
        {
            return Err(
                "Symbols can contain static artwork and linked symbols. Animate a nested clip on a timeline.".into(),
            );
        }
        artwork.validate()
    }

    /// Capture the appearance at a single tick, in visual stacking order. Source tracks stay editable.
    pub fn capture(
        name: &str,
        doc: &Document,
        frame: u32,
        selection: &[u64],
    ) -> Result<(Self, [f32; 2]), String> {
        if frame >= doc.frames {
            return Err("Choose a frame inside the clip.".into());
        }
        let selected: HashSet<_> = selection.iter().copied().collect();
        let mut objects: Vec<_> = doc
            .paint_order()
            .into_iter()
            .filter(|o| selected.contains(&o.id))
            .cloned()
            .collect();
        if objects.is_empty() || objects.len() != selected.len() {
            return Err("Select visible artwork to make a symbol.".into());
        }
        if objects.iter().any(|o| o.clip_instance.is_some()) {
            return Err(
                "Place animated clips on a timeline. Static symbols can contain other symbols."
                    .into(),
            );
        }
        let mut bounds = [
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        ];
        // Preserve named groups and group masks when flattening layers into symbol coordinates.
        let mut groups = BTreeMap::new();
        for object in &mut objects {
            let b = object.bounds(frame);
            bounds = [
                bounds[0].min(b[0]),
                bounds[1].min(b[1]),
                bounds[2].max(b[2]),
                bounds[3].max(b[3]),
            ];
            object.shape = object.skinned_shape_at(frame).into_owned();
            object.skin = None;
            let position = object.position_at(frame);
            object.scale = object.scale_at(frame);
            object.rotation = object.rotation_at(frame);
            object.rigid_bone = None;
            object.rig_pose = Default::default();
            object.rig_offset = Default::default();
            object.opacity = object.opacity_at(frame);
            object.fill = object.fill_at(frame);
            object.stroke = object.stroke_at(frame);
            object.stroke_width = object.stroke_width_at(frame);
            let skew = object.skew_at(frame);
            let visible = object.visible_at(frame);
            object.animated = crate::AnimatedProperties {
                skew,
                visible,
                ..Default::default()
            };
            object.motion = None;
            object.keys.clear();
            object.rotation_keys.clear();
            object.scale_keys.clear();
            object.opacity_keys.clear();
            object.set_position_at(0, position);
            object.group = object
                .group
                .filter(|_| {
                    object.group_name.is_some()
                        || object
                            .clip_source
                            .as_ref()
                            .is_some_and(|b| b.source.grouped)
                })
                .map(|group| {
                    let next = groups.len() as u64 + 1;
                    *groups.entry((object.layer, group)).or_insert(next)
                });
            object.layer = 1;
        }
        let origin = [bounds[0].floor(), bounds[1].floor()];
        let w = (bounds[2].ceil() - origin[0]).max(1.);
        let h = (bounds[3].ceil() - origin[1]).max(1.);
        if !w.is_finite() || !h.is_finite() || w > MAX_DIMENSION as f32 || h > MAX_DIMENSION as f32
        {
            return Err(format!(
                "Symbol artwork must fit within {MAX_DIMENSION} × {MAX_DIMENSION} pixels."
            ));
        }
        for (i, o) in objects.iter_mut().enumerate() {
            o.position[0] -= origin[0];
            o.position[1] -= origin[1];
            o.id = i as u64 + 1;
        }
        let artwork = Document {
            width: w as u32,
            height: h as u32,
            frames: 1,
            next_id: objects.len() as u64 + 1,
            objects,
            ..Document::default()
        };
        let def = Self {
            name: name.trim().to_owned(),
            artwork: Arc::new(artwork),
        };
        def.validate()?;
        Ok((def, origin))
    }
}

impl AnimationProject {
    /// Replace a contiguous selection with one instance in the same stacking position.
    pub fn convert_to_symbol(
        &mut self,
        clip: usize,
        frame: u32,
        selection: &[u64],
        name: &str,
    ) -> Result<(u64, u64), String> {
        let mut staged = self.clone();
        let result = staged.convert_selection(clip, frame, selection, name)?;
        staged.clips[clip].sync_layer_edits(&self.clips[clip], frame)?;
        *self = staged;
        Ok(result)
    }
    fn convert_selection(
        &mut self,
        clip: usize,
        frame: u32,
        selection: &[u64],
        name: &str,
    ) -> Result<(u64, u64), String> {
        let c = self.clips.get(clip).ok_or("Unknown clip.")?;
        if frame >= c.frames {
            return Err("Choose a valid frame.".into());
        }
        let doc = c.drawing_at(frame);
        let (definition, origin) = SymbolDefinition::capture(name, doc, frame, selection)?;
        let ids: HashSet<_> = selection.iter().copied().collect();
        let selected: Vec<_> = doc
            .objects
            .iter()
            .enumerate()
            .filter(|(_, o)| ids.contains(&o.id))
            .collect();
        let (at, first) = selected[0];
        let layer = first.layer;
        if !doc
            .layers
            .iter()
            .any(|l| l.id == layer && l.visible && !l.locked)
            || selected.iter().any(|(_, o)| {
                o.layer != layer || o.locked || o.last_keyframe().is_some() || o.has_animated_rig()
            })
            || doc.objects[at..=selected.last().unwrap().0]
                .iter()
                .any(|o| o.layer == layer && !ids.contains(&o.id))
        {
            return Err("Convert static, adjacent objects on one unlocked layer. Save a copy to sample any visible selection.".into());
        }
        let id = self.add_symbol(definition)?;
        let placed = self.place_symbol(id, clip, frame, layer, origin)?;
        let doc = self.clips[clip].drawing_at_mut(frame);
        let instance = doc.objects.pop().unwrap();
        doc.objects.retain(|o| !ids.contains(&o.id));
        doc.objects.insert(at, instance);
        Ok((id, placed))
    }
    pub fn add_symbol(&mut self, definition: SymbolDefinition) -> Result<u64, String> {
        definition.validate()?;
        if self.symbols.len() >= 1024 {
            return Err("A project can hold up to 1024 symbols.".into());
        }
        self.check_symbol_name(&definition.name, None)?;
        let id = (1..).find(|id| !self.symbols.contains_key(id)).unwrap();
        let mut staged = self.clone();
        staged.symbols.insert(id, definition);
        staged.hydrate_composition()?;
        *self = staged;
        Ok(id)
    }
    fn check_symbol_name(&self, name: &str, skip: Option<u64>) -> Result<(), String> {
        if self
            .symbols
            .iter()
            .any(|(&id, d)| Some(id) != skip && d.name.to_lowercase() == name.to_lowercase())
        {
            return Err("A symbol with this name already exists.".into());
        }
        Ok(())
    }
    pub fn rename_symbol(&mut self, id: u64, name: &str) -> Result<(), String> {
        let name = name.trim();
        self.check_symbol_name(name, Some(id))?;
        let def = self.symbols.get_mut(&id).ok_or("Unknown symbol.")?;
        let updated = SymbolDefinition {
            name: name.into(),
            artwork: def.artwork.clone(),
        };
        updated.validate()?;
        *def = updated;
        Ok(())
    }
    pub fn place_symbol(
        &mut self,
        id: u64,
        clip: usize,
        frame: u32,
        layer: u64,
        position: [f32; 2],
    ) -> Result<u64, String> {
        let mut staged = self.clone();
        let placed = staged.insert_symbol(id, clip, frame, layer, position)?;
        staged.clips[clip].sync_layer_edits(&self.clips[clip], frame)?;
        staged.hydrate_composition()?;
        *self = staged;
        Ok(placed)
    }
    fn insert_symbol(
        &mut self,
        id: u64,
        clip: usize,
        frame: u32,
        layer: u64,
        position: [f32; 2],
    ) -> Result<u64, String> {
        let def = self.symbols.get(&id).ok_or("Unknown symbol.")?;
        let c = self.clips.get_mut(clip).ok_or("Unknown clip.")?;
        if frame >= c.frames || !position.into_iter().all(f32::is_finite) {
            return Err("Invalid symbol placement.".into());
        }
        let doc = c.drawing_at_mut(frame);
        if !doc
            .layers
            .iter()
            .any(|l| l.id == layer && l.visible && !l.locked)
        {
            return Err("Select an unlocked, visible destination layer.".into());
        }
        let size = [def.artwork.width as f32, def.artwork.height as f32];
        let object = doc.add_shape(position, size, [0; 4], Shape::Rectangle, layer);
        doc.objects.last_mut().unwrap().symbol = Some(SymbolInstance::new(id, def.artwork.clone()));
        Ok(object)
    }
    pub fn replace_symbol_artwork(&mut self, id: u64, artwork: Document) -> Result<(), String> {
        let mut staged = self.clone();
        let def = staged.symbols.get_mut(&id).ok_or("Unknown symbol.")?;
        def.artwork = Arc::new(artwork);
        staged.hydrate_composition()?;
        staged.symbols[&id].validate()?;
        *self = staged;
        Ok(())
    }
    pub fn symbol_usage(&self, id: u64) -> usize {
        fn uses(
            doc: &Arc<Document>,
            id: u64,
            counts: &mut std::collections::HashMap<*const Document, usize>,
        ) -> usize {
            let key = Arc::as_ptr(doc);
            if let Some(count) = counts.get(&key) {
                return *count;
            }
            let count = doc
                .objects
                .iter()
                .map(|o| {
                    if o.symbol.as_ref().is_some_and(|s| s.definition == id) {
                        1
                    } else {
                        o.embedded_artwork
                            .as_ref()
                            .map_or(0, |inner| uses(inner, id, counts))
                    }
                })
                .sum();
            counts.insert(key, count);
            count
        }
        let mut counts = std::collections::HashMap::new();
        self.clips
            .iter()
            .flat_map(|c| c.cels.values())
            .chain(self.symbols.values().map(|def| &def.artwork))
            .map(|d| uses(d, id, &mut counts))
            .sum()
    }
    pub fn remove_symbol(&mut self, id: u64) -> Result<(), String> {
        if self.symbol_usage(id) != 0 {
            return Err("Remove all placed instances before deleting their symbol.".into());
        }
        self.symbols.remove(&id).ok_or("Unknown symbol.")?;
        Ok(())
    }
    pub fn hydrate_symbols(&mut self) -> Result<(), String> {
        self.hydrate_composition()
    }
    pub(crate) fn validate_symbols(&self) -> Result<(), String> {
        validate_definitions(&self.symbols)?;
        let resolved = self.resolved_snapshot()?;
        if self
            .symbols
            .iter()
            .any(|(id, s)| !Arc::ptr_eq(&s.artwork, &resolved.symbols[id].artwork))
            || self
                .clips
                .iter()
                .zip(&resolved.clips)
                .any(|(a, b)| a.cels.iter().any(|(k, doc)| !Arc::ptr_eq(doc, &b.cels[k])))
        {
            return Err("A placed instance refers to outdated artwork.".into());
        }
        Ok(())
    }
}

fn validate_definitions(symbols: &BTreeMap<u64, SymbolDefinition>) -> Result<(), String> {
    if symbols.len() > 1024 || symbols.contains_key(&0) {
        return Err("Invalid symbol library.".into());
    }
    let mut names = HashSet::new();
    for def in symbols.values() {
        def.validate()?;
        if !names.insert(def.name.to_lowercase()) {
            return Err("Symbol names must be unique.".into());
        }
    }
    Ok(())
}
