// Realises spec/components/tools/fs.md and the four file tool specs:
//   spec/tools/fs/fs-read.md
//   spec/tools/fs/fs-write.md
//   spec/tools/fs/glob.md
//   spec/tools/fs/grep.md
//
// All path resolution goes through resolve_in_workspace() — the worktree
// is the boundary; symlinks that escape are rejected after canonicalisation.
// `ignore::WalkBuilder` powers traversal so .gitignore is respected by
// default. grep is backed by grep-searcher (the ripgrep engine).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use globset::{Glob as GlobsetGlob, GlobMatcher};
use grep_matcher::Matcher;
use grep_regex::RegexMatcherBuilder;
use grep_searcher::sinks::UTF8;
use grep_searcher::{BinaryDetection, SearcherBuilder};
use ignore::WalkBuilder;
use serde::Deserialize;
use serde_json::{Value, json};

use oxidant_core::{Tool, ToolCategory, ToolContext, ToolResult};

const FS_READ_DEFAULT_CAP_BYTES: u64 = 1_048_576; // 1 MiB

pub fn standard_tools() -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(FsRead),
        Arc::new(FsWrite),
        Arc::new(Glob),
        Arc::new(Grep),
    ]
}

// ----- fs_read ----------------------------------------------------------

pub struct FsRead;

#[derive(Deserialize)]
struct FsReadArgs {
    file: String,
    #[serde(default)]
    offset: Option<usize>,
    #[serde(default)]
    limit: Option<usize>,
}

#[async_trait]
impl Tool for FsRead {
    fn name(&self) -> &str {
        "fs_read"
    }
    fn description(&self) -> &str {
        "Read a UTF-8 text file from the workspace. Optional `offset` (1-indexed start line) and `limit` (number of lines) for paging large files. Binary files return a marker instead of content."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["file"],
            "properties": {
                "file":   { "type": "string", "description": "path relative to workspace root" },
                "offset": { "type": "integer", "minimum": 1, "description": "1-indexed start line" },
                "limit":  { "type": "integer", "minimum": 1, "description": "number of lines to read" }
            }
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::ReadOnly
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let args: FsReadArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::Err(format!("invalid args: {e}")),
        };

        let workspace = workspace_root(ctx);
        let absolute = match resolve_in_workspace(&workspace, Path::new(&args.file)) {
            Ok(p) => p,
            Err(e) => return ToolResult::Err(e),
        };
        let metadata = match std::fs::metadata(&absolute) {
            Ok(m) => m,
            Err(e) => return ToolResult::Err(format!("stat failed: {e}")),
        };
        if !metadata.is_file() {
            return ToolResult::Err(format!("{} is not a regular file", args.file));
        }

        let bytes = match std::fs::read(&absolute) {
            Ok(b) => b,
            Err(e) => return ToolResult::Err(format!("read failed: {e}")),
        };

        if !is_utf8_text(&bytes) {
            return ToolResult::Ok(json!({
                "file": args.file,
                "binary": true,
                "size": metadata.len(),
            }));
        }

        if args.offset.is_none() && args.limit.is_none() && bytes.len() as u64 > FS_READ_DEFAULT_CAP_BYTES {
            return ToolResult::Err(format!(
                "file exceeds {} byte cap ({} bytes); use offset/limit to page",
                FS_READ_DEFAULT_CAP_BYTES,
                bytes.len()
            ));
        }

        let content = String::from_utf8(bytes).expect("checked utf-8 above");
        let total_lines = content.lines().count();
        let sliced = match (args.offset, args.limit) {
            (None, None) => content,
            (offset, limit) => {
                let start = offset.unwrap_or(1).saturating_sub(1);
                let len = limit.unwrap_or(usize::MAX);
                content.lines().skip(start).take(len).collect::<Vec<_>>().join("\n")
            }
        };

        ToolResult::Ok(json!({
            "file": args.file,
            "content": sliced,
            "lines": total_lines,
            "binary": false,
        }))
    }
}

// ----- fs_write ---------------------------------------------------------

pub struct FsWrite;

#[derive(Deserialize)]
struct FsWriteArgs {
    file: String,
    content: String,
}

#[async_trait]
impl Tool for FsWrite {
    fn name(&self) -> &str {
        "fs_write"
    }
    fn description(&self) -> &str {
        "Create a new file or fully overwrite an existing one. For localised edits to existing files, use edit_string or apply_edits instead — those produce diff-friendly history."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["file", "content"],
            "properties": {
                "file":    { "type": "string", "description": "path relative to workspace root" },
                "content": { "type": "string" }
            }
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Mutating
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let args: FsWriteArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::Err(format!("invalid args: {e}")),
        };

        let workspace = workspace_root(ctx);
        let relative = Path::new(&args.file);
        if let Some(parent) = relative.parent() {
            let abs_parent = workspace.join(parent);
            if !parent.as_os_str().is_empty() {
                if let Err(e) = std::fs::create_dir_all(&abs_parent) {
                    return ToolResult::Err(format!("mkdir failed: {e}"));
                }
            }
        }

        let absolute = match resolve_in_workspace(&workspace, relative) {
            Ok(p) => p,
            Err(e) => return ToolResult::Err(e),
        };
        let created = !absolute.exists();

        if absolute.extension().and_then(|s| s.to_str()) == Some("rs") {
            if let Err(e) = syn::parse_file(&args.content) {
                return ToolResult::Err(format!("syn parse failed; refused to write: {e}"));
            }
        }

        let temp = sibling_temp_path(&absolute);
        if let Err(e) = std::fs::write(&temp, args.content.as_bytes()) {
            return ToolResult::Err(format!("write temp failed: {e}"));
        }
        if let Err(e) = std::fs::rename(&temp, &absolute) {
            let _ = std::fs::remove_file(&temp);
            return ToolResult::Err(format!("rename failed: {e}"));
        }

        ToolResult::Ok(json!({
            "file": args.file,
            "bytes_written": args.content.len(),
            "created": created,
        }))
    }
}

// ----- glob -------------------------------------------------------------

pub struct Glob;

#[derive(Deserialize)]
struct GlobArgs {
    pattern: String,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    case_insensitive: Option<bool>,
}

#[async_trait]
impl Tool for Glob {
    fn name(&self) -> &str {
        "glob"
    }
    fn description(&self) -> &str {
        "List workspace files matching a glob pattern (e.g. `crates/**/*.rs`). Honours .gitignore. Returns paths relative to workspace root, sorted."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["pattern"],
            "properties": {
                "pattern":   { "type": "string", "description": "e.g. crates/**/*.rs" },
                "limit":     { "type": "integer", "default": 200, "maximum": 5000 },
                "case_insensitive": { "type": "boolean", "default": false }
            }
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::ReadOnly
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let args: GlobArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::Err(format!("invalid args: {e}")),
        };
        let limit = args.limit.unwrap_or(200).min(5000);
        let case_insensitive = args.case_insensitive.unwrap_or(false);

        let matcher = match GlobsetGlob::new(&args.pattern) {
            Ok(g) => {
                if case_insensitive {
                    GlobsetGlob::new(&format!("{{,**/}}{{,**/}}{}", args.pattern.to_lowercase()))
                        .map(|g| g.compile_matcher())
                        .unwrap_or_else(|_| g.compile_matcher())
                } else {
                    g.compile_matcher()
                }
            }
            Err(e) => return ToolResult::Err(format!("invalid glob pattern: {e}")),
        };

        let workspace = workspace_root(ctx);
        let canonical_root = match dunce::canonicalize(&workspace) {
            Ok(p) => p,
            Err(e) => return ToolResult::Err(format!("workspace canonicalize failed: {e}")),
        };

        let mut paths = collect_paths(&canonical_root, &matcher, limit, case_insensitive);
        paths.sort();
        let truncated = paths.len() == limit;

        ToolResult::Ok(json!({
            "paths": paths,
            "truncated": truncated,
            "count": paths.len(),
        }))
    }
}

fn collect_paths(
    root: &Path,
    matcher: &GlobMatcher,
    limit: usize,
    case_insensitive: bool,
) -> Vec<String> {
    let mut out = Vec::new();
    for entry in WalkBuilder::new(root).follow_links(false).build() {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }
        let rel = match entry.path().strip_prefix(root) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let rel_str = path_with_forward_slashes(rel);
        let test_str = if case_insensitive {
            rel_str.to_lowercase()
        } else {
            rel_str.clone()
        };
        if matcher.is_match(&test_str) {
            out.push(rel_str);
            if out.len() >= limit {
                break;
            }
        }
    }
    out
}

// ----- grep -------------------------------------------------------------

pub struct Grep;

#[derive(Deserialize)]
struct GrepArgs {
    pattern: String,
    #[serde(default)]
    path_glob: Option<String>,
    #[serde(default)]
    case_insensitive: Option<bool>,
    #[serde(default)]
    context: Option<usize>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(serde::Serialize)]
struct GrepMatch {
    file: String,
    line: u64,
    column: u64,
    text: String,
}

#[async_trait]
impl Tool for Grep {
    fn name(&self) -> &str {
        "grep"
    }
    fn description(&self) -> &str {
        "Search the workspace for a Rust-regex pattern (line-anchored matches). Backed by the ripgrep engine; honours .gitignore. Optional `path_glob` to restrict scope, `context` lines, `case_insensitive`."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["pattern"],
            "properties": {
                "pattern":     { "type": "string", "description": "regex (Rust regex crate syntax)" },
                "path_glob":   { "type": "string", "description": "limit to files matching this glob" },
                "case_insensitive": { "type": "boolean", "default": false },
                "context":     { "type": "integer", "default": 0, "maximum": 5 },
                "limit":       { "type": "integer", "default": 200, "maximum": 5000 }
            }
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::ReadOnly
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let args: GrepArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::Err(format!("invalid args: {e}")),
        };
        let limit = args.limit.unwrap_or(200).min(5000);
        let context = args.context.unwrap_or(0).min(5);
        let case_insensitive = args.case_insensitive.unwrap_or(false);

        let matcher = match RegexMatcherBuilder::new()
            .case_insensitive(case_insensitive)
            .build(&args.pattern)
        {
            Ok(m) => m,
            Err(e) => return ToolResult::Err(format!("invalid regex: {e}")),
        };

        let workspace = workspace_root(ctx);
        let canonical_root = match dunce::canonicalize(&workspace) {
            Ok(p) => p,
            Err(e) => return ToolResult::Err(format!("workspace canonicalize failed: {e}")),
        };

        let path_matcher = match args.path_glob.as_deref() {
            Some(p) => match GlobsetGlob::new(p) {
                Ok(g) => Some(g.compile_matcher()),
                Err(e) => return ToolResult::Err(format!("invalid path_glob: {e}")),
            },
            None => None,
        };

        let mut searcher = SearcherBuilder::new()
            .binary_detection(BinaryDetection::quit(b'\x00'))
            .before_context(context)
            .after_context(context)
            .build();

        let mut matches: Vec<GrepMatch> = Vec::new();
        'walk: for entry in WalkBuilder::new(&canonical_root).follow_links(false).build() {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            if !entry.file_type().is_some_and(|ft| ft.is_file()) {
                continue;
            }
            let rel = match entry.path().strip_prefix(&canonical_root) {
                Ok(p) => p,
                Err(_) => continue,
            };
            let rel_str = path_with_forward_slashes(rel);
            if let Some(m) = &path_matcher {
                if !m.is_match(&rel_str) {
                    continue;
                }
            }

            let path = entry.path().to_path_buf();
            let result = searcher.search_path(
                &matcher,
                &path,
                UTF8(|line_num, line| {
                    let col_byte = matcher
                        .find(line.as_bytes())
                        .ok()
                        .flatten()
                        .map(|m| m.start())
                        .unwrap_or(0);
                    matches.push(GrepMatch {
                        file: rel_str.clone(),
                        line: line_num,
                        column: col_byte as u64 + 1,
                        text: line.trim_end_matches('\n').to_string(),
                    });
                    Ok(matches.len() < limit)
                }),
            );
            if let Err(e) = result {
                tracing::debug!("grep skipped {}: {e}", rel_str);
            }
            if matches.len() >= limit {
                break 'walk;
            }
        }

        let truncated = matches.len() == limit;
        ToolResult::Ok(json!({
            "matches": matches,
            "truncated": truncated,
            "count": matches.len(),
        }))
    }
}

// ----- helpers ----------------------------------------------------------

fn workspace_root(ctx: &ToolContext) -> PathBuf {
    PathBuf::from(ctx.workspace_root.as_std_path())
}

fn resolve_in_workspace(workspace_root: &Path, relative: &Path) -> Result<PathBuf, String> {
    let joined = workspace_root.join(relative);
    let canonical_root = dunce::canonicalize(workspace_root)
        .map_err(|e| format!("workspace canonicalize failed: {e}"))?;
    let parent = joined.parent().unwrap_or(workspace_root);
    let canonical_parent = dunce::canonicalize(parent)
        .map_err(|e| format!("path {} not found: {e}", relative.display()))?;
    if !canonical_parent.starts_with(&canonical_root) {
        return Err(format!("path escapes workspace root: {}", relative.display()));
    }
    let filename = joined
        .file_name()
        .ok_or_else(|| format!("invalid path: {}", relative.display()))?;
    Ok(canonical_parent.join(filename))
}

fn is_utf8_text(bytes: &[u8]) -> bool {
    // Treat embedded NULs as binary signal (matches ripgrep convention).
    if bytes.contains(&0) {
        return false;
    }
    std::str::from_utf8(bytes).is_ok()
}

fn path_with_forward_slashes(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

fn sibling_temp_path(path: &Path) -> PathBuf {
    let mut p = path.as_os_str().to_owned();
    p.push(".oxidant-tmp");
    PathBuf::from(p)
}


#[cfg(test)]
mod tests {
    use super::*;
    use camino::Utf8PathBuf;
    use tempfile::TempDir;
    use tokio_util::sync::CancellationToken;

    fn ctx_for(dir: &Path) -> ToolContext {
        ToolContext {
            workspace_root: Utf8PathBuf::from_path_buf(dunce::canonicalize(dir).unwrap()).unwrap(),
            exploration_id: "test".into(),
            cancellation: CancellationToken::new(),
        }
    }

    #[tokio::test]
    async fn fs_read_whole_file() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello\nworld\n").unwrap();
        let result = FsRead.invoke(json!({"file": "a.txt"}), &ctx_for(dir.path())).await;
        let v = match result {
            ToolResult::Ok(v) => v,
            ToolResult::Err(e) => panic!("err: {e}"),
        };
        assert_eq!(v["content"], "hello\nworld\n");
        assert_eq!(v["lines"], 2);
        assert_eq!(v["binary"], false);
    }

    #[tokio::test]
    async fn fs_read_with_offset_and_limit() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.txt"), "one\ntwo\nthree\nfour\nfive\n").unwrap();
        let v = match FsRead
            .invoke(
                json!({"file": "a.txt", "offset": 2, "limit": 2}),
                &ctx_for(dir.path()),
            )
            .await
        {
            ToolResult::Ok(v) => v,
            ToolResult::Err(e) => panic!("err: {e}"),
        };
        assert_eq!(v["content"], "two\nthree");
        assert_eq!(v["lines"], 5);
    }

    #[tokio::test]
    async fn fs_read_binary_returns_marker() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("bin"), [0u8, 1, 2, 3, 0xFF]).unwrap();
        let v = match FsRead.invoke(json!({"file": "bin"}), &ctx_for(dir.path())).await {
            ToolResult::Ok(v) => v,
            ToolResult::Err(e) => panic!("err: {e}"),
        };
        assert_eq!(v["binary"], true);
        assert!(v.get("content").is_none());
    }

    #[tokio::test]
    async fn fs_read_rejects_escape() {
        let dir = TempDir::new().unwrap();
        let result = FsRead
            .invoke(
                json!({"file": "../../../etc/passwd"}),
                &ctx_for(dir.path()),
            )
            .await;
        assert!(matches!(result, ToolResult::Err(_)));
    }

    #[tokio::test]
    async fn fs_write_creates_then_overwrites() {
        let dir = TempDir::new().unwrap();
        // first call creates
        let v = match FsWrite
            .invoke(
                json!({"file": "n.txt", "content": "alpha"}),
                &ctx_for(dir.path()),
            )
            .await
        {
            ToolResult::Ok(v) => v,
            ToolResult::Err(e) => panic!("err: {e}"),
        };
        assert_eq!(v["created"], true);
        assert_eq!(std::fs::read_to_string(dir.path().join("n.txt")).unwrap(), "alpha");

        // second call overwrites
        let v = match FsWrite
            .invoke(
                json!({"file": "n.txt", "content": "beta"}),
                &ctx_for(dir.path()),
            )
            .await
        {
            ToolResult::Ok(v) => v,
            ToolResult::Err(e) => panic!("err: {e}"),
        };
        assert_eq!(v["created"], false);
        assert_eq!(std::fs::read_to_string(dir.path().join("n.txt")).unwrap(), "beta");
    }

    #[tokio::test]
    async fn fs_write_rejects_invalid_rust() {
        let dir = TempDir::new().unwrap();
        let result = FsWrite
            .invoke(
                json!({"file": "x.rs", "content": "fn broken("}),
                &ctx_for(dir.path()),
            )
            .await;
        assert!(matches!(result, ToolResult::Err(_)));
        assert!(!dir.path().join("x.rs").exists());
    }

    #[tokio::test]
    async fn glob_finds_files() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/a.rs"), "//").unwrap();
        std::fs::write(dir.path().join("src/b.rs"), "//").unwrap();
        std::fs::write(dir.path().join("src/c.txt"), "//").unwrap();
        let v = match Glob
            .invoke(json!({"pattern": "src/*.rs"}), &ctx_for(dir.path()))
            .await
        {
            ToolResult::Ok(v) => v,
            ToolResult::Err(e) => panic!("err: {e}"),
        };
        let paths = v["paths"].as_array().unwrap();
        assert_eq!(paths.len(), 2);
        assert!(paths.iter().any(|p| p == "src/a.rs"));
        assert!(paths.iter().any(|p| p == "src/b.rs"));
    }

    #[tokio::test]
    async fn grep_finds_matches_with_line_and_text() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.txt"), "first\nfn target() {}\nthird\n").unwrap();
        std::fs::write(dir.path().join("b.txt"), "no match here").unwrap();
        let v = match Grep
            .invoke(json!({"pattern": "fn\\s+\\w+"}), &ctx_for(dir.path()))
            .await
        {
            ToolResult::Ok(v) => v,
            ToolResult::Err(e) => panic!("err: {e}"),
        };
        let matches = v["matches"].as_array().unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0]["file"], "a.txt");
        assert_eq!(matches[0]["line"], 2);
        assert!(matches[0]["text"].as_str().unwrap().contains("fn target"));
    }

    #[tokio::test]
    async fn grep_respects_path_glob() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("included.rs"), "needle\n").unwrap();
        std::fs::write(dir.path().join("ignored.txt"), "needle\n").unwrap();
        let v = match Grep
            .invoke(
                json!({"pattern": "needle", "path_glob": "*.rs"}),
                &ctx_for(dir.path()),
            )
            .await
        {
            ToolResult::Ok(v) => v,
            ToolResult::Err(e) => panic!("err: {e}"),
        };
        let matches = v["matches"].as_array().unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0]["file"], "included.rs");
    }
}
