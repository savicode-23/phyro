//! Rebuild the combined cel and property animation and its packed Godot atlas.
use document_core::{
    AnimationProject, Easing, KeyValue, MotionPath, PathNode, Property, Shape, VectorObject,
};
use project_io::{export_new_frame, export_packed_atlas, load_animation, save_animation};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const POSE_STARTS: [u32; 8] = [0, 2, 3, 4, 6, 7, 9, 10];

fn animate_flame(flame: &mut VectorObject) -> io::Result<()> {
    // The motion guide is relative to its first anchor. Draw it at the lamp so
    // the same curve can be inspected on the canvas while editing this cel.
    let [x, y] = flame.local_to_world([0.5, 0.5], 0);
    let mut start = PathNode::corner([x, y]);
    start.outgoing = [x + 1.7, y - 0.1];
    let mut end = PathNode::corner([x + 0.2, y - 1.8]);
    end.incoming = [x + 1.1, y - 2.0];
    flame.motion = Some(MotionPath::new(vec![start, end], false).map_err(io::Error::other)?);
    for (tick, value) in [(0, 0.), (3, 1.), (7, 0.3), (11, 0.)] {
        Property::MotionProgress.set(flame, tick, KeyValue::Scalar(value));
    }
    Property::MotionProgress.set_easing(
        flame,
        0,
        Easing::Cubic {
            out: [0.18, 0.03],
            incoming: [0.64, 0.95],
        },
    );
    flame.scale_keys.extend([
        (0, [0.7, 0.85]),
        (3, [1.35, 1.5]),
        (7, [0.85, 0.95]),
        (11, [0.7, 0.85]),
    ]);
    flame
        .opacity_keys
        .extend([(0, 0.68), (3, 1.), (7, 0.8), (11, 0.68)]);

    // Move only the flame tip and its handles. All other authored nodes keep
    // their identity as the flame leans across the glass.
    let base = flame.shape.clone();
    let mut bent = base.clone();
    let tip = bent
        .contours_mut()
        .iter_mut()
        .flatten()
        .min_by(|a, b| a.point[1].total_cmp(&b.point[1]))
        .ok_or_else(|| io::Error::other("Expected an editable flame path."))?;
    for p in [&mut tip.point, &mut tip.incoming, &mut tip.outgoing] {
        p[0] += 1.15 / flame.size[0];
        p[1] -= 1.1 / flame.size[1];
    }
    for (tick, shape) in [(0, base.clone()), (4, bent), (11, base)] {
        Property::Geometry.set(flame, tick, KeyValue::Shape(shape));
    }
    Property::Geometry.set_easing(flame, 0, Easing::EaseInOut);
    Ok(())
}

fn make_patrol(source: &Path) -> io::Result<AnimationProject> {
    let swing_project = load_animation(source)?;
    let poses = &swing_project.clips[0];
    let mut patrol = AnimationProject::from_document(poses.drawing_at(0).clone());
    let clip = &mut patrol.clips[0];
    clip.name = "Lantern patrol".into();
    clip.frames = 12;
    clip.fps = 12;
    clip.origin = poses.origin;
    clip.cels.clear();
    for (index, start) in POSE_STARTS.into_iter().enumerate() {
        let mut cel = poses.drawing_at(index as u32).clone();
        cel.frames = clip.frames;
        cel.fps = clip.fps;
        let flame = cel
            .objects
            .iter_mut()
            .find(|o| o.fill == [255, 240, 166, 255])
            .ok_or_else(|| io::Error::other("Missing flame in the source drawing."))?;
        animate_flame(flame)?;
        let hood = cel.layers.iter().find(|l| l.name == "Hood").unwrap().id;
        for eye in cel
            .objects
            .iter_mut()
            .filter(|o| o.layer == hood && o.shape == Shape::Ellipse)
        {
            eye.scale_keys
                .extend([(0, [1., 1.]), (3, [1., 1.]), (4, [1., 0.35]), (6, [1., 1.])]);
        }
        clip.cels.insert(start, Arc::new(cel));
    }
    clip.events.extend([
        (0, vec!["light_on".to_owned()]),
        (2, vec!["swing_left".to_owned(), "bell".to_owned()]),
        (7, vec!["swing_right".to_owned()]),
    ]);
    patrol.preset.padding = 1;
    patrol.preset.margin = 1;
    patrol.preset.packed.enabled = true;
    patrol.preset.packed.max_size = 256;
    patrol.preset.packed.allow_rotation = true;
    patrol
        .presets
        .insert("Godot atlas".to_owned(), patrol.preset);
    patrol.validate().map_err(io::Error::other)?;
    Ok(patrol)
}

fn main() -> io::Result<()> {
    let destination = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::other("Supply a new output directory."))?;
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lantern-watch.vanim");
    let project = make_patrol(&source)?;
    std::fs::create_dir(&destination)?;
    let native = destination.join("lantern-patrol.vanim");
    save_animation(&native, &project, false)?;
    let reopened = load_animation(&native)?;
    let clip = &reopened.clips[0];
    let atlas = destination.join("lantern-patrol");
    export_packed_atlas(&atlas, clip, &reopened.preset)?;
    let reference = atlas.join("reference");
    std::fs::create_dir(&reference)?;
    for tick in 0..clip.frames {
        export_new_frame(
            &reference.join(format!("frame_{tick:04}.png")),
            clip.drawing_at(tick),
            tick,
        )?;
    }
    Ok(())
}
