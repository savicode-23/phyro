//! Analyse frames one at a time, then render each packed page in turn. Memory scales with
//! one output page and two sampled frames, even for clips of thousands of ticks.
mod packing;
mod set;
use super::{
    check_progress, invalid, parent_of, render_tick, scaled_size, Clip, ExportPreset,
    ExportReceipt, ExposureClock, FrameCache, Pixmap,
};
use packing::{Packing, PageLimit, Region, Slot};
pub use set::{
    animation_clip_example_filename, animation_set_example_filename, animation_set_file_stem,
    export_animation_files, export_animation_set, export_animation_set_with_format,
    export_clip_with_format, AnimationExportTarget, AnimationSetOutput, AnimationSheetPreviewPlan,
};
use std::collections::{hash_map::DefaultHasher, BTreeMap};
use std::hash::Hasher;
use std::io;
use std::path::Path;

struct Sample {
    clip_index: usize,
    frame: u32,
    region: Region,
}
#[derive(Clone, Copy)]
struct Tick {
    region: Region,
    unique: usize,
}
struct Atlas {
    unique: Vec<Sample>,
    ticks: Vec<Tick>,
    packing: Packing,
}

fn occupied(image: &Pixmap, trim: bool) -> Region {
    if !trim {
        return Region {
            x: 0,
            y: 0,
            w: image.width(),
            h: image.height(),
        };
    }
    match renderer::ink_bounds(image) {
        Some(r) => Region {
            x: r.x() as u32,
            y: r.y() as u32,
            w: r.width(),
            h: r.height(),
        },
        None => Region {
            x: 0,
            y: 0,
            w: 1,
            h: 1,
        },
    }
}

fn scanlines(image: &Pixmap, r: Region) -> impl Iterator<Item = &[u8]> {
    let stride = image.width() as usize * 4;
    image
        .data()
        .chunks_exact(stride)
        .skip(r.y as usize)
        .take(r.h as usize)
        .map(move |row| &row[r.x as usize * 4..(r.x + r.w) as usize * 4])
}

impl Atlas {
    fn analyse(
        clip: &Clip,
        preset: &ExportPreset,
        progress: &mut impl FnMut(u32, u32) -> bool,
    ) -> io::Result<Self> {
        Self::analyse_clips(&[clip], preset, PageLimit::Multiple, progress)
    }

    fn analyse_clips(
        clips: &[&Clip],
        preset: &ExportPreset,
        pages: PageLimit,
        progress: &mut impl FnMut(u32, u32) -> bool,
    ) -> io::Result<Self> {
        if clips.is_empty() {
            return Err(invalid("Add a clip before exporting this set."));
        }
        preset.validate().map_err(invalid)?;
        let mut total = 0;
        for clip in clips {
            clip.validate().map_err(invalid)?;
            scaled_size(clip, preset.raster_scale())?;
            total += clip.frames;
        }
        let mut occupied_pixels = 0;
        let mut unique: Vec<Sample> = Vec::new();
        let mut ticks: Vec<Tick> = Vec::with_capacity(total as usize);
        let mut signatures: BTreeMap<(u32, u32, u64), Vec<usize>> = BTreeMap::new();
        let mut reference: Option<(usize, Pixmap)> = None;
        for (clip_index, clip) in clips.iter().enumerate() {
            let offset = ticks.len() as u32;
            let mut clock = ExposureClock::default();
            for frame in 0..clip.frames {
                check_progress(offset + frame, total * 2, progress)?;
                let source = clock.sample_tick(clip, frame);
                if source < frame {
                    let mut tick = ticks[(offset + source) as usize];
                    if !preset.packed.deduplicate {
                        pages.reserve(tick.region, preset, &mut occupied_pixels)?;
                        tick.unique = unique.len();
                        unique.push(Sample {
                            clip_index,
                            frame: source,
                            region: tick.region,
                        });
                    }
                    ticks.push(tick);
                    continue;
                }
                let image = render_tick(clip, frame, preset.raster_scale())?;
                let region = occupied(&image, preset.packed.trim);
                let mut matched = None;
                if preset.packed.deduplicate {
                    let mut hash = DefaultHasher::new();
                    for row in scanlines(&image, region) {
                        hash.write(row);
                    }
                    let candidates = signatures
                        .entry((region.w, region.h, hash.finish()))
                        .or_default();
                    for &index in candidates.iter() {
                        // Compare actual pixels before sharing an atlas region, including across clips
                        // that happen to produce a hash collision.
                        let other = &unique[index];
                        if reference
                            .as_ref()
                            .is_none_or(|(cached, _)| *cached != index)
                        {
                            // Keep at most the current sample and one comparison frame.
                            drop(reference.take());
                            let pixels = render_tick(
                                clips[other.clip_index],
                                other.frame,
                                preset.raster_scale(),
                            )?;
                            reference = Some((index, pixels));
                        }
                        let (_, pixels) = reference.as_ref().expect("comparison frame is cached");
                        if scanlines(&image, region).eq(scanlines(pixels, other.region)) {
                            matched = Some(index);
                            break;
                        }
                    }
                    if matched.is_none() {
                        candidates.push(unique.len());
                    }
                }
                let index = if let Some(index) = matched {
                    index
                } else {
                    pages.reserve(region, preset, &mut occupied_pixels)?;
                    let index = unique.len();
                    unique.push(Sample {
                        clip_index,
                        frame,
                        region,
                    });
                    if preset.packed.deduplicate {
                        reference = Some((index, image));
                    }
                    index
                };
                ticks.push(Tick {
                    region,
                    unique: index,
                });
            }
        }
        drop(reference);
        let regions = unique.iter().map(|s| s.region).collect::<Vec<_>>();
        let packing = Packing::plan(&regions, preset, pages)?;
        Ok(Self {
            unique,
            ticks,
            packing,
        })
    }

    fn page(
        &self,
        clips: &[&Clip],
        preset: &ExportPreset,
        index: usize,
        copied: &mut u32,
        progress: &mut impl FnMut(u32, u32) -> bool,
    ) -> io::Result<Pixmap> {
        let page = self
            .packing
            .pages
            .get(index)
            .ok_or_else(|| invalid("Unknown atlas page."))?;
        let mut sheet = Pixmap::new(page.width, page.height)
            .ok_or_else(|| invalid("Could not allocate the atlas page."))?;
        let mut frames = FrameCache::default();
        let mut last_clip = None;
        let total = self.ticks.len() as u32;
        for &id in &page.entries {
            check_progress(total + *copied, total * 2, progress)?;
            let sample = &self.unique[id];
            if last_clip != Some(sample.clip_index) {
                frames = FrameCache::default();
                last_clip = Some(sample.clip_index);
            }
            let source = frames.sample(
                clips[sample.clip_index],
                sample.frame,
                preset.raster_scale(),
            )?;
            let slot = self.packing.slots[id];
            paste(
                &mut sheet,
                &source.image,
                sample.region,
                slot,
                preset.packed.extrude,
            );
            *copied += 1;
        }
        Ok(sheet)
    }

    fn clip_metadata(
        &self,
        clip: &Clip,
        ticks: &[Tick],
        preset: &ExportPreset,
        first_page: usize,
    ) -> io::Result<serde_json::Value> {
        let (w, h) = scaled_size(clip, preset.raster_scale())?;
        let pivot = super::scaled_pivot(clip, w, h)?;
        let frames: Vec<_> = ticks
            .iter()
            .enumerate()
            .map(|(index, tick)| {
                let slot = self.packing.slots[tick.unique];
                let r = tick.region;
                let (stored_width, stored_height) =
                    if slot.rotated { (r.h, r.w) } else { (r.w, r.h) };
                serde_json::json!({
                    "index":index,
                    "page":first_page + slot.page,
                    "x":slot.x,
                    "y":slot.y,
                    "w":stored_width,
                    "h":stored_height,
                    "rotated":slot.rotated,
                    "source_w":w,
                    "source_h":h,
                    "trim_x":r.x,
                    "trim_y":r.y,
                    "pivot":pivot,
                    "duration_ticks":1
                })
            })
            .collect();
        Ok(serde_json::json!({
            "name":clip.name,
            "fps":clip.fps,
            "looping":clip.looping,
            "frames":frames,
            "events":super::event_markers(clip)
        }))
    }

    fn header(&self, preset: &ExportPreset, version: u32) -> serde_json::Value {
        let pages: Vec<_> = (0..self.packing.pages.len()).map(filename).collect();
        serde_json::json!({
            "format_version":version, "layout":"packed", "coordinates":"top-left pixels",
            "image":pages[0], "images":pages, "scale":super::scale_json(preset.raster_scale())
        })
    }

    fn metadata(&self, clip: &Clip, preset: &ExportPreset) -> io::Result<Vec<u8>> {
        let mut header = self.header(preset, 2);
        let serde_json::Value::Object(animation) =
            self.clip_metadata(clip, &self.ticks, preset, 0)?
        else {
            unreachable!()
        };
        header.as_object_mut().unwrap().extend(animation);
        serde_json::to_vec_pretty(&header).map_err(io::Error::other)
    }
}

fn filename(index: usize) -> String {
    if index == 0 {
        "sheet.png".into()
    } else {
        format!("sheet-{}.png", index + 1)
    }
}

// Copy the centre as rows; replicate border pixels into the extrusion around all four sides.
fn paste(sheet: &mut Pixmap, image: &Pixmap, r: Region, slot: Slot, bleed: u32) {
    let (x, y) = (slot.x, slot.y);
    if slot.rotated {
        let stride = sheet.width() as usize;
        let width = image.width() as usize;
        let dest = sheet.pixels_mut();
        for dy in 0..r.w + 2 * bleed {
            let sx = r.x + dy.saturating_sub(bleed).min(r.w - 1);
            let row = (y - bleed + dy) as usize * stride + (x - bleed) as usize;
            for dx in 0..r.h + 2 * bleed {
                let sy = r.y + r.h - 1 - dx.saturating_sub(bleed).min(r.h - 1);
                dest[row + dx as usize] = image.pixels()[sy as usize * width + sx as usize];
            }
        }
        return;
    }
    let stride = sheet.width() as usize * 4;
    let source_width = image.width() as usize;
    for dy in 0..r.h + 2 * bleed {
        let sy = r.y + dy.saturating_sub(bleed).min(r.h - 1);
        let source_start = (sy as usize * source_width + r.x as usize) * 4;
        let source = &image.data()[source_start..source_start + r.w as usize * 4];
        let start = (y - bleed + dy) as usize * stride + (x - bleed) as usize * 4;
        let row = &mut sheet.data_mut()[start..start + (r.w + 2 * bleed) as usize * 4];
        for pixel in row[..bleed as usize * 4].as_chunks_mut::<4>().0 {
            pixel.copy_from_slice(&source[..4]);
        }
        let middle = bleed as usize * 4;
        row[middle..middle + source.len()].copy_from_slice(source);
        for pixel in row[middle + source.len()..].as_chunks_mut::<4>().0 {
            pixel.copy_from_slice(&source[source.len() - 4..]);
        }
    }
}

pub(super) fn export(
    directory: &Path,
    clip: &Clip,
    preset: &ExportPreset,
    previous: Option<&ExportReceipt>,
    mut progress: impl FnMut(u32, u32) -> bool,
) -> io::Result<ExportReceipt> {
    let atlas = Atlas::analyse(clip, preset, &mut progress)?;
    let staging = tempfile::tempdir_in(parent_of(directory))?;
    let mut receipt = ExportReceipt::new(directory);
    let mut copied = 0;
    for page in 0..atlas.packing.pages.len() {
        let image = atlas.page(&[clip], preset, page, &mut copied, &mut progress)?;
        receipt.stage(
            staging.path(),
            Path::new(&filename(page)),
            &image.encode_png().map_err(io::Error::other)?,
        )?;
    }
    let metadata = atlas.metadata(clip, preset)?;
    for name in ["sheet.json", "animation.phyro"] {
        receipt.stage(staging.path(), Path::new(name), &metadata)?;
    }
    check_progress(clip.frames * 2, clip.frames * 2, &mut progress)?;
    receipt.publish(staging.path(), previous)?;
    Ok(receipt)
}

pub fn export_packed_atlas(directory: &Path, clip: &Clip, preset: &ExportPreset) -> io::Result<()> {
    export(directory, clip, preset, None, |_, _| true).map(|_| ())
}

/// A reusable, pixel-free layout. Page navigation renders only the requested page.
pub struct PackedPreviewPlan {
    clip: Clip,
    preset: ExportPreset,
    atlas: Atlas,
}
impl PackedPreviewPlan {
    pub fn prepare(
        clip: Clip,
        preset: ExportPreset,
        mut progress: impl FnMut(u32, u32) -> bool,
    ) -> io::Result<Self> {
        let atlas = Atlas::analyse(&clip, &preset, &mut progress)?;
        Ok(Self {
            clip,
            preset,
            atlas,
        })
    }
    pub fn render(
        &self,
        page: usize,
        mut progress: impl FnMut(u32, u32) -> bool,
    ) -> io::Result<(Pixmap, usize)> {
        let pages = self.atlas.packing.pages.len();
        let image = self.atlas.page(
            &[&self.clip],
            &self.preset,
            page.min(pages - 1),
            &mut 0,
            &mut progress,
        )?;
        Ok((image, pages))
    }
}

pub fn render_packed_preview(
    clip: &Clip,
    preset: &ExportPreset,
    page: usize,
    mut progress: impl FnMut(u32, u32) -> bool,
) -> io::Result<(Pixmap, usize)> {
    PackedPreviewPlan::prepare(clip.clone(), *preset, &mut progress)?.render(page, progress)
}
