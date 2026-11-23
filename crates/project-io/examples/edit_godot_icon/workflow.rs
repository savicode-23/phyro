use document_core::{AnimationProject, BooleanOp, Shape};
use project_io::{export_frame, export_svg, import_svg, load_animation, save_animation};
use std::io;
use std::path::Path;

pub fn generate(destination: &Path) -> io::Result<()> {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/godot-icon/source.svg");
    let mut doc = import_svg(&source)?;
    // Recolor the icon's rounded background and shift the nose's first anchor
    // with both controls by ten source units (1.01 artboard pixels).
    doc.objects[0].fill = [49, 86, 87, 255];
    let nose = &mut doc.objects[3];
    let dx = 1.01 / nose.size[0];
    let tip = &mut nose.shape.contours_mut()[0][0];
    for p in [&mut tip.point, &mut tip.incoming, &mut tip.outgoing] {
        p[0] += dx;
    }
    // Cut a catchlight from the right pupil; the existing white eye shows through.
    let pupil = doc.objects.last().unwrap();
    let (id, layer) = (pupil.id, pupil.layer);
    let r = 19. * 0.101;
    let center = [12.322 + 700. * 0.101, 12.322 + 514. * 0.101];
    let cutter = doc.add_shape(
        [center[0] - r, center[1] - r],
        [2. * r; 2],
        [255; 4],
        Shape::Ellipse,
        layer,
    );
    doc.boolean(&[id, cutter], 0, BooleanOp::Difference)
        .map_err(io::Error::other)?;
    std::fs::create_dir(destination)?;
    let native = destination.join("edited.vanim");
    save_animation(&native, &AnimationProject::from_document(doc), false)?;
    let reopened = load_animation(&native)?;
    let scene = reopened.clips[0].drawing_at(0);
    export_svg(&destination.join("edited.svg"), scene, 0, false)?;
    export_frame(&destination.join("edited.png"), scene, 0)
}
