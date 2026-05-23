// Realises spec/components/rust-tools/cargo-runner.md and the cargo-* tool
// specs that route through it (cargo_check, cargo_test, cargo_clippy in
// MVP; build/expand/tree/metadata follow when needed).
//
// The runner spawns `cargo <sub>` via tokio::process::Command with
// `--message-format=json` (where supported), reads stdout line-by-line,
// tries each line as a cargo_metadata::Message. Lines that aren't valid
// cargo JSON fall through to a libtest text parser — that's how we
// extract per-test events on stable Rust (libtest's JSON output is
// still nightly-only behind -Z unstable-options).
//
// Cancellation: tokio::select between ctx.cancellation.cancelled() and
// the next line; on cancel, kill the child and return a structured
// cancellation result.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::LazyLock;
use std::time::Instant;

use async_trait::async_trait;
use regex::Regex;
use serde::Serialize;
use serde_json::{Value, json};
use tokio::io::AsyncBufReadExt;
use tokio::process::Command;

use oxidant_core::{Tool, ToolCategory, ToolContext, ToolResult};

// ---------------------------------------------------------------- Runner

/// Outcome of one cargo subprocess run.
#[derive(Debug, Default)]
pub struct RunOutcome {
    pub status_code: Option<i32>,
    pub messages: Vec<DiagnosticEntry>,
    pub artifacts: Vec<ArtifactEntry>,
    pub stdout_lines: Vec<String>, // non-JSON stdout lines, in order
    pub cancelled: bool,
    pub elapsed_ms: u128,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticEntry {
    pub level: String,
    pub code: Option<String>,
    pub message: String,
    pub spans: Vec<SpanEntry>,
    pub suggestion: Option<SuggestionEntry>,
    pub rendered: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpanEntry {
    pub file: String,
    pub start: PositionEntry,
    pub end: PositionEntry,
    pub is_primary: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PositionEntry {
    pub line: u32,
    pub character: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct SuggestionEntry {
    pub replacement: String,
    pub applicability: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArtifactEntry {
    pub package_id: String,
    pub kind: String,
    pub path: String,
}

/// Run `cargo <subcommand>` with extra args under `workspace_root`.
///
/// `args` should NOT include `--message-format=json` or `--manifest-path`;
/// this function injects them.
pub async fn run_cargo(
    subcommand: &str,
    args: &[String],
    ctx: &ToolContext,
) -> Result<RunOutcome, String> {
    let workspace = ctx.workspace_root.as_std_path();
    let start = Instant::now();

    let mut cmd = Command::new("cargo");
    cmd.arg(subcommand)
        .arg("--message-format=json-diagnostic-rendered-ansi")
        .arg("--color=never");
    for a in args {
        cmd.arg(a);
    }
    cmd.current_dir(workspace)
        .env("CARGO_TARGET_DIR", workspace.join("target"))
        .env("CARGO_TERM_COLOR", "never")
        .env("RUST_BACKTRACE", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    tracing::debug!(subcommand, args = ?args, "cargo run");

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("spawn cargo {subcommand} failed: {e}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "child stdout pipe missing".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "child stderr pipe missing".to_string())?;

    let mut out = RunOutcome::default();
    let mut stdout_reader = tokio::io::BufReader::new(stdout).lines();
    let mut stderr_reader = tokio::io::BufReader::new(stderr).lines();

    loop {
        tokio::select! {
            _ = ctx.cancellation.cancelled() => {
                let _ = child.start_kill();
                out.cancelled = true;
                break;
            }
            line = stdout_reader.next_line() => {
                match line {
                    Ok(Some(line)) => process_stdout_line(&line, &mut out),
                    Ok(None) => break,
                    Err(e) => return Err(format!("read stdout failed: {e}")),
                }
            }
            // Drain stderr in parallel; cargo emits human-readable progress here.
            // We don't surface it to the model, but reading prevents the pipe filling.
            line = stderr_reader.next_line() => {
                match line {
                    Ok(Some(l)) => {
                        if !l.trim().is_empty() {
                            tracing::trace!("cargo stderr: {l}");
                        }
                    }
                    Ok(None) => {} // EOF on stderr is fine; keep draining stdout
                    Err(e) => tracing::debug!("stderr read err: {e}"),
                }
            }
        }
    }

    let status = child
        .wait()
        .await
        .map_err(|e| format!("await cargo exit failed: {e}"))?;
    out.status_code = status.code();
    out.elapsed_ms = start.elapsed().as_millis();
    Ok(out)
}

fn process_stdout_line(line: &str, out: &mut RunOutcome) {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return;
    }
    if trimmed.starts_with('{') {
        match serde_json::from_str::<cargo_metadata::Message>(trimmed) {
            Ok(msg) => {
                handle_cargo_message(msg, out);
                return;
            }
            Err(e) => {
                tracing::trace!("non-cargo JSON line: {e}");
                // fall through — treat as raw text
            }
        }
    }
    out.stdout_lines.push(line.to_string());
}

fn handle_cargo_message(msg: cargo_metadata::Message, out: &mut RunOutcome) {
    use cargo_metadata::Message;
    match msg {
        Message::CompilerMessage(cm) => {
            out.messages.push(translate_diagnostic(&cm.message));
        }
        Message::CompilerArtifact(art) => {
            for path in &art.filenames {
                out.artifacts.push(ArtifactEntry {
                    package_id: art.package_id.to_string(),
                    kind: art
                        .target
                        .kind
                        .first()
                        .map(|k| k.to_string())
                        .unwrap_or_default(),
                    path: path.to_string(),
                });
            }
        }
        _ => {}
    }
}

fn translate_diagnostic(d: &cargo_metadata::diagnostic::Diagnostic) -> DiagnosticEntry {
    let mut suggestion = None;
    let mut spans = Vec::new();
    for s in &d.spans {
        spans.push(SpanEntry {
            file: s.file_name.clone(),
            start: PositionEntry {
                line: s.line_start as u32 - 1,
                character: s.column_start as u32 - 1,
            },
            end: PositionEntry {
                line: s.line_end as u32 - 1,
                character: s.column_end as u32 - 1,
            },
            is_primary: s.is_primary,
        });
        if let Some(repl) = &s.suggested_replacement {
            suggestion.get_or_insert_with(|| SuggestionEntry {
                replacement: repl.clone(),
                applicability: format!("{:?}", s.suggestion_applicability),
            });
        }
    }
    DiagnosticEntry {
        level: format!("{:?}", d.level).to_lowercase(),
        code: d.code.as_ref().map(|c| c.code.clone()),
        message: d.message.clone(),
        spans,
        suggestion,
        rendered: d.rendered.clone(),
    }
}

fn count_levels(messages: &[DiagnosticEntry]) -> (usize, usize) {
    let mut errors = 0;
    let mut warnings = 0;
    for m in messages {
        match m.level.as_str() {
            "error" | "error: internal compiler error" => errors += 1,
            "warning" => warnings += 1,
            _ => {}
        }
    }
    (errors, warnings)
}

// ------------------------------------------------------------ libtest text parser

#[derive(Debug, Clone, Default, Serialize)]
pub struct TestSummary {
    pub passed: usize,
    pub failed: usize,
    pub ignored: usize,
    pub failures: Vec<TestFailure>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TestFailure {
    pub test: String,
    pub stdout: String,
    pub stderr: String,
}

static TEST_RESULT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^test\s+(?P<name>\S+)\s+\.\.\.\s+(?P<status>ok|FAILED|ignored)\b").unwrap()
});

static FAILURE_BLOCK_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^----\s+(?P<name>\S+)\s+(?P<stream>stdout|stderr)\s+----").unwrap()
});

pub fn parse_libtest_text(lines: &[String]) -> TestSummary {
    let mut summary = TestSummary::default();
    let mut failure_outputs: HashMap<String, (String, String)> = HashMap::new();
    let mut current_capture: Option<(String, &'static str)> = None;
    let mut failure_names_in_order: Vec<String> = Vec::new();

    for line in lines {
        if let Some((name, stream)) = current_capture.as_ref() {
            if line.starts_with("----")
                || line.starts_with("test result:")
                || line.starts_with("failures:")
            {
                current_capture = None;
                // fall through to evaluate the line below
            } else {
                let (s_out, s_err) = failure_outputs.entry(name.clone()).or_default();
                if *stream == "stdout" {
                    if !s_out.is_empty() {
                        s_out.push('\n');
                    }
                    s_out.push_str(line);
                } else {
                    if !s_err.is_empty() {
                        s_err.push('\n');
                    }
                    s_err.push_str(line);
                }
                continue;
            }
        }
        if let Some(caps) = TEST_RESULT_RE.captures(line) {
            let name = caps["name"].to_string();
            match &caps["status"] {
                "ok" => summary.passed += 1,
                "FAILED" => {
                    summary.failed += 1;
                    if !failure_names_in_order.contains(&name) {
                        failure_names_in_order.push(name);
                    }
                }
                "ignored" => summary.ignored += 1,
                _ => {}
            }
            continue;
        }
        if let Some(caps) = FAILURE_BLOCK_RE.captures(line) {
            let name = caps["name"].to_string();
            let stream: &'static str = if &caps["stream"] == "stdout" {
                "stdout"
            } else {
                "stderr"
            };
            current_capture = Some((name, stream));
            continue;
        }
    }

    for name in failure_names_in_order {
        let (stdout, stderr) = failure_outputs.remove(&name).unwrap_or_default();
        summary.failures.push(TestFailure {
            test: name,
            stdout,
            stderr,
        });
    }
    summary
}

// ------------------------------------------------------------ Tools

#[derive(serde::Deserialize, Default)]
struct CheckArgs {
    #[serde(default)]
    package: Option<String>,
    #[serde(default)]
    all_targets: Option<bool>,
    #[serde(default)]
    features: Option<Vec<String>>,
    #[serde(default)]
    no_default_features: Option<bool>,
}

fn check_args_to_cli(args: &CheckArgs) -> Vec<String> {
    let mut cli = Vec::new();
    if let Some(p) = &args.package {
        cli.push("-p".into());
        cli.push(p.clone());
    }
    if args.all_targets.unwrap_or(false) {
        cli.push("--all-targets".into());
    }
    if let Some(features) = &args.features
        && !features.is_empty() {
            cli.push("--features".into());
            cli.push(features.join(","));
        }
    if args.no_default_features.unwrap_or(false) {
        cli.push("--no-default-features".into());
    }
    cli
}

pub struct CargoCheck;

#[async_trait]
impl Tool for CargoCheck {
    fn name(&self) -> &str {
        "cargo_check"
    }
    fn description(&self) -> &str {
        "Run `cargo check` against the workspace and return structured compiler diagnostics + a pass/fail summary. Fast; doesn't produce binaries."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "package":     { "type": "string" },
                "all_targets": { "type": "boolean", "default": false },
                "features":    { "type": "array", "items": { "type": "string" } },
                "no_default_features": { "type": "boolean", "default": false }
            }
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::ReadOnly
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let args: CheckArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::Err(format!("invalid args: {e}")),
        };
        let cli = check_args_to_cli(&args);
        match run_cargo("check", &cli, ctx).await {
            Ok(outcome) => ToolResult::Ok(format_check_outcome(&outcome)),
            Err(e) => ToolResult::Err(e),
        }
    }
}

pub struct CargoClippy;

#[derive(serde::Deserialize, Default)]
struct ClippyArgs {
    #[serde(default)]
    package: Option<String>,
    #[serde(default)]
    fix: Option<bool>,
    #[serde(default)]
    deny: Option<Vec<String>>,
    #[serde(default)]
    allow: Option<Vec<String>>,
}

#[async_trait]
impl Tool for CargoClippy {
    fn name(&self) -> &str {
        "cargo_clippy"
    }
    fn description(&self) -> &str {
        "Run `cargo clippy` and return structured lint diagnostics. `fix=true` (apply machine-applicable suggestions) is deferred to v2 and currently errors."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "package":     { "type": "string" },
                "fix":         { "type": "boolean", "default": false },
                "deny":        { "type": "array", "items": { "type": "string" } },
                "allow":       { "type": "array", "items": { "type": "string" } }
            }
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::ReadOnly
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let args: ClippyArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::Err(format!("invalid args: {e}")),
        };
        if args.fix.unwrap_or(false) {
            return ToolResult::Err(
                "clippy fix mode is deferred to v2; run without `fix` for now".into(),
            );
        }
        let mut cli = Vec::new();
        if let Some(p) = &args.package {
            cli.push("-p".into());
            cli.push(p.clone());
        }
        cli.push("--".into());
        for d in args.deny.iter().flatten() {
            cli.push("-D".into());
            cli.push(d.clone());
        }
        for a in args.allow.iter().flatten() {
            cli.push("-A".into());
            cli.push(a.clone());
        }
        match run_cargo("clippy", &cli, ctx).await {
            Ok(outcome) => ToolResult::Ok(format_check_outcome(&outcome)),
            Err(e) => ToolResult::Err(e),
        }
    }
}

fn format_check_outcome(outcome: &RunOutcome) -> Value {
    let (errors, warnings) = count_levels(&outcome.messages);
    let ok = !outcome.cancelled && outcome.status_code == Some(0) && errors == 0;
    json!({
        "ok": ok,
        "cancelled": outcome.cancelled,
        "messages": outcome.messages,
        "summary": {
            "errors": errors,
            "warnings": warnings,
            "elapsed_ms": outcome.elapsed_ms,
        }
    })
}

pub struct CargoTest;

#[derive(serde::Deserialize, Default)]
struct TestArgs {
    #[serde(default)]
    package: Option<String>,
    #[serde(default)]
    filter: Option<String>,
    #[serde(default)]
    features: Option<Vec<String>>,
    #[serde(default)]
    release: Option<bool>,
}

#[async_trait]
impl Tool for CargoTest {
    fn name(&self) -> &str {
        "cargo_test"
    }
    fn description(&self) -> &str {
        "Run `cargo test` (debug or release). Returns per-test pass/fail counts and stdout/stderr for failing tests, plus any compilation diagnostics."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "package":  { "type": "string" },
                "filter":   { "type": "string", "description": "test name substring filter" },
                "features": { "type": "array", "items": { "type": "string" } },
                "release":  { "type": "boolean", "default": false }
            }
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Mutating
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let args: TestArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::Err(format!("invalid args: {e}")),
        };
        let mut cli = Vec::new();
        if let Some(p) = &args.package {
            cli.push("-p".into());
            cli.push(p.clone());
        }
        if let Some(features) = &args.features
            && !features.is_empty() {
                cli.push("--features".into());
                cli.push(features.join(","));
            }
        if args.release.unwrap_or(false) {
            cli.push("--release".into());
        }
        // Separate cargo flags from libtest flags.
        cli.push("--".into());
        // Show captured output even for passing tests if the model wants it — but
        // skipping noise is preferable for MVP. Capture is enabled by default;
        // we surface output of FAILING tests via the libtest text parser.
        if let Some(filter) = &args.filter {
            cli.push(filter.clone());
        }

        let outcome = match run_cargo("test", &cli, ctx).await {
            Ok(o) => o,
            Err(e) => return ToolResult::Err(e),
        };
        let (errors, warnings) = count_levels(&outcome.messages);
        let test_summary = parse_libtest_text(&outcome.stdout_lines);
        let ok = !outcome.cancelled
            && outcome.status_code == Some(0)
            && test_summary.failed == 0
            && errors == 0;
        ToolResult::Ok(json!({
            "ok": ok,
            "cancelled": outcome.cancelled,
            "passed": test_summary.passed,
            "failed": test_summary.failed,
            "ignored": test_summary.ignored,
            "failures": test_summary.failures,
            "compile_messages": outcome.messages,
            "summary": {
                "errors": errors,
                "warnings": warnings,
                "elapsed_ms": outcome.elapsed_ms,
            }
        }))
    }
}

// ------------------------------------------------------------ Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn libtest_parser_counts_results() {
        let lines: Vec<String> = vec![
            "running 4 tests",
            "test foo::bar ... ok",
            "test foo::baz ... ignored",
            "test foo::qux ... FAILED",
            "test foo::zap ... ok",
            "",
            "failures:",
            "",
            "---- foo::qux stdout ----",
            "thread 'foo::qux' panicked at 'oops'",
            "note: run with `RUST_BACKTRACE=1`",
            "",
            "failures:",
            "    foo::qux",
            "",
            "test result: FAILED. 2 passed; 1 failed; 1 ignored; 0 measured;",
        ]
        .into_iter()
        .map(String::from)
        .collect();

        let summary = parse_libtest_text(&lines);
        assert_eq!(summary.passed, 2);
        assert_eq!(summary.failed, 1);
        assert_eq!(summary.ignored, 1);
        assert_eq!(summary.failures.len(), 1);
        assert_eq!(summary.failures[0].test, "foo::qux");
        assert!(summary.failures[0].stdout.contains("panicked at 'oops'"));
        assert!(summary.failures[0].stdout.contains("BACKTRACE=1"));
    }

    #[test]
    fn libtest_parser_handles_all_passing() {
        let lines: Vec<String> = vec![
            "running 2 tests",
            "test a ... ok",
            "test b ... ok",
            "",
            "test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured;",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let summary = parse_libtest_text(&lines);
        assert_eq!(summary.passed, 2);
        assert_eq!(summary.failed, 0);
        assert!(summary.failures.is_empty());
    }

    #[test]
    fn libtest_parser_captures_stderr_block() {
        let lines: Vec<String> = vec![
            "test a::b ... FAILED",
            "",
            "failures:",
            "",
            "---- a::b stderr ----",
            "thread 'a::b' panicked",
            "stack trace line 1",
            "stack trace line 2",
            "",
            "test result: FAILED. 0 passed; 1 failed;",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let summary = parse_libtest_text(&lines);
        assert_eq!(summary.failures.len(), 1);
        assert!(summary.failures[0].stderr.contains("panicked"));
        assert!(summary.failures[0].stderr.contains("stack trace line 2"));
    }
}
