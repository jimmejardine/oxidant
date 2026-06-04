```yaml
---
id: session-persistence
kind: component
parent: overview
order: 3
implements: []
depends_on:
  - components/core/conversation
code:
  - crates/oxidant-vcs/src/session_persist.rs
status: active
responsibility: |
  Persist and restore per-exploration conversation transcripts as append-only .jsonl files inside the worktree.
---
```

## File layout

Inside each worktree:
```
.oxidant/
├── sessions/
│   ├── <exploration_id>.jsonl   ← append-only transcript
│   └── <exploration_id>.meta    ← branch, created_at, last_seen
└── (gitignored)                  ← .gitignore generated on first write
```

`.oxidant/` is added to the worktree's `.git/info/exclude` (not the tracked `.gitignore`) so it doesn't pollute the user's commits.

## Append model

Each `Message` appended on creation as one JSON line. Crash safety: line buffer flushed on every message; `O_APPEND` open mode ensures no corruption from concurrent writers (which shouldn't exist in MVP).

## Restore

On app launch:
1. `git worktree list --porcelain` enumerates worktrees.
2. For each, scan `.oxidant/sessions/*.jsonl`.
3. Build `ExplorationSummary { id, branch, last_seen, message_count }` for each.
4. Surface in the GUI's exploration list ([[components/gui/exploration-list]]).
5. Full transcript loaded lazily when the user opens the exploration's window.

## Compaction

Append-only files grow. v1 doesn't compact. v2 may rewrite older entries through a summariser; out of scope here.

## Archive on discard

When [[components/vcs/worktree-mgmt]] discards a worktree, the transcript is moved to `~/.local/share/oxidant/archive/<id>.jsonl` (Linux/macOS) or `%LOCALAPPDATA%\oxidant\archive\<id>.jsonl` (Windows). User can purge manually.

## Test override

When `OXIDANT_DATA_LOCAL_DIR` is set, the data-local-dir helper in `session_persist` returns the env-var value verbatim and skips the `directories::ProjectDirs` lookup. Intended for tests that need to isolate from the host's real archive directory — `archive()` writes under that path instead of the user's actual `~/.local/share/oxidant/`. Production code never sets this env var.

Safe under `cargo nextest` (process-per-test); racy under `cargo test`'s parallel single-process runner because env-var mutations leak across tests in the same process. The project standard is nextest (see CLAUDE.md). Mirrors `OXIDANT_CONFIG_PATH` in [[components/config/settings]].
