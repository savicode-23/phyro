use super::{
    check_progress, export_combined, export_objects, publish_fresh_stills, require_new_directory,
    AssetFormat, ExportReceipt, SelectionAsset,
};
use crate::{export_clip_frame, export_svg, invalid, parent_of, write_file, WriteMode};
use document_core::{Clip, ExportPreset, VectorObject};
use renderer::Pixmap;
use std::{
    io::{self, Write},
    path::Path,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StillFormat {
    Png,
    Svg,
    PngAndSvg,
}

impl StillFormat {
    pub(super) fn primary(self, scale: f64) -> AssetFormat {
        match self {
            Self::Svg => AssetFormat::Svg,
            Self::Png | Self::PngAndSvg => AssetFormat::Png(scale),
        }
    }

    pub(super) fn encodings(self, scale: f64) -> impl Iterator<Item = AssetFormat> {
        std::iter::once(self.primary(scale))
            .chain((self == Self::PngAndSvg).then_some(AssetFormat::Svg))
    }

    pub(super) fn count(self) -> u32 {
        if self == Self::PngAndSvg {
            2
        } else {
            1
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StillTarget<'a> {
    Artboard,
    Selection(&'a [u64]),
    Objects(&'a [u64]),
}

#[derive(Debug, Clone, Copy)]
pub struct StillOptions<S = u32> {
    pub format: StillFormat,
    /// PNG pixels per document pixel. SVG uses native document units.
    pub scale: S,
    /// Allow replacing a single file. Batch and paired outputs require a fresh directory.
    pub replace: bool,
}

/// Export one evaluated clip frame. A combined selection includes selected groups as units.
/// For a batch or a pair of formats, the path names a fresh directory published after all
/// files are ready. Returning false from progress cancels before publishing any artwork.
pub fn export_still(
    path: &Path,
    clip: &Clip,
    frame: u32,
    target: StillTarget<'_>,
    options: StillOptions<impl Into<f64>>,
    mut progress: impl FnMut(u32, u32) -> bool,
) -> io::Result<Vec<SelectionAsset>> {
    if frame >= clip.frames || !clip.cels.contains_key(&0) {
        return Err(invalid(
            "Choose a frame inside a clip with a first drawing.",
        ));
    }
    let scale = options.scale.into();
    let selected = match target {
        StillTarget::Artboard => None,
        StillTarget::Selection(ids) => Some(ids),
        StillTarget::Objects(ids) => {
            return export_objects(
                path,
                clip.drawing_at(frame),
                frame,
                ids,
                options.format,
                scale,
                progress,
            );
        }
    };
    let total = options.format.count();
    check_progress(0, total, &mut progress)?;
    if options.format == StillFormat::PngAndSvg {
        require_new_directory(path)?;
        let staging = tempfile::tempdir_in(parent_of(path))?;
        let mut receipt = ExportReceipt::new(path);
        let stem = if selected.is_some() {
            "selection"
        } else {
            "artboard"
        };
        let mut assets = Vec::with_capacity(2);
        for format in options.format.encodings(scale) {
            let filename = format!("{stem}.{}", format.extension());
            let mut asset = write_single(
                &staging.path().join(&filename),
                clip,
                frame,
                selected,
                format,
                false,
            )?;
            receipt.register_staged(staging.path(), Path::new(&filename))?;
            asset.path = path.join(&filename);
            assets.push(asset);
            check_progress(assets.len() as u32, total, &mut progress)?;
        }
        publish_fresh_stills(staging, &receipt)?;
        Ok(assets)
    } else {
        let staging = tempfile::NamedTempFile::new_in(parent_of(path))?.into_temp_path();
        let mut asset = write_single(
            &staging,
            clip,
            frame,
            selected,
            options.format.primary(scale),
            true,
        )?;
        check_progress(1, 1, &mut progress)?;
        if options.replace {
            staging.persist(path)?;
        } else {
            staging.persist_noclobber(path)?;
        }
        asset.path = path.to_owned();
        Ok(vec![asset])
    }
}

fn write_single(
    path: &Path,
    clip: &Clip,
    frame: u32,
    selected: Option<&[u64]>,
    format: AssetFormat,
    replace: bool,
) -> io::Result<SelectionAsset> {
    let doc = clip.drawing_at(frame);
    if let Some(ids) = selected {
        if let AssetFormat::Png(scale) = format {
            return write_selection_png(path, doc, frame, ids, scale, replace);
        }
        return export_combined(path, doc, frame, ids, format, replace);
    }
    match format {
        AssetFormat::Png(scale) => export_clip_frame(path, clip, frame, scale, replace)?,
        AssetFormat::Svg => export_svg(path, doc, frame, replace)?,
    }
    Ok(SelectionAsset {
        path: path.to_owned(),
        origin: [0, 0],
        size: match format {
            AssetFormat::Png(scale) => {
                ExportPreset::scaled_size([doc.width, doc.height], scale).map_err(invalid)?
            }
            AssetFormat::Svg => [doc.width, doc.height],
        },
        object_ids: vec![],
    })
}

fn write_selection_png(
    path: &Path,
    doc: &document_core::Document,
    frame: u32,
    ids: &[u64],
    scale: f64,
    replace: bool,
) -> io::Result<SelectionAsset> {
    let members = super::selection(doc, frame, ids, scale)?;
    let art: Vec<_> = members
        .iter()
        .filter(|m| m.layer_visible)
        .map(|m| m.object)
        .collect();
    let crop = selection_crop(&art, &doc.layers, frame)?;
    let origin = crop.origin.map(|v| (f64::from(v) * scale).round());
    if origin
        .iter()
        .any(|v| !(f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(v))
    {
        return Err(invalid(
            "The selection extends beyond the supported output coordinates.",
        ));
    }
    let size = ExportPreset::scaled_size(crop.size, scale).map_err(invalid)?;
    let image = if size == crop.size {
        crop.image
            .ok_or_else(|| invalid("The selection exceeds the 16 megapixel PNG limit."))?
    } else {
        drop(crop.image);
        renderer::render_artwork_in_viewport(&art, &doc.layers, frame, crop.origin, crop.size, size)
            .map_err(invalid)?
    };
    let bytes = image.encode_png().map_err(io::Error::other)?;
    let mode = if replace {
        WriteMode::Replace
    } else {
        WriteMode::Create
    };
    write_file(path, mode, |out| out.write_all(&bytes))?;
    Ok(SelectionAsset {
        path: path.to_owned(),
        origin: origin.map(|v| v as i32),
        size,
        object_ids: members.iter().map(|m| m.object.id).collect(),
    })
}

struct Crop {
    origin: [i32; 2],
    size: [u32; 2],
    image: Option<Pixmap>,
}

fn selection_crop(
    art: &[&VectorObject],
    layers: &[document_core::Layer],
    frame: u32,
) -> io::Result<Crop> {
    let [l, t, r, b] = renderer::artwork_bounds(art, frame, 1)
        .map_err(invalid)?
        .ok_or_else(|| invalid("The selection has no visible paint."))?;
    let size = [r.abs_diff(l), b.abs_diff(t)];
    // A selection can span far beyond the page. Its vector bounds let us render a
    // scaled-down PNG even when the full-size source cannot fit in memory.
    if u64::from(size[0]) * u64::from(size[1]) > super::super::MAX_PIXELS {
        return Ok(Crop {
            origin: [l, t],
            size,
            image: None,
        });
    }
    let (image, origin) = renderer::render_artwork(art, layers, frame, 1).map_err(invalid)?;
    Ok(Crop {
        origin,
        size: [image.width(), image.height()],
        image: Some(image),
    })
}

pub struct PngDimensions {
    pub native: [u32; 2],
    pub assets: Vec<(String, [u32; 2])>,
}

/// Native visible crop used by the linked pixel fields, including for oversized selections.
pub fn selection_png_native_size(clip: &Clip, frame: u32, ids: &[u64]) -> io::Result<[u32; 2]> {
    if frame >= clip.frames || !clip.cels.contains_key(&0) {
        return Err(invalid("Choose a frame inside the clip."));
    }
    let doc = clip.drawing_at(frame);
    let members = super::selection(doc, frame, ids, 1.)?;
    let art = members
        .iter()
        .filter(|m| m.layer_visible)
        .map(|m| m.object)
        .collect::<Vec<_>>();
    Ok(selection_crop(&art, &doc.layers, frame)?.size)
}

/// Measure the same visible pixels used by the writer, including the crop of each top-level group.
/// The first size is the native reference for editing the scale. Batch files share the artboard scale.
pub fn png_dimensions(
    clip: &Clip,
    frame: u32,
    target: StillTarget<'_>,
    scale: f64,
    mut keep_going: impl FnMut() -> bool,
) -> io::Result<PngDimensions> {
    if frame >= clip.frames || !clip.cels.contains_key(&0) {
        return Err(invalid("Choose a frame inside the clip."));
    }
    let doc = clip.drawing_at(frame);
    let board = [doc.width, doc.height];
    let single = |label: &str, native: [u32; 2]| -> io::Result<PngDimensions> {
        let size = ExportPreset::scaled_size(native, scale).map_err(invalid)?;
        Ok(PngDimensions {
            native,
            assets: vec![(label.into(), size)],
        })
    };
    let ids = match target {
        StillTarget::Artboard => return single("Artboard", board),
        StillTarget::Selection(ids) => {
            return single("Selection", selection_png_native_size(clip, frame, ids)?)
        }
        StillTarget::Objects(ids) => ids,
    };
    let units = super::grouped(super::selection(doc, frame, ids, scale)?);
    let mut assets = Vec::with_capacity(units.len());
    for unit in units {
        if !keep_going() {
            return Err(io::ErrorKind::Interrupted.into());
        }
        let art = unit
            .iter()
            .filter(|m| m.layer_visible)
            .map(|m| m.object)
            .collect::<Vec<_>>();
        let object = unit[0].object;
        let name = super::Unit::of(object).label(object);
        let (image, _) =
            renderer::render_artwork(&art, &doc.layers, frame, scale).map_err(invalid)?;
        assets.push((name, [image.width(), image.height()]));
    }
    Ok(PngDimensions {
        native: board,
        assets,
    })
}
