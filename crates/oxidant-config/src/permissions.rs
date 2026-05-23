// Realises spec/components/config/permissions.md.
//
// Decision matrix for whether a tool call should auto-approve,
// auto-deny, or prompt the user. NOT a sandbox — this is the
// single layer of safety between the agent and the user's
// filesystem (per spec/decisions/0002-no-built-in-sandbox.md).
//
// Default behaviour from the spec:
//   ReadOnly  → Allow
//   Mutating  → Prompt (unless allowlisted)
//   Network   → Prompt (unless allowlisted)
//
// User can flip a session into "trust mode" — every tool auto-approves,
// surfaced as a banner in the GUI so it's visible.
//
// Pattern syntax for allowlist/denylist entries:
//   "fs_write"           — exact tool name match
//   "bash:cargo *"       — bash command glob (globset, segment-wise)
//   "bash:/^git /"       — bash regex (forward-slash delimited)
//   "bash:cat"           — bash substring match

use globset::Glob;
use oxidant_core::ToolCategory;
use regex::Regex;

use crate::settings::PermissionsSettings;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionDecision {
    Allow,
    Deny,
    Prompt,
}

/// Runtime-mutable permission state. The session can flip into
/// trust_mode (e.g. via a GUI banner toggle) which bypasses the
/// decision matrix entirely.
#[derive(Debug, Clone, Default)]
pub struct PermissionState {
    pub trust_mode: bool,
}

pub struct PermissionEngine {
    settings: PermissionsSettings,
    allow_patterns: Vec<CompiledPattern>,
    deny_patterns: Vec<CompiledPattern>,
}

impl PermissionEngine {
    pub fn new(settings: PermissionsSettings) -> Self {
        let allow_patterns = settings
            .allowlist
            .iter()
            .filter_map(|p| CompiledPattern::compile(p).ok())
            .collect();
        let deny_patterns = settings
            .denylist
            .iter()
            .filter_map(|p| CompiledPattern::compile(p).ok())
            .collect();
        Self {
            settings,
            allow_patterns,
            deny_patterns,
        }
    }

    /// Decide whether a tool call should run. `bash_command` is `Some`
    /// only when `tool_name == "bash"` — the spec's bash patterns
    /// match against the command line, not the args JSON.
    pub fn decide(
        &self,
        state: &PermissionState,
        tool_name: &str,
        category: ToolCategory,
        bash_command: Option<&str>,
    ) -> PermissionDecision {
        // Denylist always wins, even in trust mode — protects rm -rf etc.
        if matches_any(&self.deny_patterns, tool_name, bash_command) {
            return PermissionDecision::Deny;
        }
        if state.trust_mode {
            return PermissionDecision::Allow;
        }
        if matches_any(&self.allow_patterns, tool_name, bash_command) {
            return PermissionDecision::Allow;
        }
        match category {
            ToolCategory::ReadOnly => {
                if self.settings.auto_approve_readonly {
                    PermissionDecision::Allow
                } else {
                    PermissionDecision::Prompt
                }
            }
            ToolCategory::Mutating | ToolCategory::Network => PermissionDecision::Prompt,
        }
    }
}

fn matches_any(patterns: &[CompiledPattern], tool_name: &str, bash_command: Option<&str>) -> bool {
    patterns
        .iter()
        .any(|p| p.matches(tool_name, bash_command))
}

#[derive(Debug)]
enum CompiledPattern {
    /// Exact tool-name match (e.g. "fs_write", "apply_edits").
    ToolName(String),
    /// Bash command-line match.
    Bash(BashMatcher),
}

#[derive(Debug)]
enum BashMatcher {
    Substring(String),
    Glob(globset::GlobMatcher),
    Regex(Regex),
}

impl CompiledPattern {
    fn compile(raw: &str) -> Result<Self, String> {
        if let Some(rest) = raw.strip_prefix("bash:") {
            // Regex if wrapped in /.../
            if let Some(inner) = rest.strip_prefix('/').and_then(|t| t.strip_suffix('/')) {
                let re = Regex::new(inner)
                    .map_err(|e| format!("invalid regex {inner:?}: {e}"))?;
                return Ok(CompiledPattern::Bash(BashMatcher::Regex(re)));
            }
            // Glob if it contains a metacharacter; substring otherwise.
            if rest.contains(['*', '?', '[']) {
                let glob = Glob::new(rest)
                    .map_err(|e| format!("invalid glob {rest:?}: {e}"))?;
                return Ok(CompiledPattern::Bash(BashMatcher::Glob(glob.compile_matcher())));
            }
            return Ok(CompiledPattern::Bash(BashMatcher::Substring(rest.to_string())));
        }
        // No `bash:` prefix → exact tool name match.
        if raw.trim().is_empty() {
            return Err("empty pattern".into());
        }
        Ok(CompiledPattern::ToolName(raw.to_string()))
    }

    fn matches(&self, tool_name: &str, bash_command: Option<&str>) -> bool {
        match self {
            CompiledPattern::ToolName(name) => tool_name == name,
            CompiledPattern::Bash(matcher) => {
                if tool_name != "bash" {
                    return false;
                }
                let cmd = match bash_command {
                    Some(c) => c,
                    None => return false,
                };
                match matcher {
                    BashMatcher::Substring(s) => cmd.contains(s.as_str()),
                    BashMatcher::Glob(g) => g.is_match(cmd),
                    BashMatcher::Regex(re) => re.is_match(cmd),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine(allow: Vec<&str>, deny: Vec<&str>) -> PermissionEngine {
        let mut settings = PermissionsSettings::default();
        settings.allowlist = allow.into_iter().map(String::from).collect();
        settings.denylist = deny.into_iter().map(String::from).collect();
        PermissionEngine::new(settings)
    }

    fn st() -> PermissionState {
        PermissionState::default()
    }

    #[test]
    fn readonly_auto_approves_by_default() {
        let e = engine(vec![], vec![]);
        assert_eq!(
            e.decide(&st(), "fs_read", ToolCategory::ReadOnly, None),
            PermissionDecision::Allow
        );
    }

    #[test]
    fn mutating_prompts_unless_allowlisted() {
        let e = engine(vec![], vec![]);
        assert_eq!(
            e.decide(&st(), "fs_write", ToolCategory::Mutating, None),
            PermissionDecision::Prompt
        );

        let e = engine(vec!["fs_write"], vec![]);
        assert_eq!(
            e.decide(&st(), "fs_write", ToolCategory::Mutating, None),
            PermissionDecision::Allow
        );
    }

    #[test]
    fn trust_mode_bypasses_prompt_but_not_deny() {
        let e = engine(vec![], vec!["bash:rm -rf*"]);
        let mut s = st();
        s.trust_mode = true;
        assert_eq!(
            e.decide(&s, "fs_write", ToolCategory::Mutating, None),
            PermissionDecision::Allow
        );
        assert_eq!(
            e.decide(&s, "bash", ToolCategory::Mutating, Some("rm -rf /")),
            PermissionDecision::Deny
        );
    }

    #[test]
    fn bash_glob_pattern_matches() {
        let e = engine(vec!["bash:cargo *"], vec![]);
        assert_eq!(
            e.decide(&st(), "bash", ToolCategory::Mutating, Some("cargo check")),
            PermissionDecision::Allow
        );
        assert_eq!(
            e.decide(&st(), "bash", ToolCategory::Mutating, Some("npm install")),
            PermissionDecision::Prompt
        );
    }

    #[test]
    fn bash_regex_pattern_matches() {
        let e = engine(vec!["bash:/^git (status|log)$/"], vec![]);
        assert_eq!(
            e.decide(&st(), "bash", ToolCategory::Mutating, Some("git status")),
            PermissionDecision::Allow
        );
        assert_eq!(
            e.decide(&st(), "bash", ToolCategory::Mutating, Some("git log")),
            PermissionDecision::Allow
        );
        assert_eq!(
            e.decide(&st(), "bash", ToolCategory::Mutating, Some("git push")),
            PermissionDecision::Prompt
        );
    }

    #[test]
    fn bash_substring_pattern_matches() {
        let e = engine(vec!["bash:cat"], vec![]);
        assert_eq!(
            e.decide(&st(), "bash", ToolCategory::Mutating, Some("cat readme.md")),
            PermissionDecision::Allow
        );
        // Substring is naive — `cat` matches `concatenate` too. The user
        // can switch to a glob (`bash:cat *`) for stricter matching.
        assert_eq!(
            e.decide(&st(), "bash", ToolCategory::Mutating, Some("concatenate")),
            PermissionDecision::Allow
        );
    }

    #[test]
    fn deny_wins_over_allow() {
        let e = engine(vec!["bash:rm*"], vec!["bash:rm -rf*"]);
        // Both patterns match; deny wins.
        assert_eq!(
            e.decide(&st(), "bash", ToolCategory::Mutating, Some("rm -rf /tmp/x")),
            PermissionDecision::Deny
        );
    }

    #[test]
    fn empty_pattern_skipped_silently() {
        let e = engine(vec!["", "fs_read"], vec![]);
        // The empty pattern fails to compile and is dropped; fs_read still works.
        assert_eq!(
            e.decide(&st(), "fs_read", ToolCategory::Mutating, None),
            PermissionDecision::Allow
        );
    }

    #[test]
    fn auto_approve_readonly_off_prompts() {
        let mut settings = PermissionsSettings::default();
        settings.auto_approve_readonly = false;
        settings.allowlist.clear();
        settings.denylist.clear();
        let e = PermissionEngine::new(settings);
        assert_eq!(
            e.decide(&st(), "fs_read", ToolCategory::ReadOnly, None),
            PermissionDecision::Prompt
        );
    }
}
