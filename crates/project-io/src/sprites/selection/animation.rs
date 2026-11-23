//! Export a selected part in one coordinate system throughout a clip range.
mod matching;

use super::super::{
    check_progress, copy_pixels, grid_layout, new_sheet, ExportReceipt, Frame, SpriteOutput,
    MAX_PIXELS,
};
use super::require_new_directory;
use crate::{invalid, parent_of};
use document_core::{Clip, ExportPreset};
use matching::Exposure;
use renderer::Pixmap;
use serde::Serialize;
use serde_json::json;
use std::{fs, io, path::Path};

/// Zero-based clip ticks. The inclusive output range may exclude the reference
/// drawing where the artist made the selection.
#[derive(Clone, Copy, Debug)]
pub struct SelectionRange {
    pub reference: u32,
    pub first: u32,
    pub last: u32,
}

#[derive(Debug)]
pub struct SelectionAnimationExport {
    pub receipt: ExportReceipt,
    pub origin: [i32; 2],
    pub size: [u32; 2],
    pub pivot: [f32; 2],
}

impl SelectionRange {
    fn count(self, clip: &Clip) -> io::Result<u32> {
        if self.reference >= clip.frames || self.last >= clip.frames || self.first > self.last {
            return Err(invalid(format!("Choose a reference tick and an inclusive first-to-last range inside the clip's {} frames.", clip.frames)));
        }
        Ok(self.last - self.first + 1)
    }
}

#[derive(Clone, Copy)]
struct Bounds {
    left: i32,
    top: i32,
    right: i64,
    bottom: i64,
}

impl Bounds {
    fn from_image(image: &Pixmap, [left, top]: [i32; 2]) -> Self {
        Self {
            left,
            top,
            right: i64::from(left) + i64::from(image.width()),
            bottom: i64::from(top) + i64::from(image.height()),
        }
    }

    fn union(self, other: Self) -> Self {
        Self {
            left: self.left.min(other.left),
            top: self.top.min(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
        }
    }

    fn size(self) -> io::Result<[u32; 2]> {
        let width = (self.right - i64::from(self.left)) as u64;
        let height = (self.bottom - i64::from(self.top)) as u64;
        if width
            .checked_mul(height)
            .is_none_or(|area| area > MAX_PIXELS)
        {
            return Err(invalid("The selected animation spans more than 16 megapixels. Reduce the scale or shorten the range."));
        }
        Ok([width as u32, height as u32])
    }
}

fn measure(
    exposures: &[Exposure<'_>],
    scale: f64,
    range: SelectionRange,
    total: u32,
    progress: &mut impl FnMut(u32, u32) -> bool,
) -> io::Result<Bounds> {
    let mut union: Option<Bounds> = None;
    for exposure in exposures {
        for ticks in exposure.samples() {
            if let Some((image, origin)) =
                renderer::render_artwork_pose(&exposure.objects, ticks.start, scale)
                    .map_err(invalid)?
            {
                let pose = Bounds::from_image(&image, origin);
                let bounds = union.map_or(pose, |prior| prior.union(pose));
                bounds.size()?;
                union = Some(bounds);
            }
            check_progress(ticks.end - range.first, total, progress)?;
        }
    }
    // A fully transparent range still has authored timing, events and a pivot.
    Ok(union.unwrap_or(Bounds {
        left: 0,
        top: 0,
        right: 1,
        bottom: 1,
    }))
}

/// Pixel dimensions of the common crop and optional sheet, measured at the requested density.
pub fn selection_animation_dimensions(
    clip: &Clip,
    selected: &[u64],
    range: SelectionRange,
    preset: &ExportPreset,
    mode: SpriteOutput,
    mut progress: impl FnMut(u32, u32) -> bool,
) -> io::Result<([u32; 2], Option<[u32; 2]>)> {
    if mode == SpriteOutput::Packed {
        return Err(invalid(
            "Choose a grid sheet or a PNG sequence for an animated selection.",
        ));
    }
    let count = range.count(clip)?;
    preset.validate().map_err(invalid)?;
    let exposures = matching::exposures(clip, selected, range)?;
    let frame = measure(
        &exposures,
        preset.raster_scale(),
        range,
        count,
        &mut progress,
    )?
    .size()?;
    let sheet = (mode != SpriteOutput::Frames)
        .then(|| grid_layout(frame[0], frame[1], count, preset))
        .transpose()?
        .map(|grid| [grid.width, grid.height]);
    Ok((frame, sheet))
}

fn frame_png(pose: Option<&Pixmap>, offset: [u32; 2], size: [u32; 2]) -> io::Result<Vec<u8>> {
    if let Some(image) = pose.filter(|p| p.width() == size[0] && p.height() == size[1]) {
        return image.encode_png().map_err(io::Error::other);
    }
    let mut frame = Pixmap::new(size[0], size[1])
        .ok_or_else(|| invalid("Could not allocate the selection frame."))?;
    if let Some(image) = pose {
        copy_pixels(&mut frame, image, offset[0], offset[1])?;
    }
    frame.encode_png().map_err(io::Error::other)
}

#[derive(Serialize)]
struct SelectionFrame {
    #[serde(flatten)]
    frame: Frame,
    source_frame: u32,
    trim_offset: [u32; 2],
    trim_size: [u32; 2],
    #[serde(skip_serializing_if = "Option::is_none")]
    file: Option<String>,
}

/// Select IDs in the reference drawing. Across independently drawn cels a part
/// follows its unique object and layer names. Each frame uses the union of all
/// selected poses, with the clip pivot expressed in that common output space.
pub fn export_selection_animation(
    directory: &Path,
    clip: &Clip,
    selected: &[u64],
    source: SelectionRange,
    preset: &ExportPreset,
    mode: SpriteOutput,
    mut progress: impl FnMut(u32, u32) -> bool,
) -> io::Result<SelectionAnimationExport> {
    require_new_directory(directory)?;
    if mode == SpriteOutput::Packed {
        return Err(invalid(
            "Choose a grid sheet, PNG frames, or a sheet with frames for a selection animation.",
        ));
    }
    clip.validate().map_err(invalid)?;
    let count = source.count(clip)?;
    let total = count * 2;
    check_progress(0, total, &mut progress)?;
    preset.validate().map_err(invalid)?;
    let exposures = matching::exposures(clip, selected, source)?;
    let bounds = measure(
        &exposures,
        preset.raster_scale(),
        source,
        total,
        &mut progress,
    )?;
    let origin = [bounds.left, bounds.top];
    let size = bounds.size()?;
    let pivot = std::array::from_fn(|i| {
        (f64::from(clip.origin[i]) * preset.raster_scale() - f64::from(origin[i])) as f32
    });
    if !pivot.into_iter().all(f32::is_finite) {
        return Err(invalid(
            "Scaled selection pivot exceeds the coordinate range.",
        ));
    }
    let layout = (mode != SpriteOutput::Frames)
        .then(|| grid_layout(size[0], size[1], count, preset))
        .transpose()?;
    let mut sheet = layout.map(new_sheet).transpose()?;
    let individual = mode != SpriteOutput::Sheet;
    let staging = tempfile::tempdir_in(parent_of(directory))?;
    if individual {
        fs::create_dir(staging.path().join("frames"))?;
    }
    let mut receipt = ExportReceipt::new(directory);
    let mut frames = Vec::with_capacity(count as usize);
    for exposure in &exposures {
        for ticks in exposure.samples() {
            let pose = renderer::render_artwork_pose(
                &exposure.objects,
                ticks.start,
                preset.raster_scale(),
            )
            .map_err(invalid)?;
            let mut offset = [0, 0];
            let image = pose.as_ref().map(|(image, at)| {
                // The measured union contains every pose at exactly the same native density.
                offset = std::array::from_fn(|i| (i64::from(at[i]) - i64::from(origin[i])) as u32);
                image
            });
            let trim_size = image.map_or([0, 0], |p| [p.width(), p.height()]);
            let png = individual
                .then(|| frame_png(image, offset, size))
                .transpose()?;
            for tick in ticks {
                let index = tick - source.first;
                let (x, y) = layout.map_or((0, 0), |grid| grid.cell(index));
                if let (Some(atlas), Some(ink)) = (&mut sheet, image) {
                    copy_pixels(atlas, ink, x + offset[0], y + offset[1])?;
                }
                let file = if let Some(png) = &png {
                    let file = format!("frames/frame_{index:04}.png");
                    receipt.stage(staging.path(), Path::new(&file), png)?;
                    Some(file)
                } else {
                    None
                };
                frames.push(SelectionFrame {
                    frame: Frame {
                        index,
                        x,
                        y,
                        w: size[0],
                        h: size[1],
                        pivot,
                        duration_ticks: 1,
                    },
                    source_frame: tick,
                    trim_offset: offset,
                    trim_size,
                    file,
                });
                check_progress(count + index + 1, total, &mut progress)?;
            }
        }
    }
    if let Some(sheet) = sheet {
        receipt.stage(
            staging.path(),
            Path::new("sheet.png"),
            &sheet.encode_png().map_err(io::Error::other)?,
        )?;
    }
    let events: Vec<_> = clip
        .events
        .range(source.first..=source.last)
        .flat_map(|(&tick, names)| {
            names
                .iter()
                .map(move |name| json!({"frame":tick-source.first,"source_frame":tick,"name":name}))
        })
        .collect();
    let metadata = json!({
        "format_version":1, "name":clip.name, "fps":clip.fps, "looping":clip.looping,
        "source_start":source.first,"source_end":source.last,"reference_frame":source.reference,
        "origin":origin,"frame_size":size,"pivot":pivot,"scale":super::super::scale_json(preset.raster_scale()),
        "coordinates":"Top-left in output pixels. Origin is relative to the artboard. Pivot and trim offsets are relative to the common selection frame.",
        "image": layout.map(|_| "sheet.png"), "columns":layout.map(|l|l.columns),"rows":layout.map(|l|l.rows),
        "frames":frames,"events":events
    });
    receipt.stage(
        staging.path(),
        Path::new("sheet.json"),
        &serde_json::to_vec_pretty(&metadata).map_err(io::Error::other)?,
    )?;
    check_progress(total, total, &mut progress)?;
    receipt.publish(staging.path(), None)?;
    Ok(SelectionAnimationExport {
        receipt,
        origin,
        size,
        pivot,
    })
}
