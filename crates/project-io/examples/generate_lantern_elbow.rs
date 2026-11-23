//! Build the weighted-sleeve example from the original Lantern Keeper artwork.
use document_core::Document;
use project_io::{load_animation, save_animation};
use std::{io, path::PathBuf};

fn rig_sleeve(doc: &mut Document) -> Result<(), String> {
    let shoulder = doc.add_bone("Upper arm", None, [56., 70.], [70., 75.])?;
    let elbow = doc.add_bone("Forearm", Some(shoulder), [70., 75.], [89., 61.])?;
    let wrist = doc.add_bone("Lantern", Some(elbow), [89., 61.], [96., 91.])?;
    let layer_id = |name| {
        doc.layers
            .iter()
            .find(|layer| layer.name == name)
            .map(|layer| layer.id)
            .ok_or_else(|| format!("Missing {name} layer."))
    };
    let sleeve_layer = layer_id("Sleeve")?;
    let lamp_layer = layer_id("Lantern")?;
    let sleeve = doc
        .objects
        .iter_mut()
        .find(|o| o.layer == sleeve_layer && o.fill == [87, 145, 138, 255])
        .ok_or("Missing the green sleeve path.")?;
    sleeve.name = "Sleeve fabric".into();
    sleeve.group = None;
    sleeve.group_name = None;
    let sleeve = sleeve.id;
    doc.bind_skin_at(sleeve, elbow, 0)?;
    // Follow the drawn contour from shoulder to cuff, then back along the hem.
    for (index, child_weight) in [0., 0.45, 1., 1., 0.65, 0.15, 0.].into_iter().enumerate() {
        doc.set_skin_weight(sleeve, 0, index, child_weight)?;
    }
    let rigid: Vec<_> = doc
        .objects
        .iter()
        .filter_map(|o| {
            if o.layer == lamp_layer {
                Some((o.id, wrist))
            } else if o.layer == sleeve_layer && o.id != sleeve {
                // The shoulder clasp follows the upper arm; the hand and trim follow the cuff.
                let bone = if o.shape == document_core::Shape::Rectangle {
                    shoulder
                } else {
                    elbow
                };
                Some((o.id, bone))
            } else {
                None
            }
        })
        .collect();
    for (id, bone) in rigid {
        doc.bind_rigid(id, Some(bone))?;
    }
    for (bone, peak) in [(shoulder, -7.), (elbow, -32.), (wrist, 30.)] {
        for (frame, degrees) in [(0, 0.), (4, peak), (7, 0.)] {
            doc.set_bone_rotation_key(bone, frame, degrees)?;
        }
    }
    Ok(())
}

fn main() -> io::Result<()> {
    let target = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::other("Supply a new .vanim output path."))?;
    let source =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/lantern-keeper.vanim");
    let mut project = load_animation(&source)?;
    let clip = &mut project.clips[0];
    clip.name = "Lantern lift".into();
    clip.origin = [56., 114.];
    clip.set_length(8).map_err(io::Error::other)?;
    clip.set_fps(8).map_err(io::Error::other)?;
    rig_sleeve(clip.drawing_at_mut(0)).map_err(io::Error::other)?;
    project.preset.columns = 4;
    save_animation(&target, &project, false)
}
