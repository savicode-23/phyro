use super::{check_progress, ExportReceipt};
use crate::{invalid, parent_of, write_file, WriteMode};
use document_core::{Document, VectorObject};
use std::collections::{HashMap, HashSet};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
pub(super) mod animation;
mod still;
pub use still::{
    export_still, png_dimensions, selection_png_native_size, PngDimensions, StillFormat,
    StillOptions, StillTarget,
};

/// An exported selection and its placement relative to the artboard, in output pixels.
#[derive(Debug)]
pub struct SelectionAsset {
    pub path: PathBuf,
    pub object_ids: Vec<u64>,
    pub origin: [i32; 2],
    pub size: [u32; 2],
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Unit {
    Object(u64),
    Group { layer: u64, id: u64 },
}

impl Unit {
    fn of(object: &VectorObject) -> Self {
        match object.group {
            Some(id) => Self::Group {
                layer: object.layer,
                id,
            },
            None => Self::Object(object.id),
        }
    }

    fn label(self, object: &VectorObject) -> String {
        match self {
            Self::Group { id, .. } => object
                .group_name
                .as_deref()
                .map_or_else(|| format!("Group {id}"), str::to_owned),
            Self::Object(_) => object.display_name(),
        }
    }
}

#[derive(Clone, Copy)]
struct Member<'a> {
    object: &'a VectorObject,
    layer_visible: bool,
}

fn selection<'a>(
    doc: &'a Document,
    frame: u32,
    ids: &[u64],
    scale: f64,
) -> io::Result<Vec<Member<'a>>> {
    if frame >= doc.frames || !(scale > 0. && scale <= 32.) {
        return Err(invalid(
            "Choose a frame inside the drawing and a positive export scale up to 32×.",
        ));
    }
    doc.validate().map_err(invalid)?;
    selected_members(doc, ids)
}

fn selected_members<'a>(doc: &'a Document, ids: &[u64]) -> io::Result<Vec<Member<'a>>> {
    if ids.is_empty() {
        return Err(invalid("Select at least one object to export."));
    }
    let mut missing: HashSet<_> = ids.iter().copied().collect();
    let mut units = HashSet::new();
    for object in &doc.objects {
        if missing.remove(&object.id) {
            units.insert(Unit::of(object));
        }
    }
    if let Some(id) = ids.iter().find(|id| missing.contains(id)) {
        return Err(invalid(format!(
            "Object {id} is missing from this drawing."
        )));
    }
    // Keep native paint order even when unrelated group members and individual
    // objects are interleaved. Record hidden layers so a batch cannot silently
    // omit an explicitly requested unit.
    let layers: HashMap<_, _> = doc
        .layers
        .iter()
        .enumerate()
        .map(|(index, l)| (l.id, index))
        .collect();
    let mut contents = vec![Vec::new(); doc.layers.len()];
    for object in &doc.objects {
        if !object.reference_only && units.contains(&Unit::of(object)) {
            let index = layers[&object.layer];
            contents[index].push(Member {
                object,
                layer_visible: doc.layers[index].visible,
            });
        }
    }
    let members: Vec<_> = contents.into_iter().flatten().collect();
    if members.is_empty() {
        return Err(invalid("The selection contains only reference images. Turn on Include in export in Properties to export them."));
    }
    Ok(members)
}

#[derive(Clone, Copy)]
enum AssetFormat {
    Png(f64),
    Svg,
}

impl AssetFormat {
    fn extension(self) -> &'static str {
        match self {
            Self::Png(_) => "png",
            Self::Svg => "svg",
        }
    }

    fn scale(self) -> f64 {
        match self {
            Self::Png(scale) => scale,
            Self::Svg => 1.,
        }
    }
}

fn encode_asset(
    path: &Path,
    doc: &Document,
    members: &[Member<'_>],
    frame: u32,
    format: AssetFormat,
) -> io::Result<(SelectionAsset, Vec<u8>)> {
    let art: Vec<_> = members
        .iter()
        .filter(|m| m.layer_visible)
        .map(|m| m.object)
        .collect();
    let (bytes, origin, size) = match format {
        AssetFormat::Png(scale) => {
            let (image, origin) = renderer::render_artwork(&art, frame, scale).map_err(invalid)?;
            let size = [image.width(), image.height()];
            (image.encode_png().map_err(io::Error::other)?, origin, size)
        }
        AssetFormat::Svg => {
            let [left, top, right, bottom] = renderer::artwork_bounds(&art, frame, 1)
                .map_err(invalid)?
                .ok_or_else(|| {
                    invalid("The selected artwork has no visible paint at this frame.")
                })?;
            let origin = [left, top];
            let size = [right.abs_diff(left), bottom.abs_diff(top)];
            (
                crate::svg::encode_objects(&art, &doc.layers, frame, origin, size)?,
                origin,
                size,
            )
        }
    };
    let asset = SelectionAsset {
        path: path.to_owned(),
        object_ids: members.iter().map(|m| m.object.id).collect(),
        origin,
        size,
    };
    Ok((asset, bytes))
}

/// Render the current selection together, including artwork outside the page.
pub fn export_selection_png(
    path: &Path,
    doc: &Document,
    frame: u32,
    selected: &[u64],
    scale: u32,
    replace: bool,
) -> io::Result<SelectionAsset> {
    export_combined(
        path,
        doc,
        frame,
        selected,
        AssetFormat::Png(scale.into()),
        replace,
    )
}

/// Export one editable SVG cropped to the selected objects and their group siblings.
pub fn export_selection_svg(
    path: &Path,
    doc: &Document,
    frame: u32,
    selected: &[u64],
    replace: bool,
) -> io::Result<SelectionAsset> {
    export_combined(path, doc, frame, selected, AssetFormat::Svg, replace)
}

fn export_combined(
    path: &Path,
    doc: &Document,
    frame: u32,
    selected: &[u64],
    format: AssetFormat,
    replace: bool,
) -> io::Result<SelectionAsset> {
    let members = selection(doc, frame, selected, format.scale())?;
    let (asset, bytes) = encode_asset(path, doc, &members, frame, format)?;
    let mode = if replace {
        WriteMode::Replace
    } else {
        WriteMode::Create
    };
    write_file(path, mode, |out| out.write_all(&bytes))?;
    Ok(asset)
}

/// Write one PNG for each selected object or layer-local group to a fresh directory.
/// Cancellation and encoding failures leave the destination untouched.
pub fn export_object_pngs(
    directory: &Path,
    doc: &Document,
    frame: u32,
    selected: &[u64],
    scale: u32,
    progress: impl FnMut(u32, u32) -> bool,
) -> io::Result<Vec<SelectionAsset>> {
    export_objects(
        directory,
        doc,
        frame,
        selected,
        StillFormat::Png,
        scale.into(),
        progress,
    )
}

/// Export a separate, editable SVG for every selected object or layer-local group.
/// Its viewport encloses the evaluated artwork, including signed pasteboard coordinates.
pub fn export_object_svgs(
    directory: &Path,
    doc: &Document,
    frame: u32,
    selected: &[u64],
    progress: impl FnMut(u32, u32) -> bool,
) -> io::Result<Vec<SelectionAsset>> {
    export_objects(
        directory,
        doc,
        frame,
        selected,
        StillFormat::Svg,
        1.,
        progress,
    )
}

fn grouped(members: Vec<Member<'_>>) -> Vec<Vec<Member<'_>>> {
    let mut indices = HashMap::new();
    let mut units: Vec<Vec<Member<'_>>> = Vec::new();
    for member in members {
        let index = *indices.entry(Unit::of(member.object)).or_insert_with(|| {
            units.push(Vec::new());
            units.len() - 1
        });
        units[index].push(member);
    }
    units
}

fn export_objects(
    directory: &Path,
    doc: &Document,
    frame: u32,
    selected: &[u64],
    format: StillFormat,
    scale: f64,
    mut progress: impl FnMut(u32, u32) -> bool,
) -> io::Result<Vec<SelectionAsset>> {
    require_new_directory(directory)?;
    let primary = format.primary(scale);
    let members = selection(doc, frame, selected, primary.scale())?;
    let units = grouped(members);
    let total = units.len() as u32 * format.count();
    check_progress(0, total, &mut progress)?;
    let staging = tempfile::tempdir_in(parent_of(directory))?;
    let mut receipt = ExportReceipt::new(directory);
    let mut assets = Vec::with_capacity(total as usize);
    let mut names = AssetNames::default();
    for unit in units {
        let object = unit[0].object;
        let label = Unit::of(object).label(object);
        let file = names.filename(&label, primary.extension(), staging.path())?;
        for encoding in format.encodings(scale) {
            let relative = Path::new(&file).with_extension(encoding.extension());
            let (asset, bytes) =
                encode_asset(&directory.join(&relative), doc, &unit, frame, encoding).map_err(
                    |e| io::Error::new(e.kind(), format!("Could not export '{label}': {e}")),
                )?;
            receipt.stage(staging.path(), &relative, &bytes)?;
            assets.push(asset);
            check_progress(assets.len() as u32, total, &mut progress)?;
        }
    }
    publish_fresh_stills(staging, &receipt)?;
    Ok(assets)
}

fn publish_fresh_stills(staging: tempfile::TempDir, receipt: &ExportReceipt) -> io::Result<()> {
    // On our desktop platforms tempfile uses a no-replace rename when the
    // filesystem supports it. TempDir retains sole responsibility for cleanup.
    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    {
        let mut staged_path = tempfile::TempPath::from_path(staging.path());
        staged_path.disable_cleanup(true);
        match staged_path.persist_noclobber(receipt.directory()) {
            Ok(()) => {
                let _ = staging.keep();
                return Ok(());
            }
            Err(e) if e.error.kind() == io::ErrorKind::AlreadyExists => return Err(e.error),
            Err(_) => {}
        }
    }
    // Filesystems without an exclusive directory rename still get the normal
    // fresh-directory, rollback-on-failure publication used by animation exports.
    receipt.publish(staging.path(), None)
}

fn require_new_directory(directory: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(directory) {
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Choose a fresh export directory.",
        )),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[derive(Default)]
struct AssetNames {
    used: HashSet<String>,
    next_suffix: HashMap<String, usize>,
}

impl AssetNames {
    fn filename(&mut self, label: &str, extension: &str, staging: &Path) -> io::Result<String> {
        let mut stem = String::new();
        // Leave room for a suffix and extension on filesystems with a 255-byte limit.
        for c in label.trim().chars() {
            if stem.len() + c.len_utf8() > 210 {
                break;
            }
            stem.push(if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            });
        }
        let mut stem = stem.trim_matches([' ', '.']).to_string();
        if stem.is_empty() {
            stem.push_str("Asset");
        }
        if windows_device_name(&stem) {
            stem.insert(0, '_');
        }
        let suffix = self.next_suffix.entry(stem.to_uppercase()).or_insert(1);
        loop {
            let file = if *suffix == 1 {
                format!("{stem}.{extension}")
            } else {
                format!("{stem} ({suffix}).{extension}")
            };
            *suffix += 1;
            if self.used.insert(file.to_uppercase()) && !staging.join(&file).try_exists()? {
                return Ok(file);
            }
        }
    }
}

pub(super) fn windows_device_name(stem: &str) -> bool {
    let device = stem
        .split('.')
        .next()
        .unwrap()
        .trim_end()
        .to_ascii_uppercase();
    let serial = device
        .strip_prefix("COM")
        .or_else(|| device.strip_prefix("LPT"));
    matches!(
        device.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || serial.is_some_and(|s| {
        matches!(
            s,
            "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
        )
    })
}
