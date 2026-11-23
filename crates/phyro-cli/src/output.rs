use crate::{inspect::clip_by_id, CliError};
use document_core::{AnimationProject, Clip, Document, ExportPreset};
use project_io::{
    scale_json, AnimationExportTarget, AnimationSetOutput, SelectionRange, SpriteOutput,
    StillFormat, StillOptions, StillTarget,
};
use serde_json::{json, Value};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[cfg(test)]
mod tests;

pub fn preview(
    project: &AnimationProject,
    clip_id: u64,
    frame: u32,
    path: &Path,
    scale: f64,
) -> Result<Value, CliError> {
    let clip = clip_by_id(project, clip_id)?;
    drawing_at(clip, frame)?;
    let mut result = clip_output_info(clip, scale)?;
    let path = new_destination(path)?;
    project_io::export_clip_frame(&path, clip, frame, scale, false)
        .map_err(|error| output_error(error, &path))?;
    let (start, duration) = clip.exposure(frame);
    result["frame"] = json!(frame);
    result["exposure_start"] = json!(start);
    result["exposure_duration"] = json!(duration);
    result["output"] = json!(path);
    result["files"] = json!([path]);
    Ok(result)
}

fn drawing_at(clip: &Clip, frame: u32) -> Result<&Document, CliError> {
    if frame >= clip.frames {
        return Err(CliError::new(
            "frame_out_of_range",
            "Choose a zero-based frame inside the clip.",
        )
        .with_details(json!({"clip_id": clip.id, "frame": frame, "frame_count": clip.frames})));
    }
    Ok(clip.drawing_at(frame))
}

pub fn export_current_frame(
    clip: &Clip,
    frame: u32,
    target: StillTarget<'_>,
    format: &str,
    path: &Path,
    scale: f64,
) -> Result<Value, CliError> {
    drawing_at(clip, frame)?;
    let (mode, ids) = match target {
        StillTarget::Artboard => ("artboard", &[][..]),
        StillTarget::Selection(ids) => ("combined", ids),
        StillTarget::Objects(ids) => ("individual", ids),
    };
    let kind = match format {
        "png" => StillFormat::Png,
        "both" => StillFormat::PngAndSvg,
        "svg" if scale == 1. => StillFormat::Svg,
        "svg" => {
            return Err(CliError::new(
                "invalid_argument",
                "SVG uses native artwork coordinates. Use --scale 1 or omit --scale.",
            )
            .with_details(json!({"format":format,"scale":scale})))
        }
        _ => {
            return Err(
                CliError::new("invalid_format", "Choose png, svg or both for --format.")
                    .with_details(json!({"format":format})),
            )
        }
    };
    let path = new_destination(path)?;
    let assets = project_io::export_still(
        &path,
        clip,
        frame,
        target,
        StillOptions {
            format: kind,
            scale,
            replace: false,
        },
        |_, _| true,
    )
    .map_err(|error| {
        output_error(error, &path).with_details(json!({
            "output": path, "clip_id": clip.id, "frame": frame, "object_ids": ids
        }))
    })?;
    let files: Vec<_> = assets.iter().map(|asset| &asset.path).collect();
    let descriptions: Vec<_> = assets.iter().map(|asset| json!({
        "path": asset.path, "object_ids": asset.object_ids, "origin": asset.origin, "size": asset.size
    })).collect();
    let (start, duration) = clip.exposure(frame);
    Ok(json!({
        "output":path,"files":files,"assets":descriptions,"mode":mode,"format":format,"scale":scale_json(scale),
        "clip_id":clip.id,"frame":frame,"exposure_start":start,"exposure_duration":duration
    }))
}

pub fn export(
    project: &AnimationProject,
    clip_id: u64,
    path: &Path,
    format: &str,
    columns: Option<u32>,
) -> Result<Value, CliError> {
    let clip = clip_by_id(project, clip_id)?;
    let settings = sprite_settings(project.preset, format, columns)?;
    let mut result = clip_output_info(clip, settings.preset.raster_scale())?;
    let path = new_destination(path)?;
    let receipt = project_io::export_sprite_output(
        &path,
        clip,
        &settings.preset,
        settings.mode,
        None,
        |_, _| true,
    )
    .map_err(|error| output_error(error, &path))?;
    let files: Vec<_> = receipt.files().map(|file| path.join(file)).collect();
    result["format"] = json!(format);
    result["output"] = json!(path);
    result["files"] = json!(files);
    result["metadata"] =
        json!((settings.mode != SpriteOutput::Frames).then(|| path.join("sheet.json")));
    Ok(result)
}

pub(super) struct SpriteSettings<'a> {
    format: &'a str,
    mode: SpriteOutput,
    preset: ExportPreset,
}

pub(super) fn export_project(
    project: &AnimationProject,
    target: AnimationExportTarget,
    path: &Path,
    format: Option<&str>,
    columns: Option<u32>,
    sheet_layout: Option<&str>,
) -> Result<Value, CliError> {
    use AnimationExportTarget::{Clip, Set};
    let format = format.unwrap_or(match target {
        Clip(_) if sheet_layout.is_some() => "separate-sheets",
        Clip(_) => "sheet",
        Set(_) => "shared-sheet",
    });
    let output = match (target, format) {
        (_, "png-sequence") | (Set(_), "frames") => AnimationSetOutput::PngSequences,
        (_, "separate-sheets") => AnimationSetOutput::SeparateSheets,
        (_, "shared-sheet") | (Set(_), "packed") => AnimationSetOutput::SharedSheet,
        (Clip(id), "sheet" | "frames" | "sheet-and-frames" | "packed") => {
            if sheet_layout.is_some() {
                return Err(CliError::new(
                    "invalid_argument",
                    "Use --layout with separate-sheets or shared-sheet.",
                ));
            }
            return export(project, id, path, format, columns);
        }
        _ => {
            let choices = match target {
                Set(_) => "For a set, choose png-sequence, separate-sheets or shared-sheet.",
                Clip(_) => "For a clip, choose png-sequence, separate-sheets or shared-sheet. Grid and legacy layouts also accept sheet, frames, sheet-and-frames or packed.",
            };
            return Err(
                CliError::new("invalid_format", choices).with_details(json!({"format":format}))
            );
        }
    };
    let mut preset = project.preset;
    if format == "packed" {
        preset.packed.enabled = true;
    }
    if let Some(layout) = sheet_layout {
        if output == AnimationSetOutput::PngSequences || format == "packed" {
            return Err(CliError::new(
                "invalid_argument",
                "Use --layout with separate-sheets or shared-sheet.",
            ));
        }
        preset.packed.enabled = match layout {
            "packed" => true,
            "grid" => false,
            _ => {
                return Err(CliError::new(
                    "invalid_argument",
                    "Choose grid or packed for --layout.",
                ))
            }
        };
    }
    if let Some(columns) = columns {
        if output == AnimationSetOutput::PngSequences || preset.packed.enabled {
            return Err(CliError::new(
                "invalid_argument",
                "--columns applies to uniform grid sheets.",
            )
            .with_details(json!({"format":format})));
        }
        preset.columns = columns;
    }
    preset
        .validate()
        .map_err(|e| CliError::new("invalid_argument", e))?;
    let path = new_destination(path)?;
    let (set, ids, mut result) = match target {
        Set(set_id) => {
            let set = project.animation_set(set_id).ok_or_else(|| {
                CliError::new("unknown_set", "Use an animation set ID from inspect.")
                    .with_details(json!({"set_id": set_id}))
            })?;
            let ids: Vec<_> = project
                .clips
                .iter()
                .filter(|c| c.set_id == set_id && c.include_in_set_export)
                .map(|c| c.id)
                .collect();
            (set, ids, json!({}))
        }
        Clip(id) => {
            let clip = clip_by_id(project, id)?;
            let set = project
                .animation_set(clip.set_id)
                .expect("validated project");
            (
                set,
                vec![id],
                clip_output_info(clip, preset.raster_scale())?,
            )
        }
    };
    let receipt = project_io::export_animation_files(
        &path,
        project,
        target,
        &preset,
        output,
        None,
        |_, _| true,
    )
    .map_err(|error| output_error(error, &path))?;
    result["files"] = json!(receipt
        .files()
        .map(|file| path.join(file))
        .collect::<Vec<_>>());
    result["metadata"] = json!(path.join("sheet.json"));
    result["output"] = json!(path);
    result["set_id"] = json!(set.id);
    result["set_name"] = json!(set.name);
    result["clip_ids"] = json!(ids);
    result["format"] = json!(format);
    result["scale"] = scale_json(preset.raster_scale());
    if output != AnimationSetOutput::PngSequences {
        result["layout"] = json!(if preset.packed.enabled {
            "packed"
        } else {
            "grid"
        });
    }
    Ok(result)
}

pub(super) fn sprite_settings(
    mut preset: ExportPreset,
    format: &str,
    columns: Option<u32>,
) -> Result<SpriteSettings<'_>, CliError> {
    let mode = match format {
        "sheet" => SpriteOutput::Sheet,
        "sheet-and-frames" => SpriteOutput::SheetAndFrames,
        "frames" => SpriteOutput::Frames,
        "packed" => SpriteOutput::Packed,
        _ => {
            return Err(CliError::new(
                "invalid_format",
                "Choose sheet, frames, sheet-and-frames or packed.",
            )
            .with_details(json!({"format": format})))
        }
    };
    if let Some(columns) = columns {
        if !matches!(mode, SpriteOutput::Sheet | SpriteOutput::SheetAndFrames) {
            return Err(CliError::new(
                "invalid_argument",
                "--columns applies to sheet and sheet-and-frames.",
            )
            .with_details(json!({"format": format, "columns": columns})));
        }
        preset.columns = columns;
    }
    preset.packed.enabled = mode == SpriteOutput::Packed;
    preset
        .validate()
        .map_err(|e| CliError::new("invalid_argument", e))?;
    Ok(SpriteSettings {
        format,
        mode,
        preset,
    })
}

pub(super) fn selection_animation(
    clip: &Clip,
    ids: &[u64],
    range: SelectionRange,
    path: &Path,
    settings: SpriteSettings<'_>,
) -> Result<Value, CliError> {
    if settings.mode == SpriteOutput::Packed {
        return Err(CliError::new(
            "invalid_format",
            "For an animated selection choose sheet, frames or sheet-and-frames.",
        ));
    }
    if range.reference >= clip.frames || range.first > range.last || range.last >= clip.frames {
        return Err(CliError::new("frame_out_of_range", "Choose a reference frame and an inclusive range inside the clip, with --from at or before --to.")
            .with_details(json!({"clip_id":clip.id, "frame":range.reference, "from":range.first, "to":range.last, "frame_count":clip.frames})));
    }
    let first = range.first;
    let last = range.last;
    let reference = range.reference;
    let path = new_destination(path)?;
    let exported = project_io::export_selection_animation(
        &path,
        clip,
        ids,
        range,
        &settings.preset,
        settings.mode,
        |_, _| true,
    )
    .map_err(|error| {
        output_error(error, &path).with_details(json!({
            "output": path, "clip_id": clip.id, "frame": reference,
            "from": first, "to": last, "object_ids": ids
        }))
    })?;
    let files: Vec<_> = exported
        .receipt
        .files()
        .map(|file| path.join(file))
        .collect();
    Ok(json!({
        "output": path, "files": files, "metadata": path.join("sheet.json"),
        "format": settings.format, "scale": scale_json(settings.preset.raster_scale()),
        "clip_id": clip.id, "clip_name": clip.name, "reference_frame": reference,
        "object_ids": ids, "source_start": first, "source_end": last, "frame_count": last-first+1,
        "origin": exported.origin, "frame_size": exported.size, "pivot": exported.pivot,
        "fps": clip.fps, "looping": clip.looping
    }))
}

fn clip_output_info(clip: &Clip, scale: f64) -> Result<Value, CliError> {
    let drawing = clip.drawing_at(0);
    let size = ExportPreset::scaled_size([drawing.width, drawing.height], scale)
        .map_err(|e| CliError::new("invalid_argument", e))?;
    let pivot = project_io::scaled_pivot(clip, size[0], size[1])
        .map_err(|e| CliError::new("invalid_argument", e.to_string()))?;
    Ok(json!({
        "clip_id": clip.id,
        "clip_name": clip.name,
        "frame_count": clip.frames,
        "fps": clip.fps,
        "looping": clip.looping,
        "scale": scale_json(scale),
        "frame_size": size,
        "pivot": pivot
    }))
}

pub(super) fn new_destination(path: &Path) -> Result<PathBuf, CliError> {
    let path = std::path::absolute(path).map_err(|e| output_error(e, path))?;
    // JSON output paths must round-trip for a caller using them in its next command.
    if path.to_str().is_none() {
        return Err(CliError::new(
            "invalid_argument",
            "Choose an output path representable in UTF-8.",
        ));
    }
    match fs::symlink_metadata(&path) {
        Ok(_) => return Err(output_error(io::ErrorKind::AlreadyExists.into(), &path)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(output_error(e, &path)),
    }
    if let Some(parent) = path.parent() {
        if !fs::metadata(parent)
            .map_err(|e| output_error(e, &path))?
            .is_dir()
        {
            return Err(CliError::new(
                "output_io",
                "Choose an output inside an existing directory.",
            )
            .with_details(json!({"output": path})));
        }
    }
    Ok(path)
}

fn output_error(error: io::Error, path: &Path) -> CliError {
    let (code, message) = match error.kind() {
        io::ErrorKind::AlreadyExists => (
            "output_exists",
            "Choose a new output path. This destination already exists.".to_owned(),
        ),
        io::ErrorKind::InvalidData => ("invalid_output", error.to_string()),
        io::ErrorKind::NotFound => (
            "output_io",
            format!("Choose an existing parent directory: {error}"),
        ),
        _ => ("output_io", error.to_string()),
    };
    CliError::new(code, message).with_details(json!({"output": path.to_string_lossy()}))
}
