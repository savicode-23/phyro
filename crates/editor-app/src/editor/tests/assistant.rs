use super::*;
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

// CI builds both executables first. The only test-only component is the artist's
// headless window. All assistant requests travel through the shipping stdio CLI.
pub(super) struct Client {
    child: Child,
    responses: mpsc::Receiver<String>,
    seq: u64,
}

impl Client {
    pub(super) fn start() -> Self {
        let exe = std::env::var_os("PHYRO_TEST_MCP_CLI")
            .expect("set PHYRO_TEST_MCP_CLI to the built CLI");
        let mut child = Command::new(exe)
            .arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (send, responses) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if send.send(line).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            responses,
            seq: 0,
        }
    }
    fn send(&mut self, method: &str, mut params: Value) {
        self.seq += 1;
        params["_meta"] = json!({"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientInfo":{"name":"artist","version":"1"},"io.modelcontextprotocol/clientCapabilities":{}});
        let request = json!({"jsonrpc":"2.0","id":self.seq,"method":method,"params":params});
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{request}").unwrap();
        stdin.flush().unwrap();
    }
    fn rpc(&mut self, w: &mut EditorWindow, method: &str, params: Value) -> Value {
        self.send(method, params);
        let start = Instant::now();
        loop {
            match self.responses.recv_timeout(Duration::from_millis(2)) {
                Ok(line) => {
                    let response: Value = serde_json::from_str(&line).unwrap();
                    assert_eq!(response["id"], self.seq, "{response}");
                    assert!(response.get("error").is_none(), "{response}");
                    return response["result"].clone();
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    w.frame(vec![]);
                }
                Err(e) => panic!("{e}"),
            }
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "timeout {method}"
            );
        }
    }
    pub(super) fn tool(&mut self, w: &mut EditorWindow, name: &str, args: Value) -> Value {
        self.rpc(w, "tools/call", json!({"name":name,"arguments":args}))["structuredContent"]
            .clone()
    }
    pub(super) fn pair(&mut self, w: &mut EditorWindow) -> Value {
        let deadline = Instant::now() + Duration::from_secs(5);
        let code = loop {
            let pair = self.tool(w, "connect", json!({}));
            if let Some(code) = pair["code"].as_str() {
                break code.to_owned();
            }
            assert!(
                Instant::now() < deadline,
                "Previous pairing did not close: {pair}"
            );
        };
        // A fresh egui context has a sizing pass before the menu is painted.
        for _ in 0..3 {
            w.frame(vec![]);
        }
        if !w.editor.assistant.open {
            w.click("File");
            w.click("Share this tab with an assistant…");
        }
        if !w.editor.assistant_enabled {
            w.click("Enable assistant sharing");
        }
        w.frame(vec![]);
        let pos = w
            .ctx
            .read_response(egui::Id::new("assistant pairing code"))
            .unwrap()
            .rect
            .center();
        w.click_at(pos);
        w.frame(vec![egui::Event::Text(code)]);
        w.click("Share this tab");
        let start = Instant::now();
        loop {
            let result = self.tool(w, "get_context", json!({}));
            if result.get("document").is_some() {
                return result;
            }
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "pairing {result} / {}",
                w.editor.status
            );
        }
    }
    pub(super) fn render_until(
        mut self,
        clip: u64,
        stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
        rendered: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    ) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            use std::sync::atomic::Ordering;
            while !stop.load(Ordering::Acquire) {
                self.send(
                    "tools/call",
                    json!({"name":"render_frame","arguments":{"clip_id":clip,"tick":4}}),
                );
                let line = self
                    .responses
                    .recv_timeout(Duration::from_secs(10))
                    .unwrap();
                let result: Value = serde_json::from_str(&line).unwrap();
                if result["result"]["isError"] == false {
                    rendered.fetch_add(1, Ordering::Release);
                }
            }
        })
    }
    fn render(&mut self, w: &mut EditorWindow, clip: u64, tick: u32) -> Vec<u8> {
        let r = self.rpc(
            w,
            "tools/call",
            json!({"name":"render_frame","arguments":{"clip_id":clip,"tick":tick}}),
        );
        let contents = r["content"].as_array().unwrap();
        let img = contents.iter().find(|c| c["type"] == "image").unwrap();
        assert_eq!(img["mimeType"], "image/png");
        let bytes = STANDARD.decode(img["data"].as_str().unwrap()).unwrap();
        renderer::Pixmap::decode_png(&bytes).unwrap().take()
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.child.stdin.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
#[ignore = "requires built phyro-cli; exercised in desktop CI"]
fn stdio_artist_reviews_an_unsaved_sleeve_and_manual_edits_win() {
    let mut w = EditorWindow::new(egui::vec2(1440., 940.));
    assert!(!w.editor.assistant_enabled && !w.editor.assistant.sharing());
    let mut client = Client::start();
    assert_eq!(
        client.tool(&mut w, "get_context", json!({}))["code"],
        "not_paired"
    );
    let blank = client.pair(&mut w);
    let mut other = EditorWindow::new(egui::vec2(1440., 940.));
    let mut other_client = Client::start();
    let other_context = other_client.pair(&mut other);
    assert_ne!(other_context["document"], blank["document"]);
    other
        .editor
        .close_document_tab_with(0, |_| OpenDecision::Proceed);
    assert!(!other.editor.assistant.sharing());
    assert_eq!(
        client.tool(&mut w, "get_context", json!({}))["document"],
        blank["document"]
    );
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lantern-walk.vanim");
    let project = project_io::load_animation(&source).unwrap();
    w.editor.open_document_tab(project, None);
    assert!(!w.editor.assistant.sharing());
    let clip_id = w.editor.project.clips[0].id;
    let id = w
        .editor
        .scene()
        .objects
        .iter()
        .find(|o| o.name == "Sleeve fabric")
        .unwrap()
        .id;
    w.editor.selection = vec![id];
    let mut context = client.pair(&mut w);
    assert_ne!(blank["document"], context["document"]);
    assert_eq!(context["selected_object_ids"], json!([id]));
    let inspected = client.tool(
        &mut w,
        "inspect",
        json!({"clip_id":clip_id,"tick":0,"query":{"kind":"selection"}}),
    );
    let sleeve = &inspected["objects"][0];
    assert_eq!(sleeve["name"], "Sleeve fabric");
    let start = sleeve["exposure_start"].as_u64().unwrap();
    let x = sleeve["position"][0].as_f64().unwrap();
    let y = sleeve["position"][1].as_f64().unwrap();
    let ops = json!([
        {"op":"set_fill","clip_id":clip_id,"exposure_start":start,"object_id":id,"fill":[184,115,51,255]},
        {"op":"set_position_key","clip_id":clip_id,"exposure_start":start,"object_id":id,"frame":4,"position":[x+8.,y]}
    ]);
    let mut args = json!({"document":context["document"],"expected_revision":context["revision"],"request_id":"sleeve","operations":ops});
    let before = w.editor.project.clone();
    let before_pixels = client.render(&mut w, clip_id, 4);
    let mut bad = args.clone();
    bad["operations"][1]["object_id"] = json!(999999);
    let failure = client.tool(&mut w, "propose_edits", bad);
    assert_eq!(failure["details"]["operation_index"], 1);
    assert_eq!(w.editor.project, before);
    let proposed = client.tool(&mut w, "propose_edits", args.clone());
    assert_eq!(proposed["state"], "pending", "{proposed}");
    assert_eq!(client.tool(&mut w, "propose_edits", args.clone()), proposed);
    w.editor.selection.clear();
    w.editor.new_document_tab();
    assert_eq!(
        client.tool(&mut w, "get_context", json!({}))["code"],
        "paused"
    );
    w.editor.select_document_tab(0);
    wait_for_preview(&mut w);
    assert_eq!(w.editor.timeline.frame, 0);
    w.click("Apply");
    let status_args = json!({"proposal_id":proposed["proposal_id"]});
    let applied = client.tool(&mut w, "get_proposal", status_args);
    assert_eq!(
        applied["state"], "applied",
        "{applied} / {}",
        w.editor.status
    );
    assert_eq!(applied["operations"][1]["affected_ids"], json!([id]));
    assert_eq!(client.tool(&mut w, "propose_edits", args.clone()), applied);
    let accepted = w.editor.project.clone();
    assert_ne!(accepted, before);
    assert_ne!(before_pixels, client.render(&mut w, clip_id, 4));
    for tick in [0, 2, 4, 5] {
        let drawing = accepted.clips[0].drawing_at(tick);
        assert_eq!(
            client.render(&mut w, clip_id, tick),
            renderer::render(drawing, tick).unwrap().take()
        );
    }
    w.editor.restore(false);
    assert_eq!(w.editor.project, before);
    w.editor.restore(true);
    assert_eq!(w.editor.project, accepted);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accepted.vanim");
    project_io::save_animation(&path, &accepted, false).unwrap();
    assert_eq!(project_io::load_animation(&path).unwrap(), accepted);
    context = client.tool(&mut w, "get_context", json!({}));
    args["expected_revision"] = context["revision"].clone();
    args["request_id"] = json!("unchanged");
    let unchanged = client.tool(&mut w, "propose_edits", args.clone());
    wait_for_preview(&mut w);
    w.click("Apply");
    assert_eq!(
        client.tool(
            &mut w,
            "get_proposal",
            json!({"proposal_id":unchanged["proposal_id"]})
        )["committed_revision"],
        context["revision"]
    );
    args["request_id"] = json!("manual-wins");
    args["operations"][0]["fill"] = json!([100, 100, 100, 255]);
    let staged = client.tool(&mut w, "propose_edits", args.clone());
    assert_eq!(staged["state"], "pending");
    w.editor.project.clips[0].name = "Artist's change".into();
    w.editor.finish_edit();
    let status = client.tool(
        &mut w,
        "get_proposal",
        json!({"proposal_id":staged["proposal_id"]}),
    );
    assert_eq!(status["state"], "stale");
    assert_eq!(w.editor.project.clips[0].name, "Artist's change");
    assert_eq!(client.tool(&mut w, "propose_edits", args.clone()), status);
    args["expected_revision"] = json!(w.editor.history.revision);
    args["request_id"] = json!("reject");
    let staged = client.tool(&mut w, "propose_edits", args);
    w.click("Reject");
    assert_eq!(
        client.tool(
            &mut w,
            "get_proposal",
            json!({"proposal_id":staged["proposal_id"]})
        )["state"],
        "rejected"
    );
    w.click("Enable assistant sharing");
    assert!(!w.editor.assistant.sharing());
    assert_ne!(
        client.tool(&mut w, "get_context", json!({})).get("code"),
        None
    );
    client.pair(&mut w);
    let rev = w.editor.history.revision;
    let pending=client.tool(&mut w,"propose_edits",json!({"document":context["document"],"expected_revision":rev,"request_id":"disconnect","operations":ops}));
    assert_eq!(pending["state"], "pending");
    let before_disconnect = w.editor.project.clone();
    client.child.stdin.take();
    let start = Instant::now();
    while w.editor.assistant.sharing() {
        w.frame(vec![]);
        assert!(start.elapsed() < Duration::from_secs(5));
    }
    assert_eq!(w.editor.project, before_disconnect);
    assert!(client.child.wait().unwrap().success());
}

fn wait_for_preview(w: &mut EditorWindow) {
    let start = Instant::now();
    while !w
        .frame(vec![])
        .shapes
        .iter()
        .any(|s| matches!(&s.shape,egui::Shape::Text(t) if t.galley.text()=="Proposed"))
    {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "Preview did not appear: {}",
            w.editor.status
        );
    }
}

pub(super) fn wait_for_step(w: &mut EditorWindow, step: usize) {
    let label = format!("Keep through step {step}");
    let start = Instant::now();
    while !w
        .frame(vec![])
        .shapes
        .iter()
        .any(|s| matches!(&s.shape,egui::Shape::Text(t) if t.galley.text()==label))
    {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "Missing {label}: {}",
            w.editor.status
        );
    }
}

#[test]
#[ignore = "requires built phyro-cli; exercised in desktop CI"]
fn stdio_artist_reviews_live_lantern_steps() {
    let mut w = EditorWindow::new(egui::vec2(1440., 940.));
    let mut project = w.editor.project.clone();
    let clip = &mut project.clips[0];
    clip.set_length(9).unwrap();
    for tick in [2, 6] {
        clip.cels.insert(tick, clip.cels[&0].clone());
    }
    let clip = clip.id;
    w.editor.replace_project(project.clone(), None);
    let mut client = Client::start();
    let context = client.pair(&mut w);
    let begin = json!({"document":context["document"],"expected_revision":context["revision"],"request_id":"lantern"});
    let draft = client.tool(&mut w, "begin_draft", begin.clone());
    assert_eq!(draft["latest_step"], 0, "{draft}");
    assert_eq!(client.tool(&mut w, "begin_draft", begin.clone()), draft);
    let id = &draft["proposal_id"];
    let append = |step: usize, label: &str, x: usize| {
        json!({"proposal_id":id,"step":step,"label":label,"operations":[
        {"op":"create_path","clip_id":clip,"exposure_start":2,"layer_id":1,"name":label,"closed":true,"fill":[245,100,32,255],
        "nodes":[{"point":[x,12]},{"point":[x+10,12]},{"point":[x,24]}]}]})
    };
    let outline = client.tool(&mut w, "append_draft_step", append(1, "Outline", 2));
    assert_eq!(outline["step"], 1, "{outline}");
    wait_for_step(&mut w, 1);
    let mut glass = append(2, "Glass", 18);
    glass["operations"]
        .as_array_mut()
        .unwrap()
        .push(json!({"op":"set_fill","clip_id":clip,"exposure_start":2,
        "object_id":outline["operations"][0]["created_ids"][0],"fill":[35,45,55,255]}));
    let ack = client.tool(&mut w, "append_draft_step", glass.clone());
    assert_eq!(ack["step"], 2);
    assert_eq!(client.tool(&mut w, "append_draft_step", glass), ack);
    let status = json!({"proposal_id":id});
    assert_eq!(
        client.tool(&mut w, "append_draft_step", append(2, "Different", 1))["expected_next_step"],
        3
    );
    assert_eq!(
        client.tool(&mut w, "append_draft_step", append(4, "Skipped", 2))["expected_next_step"],
        3
    );
    let mut flame = append(3, "Flame", 30);
    flame["operations"][0]["layer_id"] = json!(9999);
    assert_eq!(
        client.tool(&mut w, "append_draft_step", flame)["code"],
        "invalid_operation"
    );
    wait_for_step(&mut w, 2);
    w.click("Step 2 · Glass");
    w.click("Step 1 · Outline");
    wait_for_step(&mut w, 1);
    w.click("Follow latest");
    wait_for_step(&mut w, 2);
    // Select the current step explicitly to pin it.
    w.click("Step 2 · Glass");
    let popup = w
        .frame(vec![])
        .shapes
        .into_iter()
        .filter_map(|s| match s.shape {
            egui::Shape::Text(t) if t.galley.text() == "Step 2 · Glass" => {
                Some(t.galley.rect.translate(t.pos.to_vec2()).center())
            }
            _ => None,
        })
        .max_by(|a, b| a.y.total_cmp(&b.y))
        .unwrap();
    w.click_at(popup);
    let flame = client.tool(&mut w, "append_draft_step", append(3, "Flame", 30));
    assert_eq!(flame["step"], 3, "{flame}");
    assert_eq!(w.editor.project, project);
    // Start another append while the artist is pressing Keep. It may validate
    // before or after release. Both orderings must preserve the pinned prefix.
    let keep = w.control("Keep through step 2");
    let button = |pressed| egui::Event::PointerButton {
        pos: keep,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    w.frame(vec![egui::Event::PointerMoved(keep), button(true)]);
    client.send(
        "tools/call",
        json!({"name":"append_draft_step","arguments":append(4,"Top",40)}),
    );
    w.frame(vec![button(false)]);
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(line) = client.responses.recv_timeout(Duration::from_millis(2)) {
            let reply: Value = serde_json::from_str(&line).unwrap();
            let result = &reply["result"]["structuredContent"];
            assert!(
                result["state"] == "applied" || result["step"] == 4,
                "{reply}"
            );
            break;
        }
        w.frame(vec![]);
        assert!(Instant::now() < until);
    }
    let applied = client.tool(&mut w, "get_proposal", status.clone());
    assert_eq!(applied["state"], "applied", "{applied}");
    assert_eq!(applied["kept_step"], 2);
    assert_eq!(applied["operations"].as_array().unwrap().len(), 3);
    assert_eq!(
        client.tool(&mut w, "append_draft_step", append(4, "Late", 60)),
        applied
    );
    let accepted = w.editor.project.clone();
    for tick in 0..9 {
        let actual = &accepted.clips[0].drawing_at(tick).objects;
        let original = &project.clips[0].drawing_at(tick).objects;
        if (2..6).contains(&tick) {
            assert_eq!(actual.len(), original.len() + 2);
            let added = &actual[original.len()..];
            assert_eq!(added[0].name, "Outline");
            assert_eq!(added[1].name, "Glass");
            assert_eq!(json!(added[1].id), ack["operations"][0]["created_ids"][0]);
        } else {
            assert_eq!(actual, original);
        }
    }
    w.editor.restore(false);
    assert_eq!(w.editor.project, project);
    w.editor.restore(true);
    assert_eq!(w.editor.project, accepted);
    let mut next = begin;
    next["expected_revision"] = json!(w.editor.history.revision);
    next["request_id"] = json!("second");
    let second = client.tool(&mut w, "begin_draft", next.clone());
    let mut step = append(1, "Trim", 2);
    step["proposal_id"] = second["proposal_id"].clone();
    let finish = json!({"proposal_id":second["proposal_id"],"last_step":1});
    assert_eq!(
        client.tool(&mut w, "finish_draft", finish.clone())["code"],
        "step_conflict"
    );
    client.tool(&mut w, "append_draft_step", step);
    let ready = client.tool(&mut w, "finish_draft", finish.clone());
    assert_eq!(ready["state"], "ready");
    assert_eq!(client.tool(&mut w, "finish_draft", finish.clone()), ready);
    w.click("Discard draft");
    assert_eq!(
        client.tool(&mut w, "finish_draft", finish)["state"],
        "rejected"
    );
    assert_eq!(w.editor.project, accepted);

    next["request_id"] = json!("local-edit");
    let third = client.tool(&mut w, "begin_draft", next.clone());
    let mut step = append(1, "Detail", 4);
    step["proposal_id"] = third["proposal_id"].clone();
    w.editor.new_document_tab();
    assert_eq!(
        client.tool(&mut w, "append_draft_step", step.clone())["code"],
        "paused"
    );
    w.editor.select_document_tab(0);
    assert_eq!(
        client.tool(&mut w, "append_draft_step", step.clone())["step"],
        1
    );
    w.editor.restore(false);
    assert_eq!(
        client.tool(&mut w, "append_draft_step", step.clone())["state"],
        "stale"
    );
    next["expected_revision"] = json!(w.editor.history.revision);
    next["request_id"] = json!("disconnect");
    let fourth = client.tool(&mut w, "begin_draft", next);
    step["proposal_id"] = fourth["proposal_id"].clone();
    assert_eq!(client.tool(&mut w, "append_draft_step", step)["step"], 1);
    client.child.stdin.take();
    let until = Instant::now() + Duration::from_secs(5);
    while w.editor.assistant.sharing() {
        w.frame(vec![]);
        assert!(Instant::now() < until);
    }
    assert_eq!(w.editor.project, project);
}
