// Realises spec/components/vcs/session-persistence.md.
//
// Per-exploration transcripts live at
//   <worktree>/.oxidant/sessions/<exploration_id>.jsonl    ← append-only
//   <worktree>/.oxidant/sessions/<exploration_id>.meta     ← JSON sidecar
//
// `.oxidant/` is added to the worktree's `.git/info/exclude` by
// worktree::spawn so it never enters commits. Lazy load: full
// transcript only deserialised when the user opens the exploration's
// window.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: String,
    pub branch: String,
    pub created_at: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionSummary {
    pub id: String,
    pub branch: String,
    pub last_seen: DateTime<Utc>,
    pub message_count: usize,
    pub transcript_path: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("io error on {path}: {message}", path = path.display())]
    Io { path: PathBuf, message: String },
    #[error("serialise message: {0}")]
    Serialise(String),
}

fn sessions_dir(worktree: &Path) -> PathBuf {
    worktree.join(".oxidant").join("sessions")
}

fn transcript_path(worktree: &Path, id: &str) -> PathBuf {
    sessions_dir(worktree).join(format!("{id}.jsonl"))
}

fn meta_path(worktree: &Path, id: &str) -> PathBuf {
    sessions_dir(worktree).join(format!("{id}.meta"))
}

/// Initialise a new session: write the .meta sidecar and touch the
/// .jsonl transcript file. Idempotent on the meta file (existing meta
/// is overwritten with the new branch/last_seen).
pub fn init_session(worktree: &Path, id: &str, branch: &str) -> Result<SessionMeta, SessionError> {
    let dir = sessions_dir(worktree);
    std::fs::create_dir_all(&dir).map_err(|e| SessionError::Io {
        path: dir.clone(),
        message: e.to_string(),
    })?;
    let now = Utc::now();
    let meta = SessionMeta {
        id: id.to_string(),
        branch: branch.to_string(),
        created_at: now,
        last_seen: now,
    };
    let meta_p = meta_path(worktree, id);
    let meta_json = serde_json::to_string_pretty(&meta).map_err(|e| SessionError::Serialise(e.to_string()))?;
    std::fs::write(&meta_p, meta_json).map_err(|e| SessionError::Io {
        path: meta_p,
        message: e.to_string(),
    })?;
    let t_p = transcript_path(worktree, id);
    if !t_p.exists() {
        std::fs::write(&t_p, "").map_err(|e| SessionError::Io {
            path: t_p,
            message: e.to_string(),
        })?;
    }
    Ok(meta)
}

/// Append one already-serialised message line to the transcript. The
/// caller is responsible for serialisation; we don't know the
/// Message type here (it lives in oxidant-core and we don't depend on
/// it from this component).
pub fn append_message_line(
    worktree: &Path,
    id: &str,
    json_line: &str,
) -> Result<(), SessionError> {
    let path = transcript_path(worktree, id);
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| SessionError::Io {
            path: path.clone(),
            message: e.to_string(),
        })?;
    file.write_all(json_line.as_bytes())
        .map_err(|e| SessionError::Io {
            path: path.clone(),
            message: e.to_string(),
        })?;
    if !json_line.ends_with('\n') {
        file.write_all(b"\n").map_err(|e| SessionError::Io {
            path: path.clone(),
            message: e.to_string(),
        })?;
    }
    // Update last_seen in the meta sidecar (best effort; failure
    // shouldn't poison the transcript append we just did).
    let _ = touch_last_seen(worktree, id);
    Ok(())
}

fn touch_last_seen(worktree: &Path, id: &str) -> Result<(), SessionError> {
    let p = meta_path(worktree, id);
    let body = match std::fs::read_to_string(&p) {
        Ok(b) => b,
        Err(_) => return Ok(()),
    };
    let mut meta: SessionMeta =
        serde_json::from_str(&body).map_err(|e| SessionError::Serialise(e.to_string()))?;
    meta.last_seen = Utc::now();
    let new_body = serde_json::to_string_pretty(&meta)
        .map_err(|e| SessionError::Serialise(e.to_string()))?;
    std::fs::write(&p, new_body).map_err(|e| SessionError::Io {
        path: p,
        message: e.to_string(),
    })?;
    Ok(())
}

/// Enumerate sessions in a worktree's .oxidant/sessions/ dir. Each is
/// summarised with branch, last_seen, message count.
pub fn list_sessions(worktree: &Path) -> Vec<SessionSummary> {
    let dir = sessions_dir(worktree);
    let mut out = Vec::new();
    let read_dir = match std::fs::read_dir(&dir) {
        Ok(r) => r,
        Err(_) => return out,
    };
    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        let id = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        let message_count = std::fs::read_to_string(&path)
            .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count())
            .unwrap_or(0);
        let meta = std::fs::read_to_string(meta_path(worktree, &id))
            .ok()
            .and_then(|s| serde_json::from_str::<SessionMeta>(&s).ok());
        let (branch, last_seen) = match meta {
            Some(m) => (m.branch, m.last_seen),
            None => ("unknown".to_string(), Utc::now()),
        };
        out.push(SessionSummary {
            id,
            branch,
            last_seen,
            message_count,
            transcript_path: path,
        });
    }
    out.sort_by(|a, b| b.last_seen.cmp(&a.last_seen));
    out
}

/// Archive the transcript to the user-level archive directory and
/// remove the in-worktree copies. Returns the archive path.
pub fn archive(worktree: &Path, id: &str) -> Result<PathBuf, SessionError> {
    let src = transcript_path(worktree, id);
    if !src.exists() {
        return Err(SessionError::Io {
            path: src,
            message: "transcript not found".into(),
        });
    }
    let archive_dir = match directories::ProjectDirs::from("ai", "oxidant", "oxidant") {
        Some(d) => d.data_local_dir().join("archive"),
        None => worktree.join(".oxidant").join("archive-local"),
    };
    std::fs::create_dir_all(&archive_dir).map_err(|e| SessionError::Io {
        path: archive_dir.clone(),
        message: e.to_string(),
    })?;
    let dst = archive_dir.join(format!("{id}.jsonl"));
    std::fs::copy(&src, &dst).map_err(|e| SessionError::Io {
        path: dst.clone(),
        message: e.to_string(),
    })?;
    let _ = std::fs::remove_file(&src);
    let _ = std::fs::remove_file(meta_path(worktree, id));
    Ok(dst)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn init_and_append_then_list() {
        let dir = TempDir::new().unwrap();
        let wt = dir.path();
        init_session(wt, "abc", "feat/x").unwrap();
        append_message_line(wt, "abc", r#"{"role":"user","text":"hi"}"#).unwrap();
        append_message_line(wt, "abc", r#"{"role":"assistant","text":"yo"}"#).unwrap();
        let summaries = list_sessions(wt);
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].id, "abc");
        assert_eq!(summaries[0].branch, "feat/x");
        assert_eq!(summaries[0].message_count, 2);
    }

    #[test]
    fn archive_moves_transcript_to_user_dir() {
        let dir = TempDir::new().unwrap();
        let wt = dir.path();
        init_session(wt, "xyz", "feat/x").unwrap();
        append_message_line(wt, "xyz", r#"{"role":"user"}"#).unwrap();
        let archived = archive(wt, "xyz").unwrap();
        assert!(archived.exists());
        assert!(!transcript_path(wt, "xyz").exists());
        // cleanup the user archive so the test doesn't leak
        let _ = std::fs::remove_file(&archived);
    }
}
