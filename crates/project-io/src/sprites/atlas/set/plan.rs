use super::{
    animation_set_file_stem, image_filename, named_clips, AnimationExportTarget, AnimationSetOutput,
};
use crate::invalid;
use crate::sprites::atlas::{Atlas, PageLimit};
use crate::sprites::{check_progress, grid, scaled_size, SheetLayout};
use document_core::{AnimationProject, Clip, ExportPreset};
use renderer::Pixmap;
use std::{collections::HashSet, io, sync::Arc};

pub(super) enum Layout {
    Sequence,
    Grid(SheetLayout),
    Packed(Atlas),
}

pub(super) struct Group {
    indices: Vec<usize>,
    stem: String,
    pub(super) frames: u32,
    pub(super) first_page: usize,
    pub(super) layout: Layout,
    pub(super) output: AnimationSetOutput,
}

impl Group {
    pub(super) fn clips<'a>(&self, project: &'a AnimationProject) -> Vec<&'a Clip> {
        self.indices.iter().map(|i| &project.clips[*i]).collect()
    }

    pub(super) fn page_count(&self) -> usize {
        match self.layout {
            Layout::Sequence => self.frames as usize,
            _ => 1,
        }
    }

    pub(super) fn filename(&self, index: usize) -> String {
        image_filename(&self.stem, self.output, index)
    }

    fn dimensions(&self) -> Option<[u32; 2]> {
        match &self.layout {
            Layout::Sequence => None,
            Layout::Grid(grid) => Some([grid.width, grid.height]),
            Layout::Packed(atlas) => {
                let page = &atlas.packing.pages[0];
                Some([page.width, page.height])
            }
        }
    }

    pub(super) fn render(
        &self,
        clips: &[&Clip],
        preset: &ExportPreset,
        progress: &mut impl FnMut(u32, u32) -> bool,
    ) -> io::Result<Pixmap> {
        match &self.layout {
            Layout::Grid(layout) => grid::render(clips, *layout, preset, progress),
            Layout::Packed(atlas) => atlas.page(clips, preset, 0, &mut 0, &mut |n, _| {
                progress(n - self.frames, self.frames)
            }),
            Layout::Sequence => Err(invalid("Choose a sprite sheet to preview.")),
        }
    }
}

/// A frozen, pixel-free layout for all sheets in the selected clip or set.
/// Analysis samples packed artwork once. Rendering a thumbnail or another sheet
/// uses the existing layout without repeating the fit checks.
pub struct AnimationSheetPreviewPlan {
    pub(super) project: Arc<AnimationProject>,
    pub(super) groups: Vec<Group>,
    pub(super) frames: u32,
    pub(super) set_name: String,
    pub(super) preset: ExportPreset,
    sheets: Vec<(String, [u32; 2])>,
}

fn frame_count<'a>(clips: impl Iterator<Item = &'a Clip>) -> io::Result<u32> {
    let frames: u64 = clips.map(|clip| u64::from(clip.frames)).sum();
    if frames == 0 {
        return Err(invalid("Select at least one animation to export."));
    }
    u32::try_from(frames)
        .ok()
        .filter(|n| *n <= u32::MAX / 2)
        .ok_or_else(|| invalid("Too many animation frames to export."))
}

impl AnimationSheetPreviewPlan {
    pub fn prepare(
        project: &AnimationProject,
        target: AnimationExportTarget,
        preset: &ExportPreset,
        output: AnimationSetOutput,
        progress: impl FnMut(u32, u32) -> bool,
    ) -> io::Result<Self> {
        if output == AnimationSetOutput::PngSequences {
            return Err(invalid("Choose a sprite sheet to preview."));
        }
        Self::build(project, target, preset, output, progress)
    }

    /// Reuse an immutable, hydrated artwork snapshot across preview settings.
    /// The supplied clip IDs select the standalone animations within the target.
    /// All other clips in the snapshot remain available as nested artwork.
    pub fn prepare_snapshot(
        project: Arc<AnimationProject>,
        target: AnimationExportTarget,
        preset: &ExportPreset,
        output: AnimationSetOutput,
        included: &[u64],
        progress: impl FnMut(u32, u32) -> bool,
    ) -> io::Result<Self> {
        if output == AnimationSetOutput::PngSequences {
            return Err(invalid("Choose a sprite sheet to preview."));
        }
        Self::from_snapshot(project, target, preset, output, included, progress)
    }

    pub fn sheets(&self) -> &[(String, [u32; 2])] {
        &self.sheets
    }

    pub fn render(
        &self,
        index: usize,
        mut progress: impl FnMut(u32, u32) -> bool,
    ) -> io::Result<Pixmap> {
        let group = self
            .groups
            .get(index)
            .ok_or_else(|| invalid("Unknown sprite sheet."))?;
        group.render(&group.clips(&self.project), &self.preset, &mut progress)
    }

    pub(super) fn build(
        project: &AnimationProject,
        target: AnimationExportTarget,
        preset: &ExportPreset,
        output: AnimationSetOutput,
        mut progress: impl FnMut(u32, u32) -> bool,
    ) -> io::Result<Self> {
        target.set_id(project)?;
        let selected = project.clips.iter().filter(|c| target.includes(c));
        let frames = frame_count(selected.clone())?;
        check_progress(0, frames, &mut progress)?;
        let included = selected.map(|clip| clip.id).collect::<Vec<_>>();
        // Headless callers can supply freshly edited sources with stale placements.
        let project = Arc::new(project.resolved_snapshot().map_err(invalid)?);
        Self::from_snapshot(project, target, preset, output, &included, progress)
    }

    fn from_snapshot(
        project: Arc<AnimationProject>,
        target: AnimationExportTarget,
        preset: &ExportPreset,
        output: AnimationSetOutput,
        included: &[u64],
        mut progress: impl FnMut(u32, u32) -> bool,
    ) -> io::Result<Self> {
        preset.validate().map_err(invalid)?;
        let set_id = target.set_id(&project)?;
        if matches!(target, AnimationExportTarget::Clip(id) if included != [id]) {
            return Err(invalid(
                "A single clip export needs exactly the selected clip.",
            ));
        }
        let set_name = project
            .animation_set(set_id)
            .ok_or_else(|| invalid("Unknown animation set."))?
            .name
            .clone();
        let set_stem = animation_set_file_stem(&set_name);
        let ids: HashSet<_> = included.iter().copied().collect();
        let members: Vec<_> = named_clips(&project, set_id, &set_stem)
            .filter(|(index, _)| ids.contains(&project.clips[*index].id))
            .collect();
        if members.len() != included.len() {
            return Err(invalid(
                "Choose each clip once from the selected animation set.",
            ));
        }
        let frames = frame_count(members.iter().map(|(index, _)| &project.clips[*index]))?;
        check_progress(0, frames, &mut progress)?;
        let chunk_size = if output == AnimationSetOutput::SharedSheet {
            members.len()
        } else {
            1
        };
        let mut done = 0;
        let mut first_page = 0;
        let mut groups = Vec::new();
        let mut sheets = Vec::new();
        for members in members.chunks(chunk_size) {
            let stem = if output == AnimationSetOutput::SharedSheet {
                &set_stem
            } else {
                &members[0].1
            };
            let indices: Vec<_> = members.iter().map(|(i, _)| *i).collect();
            let clips: Vec<_> = indices.iter().map(|i| &project.clips[*i]).collect();
            let span = clips.iter().map(|c| c.frames).sum::<u32>();
            let layout = match output {
                AnimationSetOutput::PngSequences => {
                    clips[0].validate().map_err(invalid)?;
                    scaled_size(clips[0], preset.raster_scale())?;
                    Layout::Sequence
                }
                _ if preset.packed.enabled => {
                    let atlas =
                        Atlas::analyse_clips(&clips, preset, PageLimit::Single, &mut |n, _| {
                            progress(done + n, frames)
                        })
                        .map_err(|e| io::Error::new(e.kind(), format!("{stem}.png: {e}")))?;
                    Layout::Packed(atlas)
                }
                _ => Layout::Grid(
                    grid::animation_grid_layout(&clips, preset)
                        .map_err(|e| io::Error::new(e.kind(), format!("{stem}.png: {e}")))?,
                ),
            };
            let group = Group {
                indices,
                stem: stem.clone(),
                frames: span,
                first_page,
                layout,
                output,
            };
            first_page += group.page_count();
            done += span;
            if let Some(dimensions) = group.dimensions() {
                sheets.push((group.filename(0), dimensions));
            }
            groups.push(group);
            check_progress(done, frames, &mut progress)?;
        }
        Ok(Self {
            project,
            groups,
            frames,
            set_name,
            preset: *preset,
            sheets,
        })
    }
}
