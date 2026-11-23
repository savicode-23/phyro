use super::*;
use document_core::{PathNode, Shape};

fn fixture(count: usize) -> AnimationProject {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lantern-walk.vanim");
    let mut project = project_io::load_animation(&path).unwrap();
    let doc = project.clips[0].drawing_at_mut(0);
    if count == 0 {
        return project;
    }
    let layer = doc.layers[0].id;
    let hidden = commands::add_layer(doc);
    doc.layers.last_mut().unwrap().visible = false;
    let locked = commands::add_layer(doc);
    doc.layers.last_mut().unwrap().locked = true;
    let mut nodes = [
        [0.1, 0.2],
        [0.75, 0.1],
        [0.9, 0.7],
        [0.45, 0.95],
        [0.025, 0.8],
    ]
    .map(PathNode::corner);
    for n in &mut nodes {
        n.incoming = [n.point[0] - 0.17, n.point[1] + 0.08];
        n.outgoing = [n.point[0] + 0.17, n.point[1] - 0.08];
    }
    let id = doc.add_shape(
        [500., 500.],
        [5., 8.],
        [210, 75, 26, 255],
        Shape::Path {
            nodes: nodes.to_vec(),
            closed: true,
        },
        layer,
    );
    let template = doc.objects.pop().unwrap();
    for i in 0..count {
        let mut o = template.clone();
        o.id = id + i as u64;
        o.position = [180. + (i % 130) as f32 * 7., 190. + (i / 130) as f32 * 9.];
        if i % 5 == 0 {
            o.position[1] = (i % 128) as f32;
        }
        if i % 5 == 1 {
            o.position[0] = (i % 128) as f32;
        }
        if i % 3 != 0 {
            o.stroke_width = 1.45;
            o.stroke = [120, 70, 25, 255];
        }
        o.rotation = (i % 47) as f32;
        o.animated.skew = if i % 3 == 0 { 12. } else { -8. };
        o.group = (i % 5 < 2).then_some(800 + (i / 5) as u64);
        o.layer = match i % 19 {
            0 => hidden,
            1 => locked,
            _ => layer,
        };
        doc.objects.push(o);
    }
    doc.next_id = id + count as u64;
    project
}

fn exact(doc: &Document, from: egui::Pos2, to: egui::Pos2, previous: &[u64]) -> Vec<u64> {
    let rect = egui::Rect::from_two_pos(from, to);
    let mut ids = previous.to_vec();
    for o in &doc.objects {
        if commands::is_editable(doc, o.layer) {
            let [l, t, r, b] = o.bounds(0);
            if rect.contains_rect(egui::Rect::from_min_max([l, t].into(), [r, b].into())) {
                ids.push(o.id);
            }
        }
    }
    commands::editable_selection(doc, &ids)
}

#[test]
#[ignore = "release timing benchmark"]
fn viewport_responsiveness_marquee() {
    for count in [0, 10_000] {
        let mut v = Viewport {
            window: EditorWindow::new(egui::vec2(1100., 740.)),
            dpi: 2.,
            time: 0.,
            board: egui::Rect::NOTHING,
        };
        v.window.editor.replace_project(fixture(count), None);
        v.frames(4, |_| Vec::new());
        let from = egui::pos2(5., 5.);
        let end = egui::pos2(81., 82.);
        let screen =
            |board: egui::Rect, p: egui::Pos2| board.min + board.size() * (p.to_vec2() / 128.);
        let mut presses = vec![];
        let mut first = vec![];
        let mut motion = vec![];
        let mut releases = vec![];
        let mut first_idle = vec![];
        v.window.editor.tool = Tool::Select;
        for i in 0..4 {
            let old = if i % 2 == 0 {
                vec![]
            } else {
                vec![
                    v.window
                        .editor
                        .scene()
                        .objects
                        .iter()
                        .find(|o| o.name == "Far boot")
                        .unwrap()
                        .id,
                ]
            };
            let mods = if old.is_empty() {
                egui::Modifiers::NONE
            } else {
                egui::Modifiers::SHIFT
            };
            let expected = exact(v.window.editor.scene(), from, end, &old);
            v.window.editor.selection = old;
            v.frames(2, |_| Vec::new());
            let start = screen(v.board, from);
            let finish = screen(v.board, end);
            let button = |pos, pressed| egui::Event::PointerButton {
                pos,
                pressed,
                button: egui::PointerButton::Primary,
                modifiers: mods,
            };
            v.frame(vec![egui::Event::PointerMoved(start)], 1. / 60.);
            presses.push(v.frame(vec![button(start, true)], 1. / 60.));
            for j in 1..=9 {
                let s = v.frame(
                    vec![egui::Event::PointerMoved(
                        start + (finish - start) * (j as f32 / 9.),
                    )],
                    1. / 60.,
                );
                if j == 1 {
                    first.push(s)
                } else {
                    motion.push(s)
                }
            }
            releases.push(v.frame(vec![button(finish, false)], 1. / 60.));
            first_idle.push(v.frame(Vec::new(), 1. / 60.));
            assert_eq!(v.window.editor.selection, expected);
        }
        println!("Lantern Walk with {count} extra objects, alternating Shift-marquee and marquee");
        report("press", presses);
        report("first move", first);
        report("steady", motion);
        report("release", releases);
        report("first idle", first_idle);
    }
}
