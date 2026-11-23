use crate::invalid;
use document_core::{AnimationProject, ClipBinding, ClipSource, Document};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::io;
use std::sync::Arc;

/// A group, or many drawing exposures, can share one original clipping path.
/// Store its editable nodes and keys once and bind each target in its own paint space.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredClips {
    sources: Vec<Arc<ClipSource>>,
    drawings: Vec<(Location, Vec<Binding>)>,
}

#[derive(Hash, PartialEq, Eq, Serialize, Deserialize)]
enum Location {
    Cel(u64, u32),
    Symbol(u64),
    Artwork(u64),
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    object: u64,
    source: usize,
    paint_space: [f32; 6],
    original_target: u64,
    original_layer: u64,
}

fn drawings<'a>(
    project: &'a mut AnimationProject,
    artwork: &'a mut BTreeMap<u64, Arc<Document>>,
) -> impl Iterator<Item = (Location, &'a mut Arc<Document>)> {
    let cels = project.clips.iter_mut().flat_map(|clip| {
        let id = clip.id;
        clip.cels
            .iter_mut()
            .map(move |(&frame, doc)| (Location::Cel(id, frame), doc))
    });
    cels.chain(
        project
            .symbols
            .iter_mut()
            .map(|(&id, def)| (Location::Symbol(id), &mut def.artwork)),
    )
    .chain(
        artwork
            .iter_mut()
            .map(|(&id, doc)| (Location::Artwork(id), doc)),
    )
}

impl StoredClips {
    pub(super) fn take(
        project: &mut AnimationProject,
        artwork: &mut BTreeMap<u64, Arc<Document>>,
    ) -> Option<Self> {
        let mut stored = Self {
            sources: Vec::new(),
            drawings: Vec::new(),
        };
        let mut indices = HashMap::new();
        for (location, doc) in drawings(project, artwork) {
            if !doc.objects.iter().any(|o| o.clip_source.is_some()) {
                continue;
            }
            let mut bindings = Vec::new();
            for object in &mut Arc::make_mut(doc).objects {
                if let Some(binding) = object.clip_source.take() {
                    let source =
                        *indices
                            .entry(Arc::as_ptr(&binding.source))
                            .or_insert_with(|| {
                                let index = stored.sources.len();
                                stored.sources.push(binding.source);
                                index
                            });
                    bindings.push(Binding {
                        object: object.id,
                        source,
                        paint_space: binding.paint_space,
                        original_target: binding.original_target,
                        original_layer: binding.original_layer,
                    });
                }
            }
            stored.drawings.push((location, bindings));
        }
        (!stored.sources.is_empty()).then_some(stored)
    }

    pub(super) fn restore(
        self,
        project: &mut AnimationProject,
        artwork: &mut BTreeMap<u64, Arc<Document>>,
    ) -> io::Result<()> {
        let mut pending = HashMap::new();
        for (location, bindings) in self.drawings {
            if bindings.is_empty() || pending.insert(location, bindings).is_some() {
                return Err(invalid("Duplicate or empty clipped drawing."));
            }
        }
        let mut used = vec![false; self.sources.len()];
        for (location, doc) in drawings(project, artwork) {
            let Some(bindings) = pending.remove(&location) else {
                continue;
            };
            let mut objects: HashMap<_, _> = Arc::make_mut(doc)
                .objects
                .iter_mut()
                .map(|o| (o.id, o))
                .collect();
            for binding in bindings {
                let object = objects
                    .remove(&binding.object)
                    .ok_or_else(|| invalid("Unknown or duplicate clipping target."))?;
                let source = self
                    .sources
                    .get(binding.source)
                    .ok_or_else(|| invalid("Missing editable clipping path."))?;
                if object.clip_source.is_some() || object.stencil.is_none() {
                    return Err(invalid("Invalid clipping target."));
                }
                used[binding.source] = true;
                object.clip_source = Some(ClipBinding {
                    source: Arc::clone(source),
                    paint_space: binding.paint_space,
                    original_target: binding.original_target,
                    original_layer: binding.original_layer,
                });
            }
        }
        if !pending.is_empty() || used.contains(&false) {
            return Err(invalid("Unreferenced clipping path or drawing."));
        }
        Ok(())
    }
}
