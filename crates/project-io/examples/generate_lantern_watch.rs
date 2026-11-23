//! Rebuild the sample from the layered illustration, writing only to a new directory.
//! cargo run -p project-io --example generate_lantern_watch -- <new-directory>

use document_core::{AnimationProject, Document, Shape, VectorObject};
use project_io::{export_sprite_bundle, import_svg, load_animation, save_animation};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

struct Pose {
    arm: f32,
    swing: f32,
    head: f32,
    bob: f32,
    scarf: f32,
    eyelids: f32,
}

// The hand leads the lantern; the scarf and gaze catch up after each reversal.
const POSES: [Pose; 8] = [
    Pose {
        arm: -16.,
        swing: -24.,
        head: -4.,
        bob: 0.,
        scarf: -3.,
        eyelids: 1.,
    },
    Pose {
        arm: -11.,
        swing: -29.,
        head: -4.,
        bob: 1.,
        scarf: -2.,
        eyelids: 1.,
    },
    Pose {
        arm: 0.,
        swing: -18.,
        head: -1.,
        bob: 2.,
        scarf: 0.,
        eyelids: 0.75,
    },
    Pose {
        arm: 10.,
        swing: -1.,
        head: 3.,
        bob: 1.5,
        scarf: 2.5,
        eyelids: 0.2,
    },
    Pose {
        arm: 15.,
        swing: 20.,
        head: 5.,
        bob: 0.,
        scarf: 4.,
        eyelids: 0.7,
    },
    Pose {
        arm: 10.,
        swing: 29.,
        head: 5.,
        bob: -1.,
        scarf: 2.5,
        eyelids: 1.,
    },
    Pose {
        arm: 0.,
        swing: 17.,
        head: 1.,
        bob: -2.,
        scarf: 0.,
        eyelids: 1.,
    },
    Pose {
        arm: -10.,
        swing: 0.,
        head: -2.,
        bob: -1.,
        scarf: -2.,
        eyelids: 1.,
    },
];

fn rotate(point: [f32; 2], pivot: [f32; 2], degrees: f32) -> [f32; 2] {
    let (s, c) = degrees.to_radians().sin_cos();
    let x = point[0] - pivot[0];
    let y = point[1] - pivot[1];
    [pivot[0] + x * c - y * s, pivot[1] + x * s + y * c]
}

fn turn(object: &mut VectorObject, pivot: [f32; 2], degrees: f32) {
    let center = object.local_to_world([0.5; 2], 0);
    let target = rotate(center, pivot, degrees);
    object.position[0] += target[0] - center[0];
    object.position[1] += target[1] - center[1];
    object.rotation += degrees;
}

fn drawing(art: &Document, pose: &Pose) -> Document {
    let mut doc = art.clone();
    doc.frames = 8;
    doc.fps = 8;
    let grip = [89., 61.];
    let shoulder = [56., 69.];
    let reach = rotate(grip, shoulder, pose.arm);
    for object in &mut doc.objects {
        let name = &doc
            .layers
            .iter()
            .find(|l| l.id == object.layer)
            .unwrap()
            .name;
        match name.as_str() {
            "Lantern" => {
                turn(object, grip, pose.swing);
                object.position[0] += reach[0] - grip[0];
                object.position[1] += reach[1] - grip[1] + pose.bob;
            }
            "Sleeve" => {
                // The brass button sits on the coat while the grasping arm pivots.
                if object.shape != Shape::Rectangle {
                    turn(object, shoulder, pose.arm);
                }
                object.position[1] += pose.bob;
            }
            "Hood" => {
                if object.shape == Shape::Ellipse {
                    let original_height = object.size[1];
                    object.size[1] *= pose.eyelids;
                    object.position[1] += (original_height - object.size[1]) * 0.5;
                }
                turn(object, [53., 60.], pose.head);
                object.position[1] += pose.bob;
            }
            "Coat" => {
                object.position[1] += pose.bob;
                if object.fill == [170, 86, 68, 255] {
                    // Keep the scarf knot attached and redraw the free end on each cel.
                    if let Shape::Path { nodes, .. } = &mut object.shape {
                        for node in nodes {
                            let shift = pose.scarf * (1. - node.point[0]).powi(2) / object.size[1];
                            for p in [&mut node.point, &mut node.incoming, &mut node.outgoing] {
                                p[1] += shift;
                            }
                        }
                    }
                }
            }
            _ => {} // The boots stay planted in all eight drawings.
        }
    }
    doc
}

fn make_project(source: &Path) -> io::Result<AnimationProject> {
    let art = import_svg(source)?;
    let mut project = AnimationProject::from_document(art.clone());
    project
        .presets
        .insert("Godot four columns".into(), project.preset);
    let swing = &mut project.clips[0];
    swing.name = "Lantern swing".into();
    swing.frames = 8;
    swing.fps = 8;
    swing.origin = [56., 114.];
    swing.cels = POSES
        .iter()
        .enumerate()
        .map(|(i, pose)| (i as u32, Arc::new(drawing(&art, pose))))
        .collect();

    let mut signal = swing.clone();
    signal.name = "Signal".into();
    signal.cels.retain(|f, _| *f == 0);
    let doc = signal.drawing_at_mut(0);
    let flame = doc
        .objects
        .iter_mut()
        .find(|o| o.fill == [255, 240, 166, 255])
        .unwrap();
    flame
        .scale_keys
        .extend([(0, [0.7, 0.8]), (3, [1.25, 1.35]), (7, [0.7, 0.8])]);
    flame.rotation_keys.extend([(0, -12.), (4, 11.), (7, -12.)]);
    flame.opacity_keys.extend([(0, 0.45), (3, 1.), (7, 0.45)]);
    let [x, y] = flame.position;
    flame
        .keys
        .extend([(0, [x, y]), (3, [x + 0.75, y - 1.5]), (7, [x, y])]);
    project.clips.push(signal);
    project.validate().map_err(io::Error::other)?;
    Ok(project)
}

fn main() -> io::Result<()> {
    let dest = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or_else(|| {
            io::Error::other("Supply a new directory for the project and its sprite bundle.")
        })?;
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lantern-keeper.svg");
    let project = make_project(&source)?;
    std::fs::create_dir(&dest)?;
    let path = dest.join("lantern-watch.vanim");
    save_animation(&path, &project, false)?;
    let reopened = load_animation(&path)?;
    export_sprite_bundle(
        &dest.join("lantern-swing"),
        &reopened.clips[0],
        &reopened.preset,
        true,
    )
}
