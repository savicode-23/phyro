use crate::{AnimationProject, Clip, Document, SymbolInstance};
use std::{collections::HashMap, sync::Arc};

/// Transfer just the selected animation and its transitive artwork into a project.
/// The placed object retains the source viewport when the projects have different sizes.
impl AnimationProject {
    pub fn place_external_clip(
        &mut self,
        source: &Self,
        source_id: u64,
        asset_name: &str,
        destination: (usize, u32, u64),
        center: [f32; 2],
    ) -> Result<u64, String> {
        let (parent, frame, layer) = destination;
        let source_clip = source
            .clip_index(source_id)
            .ok_or("That animation no longer exists.")?;
        let first = source.clips[source_clip].drawing_at(0);
        let size = [first.width, first.height];
        let parent_doc = self
            .clips
            .get(parent)
            .ok_or("Choose an existing clip.")?
            .drawing_at(0);
        let canvas = [parent_doc.width, parent_doc.height];
        let mut staged = self.clone();
        let mut import = Import {
            source,
            dest: &mut staged,
            asset_name,
            set_id: self.clips[parent].set_id,
            canvas,
            clips: HashMap::new(),
            symbols: HashMap::new(),
            drawings: HashMap::new(),
        };
        let child = import.clip(source_id, 0)?;
        let dimensions = size.map(|s| s as f32);
        let position = [
            center[0] - dimensions[0] * 0.5,
            center[1] - dimensions[1] * 0.5,
        ];
        let id = staged.place_clip(child.id, parent, frame, layer, position)?;
        let placed = staged.clips[parent]
            .drawing_at_mut(frame)
            .objects
            .last_mut()
            .unwrap();
        placed.size = dimensions;
        placed.name.clone_from(&child.name);
        placed.clip_instance.as_mut().unwrap().source_size = (size != canvas).then_some(size);
        staged.clips[parent].sync_layer_edits(&self.clips[parent], frame)?;
        staged.hydrate_composition()?;
        staged.validate()?;
        *self = staged;
        Ok(id)
    }
}

struct Import<'a> {
    source: &'a AnimationProject,
    dest: &'a mut AnimationProject,
    asset_name: &'a str,
    set_id: u64,
    canvas: [u32; 2],
    clips: HashMap<u64, Arc<Clip>>,
    symbols: HashMap<u64, u64>,
    drawings: HashMap<(*const Document, bool), Arc<Document>>,
}

impl Import<'_> {
    fn clip(&mut self, id: u64, depth: usize) -> Result<Arc<Clip>, String> {
        if let Some(clip) = self.clips.get(&id) {
            return Ok(clip.clone());
        }
        let i = self.source.clip_index(id).ok_or("Missing source clip.")?;
        let source_clip = &self.source.clips[i];
        let base = format!(
            "{} · {}",
            self.asset_name,
            self.source.qualified_clip_name(source_clip)
        );
        let mut clip = source_clip.clone();
        clip.set_id = self.set_id;
        for doc in clip.cels.values_mut() {
            *doc = self.drawing(doc, true, depth)?;
        }
        for suffix in 1.. {
            clip.name = import_name(&base, suffix);
            let name = clip.name.to_lowercase();
            if let Some(existing) = self
                .dest
                .clips
                .iter()
                .find(|c| c.set_id == self.set_id && c.name.to_lowercase() == name)
            {
                clip.id = existing.id;
                // A local set export choice does not create a different animation on reimport.
                clip.include_in_set_export = existing.include_in_set_export;
                if existing != &clip {
                    continue;
                }
                let shared = Arc::new(existing.clone());
                self.clips.insert(id, shared.clone());
                return Ok(shared);
            }
            clip.include_in_set_export = source_clip.include_in_set_export;
            let index = self.dest.append_imported_clip(clip)?;
            let shared = Arc::new(self.dest.clips[index].clone());
            self.clips.insert(id, shared.clone());
            return Ok(shared);
        }
        unreachable!()
    }

    fn symbol(&mut self, id: u64, depth: usize) -> Result<u64, String> {
        if let Some(id) = self.symbols.get(&id) {
            return Ok(*id);
        }
        let mut def = self
            .source
            .symbols
            .get(&id)
            .ok_or("Missing source symbol.")?
            .clone();
        def.artwork = self.drawing(&def.artwork, false, depth)?;
        let base = def.name.clone();
        for suffix in 1.. {
            def.name = import_name(&base, suffix);
            let name = def.name.to_lowercase();
            let new = match self
                .dest
                .symbols
                .iter()
                .find(|(_, d)| d.name.to_lowercase() == name)
            {
                Some((&new, d)) if d == &def => new,
                Some(_) => continue,
                None => self.dest.add_symbol(def)?,
            };
            self.symbols.insert(id, new);
            return Ok(new);
        }
        unreachable!()
    }

    fn drawing(
        &mut self,
        original: &Arc<Document>,
        resize: bool,
        depth: usize,
    ) -> Result<Arc<Document>, String> {
        if depth >= 16 {
            return Err("The imported artwork exceeds the composition nesting limit.".into());
        }
        let key = (Arc::as_ptr(original), resize);
        if let Some(doc) = self.drawings.get(&key) {
            return Ok(doc.clone());
        }
        let mut doc = original.clone();
        if resize && self.canvas != [original.width, original.height] {
            let copy = Arc::make_mut(&mut doc);
            [copy.width, copy.height] = self.canvas;
        }
        for (index, o) in original.objects.iter().enumerate() {
            if let Some(instance) = &o.symbol {
                let id = self.symbol(instance.definition, depth + 1)?;
                Arc::make_mut(&mut doc).objects[index].symbol = Some(SymbolInstance::new(
                    id,
                    self.dest.symbols[&id].artwork.clone(),
                ));
            } else if let Some(instance) = &o.clip_instance {
                let child = self.clip(instance.clip_id, depth + 1)?;
                let copy = Arc::make_mut(&mut doc).objects[index]
                    .clip_instance
                    .as_mut()
                    .unwrap();
                let original_size = copy
                    .source_size
                    .unwrap_or([original.width, original.height]);
                copy.source_size = (self.canvas != original_size).then_some(original_size);
                copy.clip_id = child.id;
                copy.cached = Some(child);
            } else if let Some(art) = &o.embedded_artwork {
                let resolved = self.drawing(art, false, depth + 1)?;
                if !Arc::ptr_eq(art, &resolved) {
                    Arc::make_mut(&mut doc).objects[index].embedded_artwork = Some(resolved.into());
                }
            }
        }
        self.drawings.insert(key, doc.clone());
        Ok(doc)
    }
}

fn import_name(base: &str, n: usize) -> String {
    let suffix = if n == 1 {
        String::new()
    } else {
        format!(" ({n})")
    };
    let prefix = base
        .chars()
        .filter(|c| !c.is_control())
        .take(80 - suffix.len())
        .collect::<String>();
    format!("{}{suffix}", prefix.trim())
}
