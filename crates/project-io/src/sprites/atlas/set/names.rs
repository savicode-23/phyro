use super::AnimationSetOutput;
use document_core::AnimationProject;
use std::collections::HashSet;

/// Portable lower-case component shared by the export dialog and file writer.
pub fn animation_set_file_stem(name: &str) -> String {
    let mut stem = String::new();
    let mut separator = false;
    for c in name.chars().flat_map(char::to_lowercase) {
        if !c.is_alphanumeric() {
            separator = !stem.is_empty();
            continue;
        }
        // Two components, disambiguation, a frame index and extension must fit in 255 bytes.
        if stem.len() + usize::from(separator) + c.len_utf8() > 94 {
            break;
        }
        if separator {
            stem.push('_');
            separator = false;
        }
        stem.push(c);
    }
    if stem.is_empty() {
        return "animation".into();
    }
    if crate::sprites::selection::windows_device_name(&stem) {
        stem.insert(0, '_');
    }
    stem
}

pub(super) fn named_clips<'a>(
    project: &'a AnimationProject,
    set_id: u64,
    set_stem: &'a str,
) -> impl Iterator<Item = (usize, String)> + 'a {
    let mut used = HashSet::new();
    project
        .clips
        .iter()
        .enumerate()
        .filter(move |(_, clip)| clip.set_id == set_id)
        .map(move |(index, clip)| {
            let mut stem = format!("{set_stem}_{}", animation_set_file_stem(&clip.name));
            if !used.insert(stem.to_uppercase()) {
                // The sanitizer never produces a hyphen. IDs stay stable when a sibling is unchecked.
                stem.push_str(&format!("-{}", clip.id));
            }
            (index, stem)
        })
}

pub(super) fn image_filename(stem: &str, mode: AnimationSetOutput, index: usize) -> String {
    if mode == AnimationSetOutput::PngSequences {
        format!("{stem}_{}.png", index + 1)
    } else {
        debug_assert_eq!(index, 0);
        format!("{stem}.png")
    }
}
