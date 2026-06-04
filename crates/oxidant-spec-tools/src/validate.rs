// Realises spec/components/spec-tools/validate.md.
//
// Drift detector for spec hygiene. Warnings, never errors — see
// decision 0008. Surfaces frontmatter, link, length, and code-path issues
// over the parsed spec tree.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::frontmatter::{self, ParseError, SpecFile, SpecKind};
use crate::graph::{GraphInput, Resolution, SpecGraph, resolve};

#[derive(Debug, Clone)]
pub struct Warning {
    pub spec_id: Option<String>,
    pub kind: WarningKind,
    pub message: String,
    pub location: Option<(PathBuf, usize, usize)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WarningKind {
    FrontmatterMissingRequired,
    FrontmatterInvalidValue,
    DuplicateId,
    UnresolvedRef,
    ShortFormAmbiguous,
    Orphan,
    Cycle,
    LengthBudgetExceeded,
    MissingCodePath,
    Reachability,
    ParseError,
}

const ROOT_ID: &str = "overview";

pub fn validate(repo: &Path) -> Vec<Warning> {
    let mut warnings = Vec::new();
    let spec_root = repo.join("spec");

    let mut inputs: Vec<GraphInput> = Vec::new();
    for entry in WalkDir::new(&spec_root).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Ok(canonical_id) = canonical_id(&spec_root, path) else {
            continue;
        };
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                warnings.push(Warning {
                    spec_id: Some(canonical_id),
                    kind: WarningKind::ParseError,
                    message: format!("read failed: {e}"),
                    location: Some((path.to_path_buf(), 0, 0)),
                });
                continue;
            }
        };
        match frontmatter::parse(&content) {
            Ok(file) => {
                check_per_file(repo, &canonical_id, path, &file, &mut warnings);
                inputs.push(GraphInput {
                    canonical_id,
                    file,
                    path: path.to_path_buf(),
                });
            }
            Err(e) => {
                warnings.push(parse_error_warning(canonical_id, path, e));
            }
        }
    }

    check_duplicate_ids(&inputs, &mut warnings);

    let all_ids: Vec<String> = inputs.iter().map(|i| i.canonical_id.clone()).collect();
    check_ref_resolution(&inputs, &all_ids, &mut warnings);

    let graph = SpecGraph::build(&inputs);
    check_cycles_in_deps(&graph, &mut warnings);
    check_orphans(&graph, &mut warnings);
    check_reachability(&graph, &all_ids, &mut warnings);

    warnings
}

fn canonical_id(spec_root: &Path, path: &Path) -> Result<String, ()> {
    let rel = path.strip_prefix(spec_root).map_err(|_| ())?;
    let stem = rel.with_extension("");
    let mut out = String::new();
    for (i, comp) in stem.components().enumerate() {
        if i > 0 {
            out.push('/');
        }
        out.push_str(&comp.as_os_str().to_string_lossy());
    }
    Ok(out)
}

fn parse_error_warning(canonical_id: String, path: &Path, e: ParseError) -> Warning {
    let kind = match &e {
        ParseError::MissingFrontmatter | ParseError::UnterminatedFrontmatter => {
            WarningKind::FrontmatterMissingRequired
        }
        ParseError::InvalidYaml(_) | ParseError::InvalidKind(_) | ParseError::InvalidStatus(_) => {
            WarningKind::FrontmatterInvalidValue
        }
    };
    Warning {
        spec_id: Some(canonical_id),
        kind,
        message: e.to_string(),
        location: Some((path.to_path_buf(), 1, 1)),
    }
}

fn check_per_file(
    repo: &Path,
    canonical_id: &str,
    path: &Path,
    file: &SpecFile,
    warnings: &mut Vec<Warning>,
) {
    let fm = &file.frontmatter;

    if fm.id.trim().is_empty() {
        warnings.push(Warning {
            spec_id: Some(canonical_id.to_string()),
            kind: WarningKind::FrontmatterMissingRequired,
            message: "`id` is empty".to_string(),
            location: Some((path.to_path_buf(), 1, 1)),
        });
    }

    // Decisions require a `date` field — stored in extras since it isn't a
    // typed FrontmatterRecord field.
    if matches!(fm.kind, SpecKind::Decision) && fm.extras.get("date").is_none() {
        warnings.push(Warning {
            spec_id: Some(canonical_id.to_string()),
            kind: WarningKind::FrontmatterMissingRequired,
            message: "decision is missing required `date` field".to_string(),
            location: Some((path.to_path_buf(), 1, 1)),
        });
    }

    // Most kinds carry a `responsibility:` summary. Overview/glossary don't.
    let needs_responsibility = matches!(
        fm.kind,
        SpecKind::Component
            | SpecKind::Contract
            | SpecKind::Tool
            | SpecKind::Flow
            | SpecKind::Invariant
            | SpecKind::Decision
    );
    if needs_responsibility && fm.responsibility.as_deref().unwrap_or("").trim().is_empty() {
        warnings.push(Warning {
            spec_id: Some(canonical_id.to_string()),
            kind: WarningKind::FrontmatterMissingRequired,
            message: format!("{} is missing `responsibility:`", fm.kind.as_str()),
            location: Some((path.to_path_buf(), 1, 1)),
        });
    }

    // Components and tools should declare at least one code: path.
    let needs_code = matches!(fm.kind, SpecKind::Component | SpecKind::Tool);
    if needs_code && fm.code.is_empty() {
        warnings.push(Warning {
            spec_id: Some(canonical_id.to_string()),
            kind: WarningKind::FrontmatterMissingRequired,
            message: format!("{} has no `code:` paths", fm.kind.as_str()),
            location: Some((path.to_path_buf(), 1, 1)),
        });
    }

    for code_path in &fm.code {
        let absolute = repo.join(code_path);
        if !absolute.exists() {
            warnings.push(Warning {
                spec_id: Some(canonical_id.to_string()),
                kind: WarningKind::MissingCodePath,
                message: format!("`code:` path does not exist: {}", code_path.display()),
                location: Some((path.to_path_buf(), 1, 1)),
            });
        }
    }

    let budget = body_line_budget(fm.kind);
    let body_lines = file.body.lines().count();
    if body_lines > budget {
        warnings.push(Warning {
            spec_id: Some(canonical_id.to_string()),
            kind: WarningKind::LengthBudgetExceeded,
            message: format!(
                "{} body is {body_lines} lines (budget {budget})",
                fm.kind.as_str()
            ),
            location: Some((path.to_path_buf(), 1, 1)),
        });
    }
}

fn check_duplicate_ids(inputs: &[GraphInput], warnings: &mut Vec<Warning>) {
    let mut by_short: HashMap<&str, Vec<&str>> = HashMap::new();
    for input in inputs {
        by_short
            .entry(input.file.frontmatter.id.as_str())
            .or_default()
            .push(&input.canonical_id);
    }
    for (short_id, owners) in by_short {
        if owners.len() > 1 {
            warnings.push(Warning {
                spec_id: None,
                kind: WarningKind::DuplicateId,
                message: format!(
                    "frontmatter id {short_id:?} is declared by {} specs: {}",
                    owners.len(),
                    owners.join(", ")
                ),
                location: None,
            });
        }
    }
}

fn check_ref_resolution(inputs: &[GraphInput], all_ids: &[String], warnings: &mut Vec<Warning>) {
    for input in inputs {
        let fm = &input.file.frontmatter;
        let frontmatter_refs = std::iter::once(("parent", fm.parent.clone()))
            .filter_map(|(label, opt)| opt.map(|s| (label, s, 1usize, 1usize)))
            .chain(
                fm.implements
                    .iter()
                    .map(|s| ("implements", s.clone(), 1, 1)),
            )
            .chain(
                fm.depends_on
                    .iter()
                    .map(|s| ("depends_on", s.clone(), 1, 1)),
            );

        for (label, raw, line, col) in frontmatter_refs {
            check_one_ref(input, &raw, label, line, col, all_ids, warnings);
        }
        for mention in &input.file.refs_in_body {
            check_one_ref(
                input,
                &mention.raw,
                "body",
                mention.line,
                mention.column,
                all_ids,
                warnings,
            );
        }
    }
}

fn check_one_ref(
    input: &GraphInput,
    raw: &str,
    label: &str,
    line: usize,
    col: usize,
    all_ids: &[String],
    warnings: &mut Vec<Warning>,
) {
    match resolve(raw, all_ids) {
        Resolution::Resolved(_) => {}
        Resolution::Unresolved => warnings.push(Warning {
            spec_id: Some(input.canonical_id.clone()),
            kind: WarningKind::UnresolvedRef,
            message: format!("unresolved {label} ref [[{raw}]]"),
            location: Some((input.path.clone(), line, col)),
        }),
        Resolution::Ambiguous(candidates) => warnings.push(Warning {
            spec_id: Some(input.canonical_id.clone()),
            kind: WarningKind::ShortFormAmbiguous,
            message: format!(
                "ambiguous {label} ref [[{raw}]] matches {}: {}",
                candidates.len(),
                candidates.join(", ")
            ),
            location: Some((input.path.clone(), line, col)),
        }),
    }
}

fn check_cycles_in_deps(graph: &SpecGraph, warnings: &mut Vec<Warning>) {
    // One warning per disjoint cycle, with the participating specs
    // listed in path form so the message is immediately actionable.
    // See spec/components/spec-tools/validate.md "cycle".
    for cycle in graph.dep_cycles() {
        let path = cycle.join(" → ");
        warnings.push(Warning {
            spec_id: None,
            kind: WarningKind::Cycle,
            message: format!("dependency cycle in depends_on: {path}"),
            location: None,
        });
    }
}

fn check_orphans(graph: &SpecGraph, warnings: &mut Vec<Warning>) {
    for node in graph.nodes() {
        if matches!(node.kind, SpecKind::Overview | SpecKind::Glossary) {
            continue;
        }
        if graph.inbound(&node.id).is_empty() {
            warnings.push(Warning {
                spec_id: Some(node.id.clone()),
                kind: WarningKind::Orphan,
                message: format!("{} {} has no inbound edges", node.kind.as_str(), node.id),
                location: Some((node.path.clone(), 1, 1)),
            });
        }
    }
}

/// Flag specs that are unreachable from `overview` at any depth
/// through the link graph (parent / depends_on / implements / body
/// `[[ref]]`). Depth is intentionally unbounded — deep abstraction
/// layers are a feature, not a code smell; the warning fires only
/// when a spec is truly disconnected (orphan island). See
/// spec/components/spec-tools/validate.md "reachability".
fn check_reachability(graph: &SpecGraph, all_ids: &[String], warnings: &mut Vec<Warning>) {
    if !all_ids.iter().any(|id| id == ROOT_ID) {
        return; // no root to measure from; not our concern here
    }
    let reachable: HashSet<String> = graph
        .reachable_within(ROOT_ID, usize::MAX)
        .into_iter()
        .map(|n| n.id.clone())
        .collect();
    for node in graph.nodes() {
        if matches!(node.kind, SpecKind::Glossary) {
            continue;
        }
        if !reachable.contains(&node.id) {
            warnings.push(Warning {
                spec_id: Some(node.id.clone()),
                kind: WarningKind::Reachability,
                message: format!(
                    "not reachable from `{ROOT_ID}` through the spec link graph \
                     — add a `parent:`, `depends_on:`, or body `[[ref]]` from a connected spec"
                ),
                location: Some((node.path.clone(), 1, 1)),
            });
        }
    }
}

fn body_line_budget(kind: SpecKind) -> usize {
    match kind {
        SpecKind::Overview => 120,
        SpecKind::Glossary => 200,
        SpecKind::Component => 100,
        SpecKind::Contract => 150,
        SpecKind::Tool => 80,
        SpecKind::Flow => 120,
        SpecKind::Invariant => 60,
        SpecKind::Decision => 120,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontmatter;
    use crate::graph::GraphInput;

    /// Build a tiny SpecGraph from a list of (id, kind, body_refs).
    /// Body refs become outgoing edges from the spec via the
    /// `[[ref]]` mention extractor — the same mechanism overview.md
    /// uses to reach the rest of the tree in the real spec set.
    fn build_graph(specs: &[(&str, &str, &[&str])]) -> (SpecGraph, Vec<String>) {
        let inputs: Vec<GraphInput> = specs
            .iter()
            .map(|(id, kind, refs)| {
                let body_refs = refs
                    .iter()
                    .map(|r| format!("See [[{r}]]."))
                    .collect::<Vec<_>>()
                    .join("\n");
                let raw = format!("---\nid: {id}\nkind: {kind}\n---\n{body_refs}\n(body)\n");
                GraphInput {
                    canonical_id: (*id).to_string(),
                    file: frontmatter::parse(&raw).expect("parse"),
                    path: PathBuf::from(format!("spec/{id}.md")),
                }
            })
            .collect();
        let all_ids: Vec<String> = inputs.iter().map(|i| i.canonical_id.clone()).collect();
        (SpecGraph::build(&inputs), all_ids)
    }

    #[test]
    fn reachability_does_not_complain_about_deeply_nested_but_connected_specs() {
        // overview → a → b → c → d → e — 5 hops, all connected via
        // body refs (the same mechanism real overview.md uses). The
        // old depth-4 check would have flagged `e`; the new unbounded
        // check must not.
        let (graph, all_ids) = build_graph(&[
            ("overview", "overview", &["a"]),
            ("a", "component", &["b"]),
            ("b", "component", &["c"]),
            ("c", "component", &["d"]),
            ("d", "component", &["e"]),
            ("e", "component", &[]),
        ]);
        let mut warnings = Vec::new();
        check_reachability(&graph, &all_ids, &mut warnings);
        let reachability_warnings: Vec<_> = warnings
            .iter()
            .filter(|w| matches!(w.kind, WarningKind::Reachability))
            .collect();
        assert!(
            reachability_warnings.is_empty(),
            "depth ≥ 5 must NOT trip reachability when chain is connected; got {reachability_warnings:?}"
        );
    }

    #[test]
    fn reachability_complains_about_truly_disconnected_island_specs() {
        // overview reaches `a` but has no path to `island`. Only
        // `island` should warn.
        let (graph, all_ids) = build_graph(&[
            ("overview", "overview", &["a"]),
            ("a", "component", &[]),
            ("island", "component", &[]),
        ]);
        let mut warnings = Vec::new();
        check_reachability(&graph, &all_ids, &mut warnings);
        let reachability_warnings: Vec<_> = warnings
            .iter()
            .filter(|w| matches!(w.kind, WarningKind::Reachability))
            .collect();
        assert_eq!(
            reachability_warnings.len(),
            1,
            "expected exactly one orphan warning, got {reachability_warnings:?}"
        );
        assert_eq!(
            reachability_warnings[0].spec_id.as_deref(),
            Some("island"),
            "the orphan warning must target `island`, got {:?}",
            reachability_warnings[0].spec_id
        );
    }
}
