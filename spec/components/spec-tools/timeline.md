```yaml
---
id: timeline
kind: component
parent: overview
order: 3
implements: []
depends_on:
  - components/spec-tools/index-db
  - components/vcs/git-shellout
code:
  - crates/oxidant-spec-tools/src/timeline.rs
status: active
responsibility: |
  Provide chronological queries over spec and code change history by combining git log with the SQLite metadata index.
---
```

The timeline is **git** — oxidant does not maintain a parallel change log. This component is the thin layer that turns git history into structured query results for the agent and the GUI.

See [[decisions/0010-spec-index-and-search]] for the rationale (no timestamps in frontmatter, no duplicated history).

## What it provides

- Recent-change feeds filtered by kind / status / path
- Per-spec change history (who changed this spec when, with commit SHA + message)
- Co-change detection: "when X is touched, what else tends to change in the same commit"
- Authorship summaries (one author per commit, aggregated by spec or component)

## Underlying calls

```
git log --pretty=format:'%H%x09%aI%x09%an%x09%s' --name-status -- <pathspec...>
git log -1 --format=%H%x09%aI -- <path>
git log --pretty=format:%H --follow -- <path>          # for renames
```

Results are cached in the SQLite index (`commits` and `commit_files` tables added by [[components/spec-tools/index-db]]; not duplicated here to keep this spec focused). The cache key is the commit SHA, so cached results are immutable; warming the cache is `git log --all --format=...` once on first launch.

## Co-change detection

For a target file `F`, identify the set of commits `C` that touched `F`. For each other path `G` touched in any commit in `C`, count `|C ∩ commits(G)|` divided by `|C|` → frequency that `G` co-changes with `F`. Threshold + top-N returns the co-change ranking. Cached, invalidated when new commits arrive.

This is what powers the GUI's "related" sidebar when viewing a spec or code file: *"the last 12 times this spec changed, these 3 code files also changed in 11 of them"*.

## Non-goals

- Does not replace `git log` / `git blame` — those remain the source of truth and the user can call them directly via [[tools/bash/bash]].
- Does not store deltas. We don't reconstruct content state; only enumerate which files changed in which commits.
- Does not detect *why* something changed. Commit messages are the textual record; this component just surfaces them, doesn't summarise.
