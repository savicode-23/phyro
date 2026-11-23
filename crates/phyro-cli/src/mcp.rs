use phyro_automation::{
    ipc,
    live::{self, error, Reply, Request},
};
use rmcp::{model::*, service::RequestContext, RoleServer, ServerHandler, ServiceExt};
use serde_json::{json, Value};
use std::{
    ffi::OsString,
    io,
    process::ExitCode,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tokio::{
    io::AsyncReadExt,
    sync::{mpsc, oneshot},
    time::{timeout, Instant},
};

type Call = (Request, oneshot::Sender<Reply>);

struct Pairing {
    code: String,
    deadline: Instant,
    paired: Arc<AtomicBool>,
    calls: mpsc::Sender<Call>,
}

impl Pairing {
    fn start() -> io::Result<Self> {
        let code = ipc::token()?;
        let listener = ipc::listen(&code)?;
        let (calls, mut queue) = mpsc::channel::<Call>(4);
        let paired = Arc::new(AtomicBool::new(false));
        let ready = paired.clone();
        let proof = code.clone();
        let deadline = Instant::now() + ipc::PAIRING_LIFETIME;
        tokio::spawn(async move {
            let Ok(Ok(mut stream)) =
                tokio::time::timeout_at(deadline, ipc::accept(listener, &proof)).await
            else {
                return;
            };
            ready.store(true, Ordering::Release);
            loop {
                // An idle reader notices Stop sharing without waiting for another call.
                let call = tokio::select! {
                    _ = stream.read_u8() => break,
                    next = queue.recv() => next,
                };
                let Some((request, reply)) = call else { break };
                let exchange = async {
                    ipc::send(&mut stream, &request, ipc::MAX_REQUEST).await?;
                    ipc::receive::<Reply>(&mut stream, ipc::MAX_RESPONSE).await
                };
                match timeout(std::time::Duration::from_secs(120), exchange).await {
                    Ok(Ok(result)) => {
                        let _ = reply.send(result);
                    }
                    _ => break,
                }
            }
        });
        Ok(Self {
            code,
            deadline,
            calls,
            paired,
        })
    }
}

#[derive(Default)]
struct Server {
    pairing: Mutex<Option<Pairing>>,
}

impl Server {
    fn connect(&self) -> Reply {
        let mut state = self.pairing.lock().unwrap();
        if state.as_ref().is_none_or(|p| {
            p.calls.is_closed()
                || (!p.paired.load(Ordering::Acquire) && p.deadline <= Instant::now())
        }) {
            *state = Some(Pairing::start().map_err(|e| error("pairing_failed", e))?);
        }
        let p = state.as_ref().unwrap();
        if p.paired.load(Ordering::Acquire) {
            return Ok(json!({"state":"paired"}));
        }
        Ok(
            json!({"state":"pairing","code":p.code,"expires_in_seconds":p.deadline.saturating_duration_since(Instant::now()).as_secs(),"instruction":"In Phyro choose File → Share this tab with an assistant and enter the code."}),
        )
    }

    async fn forward(&self, request: Request) -> Reply {
        if serde_json::to_vec(&request)
            .map_err(|e| error("invalid_request", e))?
            .len()
            > ipc::MAX_REQUEST
        {
            return Err(error(
                "request_too_large",
                "Use a batch smaller than one MiB.",
            ));
        }
        let (tx, rx) = oneshot::channel();
        {
            let state = self.pairing.lock().unwrap();
            let p = state
                .as_ref()
                .filter(|p| !p.calls.is_closed() && p.paired.load(Ordering::Acquire))
                .ok_or_else(|| error("not_paired", "Call connect and share a tab in Phyro."))?;
            p.calls
                .try_send((request, tx))
                .map_err(|_| error("busy", "The bridge is busy. Retry the request."))?;
        }
        rx.await
            .map_err(|_| error("disconnected", "The editor disconnected. Pair again."))?
    }
}

fn tools() -> Vec<Tool> {
    live::schemas()
        .into_iter()
        .map(|(name, desc, schema)| Tool::new(name, desc, schema.as_object().unwrap().clone()))
        .collect()
}

impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("phyro-cli", env!("CARGO_PKG_VERSION")))
    }
    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(tools()))
    }
    async fn call_tool(
        &self,
        params: CallToolRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let args = params.arguments.unwrap_or_default();
        let reply = if params.name == "connect" && args.is_empty() {
            self.connect()
        } else {
            let request = serde_json::from_value(json!({"tool":params.name,"arguments":args}))
                .map_err(|e| ErrorData::invalid_params(e.to_string(), None))?;
            self.forward(request).await
        };
        let result = match reply {
            Ok(mut value) => {
                let png = value.as_object_mut().and_then(|map| map.remove("png"));
                let mut result = CallToolResult::structured(value);
                if let Some(Value::String(data)) = png {
                    result.content.push(ContentBlock::image(data, "image/png"));
                }
                result
            }
            Err(e) => CallToolResult::structured_error(e),
        };
        Ok(result.into())
    }
}

pub fn run(args: &[OsString]) -> ExitCode {
    if !args.is_empty() {
        eprintln!("Usage: phyro-cli mcp");
        return ExitCode::FAILURE;
    }
    let serve = || -> Result<(), Box<dyn std::error::Error>> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
            .block_on(async {
                Server::default()
                    .serve(rmcp::transport::stdio_pipes())
                    .await?
                    .waiting()
                    .await?;
                Ok(())
            })
    };
    match serve() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
