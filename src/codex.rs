//! Client for the official `codex app-server` JSONL (JSON-RPC) protocol.

use std::collections::HashMap;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, broadcast, oneshot};

use crate::runtime;

const CLIENT_NAME: &str = "writing_tools_linux";
/// JSON-RPC "method not found": the Codex CLI is too old for a method.
pub const METHOD_NOT_FOUND: i64 = -32601;

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum CodexError {
    #[error("The Codex CLI is not installed or is not available on PATH.")]
    NotInstalled,
    #[error("{message}")]
    Protocol { message: String, code: Option<i64> },
    #[error("{message}")]
    Turn { message: String, info: Option<String> },
    #[error("{0}")]
    Other(String),
}

impl CodexError {
    fn protocol(message: impl Into<String>) -> Self {
        CodexError::Protocol { message: message.into(), code: None }
    }
}

type Reader = Pin<Box<dyn AsyncRead + Send>>;
type Writer = Pin<Box<dyn AsyncWrite + Send>>;

/// A started App Server: its pipes plus whatever must live as long as it.
pub struct Transport {
    pub reader: Reader,
    pub writer: Writer,
    pub child: Option<tokio::process::Child>,
    pub runtime_dir: Option<tempfile::TempDir>,
}

type TransportFactory = Box<dyn Fn() -> Result<Transport, CodexError> + Send + Sync>;

/// A JSON-RPC notification from the server.
#[derive(Clone, Debug)]
pub struct Notification {
    pub method: String,
    pub params: Value,
}

type Pending = StdMutex<HashMap<u64, oneshot::Sender<Result<Value, CodexError>>>>;

struct Connection {
    writer: Mutex<Writer>,
    pending: Pending,
    next_id: AtomicU64,
    closed: AtomicBool,
}

impl Connection {
    async fn send(&self, message: &Value) -> Result<(), CodexError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(CodexError::Other("Codex App Server is not running.".into()));
        }
        let mut line = serde_json::to_vec(message).expect("JSON values serialise");
        line.push(b'\n');
        let mut writer = self.writer.lock().await;
        let result = async {
            writer.write_all(&line).await?;
            writer.flush().await
        }
        .await;
        result.map_err(|_| CodexError::Other("The Codex App Server connection closed.".into()))
    }

    fn fail_pending(&self, error: CodexError) {
        let pending: Vec<_> = self.pending.lock().unwrap().drain().collect();
        for (_, sender) in pending {
            let _ = sender.send(Err(error.clone()));
        }
    }
}

struct Running {
    connection: Arc<Connection>,
    child: Option<tokio::process::Child>,
    _runtime_dir: Option<tempfile::TempDir>,
    cwd: Option<String>,
}

pub struct CodexClient {
    factory: TransportFactory,
    request_timeout: Duration,
    turn_timeout: Duration,
    running: Mutex<Option<Running>>,
    notifications: broadcast::Sender<Notification>,
}

/// One turn's parameters.
pub struct TurnRequest<'a> {
    pub model: &'a str,
    pub base_instructions: &'a str,
    pub developer_instructions: &'a str,
    pub input_text: &'a str,
    pub reasoning_effort: Option<&'a str>,
    pub service_tier: Option<&'a str>,
}

pub fn executable() -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths).map(|dir| dir.join("codex")).find(|path| {
        use std::os::unix::fs::PermissionsExt;
        path.metadata().map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false)
    })
}

pub fn is_available() -> bool {
    executable().is_some()
}

impl CodexClient {
    /// A client that runs `codex app-server` with `CODEX_HOME=codex_home`.
    pub fn new(codex_home: PathBuf) -> Self {
        Self::with_factory(Box::new(move || spawn_app_server(&codex_home)))
    }

    pub fn with_factory(factory: TransportFactory) -> Self {
        Self {
            factory,
            request_timeout: Duration::from_secs(15),
            turn_timeout: Duration::from_secs(180),
            running: Mutex::new(None),
            notifications: broadcast::channel(256).0,
        }
    }

    #[cfg(test)]
    pub fn set_timeouts(&mut self, request: Duration, turn: Duration) {
        self.request_timeout = request;
        self.turn_timeout = turn;
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Notification> {
        self.notifications.subscribe()
    }

    /// Start and initialise App Server if it is not already running.
    pub async fn start(&self) -> Result<(), CodexError> {
        self.connection().await.map(|_| ())
    }

    async fn connection(&self) -> Result<(Arc<Connection>, Option<String>), CodexError> {
        let mut running = self.running.lock().await;
        if let Some(current) = running.as_ref()
            && !current.connection.closed.load(Ordering::SeqCst)
        {
            return Ok((current.connection.clone(), current.cwd.clone()));
        }
        // A previous App Server may have exited without a clean shutdown.
        if let Some(stale) = running.take() {
            terminate(stale).await;
        }

        let transport = (self.factory)()?;
        let connection = Arc::new(Connection {
            writer: Mutex::new(transport.writer),
            pending: StdMutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            closed: AtomicBool::new(false),
        });
        runtime::spawn(read_loop(transport.reader, connection.clone(), self.notifications.clone()));
        let cwd = transport.runtime_dir.as_ref().map(|dir| dir.path().to_string_lossy().into_owned());
        let started = Running {
            connection: connection.clone(),
            child: transport.child,
            _runtime_dir: transport.runtime_dir,
            cwd: cwd.clone(),
        };

        let client_info =
            json!({"name": CLIENT_NAME, "title": "Writing Tools for Linux", "version": env!("CARGO_PKG_VERSION")});
        let handshake = async {
            request_on(&connection, "initialize", Some(json!({"clientInfo": client_info})), self.request_timeout)
                .await?;
            connection.send(&json!({"method": "initialized", "params": {}})).await
        };
        if let Err(e) = handshake.await {
            terminate(started).await;
            return Err(e);
        }
        *running = Some(started);
        Ok((connection, cwd))
    }

    pub async fn request(&self, method: &str, params: Option<Value>) -> Result<Value, CodexError> {
        self.request_with_timeout(method, params, self.request_timeout).await
    }

    pub async fn request_with_timeout(
        &self,
        method: &str,
        params: Option<Value>,
        timeout: Duration,
    ) -> Result<Value, CodexError> {
        let (connection, _) = self.connection().await?;
        request_on(&connection, method, params, timeout).await
    }

    /// Run one isolated turn and return its final assistant message.
    /// `on_started` receives the thread and turn IDs once the turn exists.
    pub async fn run_turn(
        &self,
        turn: TurnRequest<'_>,
        on_started: impl FnOnce(&str, &str),
    ) -> Result<String, CodexError> {
        let (connection, cwd) = self.connection().await?;
        let mut thread_params = json!({
            "cwd": cwd,
            "approvalPolicy": "never",
            "sandbox": "read-only",
            "baseInstructions": turn.base_instructions,
            "developerInstructions": turn.developer_instructions,
            "ephemeral": true,
            "serviceName": CLIENT_NAME,
        });
        if !turn.model.is_empty() {
            thread_params["model"] = json!(turn.model);
        }
        if let Some(effort) = turn.reasoning_effort {
            thread_params["config"] = json!({"model_reasoning_effort": effort});
        }
        let thread = request_on(&connection, "thread/start", Some(thread_params), self.request_timeout).await?;
        let thread_id = thread
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .ok_or_else(|| CodexError::protocol("Codex returned an invalid thread/start response."))?
            .to_owned();

        let mut notifications = self.notifications.subscribe();
        let mut turn_params = json!({
            "threadId": thread_id,
            "input": [{"type": "text", "text": turn.input_text}],
            "approvalPolicy": "never",
            "sandboxPolicy": {"type": "readOnly", "networkAccess": false},
            "summary": "none",
        });
        if let Some(tier) = turn.service_tier {
            // An explicit "default" prevents inheriting a Fast thread tier.
            turn_params["serviceTierForTurn"] = json!(tier);
        }
        let started = request_on(&connection, "turn/start", Some(turn_params), self.request_timeout).await?;
        let turn_id = started
            .pointer("/turn/id")
            .and_then(Value::as_str)
            .ok_or_else(|| CodexError::protocol("Codex returned an invalid turn/start response."))?
            .to_owned();
        on_started(&thread_id, &turn_id);

        let completed = tokio::time::timeout(self.turn_timeout, async {
            loop {
                match notifications.recv().await {
                    Ok(n)
                        if n.method == "turn/completed"
                            && n.params.get("threadId").and_then(Value::as_str) == Some(&thread_id) =>
                    {
                        return Some(n.params);
                    }
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        })
        .await;
        let params = match completed {
            Ok(Some(params)) => params,
            Ok(None) => return Err(CodexError::Other("Codex App Server exited unexpectedly.".into())),
            Err(_) => {
                let _ = request_on(
                    &connection,
                    "turn/interrupt",
                    Some(json!({"threadId": thread_id, "turnId": turn_id})),
                    Duration::from_secs(5),
                )
                .await;
                return Err(CodexError::Turn { message: "The Codex request timed out.".into(), info: None });
            }
        };
        final_answer(&params)
    }

    pub async fn interrupt(&self, thread_id: &str, turn_id: &str) -> Result<(), CodexError> {
        self.request_with_timeout(
            "turn/interrupt",
            Some(json!({"threadId": thread_id, "turnId": turn_id})),
            Duration::from_secs(5),
        )
        .await
        .map(|_| ())
    }

    /// Stop App Server and fail any outstanding callers.
    pub async fn shutdown(&self) {
        if let Some(running) = self.running.lock().await.take() {
            terminate(running).await;
        }
    }
}

/// Pick the text of a completed turn, or the error it ended with.
pub fn final_answer(params: &Value) -> Result<String, CodexError> {
    let turn = params.get("turn").cloned().unwrap_or(Value::Null);
    let status = turn.get("status").and_then(Value::as_str);
    if status != Some("completed") {
        let error = turn.get("error").cloned().unwrap_or(Value::Null);
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| format!("The Codex turn ended with status: {}.", status.unwrap_or("None")));
        let info = error.get("codexErrorInfo").and_then(Value::as_str).map(str::to_owned);
        return Err(CodexError::Turn { message, info });
    }
    let messages: Vec<&Value> = turn
        .get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| {
            item.get("type").and_then(Value::as_str) == Some("agentMessage")
                && item.get("text").and_then(Value::as_str).is_some_and(|t| !t.is_empty())
        })
        .collect();
    let selected = messages
        .iter()
        .rev()
        .find(|item| item.get("phase").and_then(Value::as_str) == Some("final_answer"))
        .or(messages.last())
        .ok_or_else(|| CodexError::protocol("Codex completed without returning any text."))?;
    Ok(selected["text"].as_str().unwrap_or_default().trim().to_owned())
}

async fn request_on(
    connection: &Connection,
    method: &str,
    params: Option<Value>,
    timeout: Duration,
) -> Result<Value, CodexError> {
    let id = connection.next_id.fetch_add(1, Ordering::SeqCst);
    let (sender, receiver) = oneshot::channel();
    connection.pending.lock().unwrap().insert(id, sender);

    let mut message = json!({"method": method, "id": id});
    if let Some(params) = params {
        message["params"] = params;
    }
    let result = async {
        connection.send(&message).await?;
        let response = tokio::time::timeout(timeout, receiver)
            .await
            .map_err(|_| CodexError::protocol(format!("Codex timed out while handling {method}.")))?
            .map_err(|_| CodexError::Other("Codex App Server stopped.".into()))??;
        if let Some(error) = response.get("error") {
            return Err(CodexError::Protocol {
                message: error
                    .get("message")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("Codex rejected {method}.")),
                code: error.get("code").and_then(Value::as_i64),
            });
        }
        response
            .get("result")
            .cloned()
            .ok_or_else(|| CodexError::protocol(format!("Codex returned an invalid response for {method}.")))
    }
    .await;
    connection.pending.lock().unwrap().remove(&id);
    result
}

async fn read_loop(reader: Reader, connection: Arc<Connection>, notifications: broadcast::Sender<Notification>) {
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            log::warn!("Ignoring malformed output from Codex App Server");
            continue;
        };
        let method = message.get("method").and_then(Value::as_str);
        match (message.get("id"), method) {
            (Some(id), Some(method)) => {
                let response = reject_server_request(id.clone(), method);
                let _ = connection.send(&response).await;
            }
            (Some(id), None) => {
                let sender = id.as_u64().and_then(|id| connection.pending.lock().unwrap().remove(&id));
                if let Some(sender) = sender {
                    let _ = sender.send(Ok(message));
                }
            }
            (None, Some(method)) => {
                let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
                let _ = notifications.send(Notification { method: method.to_owned(), params });
            }
            (None, None) => {}
        }
    }
    // An App Server stopped through shutdown() has already failed its callers.
    if !connection.closed.swap(true, Ordering::SeqCst) {
        connection.fail_pending(CodexError::Other("Codex App Server exited unexpectedly.".into()));
    }
}

/// Deny any tool or approval request; this client only generates text.
fn reject_server_request(id: Value, method: &str) -> Value {
    let result = match method {
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => json!({"decision": "decline"}),
        "item/tool/requestUserInput" => json!({"answers": {}}),
        "mcpServer/elicitation/request" => json!({"action": "decline"}),
        "item/tool/call" => json!({"contentItems": [], "success": false}),
        _ => {
            return json!({"id": id, "error": {"code": METHOD_NOT_FOUND, "message": "Writing Tools does not provide interactive tools."}});
        }
    };
    json!({"id": id, "result": result})
}

async fn terminate(mut running: Running) {
    running.connection.closed.store(true, Ordering::SeqCst);
    if let Some(child) = running.child.as_mut() {
        let _ = child.start_kill();
        if tokio::time::timeout(Duration::from_secs(2), child.wait()).await.is_err() {
            log::warn!("Codex App Server did not exit after being killed");
        }
    }
    running.connection.fail_pending(CodexError::Other("Codex App Server stopped.".into()));
}

fn spawn_app_server(codex_home: &std::path::Path) -> Result<Transport, CodexError> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    use std::process::Stdio;

    let executable = executable().ok_or(CodexError::NotInstalled)?;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(codex_home)
        .map_err(|e| CodexError::Other(format!("Could not create the Codex data directory: {e}")))?;
    if std::fs::set_permissions(codex_home, std::fs::Permissions::from_mode(0o700)).is_err() {
        log::warn!("Could not restrict permissions on the Codex data directory");
    }
    let runtime_dir = tempfile::Builder::new()
        .prefix("writing-tools-codex-")
        .tempdir()
        .map_err(|e| CodexError::Other(format!("Could not start Codex App Server: {e}")))?;

    // Spawn inside the runtime so the child's pipes register with its reactor.
    let _guard = runtime::runtime().enter();
    let mut child = tokio::process::Command::new(executable)
        .args(["app-server", "-c", "cli_auth_credentials_store=\"file\"", "--listen", "stdio://"])
        .env("CODEX_HOME", codex_home)
        .current_dir(runtime_dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| CodexError::Other(format!("Could not start Codex App Server: {e}")))?;

    // App Server can print login URLs or other sensitive diagnostics. Drain
    // the pipe to prevent blocking, but never copy it into the log.
    if let Some(mut stderr) = child.stderr.take() {
        runtime::spawn(async move {
            let _ = tokio::io::copy(&mut stderr, &mut tokio::io::sink()).await;
        });
    }
    Ok(Transport {
        reader: Box::pin(child.stdout.take().expect("piped stdout")),
        writer: Box::pin(child.stdin.take().expect("piped stdin")),
        child: Some(child),
        runtime_dir: Some(runtime_dir),
    })
}

#[cfg(test)]
pub(crate) mod fake {
    //! An in-memory App Server for tests.

    use super::*;
    use tokio::io::{DuplexStream, Lines};

    /// The server side of a fake connection.
    pub struct Server {
        pub lines: Lines<BufReader<tokio::io::ReadHalf<DuplexStream>>>,
        pub writer: tokio::io::WriteHalf<DuplexStream>,
    }

    impl Server {
        pub async fn recv(&mut self) -> Value {
            let line = self.lines.next_line().await.unwrap().expect("client closed the connection");
            serde_json::from_str(&line).unwrap()
        }

        pub async fn send(&mut self, message: Value) {
            let mut line = serde_json::to_vec(&message).unwrap();
            line.push(b'\n');
            self.writer.write_all(&line).await.unwrap();
        }

        /// Answer the initialize handshake.
        pub async fn handshake(&mut self) {
            let init = self.recv().await;
            assert_eq!(init["method"], "initialize");
            self.send(json!({"id": init["id"], "result": {}})).await;
            assert_eq!(self.recv().await["method"], "initialized");
        }

        /// Expect a request for `method` and reply with `result`.
        pub async fn expect(&mut self, method: &str, result: Value) -> Value {
            let request = self.recv().await;
            assert_eq!(request["method"], method, "unexpected request {request}");
            self.send(json!({"id": request["id"], "result": result})).await;
            request
        }
    }

    /// A client whose every start hands a new fake server to `servers`.
    pub fn client() -> (CodexClient, tokio::sync::mpsc::UnboundedReceiver<Server>) {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let client = CodexClient::with_factory(Box::new(move || {
            let (client_end, server_end) = tokio::io::duplex(1 << 16);
            let (client_read, client_write) = tokio::io::split(client_end);
            let (server_read, server_write) = tokio::io::split(server_end);
            let _ = sender.send(Server { lines: BufReader::new(server_read).lines(), writer: server_write });
            Ok(Transport {
                reader: Box::pin(client_read),
                writer: Box::pin(client_write),
                child: None,
                runtime_dir: None,
            })
        }));
        (client, receiver)
    }
}

#[cfg(test)]
mod tests {
    use super::fake::client;
    use super::*;

    #[tokio::test]
    async fn handshake_then_request() {
        let (client, mut servers) = client();
        let call = tokio::spawn(async move {
            let result = client.request("account/read", Some(json!({"refreshToken": true}))).await;
            (client, result)
        });
        let mut server = servers.recv().await.unwrap();
        let init = server.recv().await;
        assert_eq!(init["params"]["clientInfo"]["name"], "writing_tools_linux");
        server.send(json!({"id": init["id"], "result": {}})).await;
        assert_eq!(server.recv().await, json!({"method": "initialized", "params": {}}));
        let request = server.expect("account/read", json!({"account": null})).await;
        assert_eq!(request["params"], json!({"refreshToken": true}));
        let (client, result) = call.await.unwrap();
        assert_eq!(result.unwrap(), json!({"account": null}));

        // The running server is reused.
        let second = tokio::spawn(async move { client.request("model/list", None).await });
        server.expect("model/list", json!({"data": []})).await;
        assert_eq!(second.await.unwrap().unwrap(), json!({"data": []}));
        assert!(servers.try_recv().is_err());
    }

    #[tokio::test]
    async fn error_responses_carry_the_code() {
        let (client, mut servers) = client();
        let call = tokio::spawn(async move { client.request("account/login/start", None).await });
        let mut server = servers.recv().await.unwrap();
        server.handshake().await;
        let request = server.recv().await;
        server.send(json!({"id": request["id"], "error": {"code": -32601, "message": "unknown method"}})).await;
        assert_eq!(
            call.await.unwrap(),
            Err(CodexError::Protocol { message: "unknown method".into(), code: Some(METHOD_NOT_FOUND) })
        );
    }

    #[tokio::test]
    async fn server_requests_are_declined() {
        let (client, mut servers) = client();
        let call = tokio::spawn(async move { client.request("x", None).await });
        let mut server = servers.recv().await.unwrap();
        server.handshake().await;
        let pending = server.recv().await;
        server.send(json!({"id": 99, "method": "item/commandExecution/requestApproval", "params": {}})).await;
        assert_eq!(server.recv().await, json!({"id": 99, "result": {"decision": "decline"}}));
        server.send(json!({"id": 100, "method": "something/else"})).await;
        let reply = server.recv().await;
        assert_eq!(reply["error"]["code"], METHOD_NOT_FOUND);
        server.send(json!({"id": pending["id"], "result": 1})).await;
        assert_eq!(call.await.unwrap(), Ok(json!(1)));
    }

    #[tokio::test]
    async fn exit_fails_pending_requests_and_restarts() {
        let (client, mut servers) = client();
        let client = Arc::new(client);
        let call = tokio::spawn({
            let client = client.clone();
            async move { client.request("x", None).await }
        });
        let mut server = servers.recv().await.unwrap();
        server.handshake().await;
        server.recv().await;
        drop(server);
        assert_eq!(call.await.unwrap(), Err(CodexError::Other("Codex App Server exited unexpectedly.".into())));

        let call = tokio::spawn({
            let client = client.clone();
            async move { client.request("y", None).await }
        });
        let mut server = servers.recv().await.unwrap();
        server.handshake().await;
        server.expect("y", json!(true)).await;
        assert_eq!(call.await.unwrap(), Ok(json!(true)));
    }

    #[tokio::test]
    async fn runs_a_turn() {
        let (client, mut servers) = client();
        let turn = tokio::spawn(async move {
            let mut ids = None;
            let result = client
                .run_turn(
                    TurnRequest {
                        model: "gpt-x",
                        base_instructions: "base",
                        developer_instructions: "dev",
                        input_text: "hello",
                        reasoning_effort: Some("low"),
                        service_tier: Some("default"),
                    },
                    |thread, turn| ids = Some((thread.to_owned(), turn.to_owned())),
                )
                .await;
            (result, ids)
        });
        let mut server = servers.recv().await.unwrap();
        server.handshake().await;
        let thread = server.expect("thread/start", json!({"thread": {"id": "t1"}})).await;
        assert_eq!(thread["params"]["model"], "gpt-x");
        assert_eq!(thread["params"]["config"], json!({"model_reasoning_effort": "low"}));
        assert_eq!(thread["params"]["ephemeral"], true);
        let turn_start = server.expect("turn/start", json!({"turn": {"id": "u1"}})).await;
        assert_eq!(turn_start["params"]["serviceTierForTurn"], "default");
        assert_eq!(turn_start["params"]["input"], json!([{"type": "text", "text": "hello"}]));
        // A completion for another thread is ignored.
        server
            .send(json!({"method": "turn/completed", "params": {"threadId": "other", "turn": {"status": "failed"}}}))
            .await;
        server
            .send(json!({"method": "turn/completed", "params": {"threadId": "t1", "turn": {"status": "completed", "items": [
                {"type": "agentMessage", "text": "draft", "phase": "commentary"},
                {"type": "agentMessage", "text": " final \n", "phase": "final_answer"},
                {"type": "agentMessage", "text": "after"}
            ]}}}))
            .await;
        let (result, ids) = turn.await.unwrap();
        assert_eq!(result, Ok("final".into()));
        assert_eq!(ids, Some(("t1".into(), "u1".into())));
    }

    #[tokio::test]
    async fn turn_timeout_interrupts() {
        let (mut client, mut servers) = client();
        client.set_timeouts(Duration::from_secs(5), Duration::from_millis(50));
        let turn = tokio::spawn(async move {
            client
                .run_turn(
                    TurnRequest {
                        model: "",
                        base_instructions: "",
                        developer_instructions: "",
                        input_text: "",
                        reasoning_effort: None,
                        service_tier: None,
                    },
                    |_, _| {},
                )
                .await
        });
        let mut server = servers.recv().await.unwrap();
        server.handshake().await;
        let thread = server.expect("thread/start", json!({"thread": {"id": "t"}})).await;
        assert!(thread["params"].get("model").is_none());
        let turn_start = server.expect("turn/start", json!({"turn": {"id": "u"}})).await;
        assert!(turn_start["params"].get("serviceTierForTurn").is_none());
        let interrupt = server.expect("turn/interrupt", json!({})).await;
        assert_eq!(interrupt["params"], json!({"threadId": "t", "turnId": "u"}));
        assert_eq!(
            turn.await.unwrap(),
            Err(CodexError::Turn { message: "The Codex request timed out.".into(), info: None })
        );
    }

    #[test]
    fn final_answer_selection_and_errors() {
        let only_commentary = json!({"turn": {"status": "completed", "items": [
            {"type": "agentMessage", "text": "a"}, {"type": "reasoning", "text": "x"}, {"type": "agentMessage", "text": "b"}
        ]}});
        assert_eq!(final_answer(&only_commentary), Ok("b".into()));
        let failed = json!({"turn": {"status": "failed", "error": {"message": "limit", "codexErrorInfo": "usageLimitExceeded"}}});
        assert_eq!(
            final_answer(&failed),
            Err(CodexError::Turn { message: "limit".into(), info: Some("usageLimitExceeded".into()) })
        );
        let interrupted = json!({"turn": {"status": "interrupted"}});
        assert_eq!(
            final_answer(&interrupted),
            Err(CodexError::Turn { message: "The Codex turn ended with status: interrupted.".into(), info: None })
        );
        let empty = json!({"turn": {"status": "completed", "items": []}});
        assert!(matches!(final_answer(&empty), Err(CodexError::Protocol { .. })));
    }
}
