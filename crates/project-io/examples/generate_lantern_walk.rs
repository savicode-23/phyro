//! Build an in-place walk for a game character travelling three pixels per tick.
use document_core::{AnimationProject, Document, LineJoin, PathNode, Shape};
use project_io::{export_sprite_bundle, load_animation, save_animation};
use std::{io, path::PathBuf};

const INK: [u8; 4] = [25, 38, 40, 255];

fn patch(doc: &mut Document, layer: u64, name: &str, fill: [u8; 4], points: &[[f32; 2]]) -> u64 {
    let min = std::array::from_fn(|i| points.iter().map(|p| p[i]).fold(f32::INFINITY, f32::min));
    let size = std::array::from_fn(|i| {
        points
            .iter()
            .map(|p| p[i])
            .fold(f32::NEG_INFINITY, f32::max)
            - min[i]
    });
    let nodes = points
        .iter()
        .map(|p| PathNode::corner(std::array::from_fn(|i| (p[i] - min[i]) / size[i])))
        .collect();
    let id = doc.add_shape(
        min,
        size,
        fill,
        Shape::Path {
            nodes,
            closed: true,
        },
        layer,
    );
    let object = doc.objects.last_mut().unwrap();
    object.name = name.into();
    object.stroke = INK;
    object.stroke_width = 1.5;
    object.join = LineJoin::Round;
    id
}

fn boot_drawing(doc: &mut Document, curled: bool) {
    let toe = if curled { -2.5 } else { 1. };
    let sole = if curled { 1. } else { 4. };
    let layer = doc.layers[0].id;
    // Draw large in the source clip. Both placements are scaled to one quarter size.
    let boot = |points: &[[f32; 2]]| {
        points
            .iter()
            .map(|[x, y]| [60. + 4. * x, 64. + 4. * y])
            .collect::<Vec<_>>()
    };
    patch(
        doc,
        layer,
        "Leather",
        [112, 72, 53, 255],
        &boot(&[
            [-4., -5.],
            [3., -5.],
            [4., -1.],
            [9., toe],
            [10., sole],
            [7., 4.],
            [-4., 4.],
        ]),
    );
    doc.objects.last_mut().unwrap().stroke_width = 4.;
    patch(
        doc,
        layer,
        "Boot cuff",
        [193, 136, 82, 255],
        &boot(&[[-4., -5.], [3., -5.], [3., -3.], [-4., -2.5]]),
    );
    doc.objects.last_mut().unwrap().stroke_width = 2.;
}

fn add_boot_poses(project: &mut AnimationProject) -> Result<u64, String> {
    let index = project.add_clip("Boot poses")?;
    let clip = &mut project.clips[index];
    let doc = clip.drawing_at_mut(0);
    *doc = Document {
        frames: 1,
        fps: 8,
        ..Default::default()
    };
    boot_drawing(doc, false);
    clip.duplicate_frame(0)?;
    let curled = clip.drawing_at_mut(1);
    curled.objects.clear();
    boot_drawing(curled, true);
    Ok(clip.id)
}

fn far_arm(doc: &mut Document, layer: u64) -> Result<(), String> {
    let upper = doc.add_bone("Far upper arm", None, [47., 61.], [39., 73.])?;
    let forearm = doc.add_bone("Far forearm", Some(upper), [39., 73.], [31., 78.])?;
    let hand = doc.add_shape(
        [27., 74.],
        [8., 8.],
        [223, 184, 122, 255],
        Shape::Ellipse,
        layer,
    );
    doc.objects.last_mut().unwrap().stroke_width = 1.8;
    doc.bind_rigid(hand, Some(forearm))?;
    let sleeve = patch(
        doc,
        layer,
        "Far sleeve",
        [51, 91, 89, 255],
        &[
            [44., 58.],
            [51., 64.],
            [43., 76.],
            [33., 81.],
            [29., 75.],
            [36., 70.],
        ],
    );
    doc.bind_skin_at(sleeve, forearm, 0)?;
    for (i, w) in [0., 0., 0.5, 1., 1., 0.4].into_iter().enumerate() {
        doc.set_skin_weight(sleeve, 0, i, w)?;
    }
    for (id, keys) in [
        (upper, [16., 2., -16., -2., 9.]),
        (forearm, [-8., 5., 14., 0., -5.]),
    ] {
        for (tick, angle) in [0, 2, 4, 6, 7].into_iter().zip(keys) {
            doc.set_bone_rotation_key(id, tick, angle)?;
        }
    }
    Ok(())
}

fn walking_leg(
    project: &mut AnimationProject,
    source: u64,
    layer: u64,
    far: bool,
) -> Result<(), String> {
    let (side, x, shift, color) = if far {
        ("Far", 50., 4, [46, 65, 65, 255])
    } else {
        ("Near", 58., 0, [66, 83, 79, 255])
    };
    let placement = project.place_clip(source, 0, 0, layer, [x - 15., 92.])?;
    let doc = project.clips[0].drawing_at_mut(0);
    let foot = doc.objects.iter_mut().find(|o| o.id == placement).unwrap();
    foot.size = [32., 32.];
    foot.name = format!("{side} boot");
    let hip = [x, 87.];
    let knee = [x + 6., 98.];
    let ankle = [x, 108.];
    let thigh = doc.add_bone(&format!("{side} thigh"), None, hip, knee)?;
    let shin = doc.add_bone(&format!("{side} shin"), Some(thigh), knee, ankle)?;
    let sole = doc.add_bone(&format!("{side} sole"), Some(shin), ankle, [x + 8., 108.])?;
    doc.bind_rigid(placement, Some(sole))?;
    let leg = patch(
        doc,
        layer,
        &format!("{side} trousers"),
        color,
        &[
            [x - 5., 85.],
            [x + 4., 85.],
            [x + 9., 98.],
            [x + 3., 109.],
            [x - 3., 109.],
            [x + 2., 98.],
        ],
    );
    // Stack each boot over its own trouser cuff.
    let len = doc.objects.len();
    doc.objects.swap(len - 1, len - 2);
    doc.bind_skin_at(leg, shin, 0)?;
    for (node, weight) in [0., 0., 0.55, 1., 1., 0.45].into_iter().enumerate() {
        doc.set_skin_weight(leg, 0, node, weight)?;
    }
    let targets = [
        [6., 108.],
        [3., 108.],
        [0., 108.],
        [-3., 108.],
        [-6., 108.],
        [-5., 104.],
        [0., 100.],
        [5., 104.],
    ];
    let mut previous = None;
    for tick in 0..8 {
        let phase = (tick + shift) % 8;
        let [dx, y] = targets[phase];
        doc.pose_bone_ik(shin, tick as u32, [x + dx, y], false)?;
        // Counter-rotate the ankle through contact. Add toe roll only during recovery.
        let shin_angle = doc.bone_segment_at(shin, true, tick as u32)?;
        let rest = (ankle[1] - knee[1]).atan2(ankle[0] - knee[0]).to_degrees();
        let heading = (shin_angle.1[1] - shin_angle.0[1])
            .atan2(shin_angle.1[0] - shin_angle.0[0])
            .to_degrees();
        let roll = [0., 0., 0., 0., 0., 18., -9., -12.][phase];
        doc.set_bone_rotation_key(sole, tick as u32, roll - (heading - rest))?;
        let drawing = u32::from(phase >= 5);
        if previous != Some(drawing) {
            doc.set_drawing_key(placement, tick as u32, drawing)?;
            previous = Some(drawing);
        }
    }
    Ok(())
}

fn walk(project: &mut AnimationProject) -> Result<(), String> {
    let boots = add_boot_poses(project)?;
    let clip = &mut project.clips[0];
    clip.name = "Lantern walk".into();
    clip.origin = [56., 113.];
    clip.events.insert(0, vec!["near_contact".into()]);
    clip.events.insert(4, vec!["far_contact".into()]);
    let doc = clip.drawing_at_mut(0);
    let layer = doc
        .layers
        .iter()
        .find(|l| l.name == "Boots")
        .ok_or("Missing boots layer")?
        .id;
    doc.objects.retain(|o| o.layer != layer);
    far_arm(doc, layer)?;
    // The lit arm and the far sleeve swing on opposite beats.
    for (name, keys) in [
        ("Upper arm", [-9., 0., 12., 0., -6.]),
        ("Forearm", [-11., -3., 9., 0., -6.]),
        ("Lantern", [17., 1., -16., 1., 10.]),
    ] {
        let id = doc
            .skeleton
            .bones
            .iter()
            .find(|b| b.name == name)
            .ok_or("Missing arm joint")?
            .id;
        for (tick, angle) in [0, 2, 4, 6, 7].into_iter().zip(keys) {
            doc.set_bone_rotation_key(id, tick, angle)?;
        }
    }
    walking_leg(project, boots, layer, true)?;
    walking_leg(project, boots, layer, false)?;
    project.hydrate_composition()
}

fn main() -> io::Result<()> {
    let mut args = std::env::args_os().skip(1);
    let target = args.next().map(PathBuf::from).ok_or_else(|| {
        io::Error::other("Supply a new .vanim output path and optional new sprite directory.")
    })?;
    let sprites = args.next().map(PathBuf::from);
    let source =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/lantern-elbow.vanim");
    let mut project = load_animation(&source)?;
    walk(&mut project).map_err(io::Error::other)?;
    save_animation(&target, &project, false)?;
    if let Some(dir) = sprites {
        let reopened = load_animation(&target)?;
        export_sprite_bundle(&dir, &reopened.clips[0], &reopened.preset, true)?;
    }
    Ok(())
}
