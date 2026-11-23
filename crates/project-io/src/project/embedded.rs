use super::invalid;
use document_core::{AnimationProject, Document, SymbolInstance};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::{io, sync::Arc};

/// Format 10 stores each independent artwork snapshot once. Its temporary IDs
/// occupy a separate table and are expanded back into editable placements on load.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Stored {
    project: AnimationProject,
    artwork: BTreeMap<u64, Arc<Document>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    images: BTreeMap<u64, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    clipping: Option<super::clipping::StoredClips>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(super) drawing_only: bool,
}

fn drawings(project: &mut AnimationProject) -> impl Iterator<Item = &mut Arc<Document>> {
    project
        .clips
        .iter_mut()
        .flat_map(|clip| clip.cels.values_mut())
        .chain(project.symbols.values_mut().map(|s| &mut s.artwork))
}

impl Stored {
    pub(super) fn pack(mut project: AnimationProject) -> io::Result<Self> {
        let reserved = project.symbols.keys().copied().collect();
        let mut table = Packer {
            next: 1,
            reserved,
            ids: HashMap::new(),
            artwork: BTreeMap::new(),
        };
        for doc in drawings(&mut project) {
            table.pack_drawing(doc);
        }
        let mut artwork = table.artwork;
        let images = super::images::pack(drawings(&mut project).chain(artwork.values_mut()))?;
        let clipping = super::clipping::StoredClips::take(&mut project, &mut artwork);
        Ok(Self {
            project,
            artwork,
            images,
            clipping,
            drawing_only: false,
        })
    }

    pub(super) fn unpack(mut self) -> io::Result<AnimationProject> {
        super::images::unpack(
            self.images,
            drawings(&mut self.project).chain(self.artwork.values_mut()),
        )?;
        if let Some(clipping) = self.clipping.take() {
            clipping.restore(&mut self.project, &mut self.artwork)?;
        }
        if self
            .artwork
            .keys()
            .any(|id| *id == 0 || self.project.symbols.contains_key(id))
        {
            return Err(invalid(
                "Independent artwork IDs must be distinct from library symbols.",
            ));
        }
        let mut table = Unpacker {
            raw: self.artwork,
            resolved: HashMap::new(),
            active: HashSet::new(),
        };
        for doc in drawings(&mut self.project) {
            table.expand(doc, 0)?;
        }
        if !table.raw.is_empty() {
            return Err(invalid("Unreferenced independent artwork."));
        }
        Ok(self.project)
    }
}

struct Packer {
    next: u64,
    reserved: HashSet<u64>,
    ids: HashMap<*const Document, u64>,
    artwork: BTreeMap<u64, Arc<Document>>,
}

impl Packer {
    fn pack_drawing(&mut self, doc: &mut Arc<Document>) {
        if !doc.objects.iter().any(|o| o.embedded_artwork.is_some()) {
            return;
        }
        for object in &mut Arc::make_mut(doc).objects {
            let Some(artwork) = object.embedded_artwork.take() else {
                continue;
            };
            let artwork = Arc::clone(&artwork);
            let ptr = Arc::as_ptr(&artwork);
            let id = if let Some(&id) = self.ids.get(&ptr) {
                id
            } else {
                while self.reserved.contains(&self.next) {
                    self.next += 1;
                }
                let id = self.next;
                self.next += 1;
                self.ids.insert(ptr, id);
                let mut stored = artwork.clone();
                self.pack_drawing(&mut stored);
                self.artwork.insert(id, stored);
                id
            };
            // The transient link keeps the source Arc alive while identity is used for deduplication.
            // Only its numeric ID is part of the serialized placement.
            object.symbol = Some(SymbolInstance::new(id, artwork));
        }
    }
}

struct Unpacker {
    raw: BTreeMap<u64, Arc<Document>>,
    resolved: HashMap<u64, Arc<Document>>,
    active: HashSet<u64>,
}

impl Unpacker {
    fn expand(&mut self, doc: &mut Arc<Document>, depth: usize) -> io::Result<()> {
        if depth > 16 || doc.objects.iter().any(|o| o.embedded_artwork.is_some()) {
            return Err(invalid("Invalid nested independent artwork."));
        }
        let private = |id: &u64| {
            self.raw.contains_key(id) || self.resolved.contains_key(id) || self.active.contains(id)
        };
        if !doc
            .objects
            .iter()
            .any(|o| o.symbol.as_ref().is_some_and(|s| private(&s.definition)))
        {
            return Ok(());
        }
        for object in &mut Arc::make_mut(doc).objects {
            let Some(id) = object.symbol.as_ref().map(|s| s.definition) else {
                continue;
            };
            let art = if let Some(art) = self.resolved.get(&id) {
                art.clone()
            } else if let Some(mut art) = self.raw.remove(&id) {
                self.active.insert(id);
                self.expand(&mut art, depth + 1)?;
                self.active.remove(&id);
                self.resolved.insert(id, art.clone());
                art
            } else if self.active.contains(&id) {
                return Err(invalid("Cyclic independent artwork."));
            } else {
                continue;
            };
            object.symbol = None;
            object.embedded_artwork = Some(art.into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_clip_size_requires_the_new_format_and_valid_dimensions() {
        let mut p = AnimationProject::default();
        let child = p.add_clip("Child").unwrap();
        p.place_clip(p.clips[child].id, 0, 0, 1, [0.; 2]).unwrap();
        p.resize_artboard([200, 90]).unwrap();
        let payload = serde_json::to_vec(&Stored::pack(p.clone()).unwrap()).unwrap();
        let error = super::super::decode_animation(10, &payload).unwrap_err();
        assert!(error.to_string().contains("format 11"));
        let wrapper = &mut p.clips[0].drawing_at_mut(0).objects[0];
        wrapper.clip_instance.as_mut().unwrap().source_size = Some([0, 100]);
        let invalid = serde_json::to_vec(&Stored::pack(p).unwrap()).unwrap();
        let error = super::super::decode_animation(11, &invalid).unwrap_err();
        assert!(error.to_string().contains("artboard size"));
    }

    #[test]
    fn shared_copies_use_one_record_and_reopen_independently_editable() {
        let mut icon = Document {
            frames: 1,
            ..Default::default()
        };
        icon.add_rectangle([4.; 2], [8.; 2], [255; 4]);
        let artwork = Arc::new(icon);
        let mut doc = Document::default();
        for i in 0..1_000 {
            doc.add_rectangle([i as f32, 1.], [8.; 2], [0; 4]);
            doc.objects.last_mut().unwrap().embedded_artwork = Some(artwork.clone().into());
        }
        let original = AnimationProject::from_document(doc);
        let packed = Stored::pack(original.clone()).unwrap();
        assert_eq!(packed.artwork.len(), 1);
        let payload = serde_json::to_vec(&packed).unwrap();
        let reopened =
            super::super::decode_animation(super::super::EMBEDDED_ARTWORK_VERSION, &payload)
                .unwrap();
        assert_eq!(reopened, original);
        let scene = reopened.clips[0].drawing_at(0);
        assert!(Arc::ptr_eq(
            scene.objects[0].static_artwork().unwrap(),
            scene.objects[999].static_artwork().unwrap()
        ));
    }
}
