use super::Clipboard;
use document_core::{AnimationProject, SymbolDefinition, SymbolInstance, VectorObject};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::hash::{BuildHasher, RandomState};
use std::sync::{Arc, OnceLock};

pub(super) const HEADER: &str = "vector-objects/2";
pub(super) const COMPOSITION_HEADER: &str = "vector-objects/3";
pub(super) const FRAME_HEADER: &str = "vector-objects/4";

/// Session IDs are meaningful only within the editor process that allocated them.
fn process_token() -> u64 {
    static TOKEN: OnceLock<u64> = OnceLock::new();
    *TOKEN.get_or_init(|| RandomState::new().hash_one("Phyro clipboard"))
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ClipSources {
    process: u64,
    /// Persistent child clip ID to the source project's in-memory identity.
    sessions: BTreeMap<u64, u64>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Contents<Objects, Symbols, Clips> {
    objects: Objects,
    symbols: Symbols,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_frame: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    clips: Option<Clips>,
}

fn links(objects: &[VectorObject]) -> Vec<&SymbolInstance> {
    let mut result = Vec::new();
    let mut drawings = vec![objects];
    while let Some(objects) = drawings.pop() {
        for object in objects {
            if let Some(link) = &object.symbol {
                result.push(link);
            } else if let Some(art) = &object.embedded_artwork {
                drawings.push(&art.objects);
            }
        }
    }
    result
}

/// Import children first, even when an outer definition has a smaller ID.
fn dependency_order(
    objects: &[VectorObject],
    defs: &BTreeMap<u64, SymbolDefinition>,
) -> Result<Vec<u64>, String> {
    fn visit(
        id: u64,
        defs: &BTreeMap<u64, SymbolDefinition>,
        active: &mut BTreeSet<u64>,
        seen: &mut BTreeSet<u64>,
        order: &mut Vec<u64>,
    ) -> Result<(), String> {
        if seen.contains(&id) {
            return Ok(());
        }
        if active.len() >= 16 || !active.insert(id) {
            return Err("Clipboard symbol nesting is too deep or cyclic.".into());
        }
        let definition = defs
            .get(&id)
            .ok_or("Clipboard is missing a symbol definition.")?;
        for child in links(&definition.artwork.objects) {
            visit(child.definition, defs, active, seen, order)?;
        }
        active.remove(&id);
        seen.insert(id);
        order.push(id);
        Ok(())
    }
    let mut seen = BTreeSet::new();
    let mut active = BTreeSet::new();
    let mut order = Vec::new();
    for link in links(objects) {
        visit(link.definition, defs, &mut active, &mut seen, &mut order)?;
    }
    if order.len() != defs.len() {
        return Err("Clipboard contains unused symbol definitions.".into());
    }
    Ok(order)
}

fn rebind(
    objects: &mut [VectorObject],
    mapped: &BTreeMap<u64, u64>,
    defs: &BTreeMap<u64, SymbolDefinition>,
) -> Result<(), String> {
    for object in objects {
        if let Some(art) = &mut object.embedded_artwork {
            if !links(&art.objects).is_empty() {
                rebind(&mut Arc::make_mut(art).objects, mapped, defs)?;
            }
        }
        let Some(instance) = object.symbol.as_mut() else {
            continue;
        };
        let id = *mapped
            .get(&instance.definition)
            .ok_or("Clipboard is missing a nested symbol.")?;
        let def = defs
            .get(&id)
            .ok_or("The destination symbol no longer exists.")?;
        *instance = SymbolInstance::new(id, Arc::clone(&def.artwork));
    }
    Ok(())
}

impl Clipboard {
    pub(super) fn capture_definitions(&mut self) {
        self.symbols.clear();
        let mut pending: Vec<_> = links(&self.objects).into_iter().cloned().collect();
        while let Some(instance) = pending.pop() {
            if self.symbols.contains_key(&instance.definition) {
                continue;
            }
            if let Some(art) = instance.shared_artwork() {
                self.symbols.insert(
                    instance.definition,
                    SymbolDefinition {
                        name: format!("Symbol {}", instance.definition),
                        artwork: Arc::clone(art),
                    },
                );
                pending.extend(links(&art.objects).into_iter().cloned());
            }
        }
        let sessions: BTreeMap<_, _> = self
            .objects
            .iter()
            .filter_map(|o| o.clip_instance.as_ref())
            .filter_map(|i| i.shared_clip().map(|clip| (i.clip_id, clip.session_id())))
            .collect();
        self.clip_sources = (!sessions.is_empty()).then(|| ClipSources {
            process: process_token(),
            sessions,
        });
    }
    pub fn copy_project(
        &mut self,
        project: &AnimationProject,
        clip: usize,
        frame: u32,
        ids: &[u64],
    ) {
        self.copy(project.clips[clip].drawing_at(frame), ids, frame);
        for (id, def) in &mut self.symbols {
            if let Some(actual) = project.symbols.get(id) {
                def.name.clone_from(&actual.name);
            }
        }
    }
    pub(super) fn encode_with_symbols(&self) -> Option<String> {
        let header = if self.source_frame != 0 {
            FRAME_HEADER
        } else if self.clip_sources.is_some() {
            COMPOSITION_HEADER
        } else {
            HEADER
        };
        let content = Contents {
            objects: &self.objects,
            source_frame: (self.source_frame != 0).then_some(self.source_frame),
            symbols: &self.symbols,
            clips: self.clip_sources.as_ref(),
        };
        super::encode_clipboard(header, &content)
    }
    pub(super) fn decode_with_symbols(payload: &str) -> Option<Self> {
        let mut content: Contents<Vec<VectorObject>, BTreeMap<u64, SymbolDefinition>, ClipSources> =
            serde_json::from_str(payload).ok()?;
        let source_frame = content.source_frame.unwrap_or(0);
        if source_frame >= document_core::MAX_FRAMES {
            return None;
        }
        if content.symbols.len() > 1024 || content.symbols.contains_key(&0) {
            return None;
        }
        let order = dependency_order(&content.objects, &content.symbols).ok()?;
        let clip_ids: BTreeSet<_> = content
            .objects
            .iter()
            .filter_map(|o| o.clip_instance.as_ref().map(|i| i.clip_id))
            .collect();
        if content.clips.as_ref().map_or(!clip_ids.is_empty(), |s| {
            clip_ids.is_empty()
                || !s.sessions.keys().copied().eq(clip_ids.iter().copied())
                || s.sessions.values().any(|id| *id == 0)
        }) {
            return None;
        }
        if content
            .objects
            .iter()
            .any(VectorObject::conflicting_placements)
        {
            return None;
        }
        let mut source = AnimationProject {
            symbols: content.symbols,
            ..AnimationProject::default()
        };
        source.hydrate_composition().ok()?;
        source.validate().ok()?;
        let original = order.into_iter().map(|id| (id, id)).collect();
        rebind(&mut content.objects, &original, &source.symbols).ok()?;
        Some(Self {
            source_frame,
            objects: content.objects,
            symbols: source.symbols,
            clip_sources: content.clips,
        })
    }

    fn attach_symbols(
        &self,
        staged: &mut AnimationProject,
        objects: &mut [VectorObject],
    ) -> Result<(), String> {
        let mut mapped = BTreeMap::new();
        for old in dependency_order(&self.objects, &self.symbols)? {
            let mut imported = self.symbols[&old].clone();
            if !links(&imported.artwork.objects).is_empty() {
                rebind(
                    &mut Arc::make_mut(&mut imported.artwork).objects,
                    &mapped,
                    &staged.symbols,
                )?;
            }
            // A repeated system paste decodes the original payload again. Walk the same
            // suffix sequence so a previously imported identical copy is reused.
            let base: String = imported.name.chars().take(70).collect();
            let mut suffix = 2;
            let id = loop {
                let existing = staged
                    .symbols
                    .iter()
                    .find(|(_, d)| d.name.to_lowercase() == imported.name.to_lowercase());
                match existing {
                    Some((&id, d))
                        if Arc::ptr_eq(&d.artwork, &imported.artwork)
                            || d.artwork == imported.artwork =>
                    {
                        break id
                    }
                    Some(_) => {
                        imported.name = format!("{base} ({suffix})");
                        suffix += 1;
                    }
                    None => break staged.add_symbol(imported)?,
                }
            };
            mapped.insert(old, id);
        }
        rebind(objects, &mapped, &staged.symbols)
    }

    /// Publish artwork and any new definitions together after validating the composition.
    pub fn paste_project(
        &self,
        project: &mut AnimationProject,
        clip: usize,
        tick: u32,
        layer: u64,
    ) -> Result<Vec<u64>, String> {
        if let Some(source) = &self.clip_sources {
            if source.process != process_token()
                || source.sessions.iter().any(|(&id, &session)| {
                    project
                        .clip_index(id)
                        .is_none_or(|index| project.clips[index].session_id() != session)
                })
            {
                return Err("Paste linked clips into the project they were copied from. Re-place the clip after reopening a project.".into());
            }
        } else if self.objects.iter().any(|o| o.clip_instance.is_some()) {
            return Err("Clipboard is missing the linked clip's source.".into());
        }
        let mut staged = project.clone();
        let mut pasted = self.clone();
        self.attach_symbols(&mut staged, &mut pasted.objects)?;
        let dst = staged
            .clips
            .get_mut(clip)
            .filter(|c| tick < c.frames)
            .ok_or("Choose an existing frame.")?;
        if !super::is_editable(dst.drawing_at(tick), layer) {
            return Err("Choose a visible, unlocked layer to paste.".into());
        }
        let drawing = dst.drawing_at_mut(tick);
        let ids = pasted.paste(drawing, layer);
        let length = drawing.frames;
        dst.sync_layer_edits(&project.clips[clip], tick)?;
        if length != dst.frames {
            dst.set_length(length)?;
        }
        staged.hydrate_composition()?;
        staged.validate()?;
        *project = staged;
        Ok(ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::History;
    use document_core::Document;

    fn instance(color: [u8; 4]) -> (AnimationProject, u64, u64) {
        let mut scene = Document::default();
        let source = scene.add_rectangle([4., 5.], [8., 7.], color);
        let mut project = AnimationProject::from_document(scene);
        let (definition, placed) = project.convert_to_symbol(0, 0, &[source], "Cap").unwrap();
        (project, definition, placed)
    }

    #[test]
    fn pasting_an_independent_copy_remaps_its_remaining_nested_symbol_links() {
        let (mut source, _, placed) = instance([255, 100, 0, 255]);
        let (outer, object) = source.convert_to_symbol(0, 0, &[placed], "Badge").unwrap();
        source.remove_symbol_keeping_artwork(outer).unwrap();
        let mut clipboard = Clipboard::default();
        clipboard.copy_project(&source, 0, 0, &[object]);
        let decoded = Clipboard::decode(&clipboard.encode().unwrap()).unwrap();
        let (mut target, foreign, _) = instance([0, 255, 0, 255]);
        let copy = decoded.paste_project(&mut target, 0, 0, 1).unwrap()[0];
        let copy = target.clips[0]
            .drawing_at(0)
            .objects
            .iter()
            .find(|o| o.id == copy)
            .unwrap();
        assert!(copy.symbol.is_none());
        let child = copy.embedded_artwork.as_ref().unwrap().objects[0]
            .symbol
            .as_ref()
            .unwrap();
        assert_ne!(child.definition, foreign);
        assert_eq!(child.artwork().unwrap().objects[0].fill, [255, 100, 0, 255]);
        target.validate().unwrap();
    }

    #[test]
    fn system_copy_remaps_foreign_ids_and_reuses_shared_imports() {
        let (a, _, placed) = instance([255, 100, 0, 255]);
        let (mut b, foreign, _) = instance([0, 255, 0, 255]);
        let before = b.clone();
        let mut copy = Clipboard::default();
        copy.copy_project(&a, 0, 0, &[placed]);
        let decoded = Clipboard::decode(&copy.encode().unwrap()).unwrap();
        let pasted = decoded.paste_project(&mut b, 0, 0, 1).unwrap()[0];
        let destination = b.clips[0].drawing_at(0);
        let link = destination
            .objects
            .iter()
            .find(|o| o.id == pasted)
            .unwrap()
            .symbol
            .as_ref()
            .unwrap();
        assert_ne!(link.definition, foreign);
        assert_eq!(b.symbols[&link.definition].name, "Cap (2)");
        assert_eq!(link.artwork().unwrap().objects[0].fill, [255, 100, 0, 255]);
        Clipboard::decode(&copy.encode().unwrap())
            .unwrap()
            .paste_project(&mut b, 0, 0, 1)
            .unwrap();
        assert_eq!(b.symbols.len(), 2);
        b.validate().unwrap();
        let mut history = History::default();
        history.record(&before, &b, 0, 0);
        history.undo(&mut b);
        assert_eq!(b, before);
        history.redo(&mut b);
        b.validate().unwrap();
        assert_eq!(b.symbols.len(), 2);
    }

    #[test]
    fn clipboard_rejects_dangling_and_cyclic_definitions() {
        let (p, _, placed) = instance([255; 4]);
        let mut copy = Clipboard::default();
        copy.copy_project(&p, 0, 0, &[placed]);
        let encoded = copy.encode().unwrap();
        let (header, data) = encoded.split_once('\n').unwrap();
        let mut doc: serde_json::Value = serde_json::from_str(data).unwrap();
        doc["objects"][0]["symbol"]["definition"] = 99.into();
        assert!(Clipboard::decode(&format!("{header}\n{doc}")).is_none());
        let mut doc: serde_json::Value = serde_json::from_str(data).unwrap();
        doc["symbols"]["1"]["artwork"]["objects"][0]["symbol"] =
            serde_json::json!({"definition":1});
        assert!(Clipboard::decode(&format!("{header}\n{doc}")).is_none());
    }

    #[test]
    fn a_nested_symbol_imports_children_first_and_reuses_previous_imports() {
        let (mut source, parent, placed) = instance([255, 0, 0, 255]);
        let mut pixels = Document::default();
        let id = pixels.add_rectangle([2., 2.], [4., 4.], [255, 210, 10, 255]);
        let (child, _) = SymbolDefinition::capture("Star", &pixels, 0, &[id]).unwrap();
        let child = source.add_symbol(child).unwrap();
        // The child's ID is higher than the parent's.
        source.nest_symbol(parent, child).unwrap();
        let mut copy = Clipboard::default();
        copy.copy_project(&source, 0, 0, &[placed]);
        let text = copy.encode().unwrap();
        let (mut destination, collision, _) = instance([0, 0, 200, 255]);
        for _ in 0..2 {
            let decoded = Clipboard::decode(&text).unwrap();
            let id = decoded.paste_project(&mut destination, 0, 0, 1).unwrap()[0];
            let obj = destination.clips[0]
                .drawing_at(0)
                .objects
                .iter()
                .find(|o| o.id == id)
                .unwrap();
            let parent = obj.symbol.as_ref().unwrap();
            assert_ne!(parent.definition, collision);
            let placed_child = parent
                .artwork()
                .unwrap()
                .objects
                .iter()
                .find_map(|o| o.symbol.as_ref())
                .unwrap();
            assert_eq!(destination.symbols[&placed_child.definition].name, "Star");
            assert_eq!(
                placed_child.artwork().unwrap().objects[0].fill,
                [255, 210, 10, 255]
            );
            assert_eq!(destination.symbols.len(), 3);
            destination.validate().unwrap();
        }
    }

    #[test]
    fn clip_references_follow_live_child_and_foreign_or_cyclic_pastes_leave_the_project_intact() {
        let mut project = AnimationProject::default();
        let child = project.add_clip("Footstep").unwrap();
        let id = project.clips[child].id;
        let placed = project.place_clip(id, 0, 0, 1, [0., 0.]).unwrap();
        project.clips[0]
            .drawing_at_mut(0)
            .set_drawing_key(placed, 4, 0)
            .unwrap();
        let mut copy = Clipboard::default();
        copy.copy_project(&project, 0, 0, &[placed]);
        let text = copy.encode().unwrap();
        let decoded = Clipboard::decode(&text).unwrap();
        let (header, json) = text.split_once('\n').unwrap();
        let mut invalid: serde_json::Value = serde_json::from_str(json).unwrap();
        invalid["objects"][0]["clip_instance"]["drawing_keys"] =
            serde_json::json!({"4294967295":0});
        assert!(Clipboard::decode(&format!("{header}\n{invalid}")).is_none());
        let short = project.add_clip("Short").unwrap();
        project.clips[short].set_length(1).unwrap();
        decoded.paste_project(&mut project, short, 0, 1).unwrap();
        assert_eq!(project.clips[short].frames, 5);
        assert_eq!(
            project.clips[short].drawing_at(4).objects[0]
                .clip_instance
                .as_ref()
                .unwrap()
                .drawing_keys[&4],
            0
        );
        let shape =
            project.clips[child]
                .drawing_at_mut(0)
                .add_rectangle([2., 3.], [2., 2.], [255; 4]);
        let pasted = decoded.paste_project(&mut project, 0, 0, 1).unwrap()[0];
        let placement = project.clips[0]
            .drawing_at(0)
            .objects
            .iter()
            .find(|o| o.id == pasted)
            .unwrap();
        assert_eq!(placement.instance_at(0).unwrap().0.objects[0].id, shape);
        let before = project.clone();
        assert!(decoded
            .paste_project(&mut project, child, 0, 1)
            .unwrap_err()
            .contains("cycle"));
        assert_eq!(before, project);
        let mut other = AnimationProject::default();
        other.add_clip("Footstep").unwrap();
        let before = other.clone();
        assert!(decoded
            .paste_project(&mut other, 0, 0, 1)
            .unwrap_err()
            .contains("copied from"));
        assert_eq!(before, other);
    }
}
