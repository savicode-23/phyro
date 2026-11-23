mod names;
mod plan;
pub use plan::AnimationSheetPreviewPlan;

use super::{check_progress, invalid, parent_of, ExportPreset, ExportReceipt};
use crate::sprites::{grid, scale_json, ExposureClock, FrameCache};
use document_core::{AnimationProject, Clip};
pub use names::animation_set_file_stem;
use names::{image_filename, named_clips};
use plan::{Group, Layout};
use serde::ser::{Error, SerializeSeq};
use serde::{Serialize, Serializer};
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AnimationSetOutput {
    PngSequences,
    SeparateSheets,
    #[default]
    SharedSheet,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnimationExportTarget {
    Set(u64),
    Clip(u64),
}

impl AnimationExportTarget {
    fn set_id(self, project: &AnimationProject) -> io::Result<u64> {
        match self {
            Self::Set(id) => project
                .animation_set(id)
                .map(|s| s.id)
                .ok_or_else(|| invalid("Unknown animation set.")),
            Self::Clip(id) => project
                .clip_index(id)
                .map(|i| project.clips[i].set_id)
                .ok_or_else(|| invalid("Unknown animation clip.")),
        }
    }

    /// Whether this authored clip gets a standalone animation in this export.
    pub fn includes(self, clip: &Clip) -> bool {
        match self {
            Self::Set(id) => clip.set_id == id && clip.include_in_set_export,
            Self::Clip(id) => clip.id == id,
        }
    }

    fn example_filename(
        self,
        project: &AnimationProject,
        mode: AnimationSetOutput,
    ) -> Option<String> {
        let set = project.animation_set(self.set_id(project).ok()?)?;
        let stem = animation_set_file_stem(&set.name);
        let (_, clip_stem) = named_clips(project, set.id, &stem)
            .find(|(index, _)| self.includes(&project.clips[*index]))?;
        Some(image_filename(
            if mode == AnimationSetOutput::SharedSheet {
                &stem
            } else {
                &clip_stem
            },
            mode,
            0,
        ))
    }
}

pub fn animation_set_example_filename(
    project: &AnimationProject,
    set_id: u64,
    mode: AnimationSetOutput,
) -> Option<String> {
    AnimationExportTarget::Set(set_id).example_filename(project, mode)
}

pub fn animation_clip_example_filename(
    project: &AnimationProject,
    clip_id: u64,
    mode: AnimationSetOutput,
) -> Option<String> {
    AnimationExportTarget::Clip(clip_id).example_filename(project, mode)
}

/// Export the included actions of one set into one shared packed sheet.
pub fn export_animation_set(
    directory: &Path,
    project: &AnimationProject,
    set_id: u64,
    preset: &ExportPreset,
    progress: impl FnMut(u32, u32) -> bool,
) -> io::Result<ExportReceipt> {
    export_animation_set_with_format(
        directory,
        project,
        set_id,
        preset,
        AnimationSetOutput::SharedSheet,
        progress,
    )
}

pub fn export_animation_set_with_format(
    directory: &Path,
    project: &AnimationProject,
    set_id: u64,
    preset: &ExportPreset,
    output: AnimationSetOutput,
    progress: impl FnMut(u32, u32) -> bool,
) -> io::Result<ExportReceipt> {
    let mut preset = *preset;
    preset.packed.enabled = true;
    export_animation_files(
        directory,
        project,
        AnimationExportTarget::Set(set_id),
        &preset,
        output,
        None,
        progress,
    )
}

/// An explicitly selected clip can always be exported, including a nested helper.
pub fn export_clip_with_format(
    directory: &Path,
    project: &AnimationProject,
    clip_id: u64,
    preset: &ExportPreset,
    output: AnimationSetOutput,
    progress: impl FnMut(u32, u32) -> bool,
) -> io::Result<ExportReceipt> {
    let mut preset = *preset;
    preset.packed.enabled = true;
    export_animation_files(
        directory,
        project,
        AnimationExportTarget::Clip(clip_id),
        &preset,
        output,
        None,
        progress,
    )
}

/// Write the chosen clips using the project's grid or packed settings. Each
/// requested sheet is one image. A receipt permits a checked re-export over
/// files previously written by this operation.
pub fn export_animation_files(
    directory: &Path,
    project: &AnimationProject,
    target: AnimationExportTarget,
    preset: &ExportPreset,
    output: AnimationSetOutput,
    previous: Option<&ExportReceipt>,
    mut progress: impl FnMut(u32, u32) -> bool,
) -> io::Result<ExportReceipt> {
    let packed = output != AnimationSetOutput::PngSequences && preset.packed.enabled;
    let passes = if packed { 2 } else { 1 };
    let plan =
        AnimationSheetPreviewPlan::build(project, target, preset, output, |done, frames| {
            progress(if packed { done } else { 0 }, passes * frames)
        })?;
    let analysed = if packed { plan.frames } else { 0 };
    plan.write(directory, previous, |done, frames| {
        progress(analysed + done, passes * frames)
    })
}

impl AnimationSheetPreviewPlan {
    /// Render the validated snapshot using its prepared geometry and filenames.
    pub fn write(
        &self,
        directory: &Path,
        previous: Option<&ExportReceipt>,
        mut progress: impl FnMut(u32, u32) -> bool,
    ) -> io::Result<ExportReceipt> {
        let output = self.groups[0].output;
        let preset = &self.preset;
        let total = self.frames;
        let mut done = 0;
        check_progress(0, total, &mut progress)?;
        let staging = tempfile::tempdir_in(parent_of(directory))?;
        let mut receipt = ExportReceipt::new(directory);
        for group in &self.groups {
            let clips = group.clips(&self.project);
            let mut report = |n, _| progress(done + n, total);
            if let Layout::Sequence = group.layout {
                let clip = clips[0];
                let mut cache = FrameCache::default();
                let mut clock = ExposureClock::default();
                for tick in 0..clip.frames {
                    check_progress(tick, clip.frames, &mut report)?;
                    let frame =
                        cache.sample(clip, clock.sample_tick(clip, tick), preset.raster_scale())?;
                    receipt.stage(
                        staging.path(),
                        Path::new(&group.filename(tick as usize)),
                        frame.png()?,
                    )?;
                }
            } else {
                let image = group.render(&clips, preset, &mut report)?;
                receipt.stage(
                    staging.path(),
                    Path::new(&group.filename(0)),
                    &image.encode_png().map_err(io::Error::other)?,
                )?;
            }
            done += group.frames;
        }
        let json = staging.path().join("sheet.json");
        {
            let mut writer = BufWriter::new(File::create(&json)?);
            let manifest = Manifest {
                format_version: 3,
                layout: match output {
                    AnimationSetOutput::PngSequences => "sequence",
                    _ if preset.packed.enabled => "packed",
                    _ => "grid",
                },
                coordinates: "top-left pixels",
                name: &self.set_name,
                scale: scale_json(preset.raster_scale()),
                image: self.groups[0].filename(0),
                images: Images(&self.groups),
                animations: Animations {
                    groups: &self.groups,
                    project: &self.project,
                    preset,
                },
            };
            serde_json::to_writer(&mut writer, &manifest)?;
            writer.flush()?;
            writer.get_ref().sync_all()?;
        }
        fs::copy(&json, staging.path().join("animation.phyro"))?;
        for path in ["sheet.json", "animation.phyro"] {
            receipt.register_staged(staging.path(), Path::new(path))?;
        }
        check_progress(total, total, &mut progress)?;
        receipt.publish(staging.path(), previous)?;
        Ok(receipt)
    }
}

// Emit one animation at a time, including when a large set shares atlas pages.
#[derive(Serialize)]
struct Manifest<'a> {
    format_version: u32,
    layout: &'static str,
    coordinates: &'static str,
    name: &'a str,
    scale: serde_json::Value,
    image: String,
    images: Images<'a>,
    animations: Animations<'a>,
}

struct Images<'a>(&'a [Group]);

impl Serialize for Images<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut images = serializer.serialize_seq(None)?;
        for group in self.0 {
            for page in 0..group.page_count() {
                images.serialize_element(&group.filename(page))?;
            }
        }
        images.end()
    }
}

struct Animations<'a> {
    project: &'a AnimationProject,
    groups: &'a [Group],
    preset: &'a ExportPreset,
}

impl Serialize for Animations<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut animations = serializer.serialize_seq(None)?;
        for group in self.groups {
            let mut offset = 0;
            for clip in group.clips(self.project) {
                let end = offset + clip.frames as usize;
                let animation = match &group.layout {
                    Layout::Packed(atlas) => atlas.clip_metadata(
                        clip,
                        &atlas.ticks[offset..end],
                        self.preset,
                        group.first_page,
                    ),
                    Layout::Grid(layout) => grid::metadata(
                        clip,
                        self.preset.raster_scale(),
                        *layout,
                        offset,
                        group.first_page,
                    ),
                    Layout::Sequence => {
                        grid::untrimmed_metadata(clip, self.preset.raster_scale(), |index| {
                            (group.first_page + index as usize, 0, 0)
                        })
                    }
                }
                .map_err(S::Error::custom)?;
                animations.serialize_element(&animation)?;
                offset = end;
            }
        }
        animations.end()
    }
}
