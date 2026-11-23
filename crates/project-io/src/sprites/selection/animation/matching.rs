use super::super::{selected_members, Member};
use super::SelectionRange;
use crate::invalid;
use document_core::{Clip, Document, VectorObject};
use std::{collections::HashMap, io, ops::Range};

/// One selected exposure owns borrowed paint-ordered objects. Matching is done
/// once per drawing rather than once per sampled tick or once per export pass.
pub(super) struct Exposure<'a> {
    frames: Range<u32>,
    held: bool,
    pub objects: Vec<&'a VectorObject>,
}

impl Exposure<'_> {
    pub fn samples(&self) -> impl Iterator<Item = Range<u32>> {
        let step = if self.held { self.frames.len() } else { 1 };
        let end = self.frames.end;
        self.frames
            .clone()
            .step_by(step)
            .map(move |first| first..end.min(first + step as u32))
    }
}

type Names<'a> = HashMap<(&'a str, &'a str), usize>;

fn layer_names(doc: &Document) -> HashMap<u64, &str> {
    doc.layers
        .iter()
        .map(|layer| (layer.id, layer.name.as_str()))
        .collect()
}

fn source_names<'a>(
    doc: &'a Document,
    reference: &[Member<'a>],
    layers: &HashMap<u64, &'a str>,
) -> Names<'a> {
    let mut names: Names = reference
        .iter()
        .filter(|m| !m.object.name.trim().is_empty() && !layers[&m.object.layer].trim().is_empty())
        .map(|m| ((layers[&m.object.layer], m.object.name.as_str()), 0))
        .collect();
    for object in doc.objects.iter().filter(|o| !o.reference_only) {
        if let Some(count) = names.get_mut(&(layers[&object.layer], object.name.as_str())) {
            *count += 1;
        }
    }
    names
}

struct SelectedTrack<'a> {
    id: u64,
    hold: Range<u32>,
    name: Option<((&'a str, &'a str), usize)>,
}

fn selected_tracks<'a>(
    clip: &'a Clip,
    frame: u32,
    members: &[Member<'a>],
) -> Vec<SelectedTrack<'a>> {
    let doc = clip.drawing_at(frame);
    let layers = layer_names(doc);
    let names = source_names(doc, members, &layers);
    members
        .iter()
        .map(|member| {
            let object = member.object;
            let (start, duration) = clip.layer_exposure(frame, object.layer);
            let key = (layers[&object.layer], object.name.as_str());
            SelectedTrack {
                id: object.id,
                hold: start..start + duration,
                name: names.get(&key).map(|&count| (key, count)),
            }
        })
        .collect()
}

fn matched<'a>(
    doc: &'a Document,
    tracks: &[SelectedTrack<'_>],
    start: u32,
    reference: u32,
) -> io::Result<Vec<Member<'a>>> {
    let mut ids = Vec::new();
    let mut names = Names::new();
    for track in tracks {
        if track.hold.contains(&start) {
            ids.push(track.id);
        } else if let Some((key, count)) = track.name {
            names.insert(key, count);
        }
    }
    if !names.is_empty() {
        let layers = layer_names(doc);
        let mut hits = HashMap::new();
        for object in doc.objects.iter().filter(|o| !o.reference_only) {
            let pair = (layers[&object.layer], object.name.as_str());
            let Some(&source_count) = names.get(&pair) else {
                continue;
            };
            let duplicate_at = if source_count > 1 {
                Some(reference)
            } else if hits.insert(pair, object.id).is_some() {
                Some(start)
            } else {
                None
            };
            if let Some(tick) = duplicate_at {
                let (layer, object) = pair;
                return Err(invalid(format!(
                    "Ambiguous object '{object}' on layer '{layer}' at drawing {tick}. Give matching artwork unique object and layer names."
                )));
            }
        }
        ids.extend(hits.into_values());
    }
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    selected_members(doc, &ids)
}

pub(super) fn exposures<'a>(
    clip: &'a Clip,
    selected: &[u64],
    range: SelectionRange,
) -> io::Result<Vec<Exposure<'a>>> {
    let (reference_start, _) = clip.exposure(range.reference);
    let reference_doc = clip.drawing_at(range.reference);
    let reference = selected_members(reference_doc, selected)?;
    let mut tracks = None;
    let mut first = range.first;
    let mut exposures = Vec::new();
    while first <= range.last {
        let (start, duration) = clip.exposure(first);
        let members = if start == reference_start {
            reference.clone()
        } else {
            let tracks =
                tracks.get_or_insert_with(|| selected_tracks(clip, range.reference, &reference));
            matched(clip.drawing_at(first), tracks, start, reference_start)?
        };
        let objects: Vec<_> = members
            .into_iter()
            .filter(|m| m.layer_visible)
            .map(|m| m.object)
            .collect();
        let end = (start + duration).min(range.last + 1);
        let held = end > first + 1 && objects.iter().all(|o| o.frame_independent());
        exposures.push(Exposure {
            frames: first..end,
            held,
            objects,
        });
        first = end;
    }
    Ok(exposures)
}
