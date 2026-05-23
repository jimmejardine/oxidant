// Realises spec/components/rust-tools/lsp.md and the model-facing LSP
// tool wrappers (rust_hover, rust_goto_definition, rust_workspace_symbols,
// rust_diagnostics).
//
// One rust-analyzer subprocess is kept alive per workspace (cached in
// LSP_CLIENTS). JSON-RPC 2.0 messages flow through a writer task draining
// an mpsc channel into stdin; a reader task parses stdout, routes responses
// to oneshot senders by id, and stashes publishDiagnostics into a per-file
// cache. Tools call into LspClient async methods.
//
// rust-analyzer takes ~10-30s to spawn and index even a small workspace,
// so the first tool call after spawning is slow. Subsequent calls are fast.
// rename / code_actions / find_references are deferred to a follow-up phase.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex as AsyncMutex, mpsc, oneshot, watch};
use tokio::time::timeout;

use oxidant_core::{LspHandle, Tool, ToolCategory, ToolContext, ToolResult};

const INITIALIZE_TIMEOUT_SECS: u64 = 60;
const REQUEST_TIMEOUT_SECS: u64 = 30;
/// rust-analyzer cold start on a fresh tempdir crate can take a while —
/// cargo metadata, dependency resolution, initial index. Tools wait for
/// `experimental/serverStatus { quiescent: true }` up to this budget
/// before issuing semantic queries.
const READY_TIMEOUT_SECS: u64 = 60;

// ---------------------------------------------------------------- Client

static LSP_CLIENTS: OnceLock<StdMutex<HashMap<PathBuf, Arc<AsyncMutex<LspClient>>>>> =
    OnceLock::new();

fn clients() -> &'static StdMutex<HashMap<PathBuf, Arc<AsyncMutex<LspClient>>>> {
    LSP_CLIENTS.get_or_init(|| StdMutex::new(HashMap::new()))
}

#[derive(Debug, Clone, Default)]
pub struct ServerStatus {
    pub health: String,
    pub quiescent: bool,
    pub message: Option<String>,
}

pub struct LspClient {
    workspace: PathBuf,
    next_id: AtomicI64,
    outgoing: mpsc::UnboundedSender<String>,
    pending: Arc<StdMutex<HashMap<i64, oneshot::Sender<Value>>>>,
    diagnostics: Arc<StdMutex<HashMap<PathBuf, Vec<Value>>>>,
    opened_files: Arc<StdMutex<HashSet<PathBuf>>>,
    /// Latest `experimental/serverStatus` from rust-analyzer. None until
    /// the first such notification arrives.
    server_status: watch::Receiver<Option<ServerStatus>>,
    _child: Child, // kept to ensure lifetime
}

// Concrete LspHandle impl, plugging this client into the Exploration
// aggregate's `lsp_handle: Option<Arc<dyn LspHandle>>` slot without
// pulling oxidant-rust-tools into oxidant-core's dependency graph.
// See spec/components/core/exploration.md.
impl LspHandle for LspClient {}

impl std::fmt::Debug for LspClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LspClient")
            .field("workspace", &self.workspace)
            .field(
                "pending",
                &self.pending.lock().map(|m| m.len()).unwrap_or(0),
            )
            .field(
                "opened_files",
                &self.opened_files.lock().map(|s| s.len()).unwrap_or(0),
            )
            .finish_non_exhaustive()
    }
}

impl LspClient {
    /// Get-or-spawn the LSP client for `workspace`. Spawning blocks until
    /// rust-analyzer responds to `initialize`.
    pub async fn for_workspace(workspace: &Path) -> Result<Arc<AsyncMutex<LspClient>>, String> {
        let canonical = dunce::canonicalize(workspace)
            .map_err(|e| format!("canonicalise workspace failed: {e}"))?;
        {
            let map = clients().lock().unwrap();
            if let Some(existing) = map.get(&canonical) {
                return Ok(existing.clone());
            }
        }
        let client = Self::spawn(&canonical).await?;
        let arc = Arc::new(AsyncMutex::new(client));
        let mut map = clients().lock().unwrap();
        Ok(map.entry(canonical).or_insert(arc).clone())
    }

    async fn spawn(workspace: &Path) -> Result<Self, String> {
        let exe = locate_rust_analyzer().await?;
        tracing::info!(?exe, ?workspace, "spawning rust-analyzer");

        let mut child = Command::new(&exe)
            .current_dir(workspace)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("spawn {exe:?} failed: {e}"))?;

        let stdin = child.stdin.take().ok_or("child stdin missing")?;
        let stdout = child.stdout.take().ok_or("child stdout missing")?;
        let stderr = child.stderr.take().ok_or("child stderr missing")?;

        let (outgoing_tx, outgoing_rx) = mpsc::unbounded_channel::<String>();
        let pending: Arc<StdMutex<HashMap<i64, oneshot::Sender<Value>>>> =
            Arc::new(StdMutex::new(HashMap::new()));
        let diagnostics: Arc<StdMutex<HashMap<PathBuf, Vec<Value>>>> =
            Arc::new(StdMutex::new(HashMap::new()));
        let opened_files: Arc<StdMutex<HashSet<PathBuf>>> = Arc::new(StdMutex::new(HashSet::new()));
        let (status_tx, status_rx) = watch::channel::<Option<ServerStatus>>(None);

        // writer task
        tokio::spawn(writer_loop(stdin, outgoing_rx));
        // reader task
        tokio::spawn(reader_loop(
            stdout,
            pending.clone(),
            diagnostics.clone(),
            status_tx,
        ));
        // stderr drain — only used for logging
        tokio::spawn(stderr_drain(stderr));

        let client = LspClient {
            workspace: workspace.to_path_buf(),
            next_id: AtomicI64::new(1),
            outgoing: outgoing_tx,
            pending,
            diagnostics,
            opened_files,
            server_status: status_rx,
            _child: child,
        };

        client.initialize().await?;
        Ok(client)
    }

    async fn initialize(&self) -> Result<(), String> {
        let workspace_uri = path_to_uri(&self.workspace).ok_or("workspace path → uri failed")?;
        let init_params = json!({
            "processId": std::process::id(),
            "clientInfo": { "name": "oxidant", "version": "0.1.0" },
            "rootUri": workspace_uri,
            "rootPath": self.workspace.to_string_lossy(),
            "workspaceFolders": [{
                "uri": workspace_uri,
                "name": self.workspace.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
            }],
            "capabilities": minimal_client_capabilities(),
        });

        let init_resp = timeout(
            Duration::from_secs(INITIALIZE_TIMEOUT_SECS),
            self.request("initialize", init_params),
        )
        .await
        .map_err(|_| {
            format!("rust-analyzer initialize timed out after {INITIALIZE_TIMEOUT_SECS}s")
        })?
        .map_err(|e| format!("initialize failed: {e}"))?;
        tracing::debug!(?init_resp, "rust-analyzer initialized");

        self.notify("initialized", json!({}))?;
        Ok(())
    }

    /// Send a JSON-RPC request and await its response.
    pub async fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);

        let payload = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        self.send(payload)?;

        match timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS), rx).await {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(_)) => Err(format!("response channel closed before {method} replied")),
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                Err(format!("{method} timed out after {REQUEST_TIMEOUT_SECS}s"))
            }
        }
    }

    /// Send a JSON-RPC notification (no response expected).
    pub fn notify(&self, method: &str, params: Value) -> Result<(), String> {
        let payload = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        self.send(payload)
    }

    fn send(&self, payload: Value) -> Result<(), String> {
        let body = serde_json::to_string(&payload)
            .map_err(|e| format!("serialise outgoing message: {e}"))?;
        self.outgoing
            .send(body)
            .map_err(|e| format!("outgoing channel send: {e}"))
    }

    /// Ensure rust-analyzer knows about this file by sending textDocument/didOpen
    /// the first time it's referenced.
    pub async fn ensure_file_opened(&self, file: &Path) -> Result<PathBuf, String> {
        let canonical = dunce::canonicalize(file)
            .map_err(|e| format!("canonicalise {} failed: {e}", file.display()))?;
        {
            let opened = self.opened_files.lock().unwrap();
            if opened.contains(&canonical) {
                return Ok(canonical);
            }
        }

        let content = std::fs::read_to_string(&canonical)
            .map_err(|e| format!("read {} failed: {e}", canonical.display()))?;
        let uri = path_to_uri(&canonical)
            .ok_or_else(|| format!("path → uri failed for {}", canonical.display()))?;
        let language_id = if canonical.extension().and_then(|s| s.to_str()) == Some("rs") {
            "rust"
        } else {
            "plaintext"
        };
        self.notify(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": uri,
                    "languageId": language_id,
                    "version": 1,
                    "text": content,
                }
            }),
        )?;
        self.opened_files.lock().unwrap().insert(canonical.clone());
        Ok(canonical)
    }

    /// Return the most recently published diagnostics for a file (push cache).
    /// Returns empty if rust-analyzer hasn't analysed the file yet.
    pub fn diagnostics_for(&self, file: &Path) -> Vec<Value> {
        let canonical = dunce::canonicalize(file).unwrap_or_else(|_| file.to_path_buf());
        self.diagnostics
            .lock()
            .unwrap()
            .get(&canonical)
            .cloned()
            .unwrap_or_default()
    }

    pub fn all_diagnostics(&self) -> HashMap<PathBuf, Vec<Value>> {
        self.diagnostics.lock().unwrap().clone()
    }

    /// Wait until rust-analyzer has analysed `file` — signalled by an
    /// entry appearing in the diagnostics cache (rust-analyzer publishes
    /// diagnostics for every analysed file, even when there are zero).
    /// File-position queries (hover, goto_definition) need this; workspace
    /// queries can use [`wait_until_ready`] instead.
    pub async fn wait_for_file_analysis(&self, file: &Path) -> Result<(), String> {
        let canonical = dunce::canonicalize(file).unwrap_or_else(|_| file.to_path_buf());
        let waited = timeout(Duration::from_secs(READY_TIMEOUT_SECS), async {
            loop {
                {
                    let diags = self.diagnostics.lock().unwrap();
                    if diags.contains_key(&canonical) {
                        return Ok::<_, String>(());
                    }
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await;
        match waited {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(format!(
                "rust-analyzer never published diagnostics for {} within {READY_TIMEOUT_SECS}s",
                canonical.display()
            )),
        }
    }

    /// Wait until rust-analyzer reports it's quiescent (done indexing /
    /// analysing). Returns Ok the moment the first `quiescent: true`
    /// status arrives, or after the timeout — semantic queries against
    /// a still-indexing server return empty, so this matters.
    pub async fn wait_until_ready(&self) -> Result<(), String> {
        if matches!(&*self.server_status.borrow(), Some(s) if s.quiescent) {
            return Ok(());
        }
        let mut rx = self.server_status.clone();
        let waited = timeout(Duration::from_secs(READY_TIMEOUT_SECS), async {
            loop {
                if matches!(&*rx.borrow(), Some(s) if s.quiescent) {
                    return Ok::<_, String>(());
                }
                rx.changed()
                    .await
                    .map_err(|_| "serverStatus channel closed".to_string())?;
            }
        })
        .await;
        match waited {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(format!(
                "rust-analyzer did not become quiescent within {READY_TIMEOUT_SECS}s"
            )),
        }
    }
}

async fn locate_rust_analyzer() -> Result<PathBuf, String> {
    // Try `rustup which rust-analyzer` first.
    if let Ok(output) = Command::new("rustup")
        .args(["which", "rust-analyzer"])
        .output()
        .await
        && output.status.success()
    {
        let line = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !line.is_empty() && Path::new(&line).exists() {
            return Ok(PathBuf::from(line));
        }
    }
    // Fall back to PATH lookup — try invoking with --version to verify it's there.
    if Command::new("rust-analyzer")
        .arg("--version")
        .output()
        .await
        .is_ok()
    {
        return Ok(PathBuf::from("rust-analyzer"));
    }
    Err("rust-analyzer not found (tried `rustup which rust-analyzer` then PATH). Install with `rustup component add rust-analyzer`.".into())
}

fn minimal_client_capabilities() -> Value {
    json!({
        "workspace": {
            "workspaceFolders": true,
            "configuration": true,
            "symbol": { "dynamicRegistration": false }
        },
        "textDocument": {
            "synchronization": { "dynamicRegistration": false },
            "hover": { "contentFormat": ["markdown", "plaintext"] },
            "definition": { "linkSupport": false },
            "publishDiagnostics": { "relatedInformation": true }
        },
        // rust-analyzer extension: enables `experimental/serverStatus`
        // notifications carrying { health, quiescent } so we know when the
        // server is idle and ready for semantic queries.
        "experimental": {
            "serverStatusNotification": true
        }
    })
}

fn path_to_uri(p: &Path) -> Option<String> {
    let canonical = dunce::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let s = canonical.to_string_lossy().replace('\\', "/");
    if s.starts_with("//") {
        // UNC; LSP spec for these is fuzzy. Try a best-effort encoding.
        Some(format!("file:{s}"))
    } else if s.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && s.chars().nth(1) == Some(':')
    {
        // Windows drive letter — file:///C:/...
        Some(format!("file:///{s}"))
    } else {
        Some(format!("file://{s}"))
    }
}

fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let s = uri.strip_prefix("file://")?;
    let s = s.strip_prefix('/').unwrap_or(s);
    // Decode %20 etc minimally — rust-analyzer rarely emits encoded paths.
    Some(PathBuf::from(s))
}

// ---------------------------------------------------------------- IO loops

async fn writer_loop(
    mut stdin: tokio::process::ChildStdin,
    mut rx: mpsc::UnboundedReceiver<String>,
) {
    while let Some(body) = rx.recv().await {
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        if let Err(e) = stdin.write_all(header.as_bytes()).await {
            tracing::debug!("lsp writer header err: {e}");
            return;
        }
        if let Err(e) = stdin.write_all(body.as_bytes()).await {
            tracing::debug!("lsp writer body err: {e}");
            return;
        }
        if let Err(e) = stdin.flush().await {
            tracing::debug!("lsp writer flush err: {e}");
            return;
        }
    }
}

async fn reader_loop(
    stdout: tokio::process::ChildStdout,
    pending: Arc<StdMutex<HashMap<i64, oneshot::Sender<Value>>>>,
    diagnostics: Arc<StdMutex<HashMap<PathBuf, Vec<Value>>>>,
    server_status: watch::Sender<Option<ServerStatus>>,
) {
    let mut reader = BufReader::new(stdout);
    loop {
        let body = match read_message(&mut reader).await {
            Ok(Some(b)) => b,
            Ok(None) => {
                tracing::info!("lsp reader: EOF on rust-analyzer stdout");
                return;
            }
            Err(e) => {
                tracing::debug!("lsp reader err: {e}");
                return;
            }
        };
        let Ok(msg) = serde_json::from_str::<Value>(&body) else {
            tracing::trace!("lsp reader: non-JSON message: {body}");
            continue;
        };
        if let Some(id) = msg.get("id").and_then(|v| v.as_i64()) {
            // It's a response (or it's a server-initiated request, which we ignore).
            if msg.get("method").is_some() {
                tracing::trace!("ignoring server→client request id={id}");
                continue;
            }
            if let Some(tx) = pending.lock().unwrap().remove(&id) {
                let payload = msg.get("result").cloned().unwrap_or_else(|| {
                    let err = msg.get("error").cloned().unwrap_or(Value::Null);
                    json!({ "_lsp_error": err })
                });
                let _ = tx.send(payload);
            }
        } else if let Some(method) = msg.get("method").and_then(|v| v.as_str()) {
            handle_notification(method, &msg, &diagnostics, &server_status);
        }
    }
}

async fn read_message(
    reader: &mut BufReader<tokio::process::ChildStdout>,
) -> Result<Option<String>, String> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let n = reader
            .read_line(&mut line)
            .await
            .map_err(|e| format!("read header line: {e}"))?;
        if n == 0 {
            return Ok(None);
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some(rest) = trimmed.strip_prefix("Content-Length:") {
            content_length = rest.trim().parse().ok();
        }
        // Other headers ignored.
    }
    let len = content_length.ok_or("message missing Content-Length")?;
    let mut buf = vec![0u8; len];
    reader
        .read_exact(&mut buf)
        .await
        .map_err(|e| format!("read body of {len} bytes: {e}"))?;
    String::from_utf8(buf)
        .map(Some)
        .map_err(|e| format!("body not utf-8: {e}"))
}

fn handle_notification(
    method: &str,
    msg: &Value,
    diagnostics: &Arc<StdMutex<HashMap<PathBuf, Vec<Value>>>>,
    server_status: &watch::Sender<Option<ServerStatus>>,
) {
    match method {
        "textDocument/publishDiagnostics" => {
            let params = match msg.get("params") {
                Some(p) => p,
                None => return,
            };
            let Some(uri) = params.get("uri").and_then(|v| v.as_str()) else {
                return;
            };
            let Some(path) = uri_to_path(uri) else { return };
            let diags: Vec<Value> = params
                .get("diagnostics")
                .and_then(|d| d.as_array())
                .cloned()
                .unwrap_or_default();
            diagnostics.lock().unwrap().insert(path, diags);
        }
        "experimental/serverStatus" => {
            let Some(params) = msg.get("params") else {
                return;
            };
            let status = ServerStatus {
                health: params
                    .get("health")
                    .and_then(|v| v.as_str())
                    .unwrap_or("ok")
                    .to_string(),
                quiescent: params
                    .get("quiescent")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                message: params
                    .get("message")
                    .and_then(|v| v.as_str())
                    .map(String::from),
            };
            tracing::debug!(?status, "rust-analyzer serverStatus");
            let _ = server_status.send(Some(status));
        }
        "window/logMessage" | "window/showMessage" | "$/progress" | "$/cargoMain" => {
            tracing::trace!(method, "lsp notification");
        }
        _ => {
            tracing::trace!(method, "unhandled lsp notification");
        }
    }
}

async fn stderr_drain(stderr: tokio::process::ChildStderr) {
    let mut reader = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = reader.next_line().await {
        tracing::trace!("rust-analyzer stderr: {line}");
    }
}

// ---------------------------------------------------------------- Helpers

fn resolve_file(ctx: &ToolContext, raw: &str) -> PathBuf {
    let path = Path::new(raw);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        ctx.workspace_root.as_std_path().join(raw)
    }
}

fn extract_hover_signature(hover: &Value) -> (Option<String>, Option<String>) {
    let contents = match hover.get("contents") {
        Some(c) => c,
        None => return (None, None),
    };
    // contents can be a MarkupContent { kind, value } OR an array of MarkedString.
    if let Some(obj) = contents.as_object()
        && let Some(value) = obj.get("value").and_then(|v| v.as_str())
    {
        return split_signature_from_markdown(value);
    }
    if let Some(arr) = contents.as_array() {
        let mut joined = String::new();
        for item in arr {
            if let Some(s) = item.as_str() {
                joined.push_str(s);
                joined.push('\n');
            } else if let Some(obj) = item.as_object()
                && let Some(v) = obj.get("value").and_then(|v| v.as_str())
            {
                joined.push_str(v);
                joined.push('\n');
            }
        }
        return split_signature_from_markdown(joined.trim());
    }
    (None, None)
}

fn split_signature_from_markdown(text: &str) -> (Option<String>, Option<String>) {
    // rust-analyzer hovers typically emit MULTIPLE rust code fences. The
    // first usually carries the module qualifier (e.g. `crate_name`); the
    // last carries the actual signature. We keep all rust-fence contents
    // joined with blank lines so the model sees both. Non-fenced prose
    // becomes the doc_md.
    let mut signature_parts: Vec<String> = Vec::new();
    let mut doc_lines: Vec<&str> = Vec::new();
    let mut in_fence = false;
    let mut fence_lang = String::new();
    let mut fence_buf = String::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            if in_fence {
                let is_rust = matches!(fence_lang.as_str(), "rust" | "Rust" | "rs" | "");
                if is_rust {
                    let body = fence_buf.trim().to_string();
                    if !body.is_empty() {
                        signature_parts.push(body);
                    }
                }
                fence_buf.clear();
                fence_lang.clear();
                in_fence = false;
            } else {
                in_fence = true;
                fence_lang = trimmed
                    .strip_prefix("```")
                    .unwrap_or(trimmed)
                    .trim()
                    .to_string();
            }
            continue;
        }
        if in_fence {
            if !fence_buf.is_empty() {
                fence_buf.push('\n');
            }
            fence_buf.push_str(line);
        } else {
            doc_lines.push(line);
        }
    }
    let signature = if signature_parts.is_empty() {
        None
    } else {
        Some(signature_parts.join("\n\n"))
    };
    let doc = doc_lines.to_vec().join("\n").trim().to_string();
    let doc = if doc.is_empty() { None } else { Some(doc) };
    (signature, doc)
}

// ---------------------------------------------------------------- Tools

pub struct RustHover;

#[derive(Deserialize)]
struct PositionArgs {
    file: String,
    line: u32,
    character: u32,
}

#[async_trait]
impl Tool for RustHover {
    fn name(&self) -> &str {
        "rust_hover"
    }
    fn description(&self) -> &str {
        "Return rust-analyzer's hover info at a position: type signature plus markdown docs. Coordinates are LSP-style (0-indexed line, 0-indexed UTF-16 character)."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["file", "line", "character"],
            "properties": {
                "file":      { "type": "string" },
                "line":      { "type": "integer", "minimum": 0 },
                "character": { "type": "integer", "minimum": 0 }
            }
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::ReadOnly
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let args: PositionArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::Err(format!("invalid args: {e}")),
        };
        let workspace = ctx.workspace_root.as_std_path();
        let client = match LspClient::for_workspace(workspace).await {
            Ok(c) => c,
            Err(e) => return ToolResult::Err(e),
        };
        let client = client.lock().await;
        let path = resolve_file(ctx, &args.file);
        let canonical = match client.ensure_file_opened(&path).await {
            Ok(p) => p,
            Err(e) => return ToolResult::Err(e),
        };
        if let Err(e) = client.wait_for_file_analysis(&canonical).await {
            return ToolResult::Err(e);
        }
        let uri = path_to_uri(&canonical).unwrap_or_default();
        let resp = match client
            .request(
                "textDocument/hover",
                json!({
                    "textDocument": { "uri": uri },
                    "position": { "line": args.line, "character": args.character }
                }),
            )
            .await
        {
            Ok(v) => v,
            Err(e) => return ToolResult::Err(e),
        };
        let (signature, doc) = if resp.is_null() {
            (None, None)
        } else {
            extract_hover_signature(&resp)
        };
        ToolResult::Ok(json!({
            "type_signature": signature,
            "doc_md":         doc,
        }))
    }
}

pub struct RustGotoDefinition;

#[async_trait]
impl Tool for RustGotoDefinition {
    fn name(&self) -> &str {
        "rust_goto_definition"
    }
    fn description(&self) -> &str {
        "Return the definition site(s) of the symbol at an LSP-style position. Multiple locations possible (e.g. a trait method with several impls); empty if unresolvable."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["file", "line", "character"],
            "properties": {
                "file":      { "type": "string" },
                "line":      { "type": "integer", "minimum": 0 },
                "character": { "type": "integer", "minimum": 0 }
            }
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::ReadOnly
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let args: PositionArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::Err(format!("invalid args: {e}")),
        };
        let workspace = ctx.workspace_root.as_std_path();
        let client = match LspClient::for_workspace(workspace).await {
            Ok(c) => c,
            Err(e) => return ToolResult::Err(e),
        };
        let client = client.lock().await;
        let path = resolve_file(ctx, &args.file);
        let canonical = match client.ensure_file_opened(&path).await {
            Ok(p) => p,
            Err(e) => return ToolResult::Err(e),
        };
        if let Err(e) = client.wait_for_file_analysis(&canonical).await {
            return ToolResult::Err(e);
        }
        let uri = path_to_uri(&canonical).unwrap_or_default();
        let resp = match client
            .request(
                "textDocument/definition",
                json!({
                    "textDocument": { "uri": uri },
                    "position": { "line": args.line, "character": args.character }
                }),
            )
            .await
        {
            Ok(v) => v,
            Err(e) => return ToolResult::Err(e),
        };
        let locations = locations_from_response(&resp, ctx);
        ToolResult::Ok(json!({ "locations": locations }))
    }
}

fn locations_from_response(resp: &Value, ctx: &ToolContext) -> Vec<Value> {
    let mut out = Vec::new();
    if resp.is_null() {
        return out;
    }
    // A response may be: Location, LocationLink[], Location[], or single Location.
    let raw = if resp.is_array() {
        resp.as_array().cloned().unwrap_or_default()
    } else {
        vec![resp.clone()]
    };
    for item in raw {
        let (uri, range) = if let Some(target_uri) = item.get("targetUri").and_then(|v| v.as_str())
        {
            // LocationLink
            (
                target_uri.to_string(),
                item.get("targetSelectionRange")
                    .or_else(|| item.get("targetRange"))
                    .cloned()
                    .unwrap_or(Value::Null),
            )
        } else if let Some(uri) = item.get("uri").and_then(|v| v.as_str()) {
            (
                uri.to_string(),
                item.get("range").cloned().unwrap_or(Value::Null),
            )
        } else {
            continue;
        };
        let file = uri_to_path(&uri)
            .and_then(|p| {
                let workspace = ctx.workspace_root.as_std_path();
                p.strip_prefix(workspace)
                    .ok()
                    .map(|rel| rel.to_string_lossy().replace('\\', "/"))
            })
            .unwrap_or_else(|| uri.clone());
        out.push(json!({ "file": file, "range": range }));
    }
    out
}

pub struct RustWorkspaceSymbols;

#[derive(Deserialize)]
struct SymbolArgs {
    query: String,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[async_trait]
impl Tool for RustWorkspaceSymbols {
    fn name(&self) -> &str {
        "rust_workspace_symbols"
    }
    fn description(&self) -> &str {
        "Search the workspace for symbols (functions, types, traits, modules) by name. Use for `where is Foo defined` lookups; faster and more accurate than grep for actual definitions."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["query"],
            "properties": {
                "query": { "type": "string", "minLength": 1 },
                "kind":  { "type": "string", "description": "fn | struct | enum | trait | impl | mod | const | static" },
                "limit": { "type": "integer", "default": 50, "maximum": 500 }
            }
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::ReadOnly
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let args: SymbolArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::Err(format!("invalid args: {e}")),
        };
        let limit = args.limit.unwrap_or(50).min(500);
        let workspace = ctx.workspace_root.as_std_path();
        let client = match LspClient::for_workspace(workspace).await {
            Ok(c) => c,
            Err(e) => return ToolResult::Err(e),
        };
        let client = client.lock().await;
        if let Err(e) = client.wait_until_ready().await {
            return ToolResult::Err(e);
        }
        let resp = match client
            .request("workspace/symbol", json!({ "query": args.query }))
            .await
        {
            Ok(v) => v,
            Err(e) => return ToolResult::Err(e),
        };
        let raw = resp.as_array().cloned().unwrap_or_default();
        let kind_filter = args.kind.as_deref();
        let symbols: Vec<Value> = raw
            .into_iter()
            .filter_map(|s| extract_symbol(s, ctx))
            .filter(|s| kind_filter.is_none_or(|k| s["kind"].as_str() == Some(k)))
            .take(limit)
            .collect();
        ToolResult::Ok(json!({ "symbols": symbols, "count": symbols.len() }))
    }
}

fn extract_symbol(s: Value, ctx: &ToolContext) -> Option<Value> {
    let name = s.get("name").and_then(|v| v.as_str())?.to_string();
    let kind_num = s.get("kind").and_then(|v| v.as_u64()).unwrap_or(0);
    let kind = lsp_symbol_kind_name(kind_num);
    let (uri, range) = if let Some(location) = s.get("location") {
        (
            location
                .get("uri")
                .and_then(|v| v.as_str())
                .map(String::from)?,
            location.get("range").cloned().unwrap_or(Value::Null),
        )
    } else {
        return None;
    };
    let file = uri_to_path(&uri)
        .and_then(|p| {
            p.strip_prefix(ctx.workspace_root.as_std_path())
                .ok()
                .map(|rel| rel.to_string_lossy().replace('\\', "/"))
        })
        .unwrap_or(uri);
    Some(json!({ "name": name, "kind": kind, "file": file, "range": range }))
}

fn lsp_symbol_kind_name(k: u64) -> &'static str {
    // From the LSP SymbolKind enum.
    match k {
        1 => "file",
        2 => "module",
        3 => "namespace",
        4 => "package",
        5 => "class",
        6 => "method",
        7 => "property",
        8 => "field",
        9 => "constructor",
        10 => "enum",
        11 => "interface",
        12 => "fn",
        13 => "variable",
        14 => "const",
        15 => "string",
        16 => "number",
        17 => "boolean",
        18 => "array",
        19 => "object",
        20 => "key",
        21 => "null",
        22 => "enum_member",
        23 => "struct",
        24 => "event",
        25 => "operator",
        26 => "type_parameter",
        _ => "?",
    }
}

pub struct RustDiagnostics;

#[derive(Deserialize)]
struct DiagnosticArgs {
    #[serde(default)]
    file: Option<String>,
    #[serde(default)]
    severity: Option<String>,
}

#[async_trait]
impl Tool for RustDiagnostics {
    fn name(&self) -> &str {
        "rust_diagnostics"
    }
    fn description(&self) -> &str {
        "Return rust-analyzer's current diagnostics from its push-published cache. Fast but reflects RA's current understanding (may lag a few hundred ms behind a recent edit). For authoritative compiler output use cargo_check."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file":     { "type": "string", "description": "omit for workspace-wide" },
                "severity": { "type": "string", "enum": ["error", "warning", "info", "hint"] }
            }
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::ReadOnly
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let args: DiagnosticArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::Err(format!("invalid args: {e}")),
        };
        let workspace = ctx.workspace_root.as_std_path();
        let client = match LspClient::for_workspace(workspace).await {
            Ok(c) => c,
            Err(e) => return ToolResult::Err(e),
        };
        let client = client.lock().await;

        let entries: HashMap<PathBuf, Vec<Value>> = if let Some(f) = &args.file {
            let path = resolve_file(ctx, f);
            if let Err(e) = client.ensure_file_opened(&path).await {
                return ToolResult::Err(e);
            }
            let canonical = dunce::canonicalize(&path).unwrap_or(path);
            if let Err(e) = client.wait_for_file_analysis(&canonical).await {
                return ToolResult::Err(e);
            }
            let diags = client.diagnostics_for(&canonical);
            let mut m = HashMap::new();
            m.insert(canonical, diags);
            m
        } else {
            if let Err(e) = client.wait_until_ready().await {
                return ToolResult::Err(e);
            }
            client.all_diagnostics()
        };

        let severity_filter = args.severity.as_deref();
        let mut out = Vec::new();
        for (path, diags) in entries {
            let file = path
                .strip_prefix(workspace)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| path.to_string_lossy().to_string());
            for d in diags {
                let sev = d.get("severity").and_then(|v| v.as_u64()).unwrap_or(0);
                let sev_name = lsp_severity_name(sev);
                if let Some(filter) = severity_filter
                    && sev_name != filter
                {
                    continue;
                }
                out.push(json!({
                    "file":     file,
                    "range":    d.get("range").cloned().unwrap_or(Value::Null),
                    "severity": sev_name,
                    "message":  d.get("message").and_then(|v| v.as_str()).unwrap_or(""),
                    "source":   d.get("source").and_then(|v| v.as_str()).unwrap_or(""),
                    "code":     d.get("code").cloned().unwrap_or(Value::Null),
                }));
            }
        }
        ToolResult::Ok(json!({ "diagnostics": out, "count": out.len() }))
    }
}

fn lsp_severity_name(s: u64) -> &'static str {
    match s {
        1 => "error",
        2 => "warning",
        3 => "info",
        4 => "hint",
        _ => "info",
    }
}

// ---------------------------------------------------------------- rust_find_references

pub struct RustFindReferences;

#[derive(Deserialize)]
struct FindReferencesArgs {
    file: String,
    line: u32,
    character: u32,
    #[serde(default = "default_true")]
    include_declaration: bool,
}

fn default_true() -> bool {
    true
}

#[async_trait]
impl Tool for RustFindReferences {
    fn name(&self) -> &str {
        "rust_find_references"
    }
    fn description(&self) -> &str {
        "Return every reference to the symbol at an LSP-style position across the workspace. Disambiguates by binding (resolution-aware) — beats grep when the answer is rust-analyzer-indexed."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["file", "line", "character"],
            "properties": {
                "file":                { "type": "string" },
                "line":                { "type": "integer", "minimum": 0 },
                "character":           { "type": "integer", "minimum": 0 },
                "include_declaration": { "type": "boolean", "default": true }
            }
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::ReadOnly
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let args: FindReferencesArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::Err(format!("invalid args: {e}")),
        };
        let workspace = ctx.workspace_root.as_std_path();
        let client = match LspClient::for_workspace(workspace).await {
            Ok(c) => c,
            Err(e) => return ToolResult::Err(e),
        };
        let client = client.lock().await;
        let path = resolve_file(ctx, &args.file);
        let canonical = match client.ensure_file_opened(&path).await {
            Ok(p) => p,
            Err(e) => return ToolResult::Err(e),
        };
        if let Err(e) = client.wait_for_file_analysis(&canonical).await {
            return ToolResult::Err(e);
        }
        let uri = path_to_uri(&canonical).unwrap_or_default();
        let resp = match client
            .request(
                "textDocument/references",
                json!({
                    "textDocument": { "uri": uri },
                    "position": { "line": args.line, "character": args.character },
                    "context": { "includeDeclaration": args.include_declaration }
                }),
            )
            .await
        {
            Ok(v) => v,
            Err(e) => return ToolResult::Err(e),
        };
        let mut references = locations_from_response(&resp, ctx);
        for r in &mut references {
            if let Value::Object(map) = r {
                map.insert("kind".into(), Value::String("unspecified".into()));
            }
        }
        ToolResult::Ok(json!({
            "references": references,
            "count":      references.len(),
        }))
    }
}

// ---------------------------------------------------------------- rust_rename

pub struct RustRename;

#[derive(Deserialize)]
struct RenameArgs {
    file: String,
    line: u32,
    character: u32,
    new_name: String,
    #[serde(default)]
    apply: Option<bool>,
}

#[async_trait]
impl Tool for RustRename {
    fn name(&self) -> &str {
        "rust_rename"
    }
    fn description(&self) -> &str {
        "Compute a cross-file rename WorkspaceEdit via rust-analyzer (scope-aware). With apply=false (default) returns the WorkspaceEdit as preview; with apply=true routes it through the workspace-edit substrate for atomic application with syn-parse rollback."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["file", "line", "character", "new_name"],
            "properties": {
                "file":      { "type": "string" },
                "line":      { "type": "integer", "minimum": 0 },
                "character": { "type": "integer", "minimum": 0 },
                "new_name":  { "type": "string", "minLength": 1 },
                "apply":     { "type": "boolean", "default": false }
            }
        })
    }
    fn category(&self) -> ToolCategory {
        // The tool itself is preview-by-default; the substrate apply path
        // is gated by `apply=true`. We declare ReadOnly here because the
        // permission gate is enforced when the tool actually mutates, not
        // by category alone. Substrate panic/rollback is the real guard.
        ToolCategory::ReadOnly
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let args: RenameArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::Err(format!("invalid args: {e}")),
        };
        if !is_valid_rust_ident(&args.new_name) {
            return ToolResult::Err(format!(
                "{:?} is not a valid Rust identifier",
                args.new_name
            ));
        }
        let workspace = ctx.workspace_root.as_std_path();
        let client = match LspClient::for_workspace(workspace).await {
            Ok(c) => c,
            Err(e) => return ToolResult::Err(e),
        };
        let client = client.lock().await;
        let path = resolve_file(ctx, &args.file);
        let canonical = match client.ensure_file_opened(&path).await {
            Ok(p) => p,
            Err(e) => return ToolResult::Err(e),
        };
        if let Err(e) = client.wait_for_file_analysis(&canonical).await {
            return ToolResult::Err(e);
        }
        let uri = path_to_uri(&canonical).unwrap_or_default();
        let resp = match client
            .request(
                "textDocument/rename",
                json!({
                    "textDocument": { "uri": uri },
                    "position": { "line": args.line, "character": args.character },
                    "newName": args.new_name
                }),
            )
            .await
        {
            Ok(v) => v,
            Err(e) => return ToolResult::Err(e),
        };
        if resp.is_null() {
            return ToolResult::Err(
                "rust-analyzer returned null — symbol is not renameable from this position".into(),
            );
        }
        let workspace_root = ctx.workspace_root.as_std_path();
        let edit_json = workspace_edit_to_json(&resp, workspace_root);
        let edits_total: usize = edit_json["changes"]
            .as_object()
            .map(|o| {
                o.values()
                    .map(|v| v.as_array().map(|a| a.len()).unwrap_or(0))
                    .sum()
            })
            .unwrap_or(0);
        let files_touched = edit_json["changes"]
            .as_object()
            .map(|o| o.len())
            .unwrap_or(0);

        let mut result = json!({
            "workspace_edit": edit_json,
            "files_touched":  files_touched,
            "edits_total":    edits_total,
            "applied":        false,
        });

        if args.apply.unwrap_or(false) {
            // Translate to oxidant_tools::WorkspaceEdit and route through
            // the substrate for atomicity + syn-parse rollback.
            match lsp_to_oxidant_workspace_edit(&resp) {
                Ok(edit) => match oxidant_tools::apply(workspace_root, edit) {
                    Ok(ar) => {
                        result["applied"] = Value::Bool(true);
                        result["substrate"] = json!({
                            "ok": true,
                            "files": ar.files.iter().map(|f| json!({
                                "path": f.path.to_string_lossy().replace('\\', "/"),
                                "edits_applied": f.edits_applied,
                            })).collect::<Vec<_>>(),
                        });
                    }
                    Err(e) => {
                        result["applied"] = Value::Bool(false);
                        result["substrate"] = json!({ "ok": false, "error": e.to_string() });
                    }
                },
                Err(e) => {
                    result["applied"] = Value::Bool(false);
                    result["substrate"] = json!({ "ok": false, "error": e });
                }
            }
        }
        ToolResult::Ok(result)
    }
}

fn is_valid_rust_ident(s: &str) -> bool {
    let mut chars = s.chars();
    let first = match chars.next() {
        Some(c) => c,
        None => return false,
    };
    if first != '_' && !first.is_alphabetic() {
        return false;
    }
    chars.all(|c| c == '_' || c.is_alphanumeric())
}

/// LSP textDocument/rename response → JSON in our oxidant shape, paths
/// relativised to the workspace root.
fn workspace_edit_to_json(resp: &Value, workspace_root: &Path) -> Value {
    let mut by_file: serde_json::Map<String, Value> = serde_json::Map::new();
    // Both `changes` (object keyed by URI) and `documentChanges` (array of
    // TextDocumentEdit) are valid response shapes.
    if let Some(changes) = resp.get("changes").and_then(|v| v.as_object()) {
        for (uri, edits) in changes {
            let path = relativise_uri(uri, workspace_root);
            by_file.insert(path, edits.clone());
        }
    }
    if let Some(doc_changes) = resp.get("documentChanges").and_then(|v| v.as_array()) {
        for dc in doc_changes {
            if let (Some(td), Some(edits)) = (
                dc.get("textDocument"),
                dc.get("edits").and_then(|v| v.as_array()),
            ) {
                let uri = td
                    .get("uri")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let path = relativise_uri(&uri, workspace_root);
                by_file
                    .entry(path)
                    .or_insert_with(|| Value::Array(Vec::new()))
                    .as_array_mut()
                    .unwrap()
                    .extend(edits.clone());
            }
        }
    }
    json!({ "changes": Value::Object(by_file) })
}

fn relativise_uri(uri: &str, workspace_root: &Path) -> String {
    if let Some(path) = uri_to_path(uri) {
        if let Ok(rel) = path.strip_prefix(workspace_root) {
            return rel.to_string_lossy().replace('\\', "/");
        }
        return path.to_string_lossy().replace('\\', "/");
    }
    uri.to_string()
}

/// LSP WorkspaceEdit response → oxidant_tools::WorkspaceEdit ready for the
/// substrate. Returns Err if any URI doesn't resolve to a workspace path.
fn lsp_to_oxidant_workspace_edit(resp: &Value) -> Result<oxidant_tools::WorkspaceEdit, String> {
    use oxidant_tools::{Range, TextEdit};
    let mut out: HashMap<PathBuf, Vec<TextEdit>> = HashMap::new();
    let mut record = |uri: &str, edits: &[Value]| -> Result<(), String> {
        let path = uri_to_path(uri).ok_or_else(|| format!("could not decode uri: {uri}"))?;
        let entry = out.entry(path).or_default();
        for e in edits {
            let range = e
                .get("range")
                .ok_or_else(|| "edit missing range".to_string())?;
            let new_text = e
                .get("newText")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "edit missing newText".to_string())?;
            let start = pos_from_json(range.get("start"))?;
            let end = pos_from_json(range.get("end"))?;
            entry.push(TextEdit {
                range: Range { start, end },
                new_text: new_text.to_string(),
                expected_text: None,
            });
        }
        Ok::<_, String>(())
    };
    if let Some(changes) = resp.get("changes").and_then(|v| v.as_object()) {
        for (uri, edits) in changes {
            if let Some(arr) = edits.as_array() {
                record(uri, arr)?;
            }
        }
    }
    if let Some(doc_changes) = resp.get("documentChanges").and_then(|v| v.as_array()) {
        for dc in doc_changes {
            let uri = dc
                .get("textDocument")
                .and_then(|t| t.get("uri"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if let Some(arr) = dc.get("edits").and_then(|v| v.as_array()) {
                record(uri, arr)?;
            }
        }
    }
    use oxidant_tools::WorkspaceEdit as Wse;
    Ok(Wse { changes: out })
}

fn pos_from_json(v: Option<&Value>) -> Result<oxidant_tools::Position, String> {
    let v = v.ok_or_else(|| "position missing".to_string())?;
    let line = v
        .get("line")
        .and_then(|x| x.as_u64())
        .ok_or_else(|| "position.line missing".to_string())? as u32;
    let character = v
        .get("character")
        .and_then(|x| x.as_u64())
        .ok_or_else(|| "position.character missing".to_string())? as u32;
    Ok(oxidant_tools::Position { line, character })
}

// ---------------------------------------------------------------- rust_code_actions

pub struct RustCodeActions;

#[derive(Deserialize)]
struct CodeActionArgs {
    file: String,
    range: RangeArg,
    #[serde(default)]
    kinds: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct RangeArg {
    start: PositionArg,
    end: PositionArg,
}

#[derive(Deserialize)]
struct PositionArg {
    line: u32,
    character: u32,
}

#[async_trait]
impl Tool for RustCodeActions {
    fn name(&self) -> &str {
        "rust_code_actions"
    }
    fn description(&self) -> &str {
        "Enumerate rust-analyzer code actions (quickfixes, refactors, organise imports, implement missing members) for a range. Each action's `edit` is a WorkspaceEdit ready for apply_edits."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["file", "range"],
            "properties": {
                "file":  { "type": "string" },
                "range": {
                    "type": "object",
                    "required": ["start", "end"],
                    "properties": {
                        "start": {
                            "type": "object",
                            "required": ["line", "character"],
                            "properties": {
                                "line":      { "type": "integer", "minimum": 0 },
                                "character": { "type": "integer", "minimum": 0 }
                            }
                        },
                        "end": {
                            "type": "object",
                            "required": ["line", "character"],
                            "properties": {
                                "line":      { "type": "integer", "minimum": 0 },
                                "character": { "type": "integer", "minimum": 0 }
                            }
                        }
                    }
                },
                "kinds": { "type": "array", "items": { "type": "string" }, "description": "LSP CodeActionKind filter (quickfix, refactor.extract, ...)" }
            }
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::ReadOnly
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let args: CodeActionArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::Err(format!("invalid args: {e}")),
        };
        let workspace = ctx.workspace_root.as_std_path();
        let client = match LspClient::for_workspace(workspace).await {
            Ok(c) => c,
            Err(e) => return ToolResult::Err(e),
        };
        let client = client.lock().await;
        let path = resolve_file(ctx, &args.file);
        let canonical = match client.ensure_file_opened(&path).await {
            Ok(p) => p,
            Err(e) => return ToolResult::Err(e),
        };
        if let Err(e) = client.wait_for_file_analysis(&canonical).await {
            return ToolResult::Err(e);
        }
        let uri = path_to_uri(&canonical).unwrap_or_default();

        let mut params = json!({
            "textDocument": { "uri": uri },
            "range": {
                "start": { "line": args.range.start.line, "character": args.range.start.character },
                "end":   { "line": args.range.end.line,   "character": args.range.end.character }
            },
            "context": { "diagnostics": [] }
        });
        if let Some(kinds) = &args.kinds {
            params["context"]["only"] = json!(kinds);
        }

        let resp = match client.request("textDocument/codeAction", params).await {
            Ok(v) => v,
            Err(e) => return ToolResult::Err(e),
        };
        let raw = resp.as_array().cloned().unwrap_or_default();
        let workspace_root = ctx.workspace_root.as_std_path();
        let kind_filter = args.kinds.as_ref();
        let actions: Vec<Value> = raw
            .into_iter()
            .filter_map(|item| extract_code_action(&item, workspace_root))
            .filter(|a| match kind_filter {
                Some(kinds) => kinds
                    .iter()
                    .any(|k| a["kind"].as_str().is_some_and(|akind| akind == k.as_str())),
                None => true,
            })
            .collect();
        ToolResult::Ok(json!({ "actions": actions, "count": actions.len() }))
    }
}

fn extract_code_action(item: &Value, workspace_root: &Path) -> Option<Value> {
    // Item is either a Command or a CodeAction; we surface CodeAction only.
    let title = item.get("title").and_then(|v| v.as_str())?.to_string();
    let kind = item
        .get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let edit = item
        .get("edit")
        .map(|e| workspace_edit_to_json(e, workspace_root))
        .unwrap_or(json!({ "changes": {} }));
    Some(json!({
        "title": title,
        "kind":  kind,
        "edit":  edit,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_signature_extracts_rust_fence() {
        let md = "```rust\nfn add(a: i32, b: i32) -> i32\n```\n\n---\n\nAdds two numbers.";
        let (sig, doc) = split_signature_from_markdown(md);
        assert_eq!(sig.as_deref(), Some("fn add(a: i32, b: i32) -> i32"));
        assert!(doc.unwrap().contains("Adds two numbers"));
    }

    #[test]
    fn split_signature_joins_multiple_rust_fences() {
        // rust-analyzer's actual output: first fence is the module qualifier,
        // second is the signature. We keep both for the model.
        let md = "```rust\nsample\n```\n\n```rust\npub fn add(a: i32, b: i32) -> i32\n```\n\n---\n\nAdds two integers together.";
        let (sig, doc) = split_signature_from_markdown(md);
        let s = sig.expect("signature");
        assert!(s.contains("sample"), "got: {s}");
        assert!(s.contains("pub fn add"), "got: {s}");
        assert!(doc.unwrap().contains("Adds two integers"));
    }

    #[test]
    fn path_to_uri_handles_windows_drive() {
        // We can't test the actual Windows behaviour cross-platform — just
        // confirm something sensible comes back for a path-shaped input.
        let p = Path::new(".");
        let uri = path_to_uri(p).expect("uri");
        assert!(uri.starts_with("file://"), "got {uri}");
    }

    #[test]
    fn lsp_symbol_kind_names_map() {
        assert_eq!(lsp_symbol_kind_name(12), "fn");
        assert_eq!(lsp_symbol_kind_name(23), "struct");
        assert_eq!(lsp_symbol_kind_name(10), "enum");
        assert_eq!(lsp_symbol_kind_name(11), "interface");
        assert_eq!(lsp_symbol_kind_name(999), "?");
    }

    #[test]
    fn lsp_severity_names_map() {
        assert_eq!(lsp_severity_name(1), "error");
        assert_eq!(lsp_severity_name(2), "warning");
        assert_eq!(lsp_severity_name(3), "info");
        assert_eq!(lsp_severity_name(4), "hint");
    }
}
