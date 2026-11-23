//! Grid sheets are sampled on the same integer clock as the editor.
//! All atlas coordinates and pivots are in output pixels with a top-left origin.
mod atlas;
mod grid;
mod publish;
mod sampling;
mod selection;
pub use atlas::{
    animation_clip_example_filename, animation_set_example_filename, animation_set_file_stem,
    export_animation_files, export_animation_set, export_animation_set_with_format,
    export_clip_with_format, export_packed_atlas, render_packed_preview, AnimationExportTarget,
    AnimationSetOutput, AnimationSheetPreviewPlan, PackedPreviewPlan,
};
pub use grid::animation_grid_layout;
pub use selection::animation::{
    export_selection_animation, selection_animation_dimensions, SelectionAnimationExport,
    SelectionRange,
};
pub use selection::{
    export_object_pngs, export_object_svgs, export_selection_png, export_selection_svg,
    export_still, png_dimensions, selection_png_native_size, PngDimensions, SelectionAsset,
    StillFormat, StillOptions, StillTarget,
};

use crate::{invalid, parent_of, write_file, WriteMode};
use document_core::{Clip, ExportPreset};
pub use publish::ExportReceipt;
use renderer::Pixmap;
use sampling::{ExposureClock, FrameCache};
use serde::Serialize;
use std::fs;
use std::io::{self, Write};
use std::path::Path;

use document_core::MAX_RASTER_PIXELS as MAX_PIXELS;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpriteOutput {
    Sheet,
    Frames,
    SheetAndFrames,
    Packed,
}

#[derive(Clone, Copy, Debug)]
pub struct SheetLayout {
    pub columns: u32,
    pub rows: u32,
    pub width: u32,
    pub height: u32,
    pub frame_width: u32,
    pub frame_height: u32,
    margin: u32,
    padding: u32,
}

impl SheetLayout {
    fn cell(self, index: u32) -> (u32, u32) {
        (
            self.margin + (index % self.columns) * (self.frame_width + self.padding),
            self.margin + (index / self.columns) * (self.frame_height + self.padding),
        )
    }
}

/// Preserve integral scales as JSON integers for consumers using the original export contract.
pub fn scale_json(scale: f64) -> serde_json::Value {
    if scale.fract() == 0. {
        serde_json::json!(scale as u32)
    } else {
        serde_json::json!(scale)
    }
}

fn scaled_size(clip: &Clip, scale: f64) -> io::Result<(u32, u32)> {
    if !(scale > 0. && scale <= 32.)
        || clip.frames == 0
        || clip.fps == 0
        || !clip.cels.contains_key(&0)
    {
        return Err(invalid(
            "A clip needs a first drawing, positive timing and a positive PNG scale up to 32×.",
        ));
    }
    let doc = clip.drawing_at(0);
    let [w, h] = ExportPreset::scaled_size([doc.width, doc.height], scale).map_err(invalid)?;
    Ok((w, h))
}

pub fn sheet_layout(clip: &Clip, preset: &ExportPreset) -> io::Result<SheetLayout> {
    let (w, h) = scaled_size(clip, preset.raster_scale())?;
    grid_layout(w, h, clip.frames, preset)
}

fn grid_layout(w: u32, h: u32, frames: u32, preset: &ExportPreset) -> io::Result<SheetLayout> {
    if !(1..=4096).contains(&preset.columns) || preset.margin > 4096 || preset.padding > 4096 {
        return Err(invalid(
            "Choose 1–4096 columns and margins and padding of at most 4096 pixels.",
        ));
    }
    let columns = preset.columns;
    let rows = frames.div_ceil(columns);
    let extent = |n, size| {
        u64::from(n) * u64::from(size)
            + u64::from(n - 1) * u64::from(preset.padding)
            + 2 * u64::from(preset.margin)
    };
    let width = extent(columns, w);
    let height = extent(rows, h);
    if width.checked_mul(height).is_none_or(|p| p > MAX_PIXELS) {
        return Err(invalid(
            "Sheet exceeds 16 megapixels. Reduce scale or spacing, or export individual frames.",
        ));
    }
    Ok(SheetLayout {
        columns,
        rows,
        width: width as u32,
        height: height as u32,
        frame_width: w,
        frame_height: h,
        margin: preset.margin,
        padding: preset.padding,
    })
}

fn render_tick(clip: &Clip, index: u32, scale: f64) -> io::Result<Pixmap> {
    if index >= clip.frames {
        return Err(invalid("Frame is outside the clip."));
    }
    let doc = clip.drawing_at(index);
    renderer::render_for_export(doc, index, scale).map_err(invalid)
}

fn copy_cell(
    sheet: &mut Pixmap,
    image: &Pixmap,
    layout: SheetLayout,
    index: u32,
) -> io::Result<()> {
    if (image.width(), image.height()) != (layout.frame_width, layout.frame_height) {
        return Err(invalid(
            "Clip drawings have inconsistent artboard dimensions.",
        ));
    }
    let (x, y) = layout.cell(index);
    copy_pixels(sheet, image, x, y)
}

fn copy_pixels(target: &mut Pixmap, image: &Pixmap, x: u32, y: u32) -> io::Result<()> {
    if x.checked_add(image.width())
        .is_none_or(|r| r > target.width())
        || y.checked_add(image.height())
            .is_none_or(|b| b > target.height())
    {
        return Err(invalid("Rendered pixels fall outside their output frame."));
    }
    let stride = target.width() as usize * 4;
    let row_len = image.width() as usize * 4;
    for (row, pixels) in image.data().chunks_exact(row_len).enumerate() {
        let start = (y as usize + row) * stride + x as usize * 4;
        target.data_mut()[start..start + row_len].copy_from_slice(pixels);
    }
    Ok(())
}

fn new_sheet(layout: SheetLayout) -> io::Result<Pixmap> {
    Pixmap::new(layout.width, layout.height)
        .ok_or_else(|| invalid("Unable to allocate the sprite sheet."))
}

fn check_progress(
    completed: u32,
    total: u32,
    progress: &mut impl FnMut(u32, u32) -> bool,
) -> io::Result<()> {
    if progress(completed, total) {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Export cancelled.",
        ))
    }
}

/// Sample the very same cells used by the writer. Call off the UI thread.
pub fn render_sheet_preview(
    clip: &Clip,
    preset: &ExportPreset,
    mut progress: impl FnMut(u32, u32) -> bool,
) -> io::Result<Pixmap> {
    let layout = sheet_layout(clip, preset)?;
    grid::render(&[clip], layout, preset, &mut progress)
}

#[derive(Serialize)]
struct Frame {
    index: u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    pivot: [f32; 2],
    duration_ticks: u32,
}

#[derive(serde::Serialize)]
struct EventMarker<'a> {
    frame: u32,
    name: &'a str,
}
fn event_markers(clip: &Clip) -> Vec<EventMarker<'_>> {
    clip.events
        .iter()
        .flat_map(|(&frame, names)| names.iter().map(move |name| EventMarker { frame, name }))
        .collect()
}

#[derive(Serialize)]
struct Metadata<'a> {
    format_version: u32,
    image: &'static str,
    name: &'a str,
    fps: u32,
    looping: bool,
    coordinates: &'static str,
    scale: serde_json::Value,
    columns: u32,
    rows: u32,
    frames: Vec<Frame>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    events: Vec<EventMarker<'a>>,
}

/// Map the authored pivot using the exact integer dimensions chosen by the renderer.
pub fn scaled_pivot(clip: &Clip, width: u32, height: u32) -> io::Result<[f32; 2]> {
    let artboard = clip.drawing_at(0);
    let pivot = [
        clip.origin[0] * (width as f32 / artboard.width as f32),
        clip.origin[1] * (height as f32 / artboard.height as f32),
    ];
    if !pivot.into_iter().all(f32::is_finite) {
        return Err(invalid("Scaled sprite origin exceeds coordinate range."));
    }
    Ok(pivot)
}

fn metadata(clip: &Clip, preset: &ExportPreset, layout: SheetLayout) -> io::Result<Vec<u8>> {
    let pivot = scaled_pivot(clip, layout.frame_width, layout.frame_height)?;
    let frames = (0..clip.frames)
        .map(|index| {
            let (x, y) = layout.cell(index);
            Frame {
                index,
                x,
                y,
                w: layout.frame_width,
                h: layout.frame_height,
                pivot,
                duration_ticks: 1,
            }
        })
        .collect();
    serde_json::to_vec_pretty(&Metadata {
        format_version: 1,
        image: "sheet.png",
        name: &clip.name,
        fps: clip.fps,
        looping: clip.looping,
        coordinates: "top-left; pivot relative to untrimmed frame, in output pixels",
        scale: scale_json(preset.raster_scale()),
        columns: layout.columns,
        rows: layout.rows,
        frames,
        events: event_markers(clip),
    })
    .map_err(io::Error::other)
}

pub fn export_clip_frame(
    path: &Path,
    clip: &Clip,
    index: u32,
    scale: impl Into<f64>,
    replace: bool,
) -> io::Result<()> {
    let scale = scale.into();
    scaled_size(clip, scale)?;
    let png = render_tick(clip, index, scale)?
        .encode_png()
        .map_err(io::Error::other)?;
    let mode = if replace {
        WriteMode::Replace
    } else {
        WriteMode::Create
    };
    write_file(path, mode, |out| out.write_all(&png))
}

pub fn export_sprite_bundle(
    directory: &Path,
    clip: &Clip,
    preset: &ExportPreset,
    include_frames: bool,
) -> io::Result<()> {
    let mode = if include_frames {
        SpriteOutput::SheetAndFrames
    } else {
        SpriteOutput::Sheet
    };
    export_sprite_output(directory, clip, preset, mode, None, |_, _| true).map(|_| ())
}

/// Passing the receipt authorizes updates only to unchanged outputs from that export.
pub fn export_sprite_output(
    directory: &Path,
    clip: &Clip,
    preset: &ExportPreset,
    mode: SpriteOutput,
    previous: Option<&ExportReceipt>,
    mut progress: impl FnMut(u32, u32) -> bool,
) -> io::Result<ExportReceipt> {
    if mode == SpriteOutput::Packed {
        return atlas::export(directory, clip, preset, previous, progress);
    }
    scaled_size(clip, preset.raster_scale())?;
    clip.validate().map_err(invalid)?;
    check_progress(0, clip.frames, &mut progress)?;
    let layout = (mode != SpriteOutput::Frames)
        .then(|| sheet_layout(clip, preset))
        .transpose()?;
    let mut sheet = layout.map(new_sheet).transpose()?;
    let staging = tempfile::tempdir_in(parent_of(directory))?;
    let mut receipt = ExportReceipt::new(directory);
    if mode == SpriteOutput::SheetAndFrames {
        fs::create_dir(staging.path().join("frames"))?;
    }
    let mut clock = ExposureClock::default();
    let mut frames = FrameCache::default();
    for i in 0..clip.frames {
        let frame = frames.sample(clip, clock.sample_tick(clip, i), preset.raster_scale())?;
        if let (Some(layout), Some(atlas)) = (layout, &mut sheet) {
            copy_cell(atlas, &frame.image, layout, i)?;
        }
        if mode != SpriteOutput::Sheet {
            let name = format!("frame_{i:04}.png");
            let path = if mode == SpriteOutput::SheetAndFrames {
                Path::new("frames").join(name)
            } else {
                name.into()
            };
            receipt.stage(staging.path(), &path, frame.png()?)?;
        }
        check_progress(i + 1, clip.frames, &mut progress)?;
    }
    drop(frames);
    if let (Some(layout), Some(atlas)) = (layout, sheet) {
        receipt.stage(
            staging.path(),
            Path::new("sheet.png"),
            &atlas.encode_png().map_err(io::Error::other)?,
        )?;
        receipt.stage(
            staging.path(),
            Path::new("sheet.json"),
            &metadata(clip, preset, layout)?,
        )?;
    }
    check_progress(clip.frames, clip.frames, &mut progress)?;
    receipt.publish(staging.path(), previous)?;
    Ok(receipt)
}
