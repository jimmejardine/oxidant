// Parse a chat-input draft for leading-slash commands. The chat panel
// dispatches the result before reaching the agent loop. See
// spec/components/gui/chat-input-panel.md "Slash commands".

#[derive(Debug, PartialEq, Eq)]
pub enum ChatCommand<'a> {
    Clear,
    Compact,
    /// Unknown command head (without the slash). The panel surfaces
    /// this inline and preserves the draft so the user can fix the
    /// typo.
    Unknown(&'a str),
    /// Not a command — pass through to the LLM unchanged.
    Prompt(&'a str),
}

/// Parse the user's submitted draft. A leading `/` (after trimming
/// only leading whitespace) selects a command; the head is the first
/// whitespace-delimited token after the slash, lowercased.
///
/// Bare `/` (slash with no command name) is treated as a normal prompt
/// so users typing a literal slash aren't surprised.
pub fn parse(input: &str) -> ChatCommand<'_> {
    let trimmed = input.trim_start();
    let Some(after_slash) = trimmed.strip_prefix('/') else {
        return ChatCommand::Prompt(input);
    };
    let head = match after_slash.split_whitespace().next() {
        Some(h) => h,
        None => return ChatCommand::Prompt(input),
    };
    match head.to_ascii_lowercase().as_str() {
        "clear" => ChatCommand::Clear,
        "compact" => ChatCommand::Compact,
        _ => ChatCommand::Unknown(head),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_clear() {
        assert_eq!(parse("/clear"), ChatCommand::Clear);
    }

    #[test]
    fn parses_clear_case_insensitive() {
        assert_eq!(parse("/Clear"), ChatCommand::Clear);
        assert_eq!(parse("/CLEAR"), ChatCommand::Clear);
    }

    #[test]
    fn parses_compact() {
        assert_eq!(parse("/compact"), ChatCommand::Compact);
    }

    #[test]
    fn ignores_trailing_whitespace_and_args() {
        assert_eq!(parse("/clear  "), ChatCommand::Clear);
        assert_eq!(parse("/compact some args"), ChatCommand::Compact);
    }

    #[test]
    fn tolerates_leading_whitespace() {
        assert_eq!(parse("  /clear"), ChatCommand::Clear);
    }

    #[test]
    fn unknown_command_returns_head_without_slash() {
        match parse("/foo") {
            ChatCommand::Unknown(head) => assert_eq!(head, "foo"),
            other => panic!("expected Unknown(\"foo\"), got {other:?}"),
        }
    }

    #[test]
    fn unknown_with_args_keeps_just_the_head() {
        match parse("/foo bar baz") {
            ChatCommand::Unknown(head) => assert_eq!(head, "foo"),
            other => panic!("expected Unknown(\"foo\"), got {other:?}"),
        }
    }

    #[test]
    fn bare_slash_falls_through_as_prompt() {
        match parse("/") {
            ChatCommand::Prompt(p) => assert_eq!(p, "/"),
            other => panic!("expected Prompt, got {other:?}"),
        }
    }

    #[test]
    fn no_slash_is_prompt() {
        match parse("hello world") {
            ChatCommand::Prompt(p) => assert_eq!(p, "hello world"),
            other => panic!("expected Prompt, got {other:?}"),
        }
    }

    #[test]
    fn empty_input_is_prompt() {
        match parse("") {
            ChatCommand::Prompt(p) => assert_eq!(p, ""),
            other => panic!("expected Prompt, got {other:?}"),
        }
    }

    #[test]
    fn slash_inside_text_is_prompt() {
        // Only LEADING slash counts. "ratio 1/2" is just a prompt.
        match parse("ratio 1/2") {
            ChatCommand::Prompt(p) => assert_eq!(p, "ratio 1/2"),
            other => panic!("expected Prompt, got {other:?}"),
        }
    }
}
