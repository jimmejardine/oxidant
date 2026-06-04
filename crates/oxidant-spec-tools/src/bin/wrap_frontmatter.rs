// One-shot maintenance utility.
//
// Walks `spec/**/*.md` and normalises the leading frontmatter to a single
// ```yaml … ``` fenced YAML block — the canonical form. The fence makes
// raw-markdown viewers (GitHub, mdbook, egui_commonmark) render the header
// as a code block; the YAML lives directly inside it with no `---`
// delimiters (those are redundant once fenced).
//
// Converts both legacy shapes:
//   - a fence wrapping `---`…`---`  → strip the inner `---`,
//   - a bare `---`…`---` block      → wrap in a fence, drop the `---`.
// Idempotent: a file already in the canonical fence-only form is left
// untouched. The parser at `frontmatter.rs::split_frontmatter` accepts all
// three shapes, so partial runs leave the tree valid.

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

    let mut normalized = 0usize;
    let mut already_canonical = 0usize;
    let mut skipped_no_frontmatter = 0usize;
    let mut errored = Vec::<(PathBuf, String)>::new();

    for entry in WalkDir::new(&spec_root).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        match wrap_one(path) {
            Ok(WrapOutcome::Normalized) => normalized += 1,
            Ok(WrapOutcome::AlreadyCanonical) => already_canonical += 1,
            Ok(WrapOutcome::NoFrontmatter) => skipped_no_frontmatter += 1,
            Err(e) => errored.push((path.to_path_buf(), e.to_string())),
        }
    }

    println!(
        "wrap_frontmatter: normalized {normalized}, already-canonical {already_canonical}, \
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
    Normalized,
    AlreadyCanonical,
    NoFrontmatter,
}

fn wrap_one(path: &Path) -> Result<WrapOutcome> {
    let content = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let Some((yaml, body)) = extract_frontmatter(&content) else {
        return Ok(WrapOutcome::NoFrontmatter);
    };
    let normalized = render_canonical(&yaml, &body, content.ends_with('\n'));
    if normalized == content {
        return Ok(WrapOutcome::AlreadyCanonical);
    }
    fs::write(path, normalized).with_context(|| format!("write {}", path.display()))?;
    Ok(WrapOutcome::Normalized)
}

/// Pull (yaml_lines, body_lines) out of either supported on-disk shape:
/// a fenced block (canonical or fence+`---`) or a bare `---`…`---` block.
/// Returns None if the file has no recognisable frontmatter.
fn extract_frontmatter(content: &str) -> Option<(Vec<String>, Vec<String>)> {
    let lines: Vec<&str> = content.lines().collect();
    let first = lines.first()?.trim();

    let close = if first.starts_with("```") || first.starts_with("~~~") {
        lines
            .iter()
            .enumerate()
            .skip(1)
            .find(|(_, l)| {
                let t = l.trim();
                t.starts_with("```") || t.starts_with("~~~")
            })
            .map(|(i, _)| i)?
    } else if first == "---" {
        lines
            .iter()
            .enumerate()
            .skip(1)
            .find(|(_, l)| matches!(l.trim(), "---" | "..."))
            .map(|(i, _)| i)?
    } else {
        return None;
    };

    let yaml = strip_delims(&lines[1..close]);
    let mut body_start = close + 1;
    if lines.get(body_start).is_some_and(|l| l.trim().is_empty()) {
        body_start += 1;
    }
    let body = lines
        .get(body_start..)
        .unwrap_or(&[])
        .iter()
        .map(|s| s.to_string())
        .collect();
    Some((yaml, body))
}

/// Drop surrounding blank lines and an optional leading `---` /
/// trailing `---`|`...` from a frontmatter region.
fn strip_delims(lines: &[&str]) -> Vec<String> {
    let mut start = 0;
    let mut end = lines.len();
    while start < end && lines[start].trim().is_empty() {
        start += 1;
    }
    if start < end && lines[start].trim() == "---" {
        start += 1;
    }
    while end > start {
        let t = lines[end - 1].trim();
        if t.is_empty() || t == "---" || t == "..." {
            end -= 1;
        } else {
            break;
        }
    }
    lines[start..end].iter().map(|s| s.to_string()).collect()
}

/// Render the canonical fence-only form: ```yaml + yaml + ``` then, when
/// there's a body, a blank separator and the body. `trailing_newline`
/// preserves the file's final newline.
fn render_canonical(yaml: &[String], body: &[String], trailing_newline: bool) -> String {
    let mut out = String::new();
    out.push_str("```yaml\n");
    for l in yaml {
        out.push_str(l);
        out.push('\n');
    }
    out.push_str("```\n");
    if !body.is_empty() {
        out.push('\n');
        for (i, l) in body.iter().enumerate() {
            out.push_str(l);
            if i + 1 < body.len() || trailing_newline {
                out.push('\n');
            }
        }
    }
    out
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

    /// Run the full normalize pipeline on `src`, or None if no frontmatter.
    fn normalize(src: &str) -> Option<String> {
        let (yaml, body) = extract_frontmatter(src)?;
        Some(render_canonical(&yaml, &body, src.ends_with('\n')))
    }

    #[test]
    fn bare_dashes_become_fence_only() {
        let src = "---\nid: foo\nkind: component\n---\n\nBody paragraph.\n";
        let out = normalize(src).expect("normalize");
        assert_eq!(out, "```yaml\nid: foo\nkind: component\n```\n\nBody paragraph.\n");
    }

    #[test]
    fn fence_plus_dashes_loses_the_dashes() {
        let src = "```yaml\n---\nid: foo\nkind: component\n---\n```\n\nbody\n";
        let out = normalize(src).expect("normalize");
        assert_eq!(out, "```yaml\nid: foo\nkind: component\n```\n\nbody\n");
    }

    #[test]
    fn already_canonical_is_unchanged() {
        let src = "```yaml\nid: foo\nkind: component\n```\n\nbody\n";
        let out = normalize(src).expect("normalize");
        assert_eq!(out, src, "canonical input should round-trip identically");
    }

    #[test]
    fn body_started_immediately_gets_blank_separator() {
        let src = "---\nid: foo\nkind: component\n---\nBody immediately.\n";
        let out = normalize(src).expect("normalize");
        assert_eq!(out, "```yaml\nid: foo\nkind: component\n```\n\nBody immediately.\n");
    }

    #[test]
    fn frontmatter_only_no_body() {
        let src = "---\nid: foo\nkind: decision\n---\n";
        let out = normalize(src).expect("normalize");
        assert_eq!(out, "```yaml\nid: foo\nkind: decision\n```\n");
    }

    #[test]
    fn file_without_frontmatter_returns_none() {
        let src = "# Just a markdown file\n\nNo frontmatter here.\n";
        assert!(normalize(src).is_none());
    }

    #[test]
    fn handles_yaml_dot_dot_dot_close() {
        let src = "---\nid: foo\nkind: component\n...\nbody\n";
        let out = normalize(src).expect("normalize");
        assert_eq!(out, "```yaml\nid: foo\nkind: component\n```\n\nbody\n");
    }
}
