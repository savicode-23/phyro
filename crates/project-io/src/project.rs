use super::{invalid, write_file, WriteMode};
use document_core::{AnimationProject, Document, FORMAT_VERSION};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::File;
use std::io::{self, BufWriter, Cursor, Read, Seek, Write};
use std::path::Path;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};
mod clipping;
mod embedded;
mod images;

const MAX_DOCUMENT_BYTES: u64 = 16 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 4096;
const MAX_ANIMATION_BYTES: u64 = 64 * 1024 * 1024;
const ZIP_OVERHEAD_BYTES: u64 = 64 * 1024;
pub const MAX_ARCHIVE_BYTES: u64 = MAX_ANIMATION_BYTES + ZIP_OVERHEAD_BYTES;
const FIRST_ANIMATION_VERSION: u32 = 3;
const CLIP_ID_VERSION: u32 = 5;
const RIG_VERSION: u32 = 6;
const BONE_TRACK_VERSION: u32 = 7;
const SKIN_VERSION: u32 = 8;
const DRAWING_SWITCH_VERSION: u32 = 9;
const EMBEDDED_ARTWORK_VERSION: u32 = 10;
const PLACED_SOURCE_SIZE_VERSION: u32 = 11;
const PATH_EDITING_VERSION: u32 = 12;
const REFLECTION_VERSION: u32 = 13;
const PRESSURE_VERSION: u32 = 14;
const GROUP_NAME_VERSION: u32 = 15;
const OBJECT_PIVOT_VERSION: u32 = 16;
const RECTANGULAR_GRADIENT_VERSION: u32 = 17;
const STROKE_GRADIENT_VERSION: u32 = 18;
const FOOT_PIN_VERSION: u32 = 19;
const RASTER_IMAGE_VERSION: u32 = 20;
const NO_PAINT_VERSION: u32 = 21;
const GUIDED_MORPH_VERSION: u32 = 22;
const ANIMATION_SETS_AND_LAYER_TIMING_VERSION: u32 = 23;
const SET_EXPORT_INCLUSION_VERSION: u32 = 24;
const FRACTIONAL_RASTER_SCALE_VERSION: u32 = 25;
const OBJECT_LOCK_VERSION: u32 = 26;
const DOCUMENT_GRID_VERSION: u32 = 27;

fn required_object_version(version: u32, object: &document_core::VectorObject) -> u32 {
    let cutter = object.clip_source.as_ref().map(|b| &b.source.object);
    if version == OBJECT_LOCK_VERSION || object.locked || cutter.is_some_and(|o| o.locked) {
        return OBJECT_LOCK_VERSION;
    }
    if version >= GUIDED_MORPH_VERSION
        || object.animated.guided_geometry.is_some()
        || cutter.is_some_and(|o| o.animated.guided_geometry.is_some())
    {
        return version.max(GUIDED_MORPH_VERSION);
    }
    if version >= NO_PAINT_VERSION
        || object.no_fill
        || object.no_stroke
        || cutter.is_some_and(|o| o.no_fill || o.no_stroke)
    {
        return version.max(NO_PAINT_VERSION);
    }
    if object.image.is_some() {
        return RASTER_IMAGE_VERSION;
    }
    if version >= STROKE_GRADIENT_VERSION {
        return version;
    }
    let reflected = |o: &document_core::VectorObject| {
        std::iter::once(&o.scale)
            .chain(o.scale_keys.values())
            .any(|s| s[0] < 0. || s[1] < 0.)
    };
    let rectangular = |o: &document_core::VectorObject| {
        o.gradient
            .as_ref()
            .is_some_and(|g| matches!(g.kind, document_core::GradientKind::Rectangular { .. }))
    };
    let object_version = if object.stroke_gradient.is_some()
        || cutter.is_some_and(|o| o.stroke_gradient.is_some())
    {
        STROKE_GRADIENT_VERSION
    } else if rectangular(object) || cutter.is_some_and(rectangular) {
        RECTANGULAR_GRADIENT_VERSION
    } else if object.pivot.is_some() || cutter.is_some_and(|o| o.pivot.is_some()) {
        OBJECT_PIVOT_VERSION
    } else if object.group_name.is_some() {
        GROUP_NAME_VERSION
    } else if !object.stroke_profile.is_empty()
        || cutter.is_some_and(|o| !o.stroke_profile.is_empty())
    {
        PRESSURE_VERSION
    } else if reflected(object) || cutter.is_some_and(reflected) {
        REFLECTION_VERSION
    } else if object.clip_source.is_some() || object.has_styled_nodes() {
        PATH_EDITING_VERSION
    } else {
        FORMAT_VERSION
    };
    version.max(object_version)
}

fn animation_version(project: &AnimationProject) -> u32 {
    if project.grid != Default::default() {
        return DOCUMENT_GRID_VERSION;
    }
    let minimum = if std::iter::once(&project.preset)
        .chain(project.presets.values())
        .any(|preset| preset.scale_divisor != 1)
    {
        FRACTIONAL_RASTER_SCALE_VERSION
    } else if project.clips.iter().any(|clip| !clip.include_in_set_export) {
        SET_EXPORT_INCLUSION_VERSION
    } else if project.uses_animation_sets()
        || project
            .clips
            .iter()
            .any(|clip| !clip.layer_starts.is_empty())
    {
        ANIMATION_SETS_AND_LAYER_TIMING_VERSION
    } else {
        CLIP_ID_VERSION
    };
    let mut drawings: Vec<_> = project
        .clips
        .iter()
        .flat_map(|clip| clip.cels.values().map(AsRef::as_ref))
        .chain(
            project
                .symbols
                .values()
                .map(|symbol| symbol.artwork.as_ref()),
        )
        .collect();
    let mut seen = HashSet::new();
    let mut version = minimum;
    while let Some(doc) = drawings.pop() {
        if !seen.insert(std::ptr::from_ref(doc)) {
            continue;
        }
        if !doc.skeleton.foot_pins.is_empty() {
            version = version.max(FOOT_PIN_VERSION);
        }
        for object in &doc.objects {
            version = required_object_version(version, object);
            if version == OBJECT_LOCK_VERSION {
                return version;
            }
            if let Some(artwork) = &object.embedded_artwork {
                drawings.push(artwork.as_ref());
            }
        }
        version = version.max(
            if doc.objects.iter().any(|o| {
                o.clip_instance
                    .as_ref()
                    .is_some_and(|p| p.source_size.is_some())
            }) {
                PLACED_SOURCE_SIZE_VERSION
            } else if doc.objects.iter().any(|o| o.embedded_artwork.is_some()) {
                EMBEDDED_ARTWORK_VERSION
            } else if doc
                .objects
                .iter()
                .filter_map(|o| o.clip_instance.as_ref())
                .any(|place| !place.drawing_keys.is_empty())
            {
                DRAWING_SWITCH_VERSION
            } else if doc.objects.iter().any(|object| object.skin.is_some()) {
                SKIN_VERSION
            } else if doc.skeleton.bones.iter().any(|bone| {
                !bone.rotation_keys.is_empty()
                    || bone.interpolation != document_core::Interpolation::Linear
            }) {
                BONE_TRACK_VERSION
            } else if doc.uses_rig() {
                RIG_VERSION
            } else {
                CLIP_ID_VERSION
            },
        );
    }
    version
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    format: String,
    format_version: u32,
}

/// Save to a caller-approved destination. Existing contents survive any encoding failure.
pub fn save_project(path: &Path, document: &Document) -> io::Result<()> {
    write_project(path, document, WriteMode::Replace)
}

/// Save As to a new path. Fail even if a file appeared after the dialog closed.
pub fn save_new_project(path: &Path, document: &Document) -> io::Result<()> {
    write_project(path, document, WriteMode::Create)
}

fn write_project(path: &Path, document: &Document, mode: WriteMode) -> io::Result<()> {
    if document.uses_rig() {
        return Err(invalid(
            "Save the animation project to keep its skeletons and bindings.",
        ));
    }
    document.validate().map_err(invalid)?;
    if document
        .objects
        .iter()
        .any(document_core::VectorObject::is_instance)
    {
        return Err(invalid(
            "Save the animation project to include referenced symbols and clips.",
        ));
    }
    let version = document
        .objects
        .iter()
        .fold(FORMAT_VERSION, required_object_version);
    if version >= PATH_EDITING_VERSION {
        let mut stored = embedded::Stored::pack(AnimationProject::from_document(document.clone()))?;
        stored.drawing_only = true;
        return write_archive(path, &stored, version, MAX_DOCUMENT_BYTES, mode);
    }
    write_archive(path, document, FORMAT_VERSION, MAX_DOCUMENT_BYTES, mode)
}

/// Save every clip and its editable drawings. A new destination is reserved without clobbering.
pub fn save_animation(path: &Path, project: &AnimationProject, replace: bool) -> io::Result<()> {
    let snapshot = project.resolved_snapshot().map_err(invalid)?;
    snapshot.validate().map_err(invalid)?;
    let mode = if replace {
        WriteMode::Replace
    } else {
        WriteMode::Create
    };
    let version = animation_version(&snapshot);
    if version >= EMBEDDED_ARTWORK_VERSION {
        write_archive(
            path,
            &embedded::Stored::pack(snapshot)?,
            version,
            MAX_ANIMATION_BYTES,
            mode,
        )
    } else {
        write_archive(path, &snapshot, version, MAX_ANIMATION_BYTES, mode)
    }
}

/// Check a proposed bitmap insertion against exactly the resource and file size
/// limits that apply to a native save before it enters the editor's history.
pub fn validate_image_import(project: &AnimationProject) -> io::Result<()> {
    let snapshot = project.resolved_snapshot().map_err(invalid)?;
    snapshot.validate().map_err(invalid)?;
    let packed = embedded::Stored::pack(snapshot)?;
    let writer = LimitedWriter {
        writer: io::sink(),
        remaining: MAX_ANIMATION_BYTES,
    };
    serde_json::to_writer(writer, &packed)?;
    Ok(())
}

// Stream JSON to the staged archive. Checking the limit before each write also
// bounds temporary storage for projects with many copy-on-write drawings.
struct LimitedWriter<W> {
    writer: W,
    remaining: u64,
}

impl<W: Write> Write for LimitedWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() as u64 > self.remaining {
            return Err(invalid("Project data exceeds the native file size limit."));
        }
        let n = self.writer.write(bytes)?;
        self.remaining -= n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

fn write_archive(
    path: &Path,
    payload: &impl Serialize,
    version: u32,
    limit: u64,
    mode: WriteMode,
) -> io::Result<()> {
    let manifest = Manifest {
        // The .vanim wire identifier is shared with earlier editor releases.
        format: "Loopline".into(),
        format_version: version,
    };
    write_file(path, mode, |file| {
        let mut archive = ZipWriter::new(file);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        archive.start_file("manifest.json", options)?;
        serde_json::to_writer(&mut archive, &manifest)?;
        archive.start_file("document.json", options)?;
        {
            let mut writer = BufWriter::new(LimitedWriter {
                writer: &mut archive,
                remaining: limit,
            });
            serde_json::to_writer(&mut writer, payload)?;
            writer.flush()?;
        }
        archive.finish()?;
        Ok(())
    })
}

pub fn load_project(path: &Path) -> io::Result<Document> {
    let (version, data) = read_archive(path, DOCUMENT_GRID_VERSION)?;
    if version >= PATH_EDITING_VERSION {
        let stored: embedded::Stored =
            serde_json::from_slice(&data).map_err(|e| invalid(format!("Invalid drawing: {e}")))?;
        if stored.drawing_only {
            let project = stored.unpack()?;
            let required_version = animation_version(&project);
            if required_version > version {
                return Err(invalid(format!(
                    "This artwork requires native project format {required_version}."
                )));
            }
            project.validate().map_err(invalid)?;
            if let [clip] = project.clips.as_slice() {
                if let Some(doc) = clip.cels.get(&0) {
                    if !doc.uses_rig()
                        && !doc
                            .objects
                            .iter()
                            .any(document_core::VectorObject::is_instance)
                        && project == AnimationProject::from_document(doc.as_ref().clone())
                    {
                        return Ok(doc.as_ref().clone());
                    }
                }
            }
            return Err(invalid(
                "A standalone drawing cannot contain animation project data.",
            ));
        }
    }
    if version > FORMAT_VERSION {
        return Err(invalid(format!("Unsupported project format {version} for a standalone drawing. Open this file as an animation project.")));
    }
    read_document(version, &data)
}

/// Open animation projects or upgrade a v1/v2 scene to one editable clip in memory.
pub fn load_animation(path: &Path) -> io::Result<AnimationProject> {
    let (version, data) = read_archive(path, DOCUMENT_GRID_VERSION)?;
    decode_animation(version, &data)
}

/// Load the exact snapshot a headless caller has fingerprinted, including legacy files.
pub fn load_animation_bytes(bytes: &[u8]) -> io::Result<AnimationProject> {
    let (version, data) = read_archive_from(
        Cursor::new(bytes),
        bytes.len() as u64,
        DOCUMENT_GRID_VERSION,
    )?;
    decode_animation(version, &data)
}

fn decode_animation(version: u32, data: &[u8]) -> io::Result<AnimationProject> {
    let mut project = if version >= FIRST_ANIMATION_VERSION {
        let mut animation: AnimationProject = if version >= EMBEDDED_ARTWORK_VERSION {
            let packed: embedded::Stored = serde_json::from_slice(data)
                .map_err(|e| invalid(format!("Invalid animation project: {e}")))?;
            packed.unpack()?
        } else {
            serde_json::from_slice(data)
                .map_err(|e| invalid(format!("Invalid animation project: {e}")))?
        };
        let required_version = animation_version(&animation);
        if required_version >= RIG_VERSION && required_version > version {
            return Err(invalid(format!(
                "This artwork requires native project format {required_version}."
            )));
        }
        if version < CLIP_ID_VERSION {
            if animation.next_clip_id != 0 || animation.clips.iter().any(|c| c.id != 0) {
                return Err(invalid("Clip IDs require native project format 5."));
            }
            // Older projects have no clip references; archive order gives stable migration IDs.
            for (i, clip) in animation.clips.iter_mut().enumerate() {
                clip.id = i as u64 + 1;
            }
            animation.next_clip_id = animation.clips.len() as u64 + 1;
        }
        animation
    } else {
        AnimationProject::from_document(read_document(version, data)?)
    };
    project.hydrate_symbols().map_err(invalid)?;
    project.validate().map_err(invalid)?;
    Ok(project)
}

fn read_archive(path: &Path, max_version: u32) -> io::Result<(u32, Vec<u8>)> {
    let file = File::open(path)?;
    let size = file.metadata()?.len();
    read_archive_from(file, size, max_version)
}

fn read_archive_from(
    reader: impl Read + Seek,
    size: u64,
    max_version: u32,
) -> io::Result<(u32, Vec<u8>)> {
    let limit = if max_version >= FIRST_ANIMATION_VERSION {
        MAX_ANIMATION_BYTES
    } else {
        MAX_DOCUMENT_BYTES
    };
    if size > limit + ZIP_OVERHEAD_BYTES {
        return Err(invalid("This file exceeds the Phyro project size limit."));
    }
    let mut archive =
        ZipArchive::new(reader).map_err(|e| invalid(format!("Invalid project ZIP: {e}")))?;
    let header = read_entry(&mut archive, "manifest.json", MAX_MANIFEST_BYTES)?;
    let manifest: Manifest =
        serde_json::from_slice(&header).map_err(|e| invalid(format!("Invalid manifest: {e}")))?;
    if !matches!(manifest.format.as_str(), "Phyro" | "Loopline")
        || !(1..=max_version).contains(&manifest.format_version)
    {
        return Err(invalid(format!(
            "Unsupported project format '{}' version {} (expected Phyro versions 1–{max_version}).",
            manifest.format, manifest.format_version
        )));
    }
    // Do not open and later discard entries written by a more capable editor.
    if archive.len() != 2 {
        return Err(invalid(
            "Unexpected entries in this version of the project.",
        ));
    }
    let version = manifest.format_version;
    let limit = if version >= FIRST_ANIMATION_VERSION {
        MAX_ANIMATION_BYTES
    } else {
        MAX_DOCUMENT_BYTES
    };
    Ok((version, read_entry(&mut archive, "document.json", limit)?))
}

fn read_document(version: u32, data: &[u8]) -> io::Result<Document> {
    let mut document: Document =
        serde_json::from_slice(data).map_err(|e| invalid(format!("Invalid document: {e}")))?;
    let required_version = document
        .objects
        .iter()
        .fold(FORMAT_VERSION, required_object_version);
    if required_version >= REFLECTION_VERSION {
        return Err(invalid(format!(
            "This artwork requires native project format {required_version}."
        )));
    }
    if required_version == PATH_EDITING_VERSION {
        return Err(invalid(
            "Editable clipping paths and node handle types require native project format 12.",
        ));
    }
    if document
        .objects
        .iter()
        .any(|o| o.embedded_artwork.is_some())
    {
        return Err(invalid(
            "Independent artwork requires native project format 10.",
        ));
    }
    if document.uses_rig() {
        return Err(invalid(
            "Skeletons and bindings require native project format 6.",
        ));
    }
    if document.format_version != version {
        return Err(invalid(
            "The manifest and document format versions disagree.",
        ));
    }
    // The v1 rectangle, layer and paint defaults are defined on the scene types.
    // A project is upgraded in memory; opening it never rewrites the original file.
    if document.format_version == 1 {
        document.format_version = FORMAT_VERSION;
    }
    document.validate().map_err(invalid)?;
    Ok(document)
}

fn read_entry(
    archive: &mut ZipArchive<impl Read + Seek>,
    name: &str,
    limit: u64,
) -> io::Result<Vec<u8>> {
    let entry = archive
        .by_name(name)
        .map_err(|e| invalid(format!("Cannot read {name}: {e}")))?;
    if entry.size() > limit {
        return Err(invalid(format!("{name} exceeds its size limit.")));
    }
    let mut data = Vec::new();
    entry.take(limit + 1).read_to_end(&mut data)?;
    if data.len() as u64 > limit {
        return Err(invalid(format!("{name} exceeds its size limit.")));
    }
    Ok(data)
}
