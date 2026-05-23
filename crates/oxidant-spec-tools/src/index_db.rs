// Realises spec/components/spec-tools/index-db.md.
//
// SQLite metadata index of the spec tree at `<repo>/.oxidant/spec-index.db`.
// Rebuilt fully on first query per workspace per process; incremental
// updates via a notify watcher are deferred to a follow-up phase.
//
// Schema follows the component spec verbatim:
//   specs(id PK, kind, parent, path, status, order_idx, line_count,
//         last_modified, last_commit)
//   spec_edges(src_id, dst_ref, edge_kind) PK(src_id, dst_ref, edge_kind)
//   spec_code_paths(spec_id, code_path) PK(spec_id, code_path)
//
// last_modified / last_commit are NULL in MVP — populating them requires
// git log calls per file, deferred until the timeline component lands the
// shared commits cache.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};

use rusqlite::{Connection, OpenFlags, params};
use serde::Serialize;

use crate::frontmatter::SpecFile;
use crate::walker::walk_specs;

static INDEX_DBS: OnceLock<StdMutex<HashMap<PathBuf, Arc<StdMutex<IndexDb>>>>> = OnceLock::new();

fn cache() -> &'static StdMutex<HashMap<PathBuf, Arc<StdMutex<IndexDb>>>> {
    INDEX_DBS.get_or_init(|| StdMutex::new(HashMap::new()))
}

#[derive(Debug, Clone, Serialize)]
pub struct SpecRow {
    pub id: String,
    pub kind: String,
    pub parent: Option<String>,
    pub path: String,
    pub status: String,
    pub order_idx: Option<i64>,
    pub line_count: i64,
    pub last_modified: Option<String>,
    pub last_commit: Option<String>,
}

#[derive(Debug, Default, Clone)]
pub struct SpecFilter {
    pub kind: Option<String>,
    pub status: Option<String>,
    pub parent: Option<String>,
    pub implements: Option<String>,
    pub depends_on: Option<String>,
    pub depended_by: Option<String>,
    pub orphans_only: bool,
    pub limit: Option<usize>,
}

pub struct IndexDb {
    workspace: PathBuf,
    conn: Connection,
}

impl IndexDb {
    /// Get-or-create the SQLite index for `workspace_root`. The DB lives at
    /// `<workspace>/.oxidant/spec-index.db`. On first access per process,
    /// walks the spec tree and populates the tables. Subsequent calls
    /// reuse the cached connection — stale wrt on-disk edits made after
    /// init (re-indexing on notify is deferred).
    pub fn for_workspace(workspace_root: &Path) -> Result<Arc<StdMutex<IndexDb>>, String> {
        let canonical = dunce::canonicalize(workspace_root)
            .map_err(|e| format!("canonicalise workspace failed: {e}"))?;
        {
            let map = cache().lock().unwrap();
            if let Some(existing) = map.get(&canonical) {
                return Ok(existing.clone());
            }
        }
        let db = Self::build(&canonical)?;
        let arc = Arc::new(StdMutex::new(db));
        let mut map = cache().lock().unwrap();
        Ok(map.entry(canonical).or_insert(arc).clone())
    }

    fn build(workspace: &Path) -> Result<Self, String> {
        let oxidant_dir = workspace.join(".oxidant");
        std::fs::create_dir_all(&oxidant_dir).map_err(|e| format!("create .oxidant dir: {e}"))?;
        let db_path = oxidant_dir.join("spec-index.db");
        // Open the DB; remove any stale one so the schema/data starts fresh
        // each process. (Persistent caching across process restarts is a
        // follow-up — the file existence and shape are what matter for now.)
        if db_path.exists() {
            let _ = std::fs::remove_file(&db_path);
        }
        let conn = Connection::open_with_flags(
            &db_path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )
        .map_err(|e| format!("open spec-index.db: {e}"))?;
        Self::create_schema(&conn)?;
        let mut me = IndexDb {
            workspace: workspace.to_path_buf(),
            conn,
        };
        me.populate_from_walk()?;
        Ok(me)
    }

    fn create_schema(conn: &Connection) -> Result<(), String> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS specs (
                id              TEXT PRIMARY KEY,
                kind            TEXT NOT NULL,
                parent          TEXT,
                path            TEXT NOT NULL,
                status          TEXT NOT NULL,
                order_idx       INTEGER,
                line_count      INTEGER NOT NULL,
                last_modified   TEXT,
                last_commit     TEXT
             );
             CREATE TABLE IF NOT EXISTS spec_edges (
                src_id          TEXT NOT NULL,
                dst_ref         TEXT NOT NULL,
                edge_kind       TEXT NOT NULL,
                PRIMARY KEY (src_id, dst_ref, edge_kind)
             );
             CREATE TABLE IF NOT EXISTS spec_code_paths (
                spec_id         TEXT NOT NULL,
                code_path       TEXT NOT NULL,
                PRIMARY KEY (spec_id, code_path)
             );
             CREATE INDEX IF NOT EXISTS idx_specs_kind   ON specs(kind);
             CREATE INDEX IF NOT EXISTS idx_specs_parent ON specs(parent);
             CREATE INDEX IF NOT EXISTS idx_edges_dst    ON spec_edges(dst_ref);
             CREATE INDEX IF NOT EXISTS idx_edges_kind   ON spec_edges(edge_kind);
             CREATE INDEX IF NOT EXISTS idx_code_path    ON spec_code_paths(code_path);",
        )
        .map_err(|e| format!("create schema: {e}"))
    }

    fn populate_from_walk(&mut self) -> Result<(), String> {
        let records = walk_specs(&self.workspace);
        let tx = self
            .conn
            .transaction()
            .map_err(|e| format!("begin tx: {e}"))?;
        {
            let mut insert_spec = tx
                .prepare(
                    "INSERT OR REPLACE INTO specs (
                        id, kind, parent, path, status, order_idx, line_count,
                        last_modified, last_commit
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, NULL)",
                )
                .map_err(|e| format!("prepare specs insert: {e}"))?;
            let mut insert_edge = tx
                .prepare(
                    "INSERT OR IGNORE INTO spec_edges (src_id, dst_ref, edge_kind) VALUES (?1, ?2, ?3)",
                )
                .map_err(|e| format!("prepare edges insert: {e}"))?;
            let mut insert_code_path = tx
                .prepare(
                    "INSERT OR IGNORE INTO spec_code_paths (spec_id, code_path) VALUES (?1, ?2)",
                )
                .map_err(|e| format!("prepare code-paths insert: {e}"))?;

            for rec in &records {
                let path_str = rec
                    .path
                    .strip_prefix(&self.workspace)
                    .unwrap_or(rec.path.as_path())
                    .to_string_lossy()
                    .replace('\\', "/");
                let line_count = rec.file.body.lines().count() as i64;
                insert_spec
                    .execute(params![
                        rec.canonical_id,
                        rec.file.frontmatter.kind.as_str(),
                        rec.file.frontmatter.parent.as_deref(),
                        path_str,
                        rec.file.frontmatter.status.as_str(),
                        rec.file.frontmatter.order,
                        line_count,
                    ])
                    .map_err(|e| format!("insert spec {}: {e}", rec.canonical_id))?;

                insert_edges_for(&rec.canonical_id, &rec.file, &mut insert_edge)?;
                insert_code_paths_for(&rec.canonical_id, &rec.file, &mut insert_code_path)?;
            }
        }
        tx.commit().map_err(|e| format!("commit: {e}"))
    }

    /// Run a filtered query against the specs table. Multiple filters AND
    /// together. Returns rows sorted by id for stability.
    pub fn query(&self, filter: &SpecFilter) -> Result<Vec<SpecRow>, String> {
        let mut clauses: Vec<String> = Vec::new();
        let mut bindings: Vec<String> = Vec::new();

        if let Some(k) = &filter.kind {
            clauses.push("s.kind = ?".into());
            bindings.push(k.clone());
        }
        if let Some(s) = &filter.status {
            clauses.push("s.status = ?".into());
            bindings.push(s.clone());
        }
        if let Some(p) = &filter.parent {
            clauses.push("s.parent = ?".into());
            bindings.push(p.clone());
        }
        if let Some(r) = &filter.implements {
            clauses.push(
                "s.id IN (SELECT src_id FROM spec_edges WHERE dst_ref = ? AND edge_kind = 'implements')"
                    .into(),
            );
            bindings.push(r.clone());
        }
        if let Some(r) = &filter.depends_on {
            clauses.push(
                "s.id IN (SELECT src_id FROM spec_edges WHERE dst_ref = ? AND edge_kind = 'depends_on')"
                    .into(),
            );
            bindings.push(r.clone());
        }
        if let Some(r) = &filter.depended_by {
            clauses.push(
                "EXISTS (SELECT 1 FROM spec_edges e WHERE e.src_id = ? AND e.dst_ref = s.id AND e.edge_kind = 'depends_on')"
                    .into(),
            );
            bindings.push(r.clone());
        }
        if filter.orphans_only {
            clauses.push("NOT EXISTS (SELECT 1 FROM spec_edges WHERE dst_ref = s.id)".into());
            // Exclude overview/glossary from the orphan list — they're roots by
            // construction (no inbound edges is correct, not a smell).
            clauses.push("s.kind NOT IN ('overview', 'glossary')".into());
        }

        let where_clause = if clauses.is_empty() {
            "1 = 1".to_string()
        } else {
            clauses.join(" AND ")
        };
        let limit = filter.limit.unwrap_or(50_000);
        let sql = format!(
            "SELECT id, kind, parent, path, status, order_idx, line_count,
                    last_modified, last_commit
             FROM specs s
             WHERE {where_clause}
             ORDER BY id ASC
             LIMIT {limit}"
        );

        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(|e| format!("prepare query: {e}"))?;
        let bindings_refs: Vec<&dyn rusqlite::ToSql> =
            bindings.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
        let rows = stmt
            .query_map(bindings_refs.as_slice(), |row| {
                Ok(SpecRow {
                    id: row.get(0)?,
                    kind: row.get(1)?,
                    parent: row.get(2)?,
                    path: row.get(3)?,
                    status: row.get(4)?,
                    order_idx: row.get(5)?,
                    line_count: row.get(6)?,
                    last_modified: row.get(7)?,
                    last_commit: row.get(8)?,
                })
            })
            .map_err(|e| format!("query_map: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("row iter: {e}"))?;
        Ok(rows)
    }

    /// Lookup specs whose `code:` paths include the given relative path.
    pub fn specs_for_code_path(&self, code_path: &str) -> Result<Vec<String>, String> {
        let normalised = code_path.replace('\\', "/");
        let mut stmt = self
            .conn
            .prepare("SELECT spec_id FROM spec_code_paths WHERE code_path = ?1 ORDER BY spec_id")
            .map_err(|e| format!("prepare: {e}"))?;
        let rows = stmt
            .query_map(params![normalised], |row| row.get::<_, String>(0))
            .map_err(|e| format!("query_map: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("row iter: {e}"))?;
        Ok(rows)
    }
}

impl Drop for IndexDb {
    fn drop(&mut self) {
        // SQLite connection drops automatically; nothing to do here.
        // The on-disk file at .oxidant/spec-index.db persists.
    }
}

fn insert_edges_for(
    src_id: &str,
    file: &SpecFile,
    stmt: &mut rusqlite::Statement<'_>,
) -> Result<(), String> {
    let fm = &file.frontmatter;
    if let Some(parent) = &fm.parent {
        stmt.execute(params![src_id, parent, "parent"])
            .map_err(|e| format!("edge parent: {e}"))?;
    }
    for r in &fm.implements {
        stmt.execute(params![src_id, r, "implements"])
            .map_err(|e| format!("edge implements: {e}"))?;
    }
    for r in &fm.depends_on {
        stmt.execute(params![src_id, r, "depends_on"])
            .map_err(|e| format!("edge depends_on: {e}"))?;
    }
    for m in &file.refs_in_body {
        stmt.execute(params![src_id, m.raw, "body_ref"])
            .map_err(|e| format!("edge body_ref: {e}"))?;
    }
    Ok(())
}

fn insert_code_paths_for(
    spec_id: &str,
    file: &SpecFile,
    stmt: &mut rusqlite::Statement<'_>,
) -> Result<(), String> {
    for code in &file.frontmatter.code {
        let s = code.to_string_lossy().replace('\\', "/");
        stmt.execute(params![spec_id, s])
            .map_err(|e| format!("insert code_path: {e}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_spec(repo: &Path, rel: &str, body: &str) {
        let path = repo.join("spec").join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    fn fixture_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        write_spec(
            dir.path(),
            "overview.md",
            "---\nid: overview\nkind: overview\nstatus: active\nresponsibility: root\n---\nentry point\n",
        );
        write_spec(
            dir.path(),
            "components/a.md",
            "---\nid: a\nkind: component\nparent: overview\nstatus: active\nresponsibility: A\ncode:\n  - crates/foo/src/a.rs\n---\nbody\n",
        );
        write_spec(
            dir.path(),
            "components/b.md",
            "---\nid: b\nkind: component\nparent: overview\nstatus: draft\ndepends_on:\n  - components/a\nresponsibility: B\ncode:\n  - crates/foo/src/b.rs\n---\nbody\n",
        );
        dir
    }

    #[test]
    fn query_by_kind_returns_components() {
        let repo = fixture_repo();
        let db = IndexDb::for_workspace(repo.path()).unwrap();
        let db = db.lock().unwrap();
        let rows = db
            .query(&SpecFilter {
                kind: Some("component".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn query_by_status_filter() {
        let repo = fixture_repo();
        let db = IndexDb::for_workspace(repo.path()).unwrap();
        let db = db.lock().unwrap();
        let rows = db
            .query(&SpecFilter {
                status: Some("draft".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "components/b");
    }

    #[test]
    fn query_by_depends_on() {
        let repo = fixture_repo();
        let db = IndexDb::for_workspace(repo.path()).unwrap();
        let db = db.lock().unwrap();
        let rows = db
            .query(&SpecFilter {
                depends_on: Some("components/a".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "components/b");
    }

    #[test]
    fn specs_for_code_path_reverse_lookup() {
        let repo = fixture_repo();
        let db = IndexDb::for_workspace(repo.path()).unwrap();
        let db = db.lock().unwrap();
        let specs = db.specs_for_code_path("crates/foo/src/a.rs").unwrap();
        assert_eq!(specs, vec!["components/a".to_string()]);
    }
}
