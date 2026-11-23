use super::{
    check_progress, copy_cell, event_markers, grid_layout, invalid, new_sheet, scaled_pivot,
    scaled_size, Clip, ExportPreset, ExposureClock, FrameCache, Pixmap, SheetLayout,
};
use std::io;

/// Measure a uniform grid in output pixels without sampling any artwork.
pub fn animation_grid_layout(clips: &[&Clip], preset: &ExportPreset) -> io::Result<SheetLayout> {
    let (first, rest) = clips
        .split_first()
        .ok_or_else(|| invalid("Select at least one clip for the sprite sheet."))?;
    let scale = preset.raster_scale();
    let size = scaled_size(first, scale)?;
    let mut frames = first.frames;
    for clip in rest {
        if scaled_size(clip, scale)? != size {
            return Err(invalid(
                "A uniform grid needs clips of the same artboard size.",
            ));
        }
        frames = frames
            .checked_add(clip.frames)
            .ok_or_else(|| invalid("Too many frames for one sprite sheet."))?;
    }
    grid_layout(size.0, size.1, frames, preset)
}

pub(super) fn render(
    clips: &[&Clip],
    layout: SheetLayout,
    preset: &ExportPreset,
    progress: &mut impl FnMut(u32, u32) -> bool,
) -> io::Result<Pixmap> {
    let total = clips.iter().map(|c| c.frames).sum();
    let scale = preset.raster_scale();
    let mut sheet = new_sheet(layout)?;
    let mut cell = 0;
    for clip in clips {
        clip.validate().map_err(invalid)?;
        let mut clock = ExposureClock::default();
        let mut cache = FrameCache::default();
        for tick in 0..clip.frames {
            check_progress(cell, total, progress)?;
            let frame = cache.sample(clip, clock.sample_tick(clip, tick), scale)?;
            copy_cell(&mut sheet, &frame.image, layout, cell)?;
            cell += 1;
        }
    }
    Ok(sheet)
}

pub(super) fn metadata(
    clip: &Clip,
    scale: f64,
    layout: SheetLayout,
    first_cell: usize,
    page: usize,
) -> io::Result<serde_json::Value> {
    untrimmed_metadata(clip, scale, |index| {
        let (x, y) = layout.cell(first_cell as u32 + index);
        (page, x, y)
    })
}

/// Grid cells and sequence images both contain the entire artboard.
pub(super) fn untrimmed_metadata(
    clip: &Clip,
    scale: f64,
    placement: impl Fn(u32) -> (usize, u32, u32),
) -> io::Result<serde_json::Value> {
    let (width, height) = scaled_size(clip, scale)?;
    let pivot = scaled_pivot(clip, width, height)?;
    let frames: Vec<_> = (0..clip.frames)
        .map(|index| {
            let (page, x, y) = placement(index);
            serde_json::json!({
                "index":index, "page":page, "x":x, "y":y, "w":width, "h":height,
                "pivot":pivot, "duration_ticks":1
            })
        })
        .collect();
    Ok(serde_json::json!({
        "name":clip.name, "fps":clip.fps, "looping":clip.looping, "frames":frames, "events":event_markers(clip)
    }))
}
