use super::*;
use std::collections::HashMap;

/// An independently editable placement can share unchanged artwork with its
/// siblings and undo snapshots. Equality takes the same fast path as symbols.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EmbeddedArtwork(Arc<Document>);

impl PartialEq for EmbeddedArtwork {
    fn eq(&self, other: &Self) -> bool {
        crate::animation::same_drawing(&self.0, &other.0)
    }
}

impl From<Arc<Document>> for EmbeddedArtwork {
    fn from(value: Arc<Document>) -> Self {
        Self(value)
    }
}

impl std::ops::Deref for EmbeddedArtwork {
    type Target = Arc<Document>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for EmbeddedArtwork {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl AnimationProject {
    /// Keep each placement's object, animation and layer order. Its private
    /// artwork can still contain live links to other library definitions.
    pub fn remove_symbol_keeping_artwork(&mut self, id: u64) -> Result<(), String> {
        let mut staged = self.resolved_snapshot()?;
        let definition = staged.symbols.remove(&id).ok_or("Unknown symbol.")?;
        let mut drawings = HashMap::new();
        for drawing in staged
            .symbols
            .values_mut()
            .map(|def| &mut def.artwork)
            .chain(
                staged
                    .clips
                    .iter_mut()
                    .flat_map(|clip| clip.cels.values_mut()),
            )
        {
            *drawing = detach(drawing, id, &definition, &mut drawings);
        }
        staged.hydrate_composition()?;
        staged.validate()?;
        *self = staged;
        Ok(())
    }

    pub fn replace_embedded_artwork(
        &mut self,
        clip: usize,
        frame: u32,
        object: u64,
        artwork: Document,
    ) -> Result<(), String> {
        let mut staged = self.clone();
        let previous = self.clips.get(clip).ok_or("Choose an existing clip.")?;
        let clip = staged
            .clips
            .get_mut(clip)
            .filter(|c| frame < c.frames)
            .ok_or("Choose a frame inside an existing clip.")?;
        let doc = clip.drawing_at_mut(frame);
        let index = doc.editable_object_index(object)?;
        if doc.objects[index].embedded_artwork.is_none() {
            return Err("Select an independent artwork copy.".into());
        }
        doc.objects[index].embedded_artwork = Some(Arc::new(artwork).into());
        clip.sync_layer_edits(previous, frame)?;
        staged.hydrate_composition()?;
        staged.validate()?;
        *self = staged;
        Ok(())
    }
}

fn detach(
    source: &Arc<Document>,
    id: u64,
    definition: &SymbolDefinition,
    drawings: &mut HashMap<*const Document, Arc<Document>>,
) -> Arc<Document> {
    let key = Arc::as_ptr(source);
    if let Some(previous) = drawings.get(&key) {
        return previous.clone();
    }
    let mut output = source.clone();
    for (index, original) in source.objects.iter().enumerate() {
        if original
            .symbol
            .as_ref()
            .is_some_and(|link| link.definition == id)
        {
            let o = &mut Arc::make_mut(&mut output).objects[index];
            o.symbol = None;
            o.embedded_artwork = Some(definition.artwork.clone().into());
            if o.name.is_empty() {
                o.name.clone_from(&definition.name);
            }
        } else if let Some(nested) = &original.embedded_artwork {
            let child = detach(nested, id, definition, drawings);
            if !Arc::ptr_eq(nested, &child) {
                Arc::make_mut(&mut output).objects[index].embedded_artwork = Some(child.into());
            }
        }
    }
    drawings.insert(key, output.clone());
    output
}
