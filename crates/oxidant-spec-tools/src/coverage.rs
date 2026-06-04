// Spec coverage: which workspace Rust files are transitively reachable
// from the files specs declare in their `code:` frontmatter.
//
// Realises spec/tools/spec/spec-coverage.md.
//
// Specs name high-level files; those `use` utility files specs don't name
// directly. We build a file-level import graph (module tree + `use` /
// `crate::|self::|super::` path resolution), seed it with every spec
// `code:` file, BFS, and report `src/**/*.rs` files no spec transitively
// reaches. It's reachability over real import edges — deterministic, but
// macro-generated / `include!` / dynamic references and edges that only
// exist through external re-exports can be missed.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::path::{Path, PathBuf};

use serde::Serialize;
use syn::visit::Visit;

use crate::walker::walk_specs;

/// One file no spec transitively reaches.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct UncoveredFile {
    /// Workspace-relative path, forward slashes.
    pub file: String,
    /// Owning crate (underscored ident).
    pub krate: String,
}

/// Result of a coverage analysis over a workspace.
#[derive(Debug, Clone, Serialize)]
pub struct CoverageReport {
    pub seed_count: usize,
    pub covered_count: usize,
    pub uncovered: Vec<UncoveredFile>,
    /// `code:` paths declared by a spec that don't exist on disk (also
    /// surfaced by `spec diff` as MissingCodePath; repeated here as a note).
    pub missing_seeds: Vec<String>,
    /// Human-readable caveats about the analysis.
    pub notes: Vec<String>,
}

/// Analyse `repo`: seed from spec `code:` files, walk the import graph,
/// return the files nothing reaches.
pub fn analyze(repo: &Path) -> CoverageReport {
    let graph = build_graph(repo);

    // Seeds: spec `code:` files that actually exist as workspace src files.
    let mut seeds: BTreeSet<String> = BTreeSet::new();
    let mut missing_seeds: Vec<String> = Vec::new();
    let known: BTreeSet<&String> = graph.files.iter().collect();
    for rec in walk_specs(repo) {
        for code in &rec.file.frontmatter.code {
            let rel = norm(&code.to_string_lossy());
            if known.contains(&rel) {
                seeds.insert(rel);
            } else if rel.ends_with(".rs") {
                // Only flag Rust seeds — specs may legitimately point at
                // assets/Cargo.toml/etc. that aren't in the src graph.
                if !repo.join(&rel).exists() {
                    missing_seeds.push(rel);
                }
            }
        }
    }
    missing_seeds.sort();
    missing_seeds.dedup();

    let reachable = reachable(&seeds, &graph.edges);

    let mut uncovered: Vec<UncoveredFile> = graph
        .files
        .iter()
        .filter(|f| !reachable.contains(*f))
        .map(|f| UncoveredFile {
            file: f.clone(),
            krate: graph.file_crate.get(f).cloned().unwrap_or_default(),
        })
        .collect();
    uncovered.sort_by(|a, b| (&a.krate, &a.file).cmp(&(&b.krate, &b.file)));

    let notes = vec![
        "File-level import-graph reachability from spec `code:` files. A file is \
         'covered' if some spec-declared file transitively `use`s it."
            .to_string(),
        "Heuristic: macro-generated paths, `include!`, and references that only \
         exist via external re-exports may be missed. Binary entry points \
         (main.rs and CLI-only modules) appear here unless a spec declares them."
            .to_string(),
    ];

    CoverageReport {
        seed_count: seeds.len(),
        covered_count: reachable.len(),
        uncovered,
        missing_seeds,
        notes,
    }
}

/// The import graph over a workspace's `crates/*/src/**/*.rs`.
struct Graph {
    /// All analysed src files (workspace-relative, forward slashes), sorted.
    files: Vec<String>,
    /// file → files it references (via `use` / qualified paths).
    edges: HashMap<String, BTreeSet<String>>,
    /// file → owning crate ident.
    file_crate: HashMap<String, String>,
}

fn build_graph(repo: &Path) -> Graph {
    let crates = discover_crates(repo);
    let crate_idents: BTreeSet<String> = crates.iter().map(|c| c.ident.clone()).collect();

    // Enumerate src files + map each to its crate.
    let mut files: Vec<String> = Vec::new();
    let mut file_crate: HashMap<String, String> = HashMap::new();
    for c in &crates {
        for abs in walk_rs(&c.dir.join("src")) {
            if let Ok(rel) = abs.strip_prefix(repo) {
                let rel = norm(&rel.to_string_lossy());
                file_crate.insert(rel.clone(), c.ident.clone());
                files.push(rel);
            }
        }
    }
    files.sort();
    files.dedup();

    // Build the module map (crate-qualified module path → file) by
    // following `mod` declarations from each crate root.
    let mut modmap: HashMap<Vec<String>, String> = HashMap::new();
    let mut file_mod: HashMap<String, Vec<String>> = HashMap::new();
    for c in &crates {
        for root in &c.roots {
            let Ok(rel) = root.strip_prefix(repo) else {
                continue;
            };
            let rel = norm(&rel.to_string_lossy());
            let modpath = vec![c.ident.clone()];
            map_module(repo, &rel, modpath, &mut modmap, &mut file_mod);
        }
    }

    // Collect reference edges per file.
    let mut edges: HashMap<String, BTreeSet<String>> = HashMap::new();
    for f in &files {
        let krate = file_crate.get(f).cloned().unwrap_or_default();
        let cur_mod = file_mod.get(f).cloned();
        let refs = file_references(repo, f);
        let mut targets: BTreeSet<String> = BTreeSet::new();
        for segs in refs {
            if let Some(target) = resolve(&segs, &krate, cur_mod.as_deref(), &crate_idents, &modmap)
                && &target != f
            {
                targets.insert(target);
            }
        }
        edges.insert(f.clone(), targets);
    }

    Graph {
        files,
        edges,
        file_crate,
    }
}

struct CrateInfo {
    ident: String,
    dir: PathBuf,
    roots: Vec<PathBuf>,
}

fn discover_crates(repo: &Path) -> Vec<CrateInfo> {
    let mut out = Vec::new();
    let crates_dir = repo.join("crates");
    let Ok(entries) = std::fs::read_dir(&crates_dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.join("Cargo.toml").is_file() {
            continue;
        }
        let name = dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        let ident = name.replace('-', "_");
        let mut roots = Vec::new();
        for r in ["src/lib.rs", "src/main.rs"] {
            let p = dir.join(r);
            if p.is_file() {
                roots.push(p);
            }
        }
        out.push(CrateInfo { ident, dir, roots });
    }
    out
}

/// Recursively follow `mod` declarations from `file_rel`, recording the
/// crate-qualified module path of every file reached.
fn map_module(
    repo: &Path,
    file_rel: &str,
    modpath: Vec<String>,
    modmap: &mut HashMap<Vec<String>, String>,
    file_mod: &mut HashMap<String, Vec<String>>,
) {
    if modmap.contains_key(&modpath) {
        return; // already mapped (lib root wins over later visits)
    }
    modmap.insert(modpath.clone(), file_rel.to_string());
    file_mod
        .entry(file_rel.to_string())
        .or_insert(modpath.clone());

    let Ok(content) = std::fs::read_to_string(repo.join(file_rel)) else {
        return;
    };
    let Ok(ast) = syn::parse_file(&content) else {
        return;
    };
    let dir = parent_dir(file_rel);
    collect_child_mods(repo, &ast.items, &dir, &modpath, modmap, file_mod);
}

fn collect_child_mods(
    repo: &Path,
    items: &[syn::Item],
    dir: &str,
    modpath: &[String],
    modmap: &mut HashMap<Vec<String>, String>,
    file_mod: &mut HashMap<String, Vec<String>>,
) {
    for item in items {
        if let syn::Item::Mod(m) = item {
            let name = m.ident.to_string();
            let mut child = modpath.to_vec();
            child.push(name.clone());
            match &m.content {
                // Inline module: same file, extended path, recurse items.
                Some((_, inner)) => {
                    collect_child_mods(repo, inner, dir, &child, modmap, file_mod);
                }
                // External module: resolve to a sibling file and recurse.
                None => {
                    if let Some(child_rel) = resolve_mod_file(repo, dir, &name) {
                        map_module(repo, &child_rel, child, modmap, file_mod);
                    }
                }
            }
        }
    }
}

/// `mod foo;` in directory `dir` → `dir/foo.rs` or `dir/foo/mod.rs`.
fn resolve_mod_file(repo: &Path, dir: &str, name: &str) -> Option<String> {
    let flat = join(dir, &format!("{name}.rs"));
    if repo.join(&flat).is_file() {
        return Some(flat);
    }
    let nested = join(dir, &format!("{name}/mod.rs"));
    if repo.join(&nested).is_file() {
        return Some(nested);
    }
    None
}

/// Collect every reference (as a segment list) a file makes: `use` tree
/// leaves plus `crate::|self::|super::|<crate>::`-rooted path expressions.
fn file_references(repo: &Path, file_rel: &str) -> Vec<Vec<String>> {
    let Ok(content) = std::fs::read_to_string(repo.join(file_rel)) else {
        return Vec::new();
    };
    let Ok(ast) = syn::parse_file(&content) else {
        return Vec::new();
    };
    let mut out: Vec<Vec<String>> = Vec::new();
    collect_uses(&ast.items, &mut out);
    let mut pc = PathCollector { out: &mut out };
    pc.visit_file(&ast);
    out
}

fn collect_uses(items: &[syn::Item], out: &mut Vec<Vec<String>>) {
    for item in items {
        match item {
            syn::Item::Use(u) => {
                let mut prefix = Vec::new();
                flatten_use(&u.tree, &mut prefix, out);
            }
            syn::Item::Mod(m) => {
                if let Some((_, inner)) = &m.content {
                    collect_uses(inner, out);
                }
            }
            _ => {}
        }
    }
}

fn flatten_use(tree: &syn::UseTree, prefix: &mut Vec<String>, out: &mut Vec<Vec<String>>) {
    match tree {
        syn::UseTree::Path(p) => {
            prefix.push(p.ident.to_string());
            flatten_use(&p.tree, prefix, out);
            prefix.pop();
        }
        syn::UseTree::Name(n) => {
            let mut v = prefix.clone();
            v.push(n.ident.to_string());
            out.push(v);
        }
        syn::UseTree::Rename(r) => {
            // Edge target keys on the real path, not the alias.
            let mut v = prefix.clone();
            v.push(r.ident.to_string());
            out.push(v);
        }
        syn::UseTree::Glob(_) => {
            let mut v = prefix.clone();
            v.push("*".to_string());
            out.push(v);
        }
        syn::UseTree::Group(g) => {
            for t in &g.items {
                flatten_use(t, prefix, out);
            }
        }
    }
}

struct PathCollector<'a> {
    out: &'a mut Vec<Vec<String>>,
}

impl<'ast> Visit<'ast> for PathCollector<'_> {
    fn visit_path(&mut self, path: &'ast syn::Path) {
        if let Some(first) = path.segments.first() {
            let head = first.ident.to_string();
            if matches!(head.as_str(), "crate" | "self" | "super") {
                self.out
                    .push(path.segments.iter().map(|s| s.ident.to_string()).collect());
            }
        }
        // Keep descending — generic args carry their own paths.
        syn::visit::visit_path(self, path);
    }
}

/// Resolve a reference segment list to the workspace file it lands in.
fn resolve(
    segs: &[String],
    cur_krate: &str,
    cur_mod: Option<&[String]>,
    crate_idents: &BTreeSet<String>,
    modmap: &HashMap<Vec<String>, String>,
) -> Option<String> {
    let (mut base, rest_from) = match segs.first().map(String::as_str) {
        Some("crate") => (vec![cur_krate.to_string()], 1),
        Some("self") => (cur_mod?.to_vec(), 1),
        Some("super") => {
            let mut m = cur_mod?.to_vec();
            m.pop()?;
            (m, 1)
        }
        Some(ident) if crate_idents.contains(ident) => (vec![ident.to_string()], 1),
        _ => return None, // std / external / unresolved-relative — skip.
    };
    // `base` must name a known module to start the walk.
    let mut best = modmap.get(&base).cloned();
    for seg in &segs[rest_from..] {
        if seg == "*" {
            break;
        }
        base.push(seg.clone());
        match modmap.get(&base) {
            Some(f) => best = Some(f.clone()),
            None => break, // segment is an item/type within `best`'s file.
        }
    }
    best
}

fn reachable(
    seeds: &BTreeSet<String>,
    edges: &HashMap<String, BTreeSet<String>>,
) -> BTreeSet<String> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut queue: VecDeque<String> = seeds.iter().cloned().collect();
    for s in seeds {
        seen.insert(s.clone());
    }
    while let Some(f) = queue.pop_front() {
        if let Some(targets) = edges.get(&f) {
            for t in targets {
                if seen.insert(t.clone()) {
                    queue.push_back(t.clone());
                }
            }
        }
    }
    seen
}

// ----- small path helpers (workspace-relative, forward-slash strings) -----

fn norm(p: &str) -> String {
    p.replace('\\', "/")
}

fn parent_dir(rel: &str) -> String {
    match rel.rfind('/') {
        Some(i) => rel[..i].to_string(),
        None => String::new(),
    }
}

fn join(dir: &str, rest: &str) -> String {
    if dir.is_empty() {
        rest.to_string()
    } else {
        format!("{dir}/{rest}")
    }
}

fn walk_rs(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(dir).into_iter().flatten() {
        let p = entry.path();
        if p.is_file() && p.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(p.to_path_buf());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, body).unwrap();
    }

    /// crate `app` with: lib.rs → mod a, mod util; a.rs uses util; orphan.rs
    /// uses nothing and nothing uses it.
    fn fixture() -> TempDir {
        let d = TempDir::new().unwrap();
        let r = d.path();
        write(r, "crates/app/Cargo.toml", "[package]\nname=\"app\"\n");
        write(
            r,
            "crates/app/src/lib.rs",
            "pub mod a;\npub mod util;\npub mod orphan;\n",
        );
        write(
            r,
            "crates/app/src/a.rs",
            "use crate::util::helper;\npub fn go() { helper(); }\n",
        );
        write(r, "crates/app/src/util.rs", "pub fn helper() {}\n");
        write(r, "crates/app/src/orphan.rs", "pub fn dead() {}\n");
        d
    }

    #[test]
    fn util_reachable_from_seed_orphan_not() {
        let d = fixture();
        let r = d.path();
        // Spec seeds a.rs only.
        write(
            r,
            "spec/components/app/a.md",
            "---\nid: a\nkind: component\ncode:\n  - crates/app/src/a.rs\n---\nbody\n",
        );
        let report = analyze(r);
        let unc: Vec<&str> = report.uncovered.iter().map(|u| u.file.as_str()).collect();
        // a.rs is the seed (covered); util.rs reached via `use crate::util` (covered).
        assert!(!unc.contains(&"crates/app/src/a.rs"));
        assert!(!unc.contains(&"crates/app/src/util.rs"));
        // orphan.rs and lib.rs are reachable from nothing the spec declares.
        assert!(unc.contains(&"crates/app/src/orphan.rs"));
        assert!(unc.contains(&"crates/app/src/lib.rs"));
    }

    #[test]
    fn missing_seed_is_reported() {
        let d = fixture();
        let r = d.path();
        write(
            r,
            "spec/components/app/x.md",
            "---\nid: x\nkind: component\ncode:\n  - crates/app/src/gone.rs\n---\nb\n",
        );
        let report = analyze(r);
        assert!(
            report
                .missing_seeds
                .contains(&"crates/app/src/gone.rs".to_string())
        );
    }

    #[test]
    fn use_edge_resolution() {
        let d = fixture();
        let edges = build_graph(d.path()).edges;
        let a = edges.get("crates/app/src/a.rs").unwrap();
        assert!(
            a.contains("crates/app/src/util.rs"),
            "a.rs → util.rs: {a:?}"
        );
    }
}
