//! Apply the vector-milestone edits to Godot's default project icon.
#[path = "edit_godot_icon/workflow.rs"]
mod workflow;
use std::{io, path::PathBuf};

fn main() -> io::Result<()> {
    let destination = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or_else(|| {
            io::Error::other("Supply a new directory for the edited native project, SVG and PNG.")
        })?;
    workflow::generate(&destination)
}
