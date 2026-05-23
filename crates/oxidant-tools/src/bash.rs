// Realises spec/components/tools/bash-runner.md and spec/tools/bash/bash.md.
//
// The escape hatch. Runs a shell command in the workspace with a
// timeout and captured (tail-truncated) stdout/stderr. The model is
// expected to reach for first-class tools first; bash is for cases
// no structured tool covers (cargo audit, wasm-pack, ad-hoc scripts).
//
// Shell selection:
//   Windows  →  cmd.exe /S /C "<command>"
//   Unix     →  bash -c "<command>"
//
// Permission category is Mutating: bash can do anything, the prompt
// is the only safety. An allowlist heuristic lives in
// spec/components/config/permissions.md (deferred).

use std::process::Stdio;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;

use oxidant_core::{Tool, ToolCategory, ToolContext, ToolResult};

const DEFAULT_TIMEOUT_MS: u64 = 120_000;
const MAX_TIMEOUT_MS: u64 = 600_000;
const OUTPUT_CAP_BYTES: usize = 30 * 1024;

pub struct Bash;

#[derive(Deserialize)]
struct Args {
    command: String,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    stdin: Option<String>,
}

#[async_trait]
impl Tool for Bash {
    fn name(&self) -> &str {
        "bash"
    }
    fn description(&self) -> &str {
        "Run a shell command in the workspace and capture stdout/stderr (tail-truncated at 30KB). The escape hatch for cases no first-class tool covers — prefer cargo_*, rust_*, syn_*, fs_*, vcs_* first. cmd.exe on Windows, bash on Unix. Default timeout 120s, max 600s."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["command"],
            "properties": {
                "command":    { "type": "string", "description": "passed to the shell as-is" },
                "timeout_ms": { "type": "integer", "default": DEFAULT_TIMEOUT_MS, "maximum": MAX_TIMEOUT_MS },
                "stdin":      { "type": "string", "description": "optional stdin sent then closed" }
            }
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Mutating
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let args: Args = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::Err(format!("invalid args: {e}")),
        };
        if args.command.trim().is_empty() {
            return ToolResult::Err("command must not be empty".into());
        }
        let timeout_ms = args
            .timeout_ms
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .min(MAX_TIMEOUT_MS);

        let workspace = ctx.workspace_root.as_std_path();
        let mut cmd = build_shell_command(&args.command);
        cmd.current_dir(workspace)
            .env("CARGO_TARGET_DIR", workspace.join("target"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        tracing::debug!(cmd = %args.command, "bash spawn");
        let started = Instant::now();
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => return ToolResult::Err(format!("spawn shell failed: {e}")),
        };

        // Write stdin (if any) then drop the pipe so the child sees EOF.
        if let Some(stdin_text) = &args.stdin
            && let Some(mut sin) = child.stdin.take()
            && let Err(e) = sin.write_all(stdin_text.as_bytes()).await
        {
            tracing::debug!("bash stdin write err: {e}");
        }

        let stdout_handle = child.stdout.take().map(|s| {
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let _ = BufReader::new(s).read_to_end(&mut buf).await;
                buf
            })
        });
        let stderr_handle = child.stderr.take().map(|s| {
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let _ = BufReader::new(s).read_to_end(&mut buf).await;
                buf
            })
        });

        let mut timed_out = false;
        let status = match timeout(Duration::from_millis(timeout_ms), child.wait()).await {
            Ok(Ok(s)) => Some(s),
            Ok(Err(e)) => return ToolResult::Err(format!("wait on shell failed: {e}")),
            Err(_) => {
                timed_out = true;
                let _ = child.start_kill();
                let _ = child.wait().await;
                None
            }
        };

        let stdout = collect(stdout_handle).await;
        let stderr = collect(stderr_handle).await;
        let duration_ms = started.elapsed().as_millis() as u64;

        let (stdout, stdout_truncated) = truncate_tail(stdout, OUTPUT_CAP_BYTES);
        let (mut stderr, stderr_truncated) = truncate_tail(stderr, OUTPUT_CAP_BYTES);
        if timed_out {
            if !stderr.is_empty() {
                stderr.push('\n');
            }
            stderr.push_str(&format!(
                "[oxidant: bash timed out after {timeout_ms}ms — child killed]"
            ));
        }

        let exit_code: Option<i32> = status.and_then(|s| s.code());
        ToolResult::Ok(json!({
            "exit_code":        exit_code,
            "stdout":           stdout,
            "stderr":           stderr,
            "stdout_truncated": stdout_truncated,
            "stderr_truncated": stderr_truncated,
            "duration_ms":      duration_ms,
            "timed_out":        timed_out,
        }))
    }
}

fn build_shell_command(command: &str) -> Command {
    if cfg!(windows) {
        let mut cmd = Command::new("cmd.exe");
        cmd.arg("/S").arg("/C").arg(command);
        cmd
    } else {
        let mut cmd = Command::new("bash");
        cmd.arg("-c").arg(command);
        cmd
    }
}

async fn collect(handle: Option<tokio::task::JoinHandle<Vec<u8>>>) -> Vec<u8> {
    match handle {
        Some(h) => h.await.unwrap_or_default(),
        None => Vec::new(),
    }
}

/// Tail-truncate `buf` to at most `cap` bytes, prefixing a marker when
/// truncation actually happened. The marker is plain ASCII so it
/// survives UTF-8 lossy decoding cleanly.
fn truncate_tail(buf: Vec<u8>, cap: usize) -> (String, bool) {
    if buf.len() <= cap {
        return (String::from_utf8_lossy(&buf).into_owned(), false);
    }
    let skipped = buf.len() - cap;
    let tail = &buf[buf.len() - cap..];
    let marker = format!("[... truncated {skipped} earlier bytes ...]\n");
    let mut out = marker.into_bytes();
    out.extend_from_slice(tail);
    (String::from_utf8_lossy(&out).into_owned(), true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use camino::Utf8PathBuf;
    use serde_json::json;
    use tempfile::TempDir;
    use tokio_util::sync::CancellationToken;

    fn ctx_for(dir: &std::path::Path) -> ToolContext {
        ToolContext {
            workspace_root: Utf8PathBuf::from_path_buf(dunce::canonicalize(dir).unwrap()).unwrap(),
            exploration_id: "bash-test".into(),
            cancellation: CancellationToken::new(),
        }
    }

    #[tokio::test]
    async fn echo_hello_returns_zero_with_stdout() {
        let dir = TempDir::new().unwrap();
        let v = match Bash
            .invoke(json!({ "command": "echo hello" }), &ctx_for(dir.path()))
            .await
        {
            ToolResult::Ok(v) => v,
            ToolResult::Err(e) => panic!("err: {e}"),
        };
        assert_eq!(v["exit_code"], 0);
        assert!(
            v["stdout"].as_str().unwrap().contains("hello"),
            "stdout was {:?}",
            v["stdout"]
        );
        assert_eq!(v["timed_out"], false);
    }

    #[tokio::test]
    async fn stdin_is_piped_to_child() {
        // Both `bash -c cat` and `cmd /S /C more` echo stdin to stdout.
        let dir = TempDir::new().unwrap();
        let cmd = if cfg!(windows) { "more" } else { "cat" };
        let v = match Bash
            .invoke(
                json!({ "command": cmd, "stdin": "piped content\n" }),
                &ctx_for(dir.path()),
            )
            .await
        {
            ToolResult::Ok(v) => v,
            ToolResult::Err(e) => panic!("err: {e}"),
        };
        assert!(
            v["stdout"].as_str().unwrap().contains("piped content"),
            "stdout was {:?}",
            v["stdout"]
        );
    }

    #[tokio::test]
    async fn nonzero_exit_is_captured() {
        let dir = TempDir::new().unwrap();
        // `exit 7` is available in both cmd.exe and bash.
        let v = match Bash
            .invoke(json!({ "command": "exit 7" }), &ctx_for(dir.path()))
            .await
        {
            ToolResult::Ok(v) => v,
            ToolResult::Err(e) => panic!("err: {e}"),
        };
        assert_eq!(v["exit_code"], 7);
    }

    #[tokio::test]
    async fn timeout_kills_child_and_reports() {
        let dir = TempDir::new().unwrap();
        // Cross-shell sleep: `timeout` on Windows takes seconds and is
        // available out of the box; `sleep` on Unix.
        // Cross-shell sleep. Windows `timeout /T` errors when stdin
        // isn't a console (it is piped here), so use `ping` to
        // localhost for a ~5s wait that works headlessly. Unix gets
        // `sleep 5`.
        let cmd = if cfg!(windows) {
            "ping -n 6 127.0.0.1 > NUL"
        } else {
            "sleep 5"
        };
        let v = match Bash
            .invoke(
                json!({ "command": cmd, "timeout_ms": 250 }),
                &ctx_for(dir.path()),
            )
            .await
        {
            ToolResult::Ok(v) => v,
            ToolResult::Err(e) => panic!("err: {e}"),
        };
        assert_eq!(v["timed_out"], true);
        assert!(
            v["stderr"]
                .as_str()
                .unwrap()
                .contains("oxidant: bash timed out"),
            "expected marker in stderr, got {:?}",
            v["stderr"]
        );
    }

    #[tokio::test]
    async fn empty_command_rejected() {
        let dir = TempDir::new().unwrap();
        let result = Bash
            .invoke(json!({ "command": "   " }), &ctx_for(dir.path()))
            .await;
        assert!(matches!(result, ToolResult::Err(_)));
    }

    #[test]
    fn truncate_tail_keeps_under_cap_intact() {
        let (s, truncated) = truncate_tail(b"hello".to_vec(), 100);
        assert!(!truncated);
        assert_eq!(s, "hello");
    }

    #[test]
    fn truncate_tail_marks_when_over_cap() {
        let big = vec![b'x'; 200];
        let (s, truncated) = truncate_tail(big, 50);
        assert!(truncated);
        assert!(s.starts_with("[... truncated 150 earlier bytes ...]"));
        assert_eq!(s.chars().filter(|c| *c == 'x').count(), 50);
    }
}
