//! Rebuild the skeletal swing and its sprite sheet from the static rig.
use project_io::{export_sprite_bundle, load_animation, save_animation};
use std::{io, path::PathBuf};

fn main() -> io::Result<()> {
    let dest = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::other("Supply a new directory for the project and sheet."))?;
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/lantern-rig.vanim");
    let mut project = load_animation(&source)?;
    let clip = &mut project.clips[0];
    clip.name = "Lantern swing".into();
    clip.set_length(8).map_err(io::Error::other)?;
    clip.set_fps(8).map_err(io::Error::other)?;
    clip.origin = [56., 114.];
    let drawing = clip.drawing_at_mut(0);
    // The hanging lamp passes its extreme one tick after the arm reverses.
    let tracks: [(&str, &[(u32, f32)]); 2] = [
        ("Sleeve", &[(0, -12.), (4, 12.), (7, -6.)]),
        ("Lantern", &[(0, 5.), (2, 16.), (5, -8.), (7, -3.)]),
    ];
    for (name, keys) in tracks {
        let id = drawing
            .skeleton
            .bones
            .iter()
            .find(|bone| bone.name == name)
            .ok_or_else(|| io::Error::other(format!("The fixture needs the {name} bone.")))?
            .id;
        for &(frame, angle) in keys {
            drawing
                .set_bone_rotation_key(id, frame, angle)
                .map_err(io::Error::other)?;
        }
    }
    std::fs::create_dir(&dest)?;
    let path = dest.join("lantern-keyed.vanim");
    save_animation(&path, &project, false)?;
    let reopened = load_animation(&path)?;
    export_sprite_bundle(
        &dest.join("lantern-swing"),
        &reopened.clips[0],
        &reopened.preset,
        true,
    )
}
