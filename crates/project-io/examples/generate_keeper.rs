use std::path::Path;

// Regenerate the editable sample and its preview after editing the source SVG.
fn main() -> std::io::Result<()> {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let artwork = project_io::import_svg(&folder.join("lantern-keeper.svg"))?;
    let native = folder.join("lantern-keeper.vanim");
    project_io::save_project(&native, &artwork)?;
    project_io::export_frame(
        &folder.join("lantern-keeper.png"),
        &project_io::load_project(&native)?,
        0,
    )
}
