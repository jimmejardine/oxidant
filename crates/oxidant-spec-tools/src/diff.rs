// Realises spec/components/spec-tools/diff.md.
//
// Two checks the spec mandates for MVP:
//   1. Contract trait drift — parse ```rust blocks from the body of every
//      `kind: contract` spec, locate the same-named trait in the spec's
//      code: paths, and compare method signatures after path-prefix
//      normalisation.
//   2. Missing code: paths — every `code:` entry must point at an existing
//      file relative to the worktree root.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use quote::ToTokens;
use regex::Regex;
use serde::Serialize;

use crate::frontmatter::{self, FencedBlock, SpecFile, SpecKind, extract_fenced_blocks};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind")]
pub enum Drift {
    MethodAdded {
        contract_id: String,
        method: String,
    },
    MethodRemoved {
        contract_id: String,
        method: String,
    },
    MethodSignatureChanged {
        contract_id: String,
        method: String,
        spec: String,
        code: String,
    },
    MissingCodePath {
        spec_id: String,
        path: PathBuf,
    },
}

/// Run drift detection across every spec file under `repo/spec`. Returns the
/// flat list of drifts in deterministic order (depth-first by spec id, then
/// in-order within a spec).
pub fn diff_all(repo: &Path) -> Vec<Drift> {
    let mut out = Vec::new();
    let spec_root = repo.join("spec");
    let mut specs: Vec<(String, PathBuf, SpecFile)> = Vec::new();
    for entry in walkdir::WalkDir::new(&spec_root)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Ok(canonical_id) = canonical_id(&spec_root, path) else {
            continue;
        };
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        let Ok(file) = frontmatter::parse(&content) else {
            continue;
        };
        specs.push((canonical_id, path.to_path_buf(), file));
    }
    specs.sort_by(|a, b| a.0.cmp(&b.0));

    for (id, _path, file) in &specs {
        match file.frontmatter.kind {
            SpecKind::Contract => {
                out.extend(check_contract_drift(repo, id, file));
                out.extend(check_code_paths(repo, id, file));
            }
            SpecKind::Component | SpecKind::Tool => {
                out.extend(check_code_paths(repo, id, file));
            }
            _ => {}
        }
    }
    out
}

/// Drift for a single spec id (in `path/under/spec` form, e.g. `contracts/tool`).
/// Returns an empty Vec if the id doesn't resolve to a known spec.
pub fn diff_spec(repo: &Path, spec_id: &str) -> Vec<Drift> {
    let spec_root = repo.join("spec");
    let mut path = spec_root.clone();
    for segment in spec_id.split('/') {
        path = path.join(segment);
    }
    path.set_extension("md");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let Ok(file) = frontmatter::parse(&content) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    match file.frontmatter.kind {
        SpecKind::Contract => {
            out.extend(check_contract_drift(repo, spec_id, &file));
            out.extend(check_code_paths(repo, spec_id, &file));
        }
        SpecKind::Component | SpecKind::Tool => {
            out.extend(check_code_paths(repo, spec_id, &file));
        }
        _ => {}
    }
    out
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

fn check_code_paths(repo: &Path, spec_id: &str, file: &SpecFile) -> Vec<Drift> {
    let mut out = Vec::new();
    for code_path in &file.frontmatter.code {
        let absolute = repo.join(code_path);
        if !absolute.exists() {
            out.push(Drift::MissingCodePath {
                spec_id: spec_id.to_string(),
                path: code_path.clone(),
            });
        }
    }
    out
}

fn check_contract_drift(repo: &Path, contract_id: &str, file: &SpecFile) -> Vec<Drift> {
    let mut out = Vec::new();

    // Pull every ```rust block out of the body, parse, collect declared traits.
    let blocks = extract_fenced_blocks(&file.body, 1);
    let spec_traits: Vec<syn::ItemTrait> = blocks
        .iter()
        .filter(|b| is_rust_block(b))
        .flat_map(parse_traits_from_block)
        .collect();
    if spec_traits.is_empty() {
        return out;
    }

    // Parse every existing code: path; collect traits visible from those files.
    let mut code_traits: Vec<syn::ItemTrait> = Vec::new();
    for code_path in &file.frontmatter.code {
        let absolute = repo.join(code_path);
        let Ok(content) = std::fs::read_to_string(&absolute) else {
            continue;
        };
        let Ok(parsed) = syn::parse_file(&content) else {
            continue;
        };
        for item in parsed.items {
            if let syn::Item::Trait(t) = item {
                code_traits.push(t);
            }
        }
    }

    for spec_trait in &spec_traits {
        let name = spec_trait.ident.to_string();
        let Some(code_trait) = code_traits.iter().find(|t| t.ident == name) else {
            // The spec declares a trait the code doesn't have. We don't emit
            // a Drift for the whole trait — MethodRemoved on each method tells
            // the same story.
            for method in spec_methods(spec_trait) {
                out.push(Drift::MethodRemoved {
                    contract_id: contract_id.to_string(),
                    method: method_name(method),
                });
            }
            continue;
        };
        out.extend(compare_methods(contract_id, spec_trait, code_trait));
    }

    out
}

fn is_rust_block(b: &FencedBlock) -> bool {
    matches!(b.language.as_str(), "rust" | "Rust" | "rs")
}

fn parse_traits_from_block(block: &FencedBlock) -> Vec<syn::ItemTrait> {
    match syn::parse_file(&block.content) {
        Ok(file) => file
            .items
            .into_iter()
            .filter_map(|item| match item {
                syn::Item::Trait(t) => Some(t),
                _ => None,
            })
            .collect(),
        Err(_) => Vec::new(),
    }
}

fn spec_methods(t: &syn::ItemTrait) -> Vec<&syn::TraitItemFn> {
    t.items
        .iter()
        .filter_map(|i| match i {
            syn::TraitItem::Fn(f) => Some(f),
            _ => None,
        })
        .collect()
}

fn method_name(m: &syn::TraitItemFn) -> String {
    m.sig.ident.to_string()
}

fn compare_methods(
    contract_id: &str,
    spec_trait: &syn::ItemTrait,
    code_trait: &syn::ItemTrait,
) -> Vec<Drift> {
    let mut out = Vec::new();
    let in_spec = spec_methods(spec_trait);
    let in_code = spec_methods(code_trait);

    for spec_m in &in_spec {
        let name = method_name(spec_m);
        match in_code.iter().find(|c| method_name(c) == name) {
            None => out.push(Drift::MethodRemoved {
                contract_id: contract_id.to_string(),
                method: name,
            }),
            Some(code_m) => {
                let spec_sig = normalise_signature(&spec_m.sig);
                let code_sig = normalise_signature(&code_m.sig);
                if spec_sig != code_sig {
                    out.push(Drift::MethodSignatureChanged {
                        contract_id: contract_id.to_string(),
                        method: name,
                        spec: spec_sig,
                        code: code_sig,
                    });
                }
            }
        }
    }
    for code_m in &in_code {
        let name = method_name(code_m);
        if !in_spec.iter().any(|s| method_name(s) == name) {
            out.push(Drift::MethodAdded {
                contract_id: contract_id.to_string(),
                method: name,
            });
        }
    }

    out
}

static PATH_PREFIX_RE: LazyLock<Regex> = LazyLock::new(|| {
    // Matches a sequence like `serde_json :: Value` or `crate :: foo :: Bar`
    // (quote's spaces are part of the produced token stream).
    Regex::new(r"\b\w+(?:\s*::\s*\w+)+").expect("known-good regex compiles")
});

/// Render a syn::Signature as a normalised string for comparison: token
/// stream from `quote!{}`, then collapse every multi-segment path to its
/// last segment. So `serde_json::Value` and `Value` compare equal, as do
/// `crate::foo::Bar` and `Bar`, while `Result<T>` vs `Option<T>` remain
/// distinct.
fn normalise_signature(sig: &syn::Signature) -> String {
    let tokens = sig.to_token_stream().to_string();
    PATH_PREFIX_RE
        .replace_all(&tokens, |caps: &regex::Captures| {
            let full = caps.get(0).unwrap().as_str();
            full.rsplit("::").next().unwrap_or(full).trim().to_string()
        })
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extract_first_trait(src: &str) -> syn::ItemTrait {
        let file = syn::parse_file(src).expect("parse");
        for item in file.items {
            if let syn::Item::Trait(t) = item {
                return t;
            }
        }
        panic!("no trait found")
    }

    #[test]
    fn identical_traits_produce_no_drift() {
        let s = "pub trait X { fn a(&self) -> u32; }";
        let drifts = compare_methods("c/x", &extract_first_trait(s), &extract_first_trait(s));
        assert!(drifts.is_empty());
    }

    #[test]
    fn method_in_spec_not_code_is_method_removed() {
        let spec = "pub trait X { fn a(&self); fn b(&self); }";
        let code = "pub trait X { fn a(&self); }";
        let drifts = compare_methods(
            "c/x",
            &extract_first_trait(spec),
            &extract_first_trait(code),
        );
        assert_eq!(drifts.len(), 1);
        assert!(matches!(&drifts[0], Drift::MethodRemoved { method, .. } if method == "b"));
    }

    #[test]
    fn method_in_code_not_spec_is_method_added() {
        let spec = "pub trait X { fn a(&self); }";
        let code = "pub trait X { fn a(&self); fn extra(&self); }";
        let drifts = compare_methods(
            "c/x",
            &extract_first_trait(spec),
            &extract_first_trait(code),
        );
        assert_eq!(drifts.len(), 1);
        assert!(matches!(&drifts[0], Drift::MethodAdded { method, .. } if method == "extra"));
    }

    #[test]
    fn different_signatures_produce_signature_changed() {
        let spec = "pub trait X { fn a(&self, x: u32) -> u32; }";
        let code = "pub trait X { fn a(&self, x: u64) -> u32; }";
        let drifts = compare_methods(
            "c/x",
            &extract_first_trait(spec),
            &extract_first_trait(code),
        );
        assert_eq!(drifts.len(), 1);
        assert!(matches!(drifts[0], Drift::MethodSignatureChanged { .. }));
    }

    #[test]
    fn path_prefix_normalisation_collapses_to_last_segment() {
        let spec = "pub trait X { fn a(&self) -> serde_json::Value; }";
        let code = "pub trait X { fn a(&self) -> Value; }";
        let drifts = compare_methods(
            "c/x",
            &extract_first_trait(spec),
            &extract_first_trait(code),
        );
        assert!(
            drifts.is_empty(),
            "expected no drift after path normalisation, got: {:?}",
            drifts
        );
    }

    #[test]
    fn nested_generic_with_path_prefixes_normalised() {
        let spec =
            "pub trait X { fn a(&self) -> Result<std::collections::HashMap<String, foo::Bar>>; }";
        let code = "pub trait X { fn a(&self) -> Result<HashMap<String, Bar>>; }";
        let drifts = compare_methods(
            "c/x",
            &extract_first_trait(spec),
            &extract_first_trait(code),
        );
        assert!(drifts.is_empty(), "got drifts: {:?}", drifts);
    }
}
