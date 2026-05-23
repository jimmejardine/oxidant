// Realises spec/components/spec-tools/timeline.md.
//
// Thin layer over `git log` for chronological queries. Per the spec the
// timeline is git — oxidant does not maintain a parallel change log. This
// module shells out to git, parses structured output, returns Vec<Commit>
// for tools/timeline/{spec,code}-changes.rs to consume.
//
// SQLite caching of commits and co-change detection are deferred per the
// component spec ("warming the cache is git log --all --format=... once
// on first launch" — follow-up phase).

use std::path::Path;

use serde::Serialize;
use tokio::process::Command;

#[derive(Debug, Default, Clone)]
pub struct TimelineFilter {
    /// Restrict to commits touching at least one of these path globs.
    pub pathspecs: Vec<String>,
    /// e.g. "7 days ago", "2026-05-01", "v0.2".
    pub since: Option<String>,
    pub until: Option<String>,
    pub author: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Commit {
    pub sha: String,
    pub iso_date: String,
    pub author: String,
    pub subject: String,
    pub files_touched: Vec<String>,
}

pub struct Timeline;

impl Timeline {
    /// Query `git log` under `workspace`. Returns commits newest-first.
    pub async fn query(workspace: &Path, filter: &TimelineFilter) -> Result<Vec<Commit>, String> {
        // `--pretty=format:<commit-header>` then per-commit a name-status block.
        // Use a sentinel between commits so multi-line subjects don't confuse
        // the parser. Format: <SHA>\t<ISO>\t<author>\t<subject>\n then
        // name-status lines, then a blank line separator (default git behaviour).
        let mut cmd = Command::new("git");
        cmd.arg("log")
            .arg("--pretty=format:OXIDANT-COMMIT%x09%H%x09%aI%x09%an%x09%s")
            .arg("--name-status")
            .arg("--no-renames")
            .current_dir(workspace);

        if let Some(s) = &filter.since {
            cmd.arg(format!("--since={s}"));
        }
        if let Some(s) = &filter.until {
            cmd.arg(format!("--until={s}"));
        }
        if let Some(a) = &filter.author {
            cmd.arg(format!("--author={a}"));
        }
        if let Some(l) = filter.limit {
            cmd.arg(format!("-{l}"));
        }
        if !filter.pathspecs.is_empty() {
            cmd.arg("--");
            for p in &filter.pathspecs {
                cmd.arg(p);
            }
        }

        let output = cmd
            .output()
            .await
            .map_err(|e| format!("spawn git log: {e}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!(
                "git log exited {:?}: {}",
                output.status.code(),
                stderr
            ));
        }
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        Ok(parse_log_output(&stdout))
    }
}

fn parse_log_output(text: &str) -> Vec<Commit> {
    let mut commits = Vec::new();
    let mut current: Option<Commit> = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("OXIDANT-COMMIT\t") {
            if let Some(commit) = current.take() {
                commits.push(commit);
            }
            let parts: Vec<&str> = rest.splitn(4, '\t').collect();
            if parts.len() == 4 {
                current = Some(Commit {
                    sha: parts[0].to_string(),
                    iso_date: parts[1].to_string(),
                    author: parts[2].to_string(),
                    subject: parts[3].to_string(),
                    files_touched: Vec::new(),
                });
            }
        } else if !line.trim().is_empty() {
            // name-status line: <STATUS>\t<path> (or <STATUS>\t<old>\t<new> for renames)
            let cols: Vec<&str> = line.split('\t').collect();
            let file_path = if cols.len() >= 3 && cols[0].starts_with('R') {
                cols[2].to_string()
            } else if cols.len() >= 2 {
                cols[1].to_string()
            } else {
                continue;
            };
            if let Some(c) = current.as_mut() {
                c.files_touched.push(file_path);
            }
        }
    }
    if let Some(c) = current {
        commits.push(c);
    }
    commits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_commit_with_files() {
        let out = "OXIDANT-COMMIT\tabc123\t2026-05-21T14:00:00Z\tJane\tFix the thing\nM\tsrc/foo.rs\nA\tspec/x.md\n";
        let commits = parse_log_output(out);
        assert_eq!(commits.len(), 1);
        let c = &commits[0];
        assert_eq!(c.sha, "abc123");
        assert_eq!(c.author, "Jane");
        assert_eq!(c.subject, "Fix the thing");
        assert_eq!(c.files_touched, vec!["src/foo.rs", "spec/x.md"]);
    }

    #[test]
    fn parses_multiple_commits() {
        let out = "\
OXIDANT-COMMIT\tdeadbee\t2026-05-22T09:00:00Z\tBob\tSecond commit
M\tCargo.toml
OXIDANT-COMMIT\tcafe123\t2026-05-21T14:00:00Z\tAlice\tFirst commit
A\treadme.md
A\tsrc/lib.rs
";
        let commits = parse_log_output(out);
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].sha, "deadbee");
        assert_eq!(commits[1].sha, "cafe123");
        assert_eq!(commits[1].files_touched, vec!["readme.md", "src/lib.rs"]);
    }

    #[test]
    fn handles_renames() {
        let out =
            "OXIDANT-COMMIT\tabc\t2026-05-22T00:00:00Z\tX\tRename\nR100\tsrc/old.rs\tsrc/new.rs\n";
        let commits = parse_log_output(out);
        assert_eq!(commits[0].files_touched, vec!["src/new.rs"]);
    }
}
