// Shared spec-tree walker. Reads `<repo>/spec/**/*.md`, parses each file
// via the frontmatter parser, and yields a SpecRecord per file with its
// canonical id, on-disk path, and parsed contents.
//
// Used by spec_read, spec_for_file, and any future model-facing tool that
// needs the full spec set. Until [[components/spec-tools/index-db]] lands
// this walk is O(n) per call — acceptable for hundreds of specs but worth
// caching once the SQLite index exists.

use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::frontmatter::{self, SpecFile};

#[derive(Debug, Clone)]
pub struct SpecRecord {
    /// Path-style ref relative to `spec/` minus `.md`, with forward slashes.
    /// e.g. `tools/edit/apply-edits`.
    pub canonical_id: String,
    /// Absolute on-disk path.
    pub path: PathBuf,
    /// Parsed file (frontmatter + body + body refs).
    pub file: SpecFile,
}

/// Walk `<repo>/spec/**/*.md` and return one SpecRecord per parseable file.
/// Files that fail to parse are skipped (validate surfaces those separately).
pub fn walk_specs(repo: &Path) -> Vec<SpecRecord> {
    let spec_root = repo.join("spec");
    let mut out: Vec<SpecRecord> = Vec::new();
    for entry in WalkDir::new(&spec_root).into_iter().filter_map(|e| e.ok()) {
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
        out.push(SpecRecord {
            canonical_id,
            path: path.to_path_buf(),
            file,
        });
    }
    out.sort_by(|a, b| a.canonical_id.cmp(&b.canonical_id));
    out
}

#[allow(clippy::result_unit_err)] // internal helper; () error is sufficient and stable
pub fn canonical_id(spec_root: &Path, path: &Path) -> Result<String, ()> {
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
