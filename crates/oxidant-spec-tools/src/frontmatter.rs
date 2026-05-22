// Realises spec/components/spec-tools/frontmatter.md.
//
// Pure parser: bytes in, structured record out. Foundation under graph,
// validate, diff, index-db, and search-index — centralising what counts as
// a valid spec file.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::LazyLock;

use regex::Regex;
use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct SpecFile {
    pub frontmatter: FrontmatterRecord,
    pub body: String,
    pub refs_in_body: Vec<RefMention>,
}

#[derive(Debug, Clone)]
pub struct FrontmatterRecord {
    pub id: String,
    pub kind: SpecKind,
    pub order: Option<i64>,
    pub parent: Option<String>,
    pub implements: Vec<String>,
    pub depends_on: Vec<String>,
    pub code: Vec<PathBuf>,
    pub status: SpecStatus,
    pub responsibility: Option<String>,
    pub extras: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct RefMention {
    /// Inner text of the `[[...]]` mention, verbatim.
    pub raw: String,
    /// 1-indexed line number in the original source.
    pub line: usize,
    /// 1-indexed column of the opening `[[`.
    pub column: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpecKind {
    Overview,
    Glossary,
    Component,
    Contract,
    Tool,
    Flow,
    Invariant,
    Decision,
}

impl SpecKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SpecKind::Overview => "overview",
            SpecKind::Glossary => "glossary",
            SpecKind::Component => "component",
            SpecKind::Contract => "contract",
            SpecKind::Tool => "tool",
            SpecKind::Flow => "flow",
            SpecKind::Invariant => "invariant",
            SpecKind::Decision => "decision",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpecStatus {
    Draft,
    Active,
    Deprecated,
}

impl SpecStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            SpecStatus::Draft => "draft",
            SpecStatus::Active => "active",
            SpecStatus::Deprecated => "deprecated",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("missing frontmatter: file must begin with `---` on its first line")]
    MissingFrontmatter,
    #[error("unterminated frontmatter: opening `---` has no closing `---` or `...`")]
    UnterminatedFrontmatter,
    #[error("invalid YAML in frontmatter: {0}")]
    InvalidYaml(String),
    #[error("invalid `kind` value: {0:?} (expected one of overview|glossary|component|contract|tool|flow|invariant|decision)")]
    InvalidKind(String),
    #[error("invalid `status` value: {0:?} (expected one of draft|active|deprecated)")]
    InvalidStatus(String),
}

pub fn parse(content: &str) -> Result<SpecFile, ParseError> {
    let (yaml_text, body, body_line_offset) = split_frontmatter(content)?;
    let raw: RawFrontmatter = serde_yaml_ng::from_str(yaml_text)
        .map_err(|e| ParseError::InvalidYaml(e.to_string()))?;
    let frontmatter = raw.into_record()?;
    let refs_in_body = extract_refs(&body, body_line_offset);
    Ok(SpecFile { frontmatter, body, refs_in_body })
}

fn split_frontmatter(content: &str) -> Result<(&str, String, usize), ParseError> {
    let mut lines = content.lines().enumerate();
    let Some((_, first)) = lines.next() else {
        return Err(ParseError::MissingFrontmatter);
    };
    if first.trim() != "---" {
        return Err(ParseError::MissingFrontmatter);
    }

    let mut yaml_end_byte: Option<usize> = None;
    let mut body_start_line: Option<usize> = None;
    let mut running_byte = first.len() + 1; // +1 for the newline we consumed
    for (line_idx, line) in lines {
        let trimmed = line.trim();
        if trimmed == "---" || trimmed == "..." {
            yaml_end_byte = Some(running_byte);
            body_start_line = Some(line_idx + 1); // first body line is the one after the closer
            break;
        }
        running_byte += line.len() + 1;
    }
    let yaml_end = yaml_end_byte.ok_or(ParseError::UnterminatedFrontmatter)?;
    let body_start = body_start_line.unwrap();

    let yaml_text = &content[first.len() + 1..yaml_end];
    let body = collect_body(content, body_start);
    // body_line_offset: the line number the first body line had in the original source (1-indexed).
    let body_line_offset = body_start + 1;
    Ok((yaml_text, body, body_line_offset))
}

fn collect_body(content: &str, start_line: usize) -> String {
    content
        .lines()
        .skip(start_line)
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Deserialize)]
struct RawFrontmatter {
    id: String,
    kind: String,
    #[serde(default)]
    order: Option<i64>,
    #[serde(default)]
    parent: Option<String>,
    #[serde(default)]
    implements: Vec<String>,
    #[serde(default)]
    depends_on: Vec<String>,
    #[serde(default)]
    code: Vec<PathBuf>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    responsibility: Option<String>,
    #[serde(flatten)]
    extras: HashMap<String, serde_yaml_ng::Value>,
}

impl RawFrontmatter {
    fn into_record(self) -> Result<FrontmatterRecord, ParseError> {
        let kind = match self.kind.as_str() {
            "overview" => SpecKind::Overview,
            "glossary" => SpecKind::Glossary,
            "component" => SpecKind::Component,
            "contract" => SpecKind::Contract,
            "tool" => SpecKind::Tool,
            "flow" => SpecKind::Flow,
            "invariant" => SpecKind::Invariant,
            "decision" => SpecKind::Decision,
            other => return Err(ParseError::InvalidKind(other.to_string())),
        };
        let status = match self.status.as_deref().unwrap_or("active") {
            "draft" => SpecStatus::Draft,
            "active" => SpecStatus::Active,
            "deprecated" => SpecStatus::Deprecated,
            other => return Err(ParseError::InvalidStatus(other.to_string())),
        };
        let extras = yaml_map_to_json(self.extras);
        Ok(FrontmatterRecord {
            id: self.id,
            kind,
            order: self.order,
            parent: self.parent,
            implements: self.implements,
            depends_on: self.depends_on,
            code: self.code,
            status,
            responsibility: self.responsibility,
            extras,
        })
    }
}

fn yaml_map_to_json(map: HashMap<String, serde_yaml_ng::Value>) -> serde_json::Value {
    let entries: serde_json::Map<String, serde_json::Value> = map
        .into_iter()
        .map(|(k, v)| (k, yaml_to_json(v)))
        .collect();
    serde_json::Value::Object(entries)
}

fn yaml_to_json(value: serde_yaml_ng::Value) -> serde_json::Value {
    match value {
        serde_yaml_ng::Value::Null => serde_json::Value::Null,
        serde_yaml_ng::Value::Bool(b) => serde_json::Value::Bool(b),
        serde_yaml_ng::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                serde_json::Value::Number(i.into())
            } else if let Some(u) = n.as_u64() {
                serde_json::Value::Number(u.into())
            } else if let Some(f) = n.as_f64() {
                serde_json::Number::from_f64(f)
                    .map(serde_json::Value::Number)
                    .unwrap_or(serde_json::Value::Null)
            } else {
                serde_json::Value::Null
            }
        }
        serde_yaml_ng::Value::String(s) => serde_json::Value::String(s),
        serde_yaml_ng::Value::Sequence(seq) => {
            serde_json::Value::Array(seq.into_iter().map(yaml_to_json).collect())
        }
        serde_yaml_ng::Value::Mapping(map) => {
            let mut obj = serde_json::Map::new();
            for (k, v) in map {
                let key = match k {
                    serde_yaml_ng::Value::String(s) => s,
                    other => serde_yaml_ng::to_string(&other)
                        .unwrap_or_default()
                        .trim()
                        .to_string(),
                };
                obj.insert(key, yaml_to_json(v));
            }
            serde_json::Value::Object(obj)
        }
        serde_yaml_ng::Value::Tagged(tagged) => yaml_to_json(tagged.value),
    }
}

// Fence-aware body ref extraction. Lines inside ```...``` or ~~~...~~~ blocks
// are skipped, and within a non-fenced line, content between single backticks
// is also skipped — so literals like `[[ref]]` in prose don't count.
static REF_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\[\[([^\]\n]+)\]\]").expect("known-good regex compiles")
});

fn extract_refs(body: &str, line_offset: usize) -> Vec<RefMention> {
    let mut out = Vec::new();
    let mut fence: Option<&str> = None;
    for (idx, line) in body.lines().enumerate() {
        let absolute_line = line_offset + idx;
        let trimmed = line.trim_start();
        if let Some(open) = fence {
            if trimmed.starts_with(open) {
                fence = None;
            }
            continue;
        }
        if trimmed.starts_with("```") {
            fence = Some("```");
            continue;
        }
        if trimmed.starts_with("~~~") {
            fence = Some("~~~");
            continue;
        }
        extract_refs_in_line(line, absolute_line, &mut out);
    }
    out
}

/// Scan a single non-fenced line for refs, skipping content inside inline
/// code spans. Follows the CommonMark rule: a run of N backticks opens a
/// span that closes at the next run of exactly N backticks on the same
/// line. Backticks are ASCII, so byte indexing is char-safe.
fn extract_refs_in_line(line: &str, absolute_line: usize, out: &mut Vec<RefMention>) {
    let bytes = line.as_bytes();
    let mut pos = 0;
    while pos < bytes.len() {
        let bt_start = match bytes[pos..].iter().position(|&b| b == b'`') {
            Some(rel) => pos + rel,
            None => {
                scan_segment(line, pos, bytes.len(), absolute_line, out);
                return;
            }
        };
        scan_segment(line, pos, bt_start, absolute_line, out);

        let open_len = bytes[bt_start..].iter().take_while(|&&b| b == b'`').count();
        let mut search = bt_start + open_len;
        loop {
            let close_start = match bytes[search..].iter().position(|&b| b == b'`') {
                Some(rel) => search + rel,
                None => return, // unterminated span — bail
            };
            let close_len = bytes[close_start..].iter().take_while(|&&b| b == b'`').count();
            if close_len == open_len {
                pos = close_start + close_len;
                break;
            }
            search = close_start + close_len;
        }
    }
}

fn scan_segment(line: &str, start: usize, end: usize, absolute_line: usize, out: &mut Vec<RefMention>) {
    if start >= end {
        return;
    }
    let segment = &line[start..end];
    for cap in REF_RE.captures_iter(segment) {
        let m = cap.get(0).unwrap();
        out.push(RefMention {
            raw: cap[1].to_string(),
            line: absolute_line,
            column: start + m.start() + 1,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_frontmatter() {
        let src = "---\nid: foo\nkind: component\n---\nbody text\n";
        let f = parse(src).expect("parse");
        assert_eq!(f.frontmatter.id, "foo");
        assert!(matches!(f.frontmatter.kind, SpecKind::Component));
        assert!(matches!(f.frontmatter.status, SpecStatus::Active));
        assert_eq!(f.body.trim(), "body text");
    }

    #[test]
    fn captures_unknown_keys_in_extras() {
        let src = "---\nid: d1\nkind: decision\ndate: 2026-05-21\nstatus: active\n---\n";
        let f = parse(src).expect("parse");
        let date = f.frontmatter.extras.get("date").expect("date in extras");
        assert_eq!(date.as_str(), Some("2026-05-21"));
    }

    #[test]
    fn extracts_body_refs_with_position() {
        let src = "---\nid: a\nkind: tool\n---\nSee [[x/y]] and [[z]].\n";
        let f = parse(src).expect("parse");
        let raws: Vec<_> = f.refs_in_body.iter().map(|r| r.raw.as_str()).collect();
        assert_eq!(raws, vec!["x/y", "z"]);
        // first body line is at absolute line 5 (frontmatter occupies 1..=4).
        assert_eq!(f.refs_in_body[0].line, 5);
    }

    #[test]
    fn ignores_refs_inside_fenced_code() {
        let src = "---\nid: a\nkind: tool\n---\nReal [[x]]\n```\nFake [[y]]\n```\nReal [[z]]\n";
        let f = parse(src).expect("parse");
        let raws: Vec<_> = f.refs_in_body.iter().map(|r| r.raw.as_str()).collect();
        assert_eq!(raws, vec!["x", "z"]);
    }

    #[test]
    fn ignores_refs_inside_inline_backticks() {
        let src = "---\nid: a\nkind: tool\n---\nUse `[[name]]` syntax to reference [[real/target]].\n";
        let f = parse(src).expect("parse");
        let raws: Vec<_> = f.refs_in_body.iter().map(|r| r.raw.as_str()).collect();
        assert_eq!(raws, vec!["real/target"]);
    }

    #[test]
    fn handles_double_backtick_spans() {
        // Double backticks let inline code contain literal single backticks.
        // The span ``...`` must close at the next run of exactly two backticks.
        let src = "---\nid: a\nkind: tool\n---\nLiterals like `` `[[ref]]` `` don't count, but [[real/x]] does.\n";
        let f = parse(src).expect("parse");
        let raws: Vec<_> = f.refs_in_body.iter().map(|r| r.raw.as_str()).collect();
        assert_eq!(raws, vec!["real/x"]);
    }

    #[test]
    fn unterminated_inline_span_does_not_eat_following_lines() {
        // An open backtick on one line shouldn't suppress refs on subsequent
        // lines — the span check is line-local.
        let src = "---\nid: a\nkind: tool\n---\nA stray ` backtick\nReal [[x]] here.\n";
        let f = parse(src).expect("parse");
        let raws: Vec<_> = f.refs_in_body.iter().map(|r| r.raw.as_str()).collect();
        assert_eq!(raws, vec!["x"]);
    }

    #[test]
    fn rejects_missing_frontmatter() {
        let err = parse("no frontmatter here\n").unwrap_err();
        assert!(matches!(err, ParseError::MissingFrontmatter));
    }

    #[test]
    fn rejects_unterminated_frontmatter() {
        let err = parse("---\nid: foo\nkind: tool\nbody but no closer\n").unwrap_err();
        assert!(matches!(err, ParseError::UnterminatedFrontmatter));
    }

    #[test]
    fn rejects_unknown_kind() {
        let err = parse("---\nid: foo\nkind: spaghetti\n---\n").unwrap_err();
        assert!(matches!(err, ParseError::InvalidKind(_)));
    }
}
