//! Native project persistence and transparent PNG export, usable without a window.

mod export;
mod project;
mod reference;
mod sprites;
mod svg;

pub use export::{export_frame, export_new_frame, export_sequence, export_sequence_with_progress};
pub use project::{
    load_animation, load_animation_bytes, load_project, save_animation, save_new_project,
    save_project, validate_image_import, MAX_ARCHIVE_BYTES,
};
pub use reference::import_reference_image;
pub use sprites::{
    animation_clip_example_filename, animation_grid_layout, animation_set_example_filename,
    animation_set_file_stem, export_animation_files, export_animation_set,
    export_animation_set_with_format, export_clip_frame, export_clip_with_format,
    export_object_pngs, export_object_svgs, export_packed_atlas, export_selection_animation,
    export_selection_png, export_sprite_bundle, export_sprite_output, render_packed_preview,
    render_sheet_preview, selection_animation_dimensions, sheet_layout, AnimationExportTarget,
    AnimationSetOutput, AnimationSheetPreviewPlan, ExportReceipt, PackedPreviewPlan,
    SelectionAnimationExport, SelectionAsset, SelectionRange, SheetLayout, SpriteOutput,
};
pub use sprites::{
    export_selection_svg, export_still, png_dimensions, selection_png_native_size, PngDimensions,
    StillFormat, StillOptions, StillTarget,
};
pub use sprites::{scale_json, scaled_pivot};
pub use svg::{export_svg, import_svg};

use std::fs::File;
use std::io;
use std::path::Path;
use tempfile::NamedTempFile;

fn parent_of(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

enum WriteMode {
    Create,
    Replace,
}

/// Prepare the complete file on the same filesystem before publishing it.
fn write_file(
    path: &Path,
    mode: WriteMode,
    write: impl FnOnce(&mut File) -> io::Result<()>,
) -> io::Result<()> {
    let mut temporary = NamedTempFile::new_in(parent_of(path))?;
    write(temporary.as_file_mut())?;
    temporary.as_file().sync_all()?;
    match mode {
        WriteMode::Create => temporary.persist_noclobber(path)?,
        WriteMode::Replace => temporary.persist(path)?,
    };
    Ok(())
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
