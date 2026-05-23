// Realises spec/tools/edit/edit-string.md and spec/tools/edit/apply-edits.md.
//
// Both tools build a WorkspaceEdit and route through the substrate; all
// atomicity, expected_text checks, and syn parse validation live there.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use oxidant_core::{Tool, ToolCategory, ToolContext, ToolResult};

use crate::workspace_edit::{
    ApplyError, ApplyResult, Position, Range, TextEdit, WorkspaceEdit, apply,
};

// ----- edit_string ------------------------------------------------------

pub struct EditString;

#[derive(Deserialize)]
struct EditStringArgs {
    file: String,
    old_string: String,
    new_string: String,
    #[serde(default)]
    replace_all: Option<bool>,
}

#[async_trait]
impl Tool for EditString {
    fn name(&self) -> &str {
        "edit_string"
    }
    fn description(&self) -> &str {
        "Replace one (or all, with replace_all=true) occurrences of `old_string` with `new_string` in `file`. Errors if `old_string` is missing, or matches multiple times unless replace_all is set. Atomic; .rs files must still parse after the edit."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["file", "old_string", "new_string"],
            "properties": {
                "file":        { "type": "string" },
                "old_string":  { "type": "string", "minLength": 1 },
                "new_string":  { "type": "string" },
                "replace_all": { "type": "boolean", "default": false }
            }
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Mutating
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let args: EditStringArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::Err(format!("invalid args: {e}")),
        };
        let replace_all = args.replace_all.unwrap_or(false);

        let workspace = PathBuf::from(ctx.workspace_root.as_std_path());
        let absolute = workspace.join(&args.file);
        let content = match std::fs::read_to_string(&absolute) {
            Ok(c) => c,
            Err(e) => return ToolResult::Err(format!("read {} failed: {e}", args.file)),
        };

        let matches: Vec<usize> = byte_positions_of(&content, &args.old_string);
        if matches.is_empty() {
            return ToolResult::Err(format!(
                "`old_string` not found in {} (searched {} bytes)",
                args.file,
                content.len()
            ));
        }
        if matches.len() > 1 && !replace_all {
            return ToolResult::Err(format!(
                "`old_string` matched {} times in {}; pass replace_all=true to replace every occurrence",
                matches.len(),
                args.file
            ));
        }

        let mut edits = Vec::with_capacity(matches.len());
        for byte_start in &matches {
            let byte_end = byte_start + args.old_string.len();
            let start = byte_offset_to_position(&content, *byte_start);
            let end = byte_offset_to_position(&content, byte_end);
            edits.push(TextEdit {
                range: Range { start, end },
                new_text: args.new_string.clone(),
                expected_text: Some(args.old_string.clone()),
            });
        }

        let mut changes = HashMap::new();
        changes.insert(PathBuf::from(&args.file), edits);
        let result = apply(&workspace, WorkspaceEdit { changes });
        format_result(&args.file, result)
    }
}

// ----- apply_edits ------------------------------------------------------

pub struct ApplyEdits;

#[derive(Deserialize)]
struct ApplyEditsArgs {
    edits: Vec<EditEntry>,
}

#[derive(Deserialize)]
struct EditEntry {
    file: String,
    range: RangeJson,
    new_text: String,
    #[serde(default)]
    expected_text: Option<String>,
}

#[derive(Deserialize)]
struct RangeJson {
    start: PositionJson,
    end: PositionJson,
}

#[derive(Deserialize)]
struct PositionJson {
    line: u32,
    character: u32,
}

#[async_trait]
impl Tool for ApplyEdits {
    fn name(&self) -> &str {
        "apply_edits"
    }
    fn description(&self) -> &str {
        "Apply one or more span-precise text edits across one or more files, atomically. Ranges follow LSP convention (0-indexed line, 0-indexed UTF-16 character) so spans from cargo_check, rust_hover, syn queries paste in directly. Optional expected_text on each edit for optimistic-concurrency."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["edits"],
            "properties": {
                "edits": {
                    "type": "array",
                    "minItems": 1,
                    "items": {
                        "type": "object",
                        "required": ["file", "range", "new_text"],
                        "properties": {
                            "file":          { "type": "string" },
                            "range": {
                                "type": "object",
                                "required": ["start", "end"],
                                "properties": {
                                    "start": {
                                        "type": "object",
                                        "required": ["line", "character"],
                                        "properties": {
                                            "line": { "type": "integer", "minimum": 0 },
                                            "character": { "type": "integer", "minimum": 0 }
                                        }
                                    },
                                    "end": {
                                        "type": "object",
                                        "required": ["line", "character"],
                                        "properties": {
                                            "line": { "type": "integer", "minimum": 0 },
                                            "character": { "type": "integer", "minimum": 0 }
                                        }
                                    }
                                }
                            },
                            "new_text":      { "type": "string" },
                            "expected_text": { "type": "string" }
                        }
                    }
                }
            }
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Mutating
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let args: ApplyEditsArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::Err(format!("invalid args: {e}")),
        };
        if args.edits.is_empty() {
            return ToolResult::Err("`edits` must not be empty".into());
        }

        let mut changes: HashMap<PathBuf, Vec<TextEdit>> = HashMap::new();
        let mut file_order: Vec<String> = Vec::new();
        for entry in args.edits {
            let path = PathBuf::from(&entry.file);
            if !changes.contains_key(&path) {
                file_order.push(entry.file.clone());
            }
            changes.entry(path).or_default().push(TextEdit {
                range: Range {
                    start: Position {
                        line: entry.range.start.line,
                        character: entry.range.start.character,
                    },
                    end: Position {
                        line: entry.range.end.line,
                        character: entry.range.end.character,
                    },
                },
                new_text: entry.new_text,
                expected_text: entry.expected_text,
            });
        }

        let workspace = PathBuf::from(ctx.workspace_root.as_std_path());
        let result = apply(&workspace, WorkspaceEdit { changes });
        match result {
            Ok(r) => ToolResult::Ok(json!({
                "files": file_order.iter().filter_map(|f| {
                    let path_buf = PathBuf::from(f);
                    let file_result = r.files.iter().find(|fr| fr.path == path_buf)?;
                    Some(json!({
                        "file": f,
                        "edits_applied": file_result.edits_applied,
                        "post_edit_byte_ranges": file_result.post_edit_byte_ranges.iter().map(|br| json!({
                            "start": br.start,
                            "end": br.end,
                        })).collect::<Vec<_>>(),
                    }))
                }).collect::<Vec<_>>(),
                "total_files": r.files.len(),
                "total_edits": r.files.iter().map(|f| f.edits_applied).sum::<usize>(),
            })),
            Err(e) => ToolResult::Err(format_apply_error(&e)),
        }
    }
}

fn format_result(file: &str, result: Result<ApplyResult, ApplyError>) -> ToolResult {
    match result {
        Ok(r) => {
            let file_result = r.files.first();
            ToolResult::Ok(json!({
                "file": file,
                "replacements": file_result.map(|fr| fr.edits_applied).unwrap_or(0),
                "post_edit_byte_ranges": file_result
                    .map(|fr| fr.post_edit_byte_ranges.iter().map(|br| json!({
                        "start": br.start,
                        "end": br.end,
                    })).collect::<Vec<_>>())
                    .unwrap_or_default(),
            }))
        }
        Err(e) => ToolResult::Err(format_apply_error(&e)),
    }
}

fn format_apply_error(e: &ApplyError) -> String {
    e.to_string()
}

fn byte_positions_of(haystack: &str, needle: &str) -> Vec<usize> {
    let mut out = Vec::new();
    let mut start = 0;
    while let Some(rel) = haystack[start..].find(needle) {
        out.push(start + rel);
        start += rel + needle.len();
        if needle.is_empty() {
            break;
        }
    }
    out
}

fn byte_offset_to_position(source: &str, byte: usize) -> Position {
    let mut line: u32 = 0;
    let mut last_newline: usize = 0;
    for (i, b) in source.as_bytes()[..byte].iter().enumerate() {
        if *b == b'\n' {
            line += 1;
            last_newline = i + 1;
        }
    }
    let line_slice = &source[last_newline..byte];
    let character: u32 = line_slice.chars().map(|c| c.len_utf16() as u32).sum();
    Position { line, character }
}

#[allow(dead_code)]
fn _path_buf_from(p: &Path) -> PathBuf {
    p.to_path_buf()
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
    async fn edit_string_unique_replacement() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello world").unwrap();
        let v = match EditString
            .invoke(
                json!({"file": "a.txt", "old_string": "world", "new_string": "Rust"}),
                &ctx_for(dir.path()),
            )
            .await
        {
            ToolResult::Ok(v) => v,
            ToolResult::Err(e) => panic!("err: {e}"),
        };
        assert_eq!(v["replacements"], 1);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "hello Rust"
        );
    }

    #[tokio::test]
    async fn edit_string_rejects_ambiguous_match() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.txt"), "foo foo foo").unwrap();
        let result = EditString
            .invoke(
                json!({"file": "a.txt", "old_string": "foo", "new_string": "bar"}),
                &ctx_for(dir.path()),
            )
            .await;
        assert!(matches!(result, ToolResult::Err(_)));
        // file untouched
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "foo foo foo"
        );
    }

    #[tokio::test]
    async fn edit_string_replace_all() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.txt"), "foo foo foo").unwrap();
        let v = match EditString
            .invoke(
                json!({"file": "a.txt", "old_string": "foo", "new_string": "bar", "replace_all": true}),
                &ctx_for(dir.path()),
            )
            .await
        {
            ToolResult::Ok(v) => v,
            ToolResult::Err(e) => panic!("err: {e}"),
        };
        assert_eq!(v["replacements"], 3);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "bar bar bar"
        );
    }

    #[tokio::test]
    async fn edit_string_missing_returns_clear_error() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.txt"), "abc").unwrap();
        let result = EditString
            .invoke(
                json!({"file": "a.txt", "old_string": "xyz", "new_string": "q"}),
                &ctx_for(dir.path()),
            )
            .await;
        let err = match result {
            ToolResult::Err(e) => e,
            _ => panic!("expected err"),
        };
        assert!(err.contains("not found"));
    }

    #[tokio::test]
    async fn apply_edits_span_precise() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello world").unwrap();
        let v = match ApplyEdits
            .invoke(
                json!({
                    "edits": [
                        {
                            "file": "a.txt",
                            "range": {
                                "start": {"line": 0, "character": 6},
                                "end":   {"line": 0, "character": 11}
                            },
                            "new_text": "Rust",
                            "expected_text": "world"
                        }
                    ]
                }),
                &ctx_for(dir.path()),
            )
            .await
        {
            ToolResult::Ok(v) => v,
            ToolResult::Err(e) => panic!("err: {e}"),
        };
        assert_eq!(v["total_edits"], 1);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "hello Rust"
        );
    }

    #[tokio::test]
    async fn byte_offset_to_position_works_across_lines() {
        let src = "abc\ndefg\nhi";
        assert_eq!(byte_offset_to_position(src, 0).line, 0);
        assert_eq!(byte_offset_to_position(src, 0).character, 0);
        assert_eq!(byte_offset_to_position(src, 4).line, 1);
        assert_eq!(byte_offset_to_position(src, 4).character, 0);
        assert_eq!(byte_offset_to_position(src, 6).line, 1);
        assert_eq!(byte_offset_to_position(src, 6).character, 2);
    }
}
