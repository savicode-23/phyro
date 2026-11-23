use super::{
    review::{render_clip, Review},
    Editor, Event, Record, Session,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use document_core::AnimationProject;
use eframe::egui;
use phyro_automation::{
    inspect, ipc,
    live::{error, Query, Reply, Request},
};
use serde_json::{json, Value};
use std::{
    hash::{DefaultHasher, Hash, Hasher},
    io,
    sync::{mpsc::SyncSender, Arc},
    time::Duration,
};
use tokio::{io::AsyncReadExt, sync::oneshot};

struct Ui {
    events: SyncSender<Event>,
    ctx: egui::Context,
}

impl Ui {
    async fn call<T: Send + 'static>(
        &self,
        f: impl FnOnce(&Editor, &mut Session) -> Reply<T> + Send + 'static,
    ) -> Reply<T> {
        let (tx, rx) = oneshot::channel();
        self.events
            .try_send(Box::new(move |editor, session, access| {
                if !tx.is_closed() {
                    let _ = tx.send(access.and_then(|()| f(editor, session)));
                }
            }))
            .map_err(|_| error("busy", "Editor unavailable. Retry the request."))?;
        self.ctx.request_repaint();
        rx.await
            .map_err(|_| error("disconnected", "Sharing stopped."))?
    }

    async fn snapshot(&self) -> Reply<Snapshot> {
        self.call(|e, s| {
            Ok(Snapshot {
                project: Arc::new(e.baseline.clone()),
                revision: e.history.revision,
                handle: s.handle.clone(),
                clip: e.project.clips[e.timeline.clip].id,
                tick: e.timeline.frame,
                selection: e.selection.clone(),
            })
        })
        .await
    }

    async fn check(&self, revision: u64) -> Reply<()> {
        self.call(move |e, _| check_revision(e, revision)).await
    }
}

fn check_revision(e: &Editor, expected: u64) -> Reply<()> {
    if e.history.revision != expected {
        return Err(error("stale", "The document changed. Inspect again."));
    }
    Ok(())
}

struct Snapshot {
    project: Arc<AnimationProject>,
    revision: u64,
    handle: String,
    clip: u64,
    tick: u32,
    selection: Vec<u64>,
}

pub(super) fn run(
    code: String,
    events: SyncSender<Event>,
    ctx: &egui::Context,
    mut cancel: oneshot::Receiver<()>,
) -> io::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(async {
        tokio::select! {
            _ = &mut cancel => Ok(()),
            result = serve(code,Ui {events,ctx:ctx.clone()}) => result,
        }
    });
    runtime.shutdown_background();
    result
}

async fn serve(code: String, ui: Ui) -> io::Result<()> {
    let stream = tokio::time::timeout(Duration::from_secs(5), ipc::pair(&code)).await??;
    let (mut read, mut write) = tokio::io::split(stream);
    loop {
        let request = ipc::receive(&mut read, ipc::MAX_REQUEST).await?;
        // The bridge serializes requests. EOF during a render or validation cancels
        // publication, even if a CPU job takes longer to unwind.
        let reply = tokio::select! {
            biased;
            _ = read.read_u8() => return Ok(()),
            result = dispatch(request,&ui) => result,
        };
        if serde_json::to_vec(&reply)?.len() > ipc::MAX_RESPONSE {
            ipc::send(
                &mut write,
                &Reply::<Value>::Err(error("result_too_large", "Use a smaller page size.")),
                ipc::MAX_RESPONSE,
            )
            .await?;
        } else {
            ipc::send(&mut write, &reply, ipc::MAX_RESPONSE).await?;
        }
    }
}

async fn cpu<T: Send + 'static>(job: impl FnOnce() -> Reply<T> + Send + 'static) -> Reply<T> {
    tokio::task::spawn_blocking(job)
        .await
        .map_err(|_| error("worker_failed", "The artwork worker stopped."))?
}

async fn dispatch(request: Request, ui: &Ui) -> Reply {
    match request {
        Request::GetContext {} => ui.call(|editor, session| {
            let clip = &editor.baseline.clips[editor.timeline.clip];
            let tick = editor.timeline.frame;
            let (start, duration) = clip.exposure(tick);
            Ok(json!({
                "document": session.handle, "name": editor.name(), "revision": editor.history.revision,
                "clip_id": clip.id, "clip_name": clip.name, "frames": clip.frames,
                "tick": tick, "frame_number": tick + 1, "active_layer": editor.style.layer,
                "selected_object_ids": editor.selection,
                "exposure_start": start, "exposure_duration": duration
            }))
        }).await,
        Request::GetProposal { proposal_id } => ui.call(move |_, session| {
            session.records.get(&proposal_id).map(|r| r.status.clone())
                .ok_or_else(|| error("unknown_proposal", "Unknown proposal in this session."))
        }).await,
        Request::BeginDraft { document, expected_revision, request_id } => ui.call(move |e,s| s.begin(e, document, expected_revision, request_id)).await,
        Request::FinishDraft { proposal_id, last_step } => ui.call(move |_,s| s.seal(&proposal_id, last_step)).await,
        Request::AppendDraftStep { proposal_id, step, label, operations } => append(ui, proposal_id, step, label, operations).await,
        Request::ProposeEdits { document, expected_revision, request_id, operations } =>
            propose(ui, document, expected_revision, request_id, operations).await,
        Request::Inspect { clip_id, tick, query, offset, limit } => {
            read_frame(ui, clip_id, tick, move |snapshot, clip| {
                if matches!(query, Query::Selection)
                    && (snapshot.clip != clip.id || clip.exposure(snapshot.tick).0 != clip.exposure(tick).0)
                {
                    return Err(error("selection_scope", "Selection IDs belong to another drawing. Inspect that drawing or query by name."));
                }
                inspect::page(clip, tick, &query, &snapshot.selection, offset, limit).map_err(|e| json!(e))
            }).await
        }
        Request::RenderFrame { clip_id, tick } => {
            read_frame(ui, clip_id, tick, move |_, clip| {
                let pixmap = render_clip(clip, tick, 1024)?;
                let png = pixmap.encode_png().map_err(|e| error("render_failed", e))?;
                Ok(json!({
                    "width": pixmap.width(), "height": pixmap.height(),
                    "png": STANDARD.encode(png)
                }))
            }).await
        }
    }
}

async fn read_frame(
    ui: &Ui,
    clip_id: u64,
    tick: u32,
    read: impl FnOnce(&Snapshot, &document_core::Clip) -> Reply + Send + 'static,
) -> Reply {
    let snapshot = ui.snapshot().await?;
    let revision = snapshot.revision;
    let result = cpu(move || {
        let clip = inspect::clip_by_id(&snapshot.project, clip_id).map_err(|e| json!(e))?;
        let mut page = read(&snapshot, clip)?;
        page["document"] = json!(snapshot.handle);
        page["revision"] = json!(revision);
        page["clip_id"] = json!(clip_id);
        page["tick"] = json!(tick);
        page["frame_number"] = json!(tick + 1);
        Ok(page)
    })
    .await?;
    ui.check(revision).await?;
    Ok(result)
}

enum Stage {
    Existing(Value),
    New(Arc<AnimationProject>),
}

async fn propose(
    ui: &Ui,
    document: String,
    revision: u64,
    request_id: String,
    operations: Value,
) -> Reply {
    if request_id.is_empty()
        || request_id.len() > 128
        || !operations
            .as_array()
            .is_some_and(|a| (1..=128).contains(&a.len()))
    {
        return Err(error(
            "invalid_batch",
            "Supply a request ID of up to 128 bytes and 1–128 operations.",
        ));
    }
    let mut hash = DefaultHasher::new();
    (revision, &operations).hash(&mut hash);
    let fingerprint = hash.finish();
    let key = request_id.clone();
    let stage = ui
        .call(move |e, s| {
            if document != s.handle {
                return Err(error(
                    "wrong_document",
                    "Inspect the shared document again.",
                ));
            }
            if let Some(record) = s.records.values().find(|r| r.request_id == key) {
                if record.status.get("latest_step").is_some() || record.fingerprint != fingerprint {
                    return Err(error(
                        "request_conflict",
                        "This request ID belongs to a different batch.",
                    ));
                }
                return Ok(Stage::Existing(record.status.clone()));
            }
            check_revision(e, revision)?;
            if s.pending.is_some() || s.draft.is_some() {
                return Err(error("busy", "A proposal is waiting for artist review."));
            }
            if s.records.len() >= 256 {
                return Err(error(
                    "session_full",
                    "Pair again to start another review session.",
                ));
            }
            Ok(Stage::New(Arc::new(e.baseline.clone())))
        })
        .await?;
    let before = match stage {
        Stage::Existing(result) => return Ok(result),
        Stage::New(before) => before,
    };
    let review = cpu(move || Review::prepare(before, revision, operations)).await?;
    ui.call(move |e, s| {
        check_revision(e, revision)?;
        let id = review.id.clone();
        let status = json!({"proposal_id":id,"state":"pending","expected_revision":revision});
        s.records.insert(
            id,
            Record {
                request_id,
                fingerprint,
                status: status.clone(),
            },
        );
        s.pending = Some(review);
        Ok(status)
    })
    .await
}

async fn append(ui: &Ui, id: String, number: usize, label: String, operations: Value) -> Reply {
    use super::draft::{Append, Step};
    let step = Arc::new(Step { label, operations });
    let input = step.clone();
    let proposal = id.clone();
    let bytes =
        serde_json::to_vec(&json!({"step":number,"label":step.label,"operations":step.operations}))
            .unwrap()
            .len();
    let stage = ui
        .call(move |_, s| s.stage(&proposal, number, &input, bytes))
        .await?;
    let latest = match stage {
        Append::Answer(status) => return Ok(status),
        Append::Validate(project) => project,
    };
    let validated = cpu(move || {
        let mut project = (*latest).clone();
        let results =
            phyro_automation::edit::apply(&mut project, &step.operations).map_err(|e| json!(e))?;
        Ok((Arc::new(project), results, step))
    })
    .await;
    ui.call(move |_, s| {
        if let Some(status) = s.closed_draft(&id)? {
            return Ok(status);
        }
        let (after, results, step) = validated?;
        s.accept_step(&id, number, step, after, results, bytes)
    })
    .await
}
