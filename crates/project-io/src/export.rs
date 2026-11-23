use super::{invalid, parent_of, write_file, WriteMode};
use document_core::Document;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use tempfile::NamedTempFile;

/// Export the frame at a zero-based index to a caller-approved file.
pub fn export_frame(path: &Path, document: &Document, frame: u32) -> io::Result<()> {
    export_to_file(path, document, frame, WriteMode::Replace)
}

/// Export to a new file without replacing any intervening file at this path.
pub fn export_new_frame(path: &Path, document: &Document, frame: u32) -> io::Result<()> {
    export_to_file(path, document, frame, WriteMode::Create)
}

fn export_to_file(path: &Path, document: &Document, frame: u32, mode: WriteMode) -> io::Result<()> {
    document.validate().map_err(invalid)?;
    write_file(path, mode, |file| write_png(file, document, frame))
}

fn write_png(file: &mut fs::File, document: &Document, frame: u32) -> io::Result<()> {
    if frame >= document.frames {
        return Err(invalid(format!("Frame index {frame} is outside the clip.")));
    }
    let pixmap = renderer::render_for_export(document, frame, 1.).map_err(invalid)?;
    let png = pixmap.encode_png().map_err(io::Error::other)?;
    file.write_all(&png)
}

/// Export every frame into a newly created directory. An existing directory is never reused.
pub fn export_sequence(directory: &Path, document: &Document) -> io::Result<()> {
    export_sequence_with_progress(directory, document, |_, _| true)
}

/// Report progress before rendering and after each frame. Returning false cancels publication.
pub fn export_sequence_with_progress(
    directory: &Path,
    document: &Document,
    mut progress: impl FnMut(u32, u32) -> bool,
) -> io::Result<()> {
    document.validate().map_err(invalid)?;
    match fs::symlink_metadata(directory) {
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "Choose a new folder name for the frame sequence.",
            ))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }

    let cancelled = || io::Error::new(io::ErrorKind::Interrupted, "Sequence export cancelled.");
    if !progress(0, document.frames) {
        return Err(cancelled());
    }
    let parent = parent_of(directory);
    let staging = tempfile::tempdir_in(parent)?;
    let mut files = Vec::new();
    let width = document.frames.saturating_sub(1).to_string().len().max(4);
    for frame in 0..document.frames {
        let mut file = NamedTempFile::new_in(staging.path())?;
        write_png(file.as_file_mut(), document, frame)?;
        file.as_file().sync_all()?;
        let destination = directory.join(format!("frame_{frame:0width$}.png"));
        files.push((file.into_temp_path(), destination));
        if !progress(frame + 1, document.frames) {
            return Err(cancelled());
        }
    }

    // Reserve the destination only after rendering. Two exporters racing for the
    // same name cannot interleave frames, and a cancelled export has no visible output.
    fs::create_dir(directory)?;
    let mut published = Vec::new();
    for (file, destination) in files {
        if let Err(e) = file.persist_noclobber(&destination) {
            for owned_file in published {
                let _ = fs::remove_file(owned_file);
            }
            let _ = fs::remove_dir(directory);
            return Err(e.error);
        }
        published.push(destination);
    }
    Ok(())
}
