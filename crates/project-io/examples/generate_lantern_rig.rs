//! Reproduce the two-bone Lantern Keeper posing example.
use project_io::{load_animation, save_animation};
use std::{io, path::PathBuf};

fn main() -> io::Result<()> {
    let target = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "Supply a new .vanim output path.",
            )
        })?;
    let sample =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/lantern-keeper.vanim");
    let mut project = load_animation(&sample)?;
    project
        .rename_clip(0, "Lantern pose")
        .map_err(io::Error::other)?;
    let doc = project.clips[0].drawing_at_mut(0);
    let shoulder = doc
        .add_bone("Sleeve", None, [57., 75.], [89., 61.])
        .map_err(io::Error::other)?;
    let wrist = doc
        .add_bone("Lantern", Some(shoulder), [89., 61.], [96., 91.])
        .map_err(io::Error::other)?;
    // The source drawing's sleeve and lantern layers retain all their original vector objects.
    for (name, bone) in [("Sleeve", shoulder), ("Lantern", wrist)] {
        let layer = doc
            .layers
            .iter()
            .find(|l| l.name == name)
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, format!("Missing {name} layer."))
            })?
            .id;
        let pieces: Vec<_> = doc
            .objects
            .iter()
            .filter(|o| o.layer == layer)
            .map(|o| o.id)
            .collect();
        for piece in pieces {
            doc.bind_rigid(piece, Some(bone))
                .map_err(io::Error::other)?;
        }
    }
    save_animation(&target, &project, false)
}
