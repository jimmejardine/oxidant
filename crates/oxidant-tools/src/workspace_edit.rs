// Realises spec/contracts/workspace-edit.md (types) and
// spec/components/tools/workspace-edit-substrate.md (apply path),
// plus spec/invariants/{edits-are-atomic,rust-files-parse-after-edit}.md.
//
// The substrate is private to oxidant-tools — model-facing edit tools
// (edit_string, apply_edits) and smart-tool refactors (LSP rename, syn
// transforms, clippy-fix) all route through `apply()`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct WorkspaceEdit {
    pub changes: HashMap<PathBuf, Vec<TextEdit>>,
}

#[derive(Debug, Clone)]
pub struct TextEdit {
    pub range: Range,
    pub new_text: String,
    /// Optional optimistic-concurrency check: if set, the current bytes at
    /// `range` must match this string or the substrate aborts the edit.
    pub expected_text: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

#[derive(Debug, Clone, Copy)]
pub struct Position {
    /// 0-indexed line number.
    pub line: u32,
    /// 0-indexed UTF-16 code unit offset within the line (LSP convention).
    /// The substrate converts to byte offsets once at apply time.
    pub character: u32,
}

#[derive(Debug, Clone)]
pub struct ApplyResult {
    pub files: Vec<FileApplyResult>,
}

#[derive(Debug, Clone)]
pub struct FileApplyResult {
    pub path: PathBuf,
    pub edits_applied: usize,
    /// Byte-range positions of each edit AFTER application, in the order
    /// edits were supplied. Useful for chained edits within one agent turn.
    pub post_edit_byte_ranges: Vec<ByteRange>,
}

#[derive(Debug, Clone, Copy)]
pub struct ByteRange {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum ApplyError {
    #[error("path escapes workspace root: {0}")]
    PathEscapesWorkspace(PathBuf),
    #[error("file does not exist: {0}")]
    FileNotFound(PathBuf),
    #[error("range outside file bounds in {file}: line {line} char {character}")]
    RangeOutOfBounds {
        file: PathBuf,
        line: u32,
        character: u32,
    },
    #[error("overlapping edits in {file}: [{a_start}..{a_end}] and [{b_start}..{b_end}]")]
    OverlappingEdits {
        file: PathBuf,
        a_start: usize,
        a_end: usize,
        b_start: usize,
        b_end: usize,
    },
    #[error("expected_text mismatch in {file}: expected {expected:?}, found {actual:?}")]
    ExpectedTextMismatch {
        file: PathBuf,
        expected: String,
        actual: String,
    },
    #[error("syn parse failed after edit in {file}: {message}")]
    SynParseFailed { file: PathBuf, message: String },
    #[error("io error on {file}: {source}")]
    Io {
        file: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("the file is not valid UTF-8: {0}")]
    FileNotUtf8(PathBuf),
}

/// Apply a WorkspaceEdit atomically. See the component spec for the full
/// behaviour list — in short: normalize ranges, sort descending, check
/// overlaps and expected_text, syn-validate touched `.rs` files in-memory,
/// then temp-file + rename per file with rollback on partial failure.
pub fn apply(workspace_root: &Path, edit: WorkspaceEdit) -> Result<ApplyResult, ApplyError> {
    if edit.changes.is_empty() {
        return Ok(ApplyResult { files: Vec::new() });
    }

    let mut plans: Vec<FilePlan> = Vec::with_capacity(edit.changes.len());
    for (rel_path, edits) in edit.changes {
        let plan = prepare_file(workspace_root, rel_path, edits)?;
        plans.push(plan);
    }

    // Atomically rename all temp files into place. If any rename fails,
    // restore the originals from the backups we held aside.
    let mut completed: Vec<CompletedRename> = Vec::with_capacity(plans.len());
    for plan in &plans {
        match rename_into_place(plan) {
            Ok(completed_rename) => completed.push(completed_rename),
            Err(e) => {
                for rolled in completed.iter().rev() {
                    let _ = std::fs::rename(&rolled.backup_path, &rolled.final_path);
                }
                cleanup_temp_files(&plans);
                return Err(e);
            }
        }
    }

    // Success — delete backups.
    for rolled in &completed {
        let _ = std::fs::remove_file(&rolled.backup_path);
    }

    let files = plans
        .into_iter()
        .map(|p| FileApplyResult {
            path: p.relative_path,
            edits_applied: p.applied_edits,
            post_edit_byte_ranges: p.post_edit_byte_ranges,
        })
        .collect();
    Ok(ApplyResult { files })
}

struct FilePlan {
    relative_path: PathBuf,
    absolute_path: PathBuf,
    temp_path: PathBuf,
    applied_edits: usize,
    post_edit_byte_ranges: Vec<ByteRange>,
}

struct CompletedRename {
    final_path: PathBuf,
    backup_path: PathBuf,
}

fn prepare_file(
    workspace_root: &Path,
    relative_path: PathBuf,
    edits: Vec<TextEdit>,
) -> Result<FilePlan, ApplyError> {
    let absolute = resolve_in_workspace(workspace_root, &relative_path)?;
    if !absolute.exists() {
        return Err(ApplyError::FileNotFound(relative_path));
    }

    let original_bytes = std::fs::read(&absolute).map_err(|e| ApplyError::Io {
        file: relative_path.clone(),
        source: e,
    })?;
    let original = std::str::from_utf8(&original_bytes)
        .map_err(|_| ApplyError::FileNotUtf8(relative_path.clone()))?
        .to_string();

    // Convert each LSP-style range to a byte range using the file's actual contents.
    let mut byte_edits: Vec<ByteEdit> = Vec::with_capacity(edits.len());
    for (idx, edit) in edits.into_iter().enumerate() {
        let start_byte = position_to_byte(&original, edit.range.start).ok_or_else(|| {
            ApplyError::RangeOutOfBounds {
                file: relative_path.clone(),
                line: edit.range.start.line,
                character: edit.range.start.character,
            }
        })?;
        let end_byte = position_to_byte(&original, edit.range.end).ok_or_else(|| {
            ApplyError::RangeOutOfBounds {
                file: relative_path.clone(),
                line: edit.range.end.line,
                character: edit.range.end.character,
            }
        })?;
        byte_edits.push(ByteEdit {
            original_index: idx,
            start: start_byte,
            end: end_byte,
            new_text: edit.new_text,
            expected_text: edit.expected_text,
        });
    }

    // Sort by ascending start for overlap check, then again descending for application.
    byte_edits.sort_by_key(|e| (e.start, e.end));
    for window in byte_edits.windows(2) {
        let a = &window[0];
        let b = &window[1];
        if a.end > b.start {
            return Err(ApplyError::OverlappingEdits {
                file: relative_path,
                a_start: a.start,
                a_end: a.end,
                b_start: b.start,
                b_end: b.end,
            });
        }
    }

    // Expected_text check on the ORIGINAL bytes — must hold before any edit applied.
    for edit in &byte_edits {
        if let Some(expected) = &edit.expected_text {
            let actual = &original[edit.start..edit.end];
            if actual != expected {
                return Err(ApplyError::ExpectedTextMismatch {
                    file: relative_path,
                    expected: expected.clone(),
                    actual: actual.to_string(),
                });
            }
        }
    }

    // Apply in descending order so each earlier edit's byte range stays valid.
    let mut byte_edits_desc = byte_edits.clone();
    byte_edits_desc.sort_by_key(|e| std::cmp::Reverse(e.start));
    let mut buf = original.into_bytes();
    for edit in &byte_edits_desc {
        buf.splice(edit.start..edit.end, edit.new_text.bytes());
    }
    let new_content =
        String::from_utf8(buf).map_err(|_| ApplyError::FileNotUtf8(relative_path.clone()))?;

    // Compute post-edit byte ranges in original order by re-tracing positions.
    // For an edit at original byte range [s, e) with new_text n, its post-edit
    // range is [s + sum_of_size_changes_for_edits_with_start<s, that + n.len()).
    let post_edit_byte_ranges = compute_post_edit_ranges(&byte_edits);

    // .rs files must parse with syn after the edit, or we refuse the change.
    if absolute.extension().and_then(|s| s.to_str()) == Some("rs")
        && let Err(e) = syn::parse_file(&new_content)
    {
        return Err(ApplyError::SynParseFailed {
            file: relative_path,
            message: e.to_string(),
        });
    }

    // Write the prospective new content to a sibling temp file.
    let temp_path = sibling_temp(&absolute);
    std::fs::write(&temp_path, new_content.as_bytes()).map_err(|e| ApplyError::Io {
        file: relative_path.clone(),
        source: e,
    })?;

    Ok(FilePlan {
        relative_path,
        absolute_path: absolute,
        temp_path,
        applied_edits: byte_edits.len(),
        post_edit_byte_ranges,
    })
}

fn rename_into_place(plan: &FilePlan) -> Result<CompletedRename, ApplyError> {
    let backup = sibling_backup(&plan.absolute_path);
    std::fs::rename(&plan.absolute_path, &backup).map_err(|e| ApplyError::Io {
        file: plan.relative_path.clone(),
        source: e,
    })?;
    if let Err(e) = std::fs::rename(&plan.temp_path, &plan.absolute_path) {
        // restore original; the outer caller will roll back already-completed ones.
        let _ = std::fs::rename(&backup, &plan.absolute_path);
        return Err(ApplyError::Io {
            file: plan.relative_path.clone(),
            source: e,
        });
    }
    Ok(CompletedRename {
        final_path: plan.absolute_path.clone(),
        backup_path: backup,
    })
}

fn cleanup_temp_files(plans: &[FilePlan]) {
    for plan in plans {
        let _ = std::fs::remove_file(&plan.temp_path);
    }
}

#[derive(Clone)]
struct ByteEdit {
    original_index: usize,
    start: usize,
    end: usize,
    new_text: String,
    expected_text: Option<String>,
}

fn compute_post_edit_ranges(edits: &[ByteEdit]) -> Vec<ByteRange> {
    // Walk edits in ascending start order, tracking cumulative shift.
    let mut asc = edits.to_vec();
    asc.sort_by_key(|e| e.start);
    let mut ranges_in_source_order: Vec<(usize, ByteRange)> = Vec::with_capacity(edits.len());
    let mut cumulative_shift: isize = 0;
    for edit in &asc {
        let new_start = (edit.start as isize + cumulative_shift) as usize;
        let new_end = new_start + edit.new_text.len();
        ranges_in_source_order.push((
            edit.original_index,
            ByteRange {
                start: new_start,
                end: new_end,
            },
        ));
        cumulative_shift += edit.new_text.len() as isize - (edit.end - edit.start) as isize;
    }
    ranges_in_source_order.sort_by_key(|(idx, _)| *idx);
    ranges_in_source_order.into_iter().map(|(_, r)| r).collect()
}

fn resolve_in_workspace(workspace_root: &Path, relative: &Path) -> Result<PathBuf, ApplyError> {
    let joined = workspace_root.join(relative);
    let parent_for_canonicalize = joined.parent().unwrap_or(workspace_root);
    // Canonicalize the parent (must exist) then push the filename back, so the
    // check works even when we're creating a file via fs_write later.
    let canonical_root = dunce::canonicalize(workspace_root).map_err(|e| ApplyError::Io {
        file: relative.to_path_buf(),
        source: e,
    })?;
    let canonical_parent =
        dunce::canonicalize(parent_for_canonicalize).map_err(|e| ApplyError::Io {
            file: relative.to_path_buf(),
            source: e,
        })?;
    if !canonical_parent.starts_with(&canonical_root) {
        return Err(ApplyError::PathEscapesWorkspace(relative.to_path_buf()));
    }
    let filename = joined
        .file_name()
        .ok_or_else(|| ApplyError::PathEscapesWorkspace(relative.to_path_buf()))?;
    Ok(canonical_parent.join(filename))
}

/// Convert an LSP-style (line, UTF-16 char) Position into a byte offset
/// within `source`. Returns None if line/character is past the end.
fn position_to_byte(source: &str, pos: Position) -> Option<usize> {
    let mut current_line: u32 = 0;
    let mut byte_at_line_start: usize = 0;
    let bytes = source.as_bytes();
    while current_line < pos.line {
        let next_newline = bytes[byte_at_line_start..].iter().position(|&b| b == b'\n');
        match next_newline {
            Some(rel) => {
                byte_at_line_start += rel + 1;
                current_line += 1;
            }
            None => {
                // Allow pointing at one-past-the-last-line if character is 0
                if current_line + 1 == pos.line && pos.character == 0 {
                    return Some(bytes.len());
                }
                return None;
            }
        }
    }

    // Walk the line in chars, accumulating UTF-16 code units until we hit pos.character.
    let line_slice = &source[byte_at_line_start..];
    let line_end_byte = line_slice
        .as_bytes()
        .iter()
        .position(|&b| b == b'\n')
        .unwrap_or(line_slice.len());
    let mut byte_within_line = 0;
    let mut utf16_count: u32 = 0;
    for ch in line_slice[..line_end_byte].chars() {
        if utf16_count == pos.character {
            return Some(byte_at_line_start + byte_within_line);
        }
        utf16_count += ch.len_utf16() as u32;
        byte_within_line += ch.len_utf8();
    }
    if utf16_count == pos.character {
        return Some(byte_at_line_start + byte_within_line);
    }
    None
}

fn sibling_temp(path: &Path) -> PathBuf {
    let mut p = path.as_os_str().to_owned();
    p.push(".oxidant-tmp");
    PathBuf::from(p)
}

fn sibling_backup(path: &Path) -> PathBuf {
    let mut p = path.as_os_str().to_owned();
    p.push(".oxidant-bak");
    PathBuf::from(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn pos(line: u32, ch: u32) -> Position {
        Position {
            line,
            character: ch,
        }
    }

    fn rng(s: Position, e: Position) -> Range {
        Range { start: s, end: e }
    }

    #[test]
    fn position_to_byte_ascii() {
        let src = "abc\ndefg\nhi";
        assert_eq!(position_to_byte(src, pos(0, 0)), Some(0));
        assert_eq!(position_to_byte(src, pos(0, 3)), Some(3));
        assert_eq!(position_to_byte(src, pos(1, 0)), Some(4));
        assert_eq!(position_to_byte(src, pos(1, 4)), Some(8));
        assert_eq!(position_to_byte(src, pos(2, 0)), Some(9));
        assert_eq!(position_to_byte(src, pos(2, 2)), Some(11));
    }

    #[test]
    fn position_to_byte_utf16_surrogate_pair() {
        // 😀 (U+1F600) is one Rust char, 4 UTF-8 bytes, 2 UTF-16 code units.
        let src = "a😀b";
        assert_eq!(position_to_byte(src, pos(0, 0)), Some(0));
        assert_eq!(position_to_byte(src, pos(0, 1)), Some(1));
        assert_eq!(position_to_byte(src, pos(0, 3)), Some(5));
    }

    #[test]
    fn apply_single_edit_in_place() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("hello.txt");
        std::fs::write(&file, "hello world\n").unwrap();

        let mut changes = HashMap::new();
        changes.insert(
            PathBuf::from("hello.txt"),
            vec![TextEdit {
                range: rng(pos(0, 6), pos(0, 11)),
                new_text: "Rust".into(),
                expected_text: Some("world".into()),
            }],
        );
        let result = apply(dir.path(), WorkspaceEdit { changes }).unwrap();
        assert_eq!(result.files.len(), 1);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "hello Rust\n");
    }

    #[test]
    fn apply_rejects_overlapping_edits() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("x.txt");
        std::fs::write(&file, "abcdef").unwrap();
        let mut changes = HashMap::new();
        changes.insert(
            PathBuf::from("x.txt"),
            vec![
                TextEdit {
                    range: rng(pos(0, 0), pos(0, 3)),
                    new_text: "A".into(),
                    expected_text: None,
                },
                TextEdit {
                    range: rng(pos(0, 2), pos(0, 5)),
                    new_text: "B".into(),
                    expected_text: None,
                },
            ],
        );
        let err = apply(dir.path(), WorkspaceEdit { changes }).unwrap_err();
        assert!(matches!(err, ApplyError::OverlappingEdits { .. }));
        // file untouched
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "abcdef");
    }

    #[test]
    fn apply_rejects_expected_text_mismatch() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("x.txt");
        std::fs::write(&file, "hello").unwrap();
        let mut changes = HashMap::new();
        changes.insert(
            PathBuf::from("x.txt"),
            vec![TextEdit {
                range: rng(pos(0, 0), pos(0, 5)),
                new_text: "HELLO".into(),
                expected_text: Some("HELLO".into()), // mismatch
            }],
        );
        let err = apply(dir.path(), WorkspaceEdit { changes }).unwrap_err();
        assert!(matches!(err, ApplyError::ExpectedTextMismatch { .. }));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "hello");
    }

    #[test]
    fn apply_rust_file_with_syn_failure_rolls_back() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("a.rs");
        std::fs::write(&file, "fn main() {}\n").unwrap();
        let mut changes = HashMap::new();
        changes.insert(
            PathBuf::from("a.rs"),
            vec![TextEdit {
                range: rng(pos(0, 0), pos(0, 12)),
                new_text: "fn broken( {".into(), // unmatched paren
                expected_text: None,
            }],
        );
        let err = apply(dir.path(), WorkspaceEdit { changes }).unwrap_err();
        assert!(matches!(err, ApplyError::SynParseFailed { .. }));
        // file untouched
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "fn main() {}\n");
    }

    #[test]
    fn apply_multi_file_atomicity() {
        let dir = TempDir::new().unwrap();
        let a = dir.path().join("a.txt");
        let b = dir.path().join("b.rs");
        std::fs::write(&a, "before-a\n").unwrap();
        std::fs::write(&b, "fn main() {}\n").unwrap();

        let mut changes = HashMap::new();
        changes.insert(
            PathBuf::from("a.txt"),
            vec![TextEdit {
                range: rng(pos(0, 0), pos(0, 8)),
                new_text: "after-a-".into(),
                expected_text: Some("before-a".into()),
            }],
        );
        changes.insert(
            PathBuf::from("b.rs"),
            vec![TextEdit {
                range: rng(pos(0, 0), pos(0, 12)),
                new_text: "fn main(){".into(), // breaks parse
                expected_text: None,
            }],
        );
        let err = apply(dir.path(), WorkspaceEdit { changes }).unwrap_err();
        assert!(matches!(err, ApplyError::SynParseFailed { .. }));
        // both files untouched
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "before-a\n");
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "fn main() {}\n");
    }

    #[test]
    fn post_edit_byte_ranges_track_shifts() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("x.txt");
        std::fs::write(&file, "AABBCC").unwrap();

        let mut changes = HashMap::new();
        changes.insert(
            PathBuf::from("x.txt"),
            vec![
                TextEdit {
                    range: rng(pos(0, 0), pos(0, 2)),
                    new_text: "xxxx".into(),
                    expected_text: None,
                },
                TextEdit {
                    range: rng(pos(0, 4), pos(0, 6)),
                    new_text: "y".into(),
                    expected_text: None,
                },
            ],
        );
        let result = apply(dir.path(), WorkspaceEdit { changes }).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "xxxxBBy");
        let ranges = &result.files[0].post_edit_byte_ranges;
        assert_eq!(ranges[0].start, 0);
        assert_eq!(ranges[0].end, 4);
        assert_eq!(ranges[1].start, 6);
        assert_eq!(ranges[1].end, 7);
    }
}
