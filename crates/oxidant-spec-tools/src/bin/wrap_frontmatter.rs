// One-shot maintenance utility.
//
// Walks `spec/**/*.md` and wraps the leading `---`-delimited YAML
// frontmatter in ```yaml … ``` code fences so raw-markdown viewers
// (GitHub, mdbook, egui_commonmark) render the header as a code block
// instead of collapsing it onto a single line via the `---` =
// horizontal-rule interpretation.
//
// Idempotent: a file whose first non-blank line is already a ```/~~~
// fence is skipped. The parser at `frontmatter.rs::split_frontmatter`
// accepts both fenced and unfenced shapes, so partial runs leave the
// rest of the tree in a valid state.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use walkdir::WalkDir;

fn main() -> Result<()> {
    let repo_root = locate_repo_root()?;
    let spec_root = repo_root.join("spec");
    if !spec_root.is_dir() {
        bail!("expected `spec/` under {}", repo_root.display());
    }

    let mut wrapped = 0usize;
    let mut skipped_already_wrapped = 0usize;
    let mut skipped_no_frontmatter = 0usize;
    let mut errored = Vec::<(PathBuf, String)>::new();

    for entry in WalkDir::new(&spec_root).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        match wrap_one(path) {
            Ok(WrapOutcome::Wrapped) => wrapped += 1,
            Ok(WrapOutcome::AlreadyWrapped) => skipped_already_wrapped += 1,
            Ok(WrapOutcome::NoFrontmatter) => skipped_no_frontmatter += 1,
            Err(e) => errored.push((path.to_path_buf(), e.to_string())),
        }
    }

    println!(
        "wrap_frontmatter: wrapped {wrapped}, already-wrapped {skipped_already_wrapped}, \
         no-frontmatter {skipped_no_frontmatter}, errored {}",
        errored.len()
    );
    for (p, e) in &errored {
        eprintln!("  ! {}: {e}", p.display());
    }
    if !errored.is_empty() {
        bail!("{} file(s) errored", errored.len());
    }
    Ok(())
}

#[derive(Debug)]
enum WrapOutcome {
    Wrapped,
    AlreadyWrapped,
    NoFrontmatter,
}

fn wrap_one(path: &Path) -> Result<WrapOutcome> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("read {}", path.display()))?;
    let Some(wrapped) = produce_wrapped(&content) else {
        // Either no frontmatter or already wrapped — disambiguate.
        let first = content.lines().next().unwrap_or("").trim();
        return Ok(if first.starts_with("```") || first.starts_with("~~~") {
            WrapOutcome::AlreadyWrapped
        } else {
            WrapOutcome::NoFrontmatter
        });
    };
    fs::write(path, wrapped).with_context(|| format!("write {}", path.display()))?;
    Ok(WrapOutcome::Wrapped)
}

/// Produce the wrapped form of `content`, or None if no change is
/// needed (file already wrapped, or file lacks a frontmatter block).
fn produce_wrapped(content: &str) -> Option<String> {
    let mut lines = content.lines();
    let first = lines.next()?;
    if first.trim().starts_with("```") || first.trim().starts_with("~~~") {
        return None; // already fenced
    }
    if first.trim() != "---" {
        return None; // not a spec with frontmatter
    }

    // Find the closing `---` (or `...`) line. Tracks the index of the
    // closing line within `content.lines()`.
    let mut close_line_index: Option<usize> = None;
    for (idx, line) in content.lines().enumerate().skip(1) {
        let trimmed = line.trim();
        if trimmed == "---" || trimmed == "..." {
            close_line_index = Some(idx);
            break;
        }
    }
    let close_idx = close_line_index?;

    let all_lines: Vec<&str> = content.lines().collect();
    let already_has_blank_separator = all_lines
        .get(close_idx + 1)
        .map(|s| s.trim().is_empty())
        .unwrap_or(true);

    let mut out = String::with_capacity(content.len() + 16);
    out.push_str("```yaml\n");
    for line in &all_lines[..=close_idx] {
        out.push_str(line);
        out.push('\n');
    }
    out.push_str("```\n");
    if !already_has_blank_separator {
        out.push('\n');
    }
    for (i, line) in all_lines.iter().enumerate().skip(close_idx + 1) {
        out.push_str(line);
        if i + 1 < all_lines.len() || content.ends_with('\n') {
            out.push('\n');
        }
    }

    if content.ends_with('\n') && !out.ends_with('\n') {
        out.push('\n');
    }

    Some(out)
}

/// Find the repo root by walking up from CWD until we find the
/// workspace `Cargo.toml`.
fn locate_repo_root() -> Result<PathBuf> {
    let cwd = std::env::current_dir()?;
    let mut p = cwd.as_path();
    loop {
        let cargo = p.join("Cargo.toml");
        if cargo.is_file()
            && fs::read_to_string(&cargo)
                .map(|s| s.contains("[workspace]"))
                .unwrap_or(false)
        {
            return Ok(p.to_path_buf());
        }
        p = p
            .parent()
            .ok_or_else(|| anyhow::anyhow!("workspace Cargo.toml not found above CWD"))?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_a_typical_spec() {
        let src = "---\nid: foo\nkind: component\n---\n\nBody paragraph.\n";
        let out = produce_wrapped(src).expect("wrap");
        assert!(out.starts_with("```yaml\n---\n"));
        assert!(out.contains("id: foo\nkind: component\n---\n```\n\nBody paragraph."));
    }

    #[test]
    fn inserts_blank_separator_when_body_starts_immediately() {
        let src = "---\nid: foo\nkind: component\n---\nBody immediately.\n";
        let out = produce_wrapped(src).expect("wrap");
        assert!(out.contains("---\n```\n\nBody immediately."));
    }

    #[test]
    fn already_wrapped_returns_none() {
        let src = "```yaml\n---\nid: foo\nkind: component\n---\n```\n\nbody\n";
        assert!(produce_wrapped(src).is_none());
    }

    #[test]
    fn file_without_frontmatter_returns_none() {
        let src = "# Just a markdown file\n\nNo frontmatter here.\n";
        assert!(produce_wrapped(src).is_none());
    }

    #[test]
    fn handles_yaml_dot_dot_dot_close() {
        let src = "---\nid: foo\nkind: component\n...\nbody\n";
        let out = produce_wrapped(src).expect("wrap");
        assert!(out.contains("...\n```\n"));
    }
}
