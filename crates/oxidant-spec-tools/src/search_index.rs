// Realises spec/components/spec-tools/search-index.md.
//
// A single Tantivy full-text index at <repo>/.oxidant/search-index/ over
// both spec markdown and Rust source. BM25 ranking, snippets, source/
// kind/lang filters. Per-process lazy init keyed by canonical workspace
// path — first call builds the index from scratch (re-indexing on file
// change is a follow-up phase).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};

use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::{Field, INDEXED, STORED, STRING, Schema, TEXT, Value};
use tantivy::snippet::SnippetGenerator;
use tantivy::{Index, IndexReader, TantivyDocument, doc};

use crate::frontmatter;
use crate::walker::walk_specs;

static SEARCH_INDEXES: OnceLock<StdMutex<HashMap<PathBuf, Arc<SearchIndex>>>> = OnceLock::new();

fn cache() -> &'static StdMutex<HashMap<PathBuf, Arc<SearchIndex>>> {
    SEARCH_INDEXES.get_or_init(|| StdMutex::new(HashMap::new()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchSource {
    Spec,
    Code,
    Both,
}

#[derive(Debug, Clone)]
pub struct SearchQuery {
    pub text: String,
    pub source: SearchSource,
    pub kind: Option<String>,
    pub lang: Option<String>,
    pub limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub path: String,
    pub source: String,
    pub kind: Option<String>,
    pub lang: Option<String>,
    pub frontmatter_id: Option<String>,
    pub score: f32,
    pub snippet: String,
}

struct Fields {
    path: Field,
    source: Field,
    kind: Field,
    lang: Field,
    frontmatter_id: Field,
    content: Field,
    headings: Field,
    mtime: Field,
}

pub struct SearchIndex {
    workspace: PathBuf,
    index: Index,
    reader: IndexReader,
    fields: Fields,
}

impl SearchIndex {
    /// Get-or-build the Tantivy index for `workspace_root`. First call
    /// walks spec/**/*.md and crates/**/*.rs and writes documents.
    pub fn for_workspace(workspace_root: &Path) -> Result<Arc<SearchIndex>, String> {
        let canonical = dunce::canonicalize(workspace_root)
            .map_err(|e| format!("canonicalise workspace failed: {e}"))?;
        {
            let map = cache().lock().unwrap();
            if let Some(existing) = map.get(&canonical) {
                return Ok(existing.clone());
            }
        }
        let built = Self::build(&canonical)?;
        let arc = Arc::new(built);
        let mut map = cache().lock().unwrap();
        Ok(map.entry(canonical).or_insert(arc).clone())
    }

    fn build(workspace: &Path) -> Result<Self, String> {
        let index_dir = workspace.join(".oxidant").join("search-index");
        // Start from a clean directory each process; the on-disk presence
        // satisfies the spec's "single Tantivy index at .oxidant/search-index/"
        // requirement, but cross-process incremental updates are deferred.
        if index_dir.exists() {
            let _ = std::fs::remove_dir_all(&index_dir);
        }
        std::fs::create_dir_all(&index_dir)
            .map_err(|e| format!("create search-index dir: {e}"))?;

        let mut schema_builder = Schema::builder();
        let path = schema_builder.add_text_field("path", TEXT | STORED);
        let source = schema_builder.add_text_field("source", STRING | STORED);
        let kind = schema_builder.add_text_field("kind", STRING | STORED);
        let lang = schema_builder.add_text_field("lang", STRING | STORED);
        let frontmatter_id = schema_builder.add_text_field("frontmatter_id", STRING | STORED);
        let content = schema_builder.add_text_field("content", TEXT);
        let headings = schema_builder.add_text_field("headings", TEXT | STORED);
        let mtime = schema_builder.add_i64_field("mtime", INDEXED | STORED);
        let schema = schema_builder.build();

        let index = Index::create_in_dir(&index_dir, schema.clone())
            .map_err(|e| format!("create tantivy index: {e}"))?;
        let fields = Fields {
            path,
            source,
            kind,
            lang,
            frontmatter_id,
            content,
            headings,
            mtime,
        };

        let mut writer = index
            .writer(50_000_000)
            .map_err(|e| format!("tantivy writer: {e}"))?;
        Self::index_specs(workspace, &mut writer, &fields)?;
        Self::index_code(workspace, &mut writer, &fields)?;
        writer
            .commit()
            .map_err(|e| format!("tantivy commit: {e}"))?;

        let reader = index
            .reader_builder()
            .reload_policy(tantivy::ReloadPolicy::Manual)
            .try_into()
            .map_err(|e| format!("tantivy reader: {e}"))?;

        Ok(SearchIndex {
            workspace: workspace.to_path_buf(),
            index,
            reader,
            fields,
        })
    }

    fn index_specs(
        workspace: &Path,
        writer: &mut tantivy::IndexWriter,
        f: &Fields,
    ) -> Result<(), String> {
        for record in walk_specs(workspace) {
            let rel = record
                .path
                .strip_prefix(workspace)
                .unwrap_or(record.path.as_path())
                .to_string_lossy()
                .replace('\\', "/");
            let mtime = std::fs::metadata(&record.path)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            let headings = extract_headings(&record.file.body);
            writer
                .add_document(doc!(
                    f.path => rel,
                    f.source => "spec",
                    f.kind => record.file.frontmatter.kind.as_str(),
                    f.lang => "",
                    f.frontmatter_id => record.file.frontmatter.id.clone(),
                    f.content => record.file.body.clone(),
                    f.headings => headings,
                    f.mtime => mtime,
                ))
                .map_err(|e| format!("add spec doc: {e}"))?;
        }
        Ok(())
    }

    fn index_code(
        workspace: &Path,
        writer: &mut tantivy::IndexWriter,
        f: &Fields,
    ) -> Result<(), String> {
        let crates_dir = workspace.join("crates");
        if !crates_dir.exists() {
            return Ok(());
        }
        for entry in WalkBuilder::new(&crates_dir).follow_links(false).build() {
            let Ok(entry) = entry else { continue };
            if !entry.file_type().is_some_and(|ft| ft.is_file()) {
                continue;
            }
            let path = entry.path();
            let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
            if ext != "rs" {
                continue;
            }
            let rel = path
                .strip_prefix(workspace)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/");
            let Ok(content) = std::fs::read_to_string(path) else {
                continue;
            };
            let mtime = std::fs::metadata(path)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            writer
                .add_document(doc!(
                    f.path => rel,
                    f.source => "code",
                    f.kind => "",
                    f.lang => "rs",
                    f.frontmatter_id => "",
                    f.content => content,
                    f.headings => "",
                    f.mtime => mtime,
                ))
                .map_err(|e| format!("add code doc: {e}"))?;
        }
        Ok(())
    }

    pub fn search(&self, q: &SearchQuery) -> Result<Vec<SearchHit>, String> {
        let searcher = self.reader.searcher();
        let mut parser = QueryParser::for_index(
            &self.index,
            vec![self.fields.content, self.fields.headings, self.fields.path],
        );
        parser.set_conjunction_by_default();

        let mut tantivy_q = q.text.clone();
        // Compose source/kind/lang filters into the query string — Tantivy's
        // QueryParser accepts "field:value" terms for indexed string fields.
        match q.source {
            SearchSource::Spec => tantivy_q = format!("({tantivy_q}) AND source:spec"),
            SearchSource::Code => tantivy_q = format!("({tantivy_q}) AND source:code"),
            SearchSource::Both => {}
        }
        if let Some(k) = &q.kind {
            tantivy_q = format!("({tantivy_q}) AND kind:{k}");
        }
        if let Some(l) = &q.lang {
            tantivy_q = format!("({tantivy_q}) AND lang:{l}");
        }

        let parsed = parser
            .parse_query(&tantivy_q)
            .map_err(|e| format!("parse query {tantivy_q:?}: {e}"))?;
        let top = searcher
            .search(&parsed, &TopDocs::with_limit(q.limit))
            .map_err(|e| format!("search failed: {e}"))?;

        let snippet_gen = SnippetGenerator::create(&searcher, &*parsed, self.fields.content)
            .map_err(|e| format!("snippet generator: {e}"))?;

        let mut hits = Vec::new();
        for (score, doc_addr) in top {
            let doc: TantivyDocument = searcher
                .doc(doc_addr)
                .map_err(|e| format!("fetch doc: {e}"))?;
            let path = string_field(&doc, self.fields.path).unwrap_or_default();
            let source = string_field(&doc, self.fields.source).unwrap_or_default();
            let kind = optional_string_field(&doc, self.fields.kind);
            let lang = optional_string_field(&doc, self.fields.lang);
            let frontmatter_id = optional_string_field(&doc, self.fields.frontmatter_id);
            let snippet = snippet_gen.snippet_from_doc(&doc).to_html();
            hits.push(SearchHit {
                path,
                source,
                kind,
                lang,
                frontmatter_id,
                score,
                snippet,
            });
        }
        Ok(hits)
    }
}

fn string_field(doc: &TantivyDocument, field: Field) -> Option<String> {
    doc.get_first(field)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
}

fn optional_string_field(doc: &TantivyDocument, field: Field) -> Option<String> {
    string_field(doc, field).filter(|s| !s.is_empty())
}

fn extract_headings(body: &str) -> String {
    let mut out = String::new();
    for line in body.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            out.push_str(trimmed);
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write(repo: &Path, rel: &str, body: &str) {
        let path = repo.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    fn fixture_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        write(
            dir.path(),
            "spec/components/x.md",
            "---\nid: x\nkind: component\nstatus: active\nresponsibility: handle workspace edits\n---\n# Heading\n\nThis component handles workspace edits atomically.\n",
        );
        write(
            dir.path(),
            "crates/foo/src/lib.rs",
            "pub fn workspace_edit_apply() -> bool { true }\n",
        );
        dir
    }

    #[test]
    fn search_finds_spec_and_code() {
        let repo = fixture_repo();
        let idx = SearchIndex::for_workspace(repo.path()).unwrap();
        let hits = idx
            .search(&SearchQuery {
                text: "workspace".into(),
                source: SearchSource::Both,
                kind: None,
                lang: None,
                limit: 10,
            })
            .unwrap();
        assert!(!hits.is_empty(), "expected hits, got 0");
        let paths: Vec<&str> = hits.iter().map(|h| h.path.as_str()).collect();
        assert!(
            paths.iter().any(|p| p.contains("spec/components/x.md")),
            "expected spec hit, got {paths:?}"
        );
    }

    #[test]
    fn search_source_filter_excludes_other() {
        let repo = fixture_repo();
        let idx = SearchIndex::for_workspace(repo.path()).unwrap();
        let hits = idx
            .search(&SearchQuery {
                text: "workspace".into(),
                source: SearchSource::Code,
                kind: None,
                lang: None,
                limit: 10,
            })
            .unwrap();
        assert!(hits.iter().all(|h| h.source == "code"), "got {hits:?}");
    }
}
