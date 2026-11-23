use super::*;
use crate::editor::tests::assistant::Client;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};

#[test]
#[ignore = "release timing benchmark with built PHYRO_TEST_MCP_CLI"]
fn live_assistant_preview_input_timings() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lantern-keeper.vanim");
    let keeper = project_io::load_animation(&path).unwrap();
    let mut scene =
        AnimationProject::from_document(tiled_scene(keeper.clips[0].drawing_at(0), 1000));
    scene.clips[0].set_length(1000).unwrap();
    for shared in [false, true] {
        let mut view = Viewport {
            window: EditorWindow::new(egui::vec2(1100., 740.)),
            dpi: 1.,
            time: 0.,
            board: egui::Rect::NOTHING,
        };
        view.window.editor.replace_project(scene.clone(), None);
        view.window.editor.tool = Tool::Select;
        view.frames(5, |_| vec![]);
        let stop = Arc::new(AtomicBool::new(false));
        let rendered = Arc::new(AtomicUsize::new(0));
        let job = shared.then(|| {
            let mut client = Client::start();
            let context = client.pair(&mut view.window);
            view.window.editor.assistant.open = false;
            let thread = client.render_until(
                context["clip_id"].as_u64().unwrap(),
                stop.clone(),
                rendered.clone(),
            );
            let start = Instant::now();
            while rendered.load(Ordering::Acquire) == 0 {
                view.frame(vec![], 1. / 60.);
                assert!(start.elapsed() < Duration::from_secs(20));
            }
            thread
        });
        println!(
            "1000 objects, 1000 frames, 1100×740 at 1×, sharing and continuous preview = {shared}"
        );
        let doc = view.window.editor.scene();
        let tile = (doc.width / 128 / 2 * 128) as f32;
        let p = view.board.min
            + view.board.size()
                * egui::vec2(
                    (tile + 55.) / doc.width as f32,
                    (tile + 41.) / doc.height as f32,
                );
        let mut first = Vec::new();
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            pressed,
            button: egui::PointerButton::Primary,
            modifiers: Default::default(),
        };
        let q = p + egui::vec2(9., 4.);
        for _ in 0..8 {
            view.window.editor.selection.clear();
            view.frame(vec![egui::Event::PointerMoved(p)], 1. / 60.);
            view.frame(vec![button(p, true)], 1. / 60.);
            first.push(view.frame(vec![egui::Event::PointerMoved(q)], 1. / 60.));
            view.frame(vec![button(q, false)], 1. / 60.);
            assert!(view.window.editor.history.can_undo());
            view.window.editor.restore(false);
        }
        report("first drag movement", first);
        let mut steps = Vec::new();
        for tick in 400..460 {
            view.window.editor.timeline.frame = tick;
            steps.push(view.frame(vec![], 1. / 60.));
        }
        report("timeline frame step", steps);
        if let Some(job) = job {
            stop.store(true, Ordering::Release);
            while !job.is_finished() {
                view.frame(vec![], 1. / 60.);
            }
            job.join().unwrap();
            println!(
                "{} verified PNG responses",
                rendered.load(Ordering::Acquire)
            );
        }
    }
}

#[test]
#[ignore = "release timing benchmark with built PHYRO_TEST_MCP_CLI"]
fn live_draft_step_to_visible_preview() {
    use crate::editor::tests::assistant::wait_for_step;
    use serde_json::json;
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lantern-keeper.vanim");
    let keeper = project_io::load_animation(&path).unwrap();
    let mut scene =
        AnimationProject::from_document(tiled_scene(keeper.clips[0].drawing_at(0), 1000));
    scene.clips[0].set_length(1000).unwrap();
    let mut w = EditorWindow::new(egui::vec2(1100., 740.));
    w.editor.replace_project(scene, None);
    let mut client = Client::start();
    let context = client.pair(&mut w);
    let draft=client.tool(&mut w,"begin_draft",json!({"document":context["document"],"expected_revision":context["revision"],"request_id":"timing"}));
    let nodes: Vec<_> = (0..32)
        .map(|i| {
            let a = std::f32::consts::TAU * i as f32 / 32.;
            json!({"point":[64.+20.*a.cos(),64.+20.*a.sin()]})
        })
        .collect();
    let mut send = |w: &mut EditorWindow, n| {
        let args = json!({"proposal_id":draft["proposal_id"],"step":n,"label":format!("Path {n}"),"operations":[
            {"op":"create_path","clip_id":context["clip_id"],"exposure_start":0,"layer_id":context["active_layer"],"name":format!("Part {n}"),
             "closed":true,"fill":[168,68,12,255],"nodes":nodes}]});
        let ack = client.tool(w, "append_draft_step", args);
        assert_eq!(ack["step"], n, "{ack}");
    };
    let mut samples = vec![];
    for n in 1..=20 {
        let start = Instant::now();
        send(&mut w, n);
        wait_for_step(&mut w, n);
        samples.push(start.elapsed());
        std::thread::sleep(Duration::from_secs(1).saturating_sub(start.elapsed()));
    }
    samples.sort();
    println!(
        "1000 objects, 32-node step, MCP call to painted preview p95 {:.1} ms",
        samples[18].as_secs_f64() * 1000.
    );
    let burst = Instant::now();
    for n in 21..=32 {
        send(&mut w, n);
    }
    let last_ack = Instant::now();
    wait_for_step(&mut w, 32);
    println!(
        "12-step burst including calls {:.1} ms, last acknowledgement to visible {:.1} ms",
        burst.elapsed().as_secs_f64() * 1000.,
        last_ack.elapsed().as_secs_f64() * 1000.
    );
}
