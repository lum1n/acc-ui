//! Shell wrappers off commands, so the UI shows what the agent asked for.

const SHELLS: &[&str] = &["sh", "bash", "zsh", "dash", "fish", "ksh"];

/// `"/usr/bin/zsh -lc 'cargo test'"` becomes `cargo test`. Anything that
/// isn't a shell run with `-c` (or `-lc`, `-ic`) and one quoted script comes
/// back unchanged.
pub fn unwrap_shell(command: &str) -> String {
    try_unwrap(command.trim()).unwrap_or_else(|| command.trim().to_owned())
}

fn try_unwrap(command: &str) -> Option<String> {
    let (program, rest) = command.split_once(' ')?;
    let name = program.rsplit('/').next()?;
    if !SHELLS.contains(&name) {
        return None;
    }
    let (flag, script) = rest.trim_start().split_once(' ')?;
    if !(flag.starts_with('-') && flag.ends_with('c') && flag[1..].bytes().all(|b| b.is_ascii_alphabetic())) {
        return None;
    }
    unquote(script.trim())
}

/// One shell word that starts with a quote, or a bare script.
fn unquote(s: &str) -> Option<String> {
    match s.chars().next()? {
        '\'' | '"' => word(s),
        _ => Some(s.to_owned()),
    }
}

/// `s` as exactly one shell word, its quoted and bare pieces run together:
/// `'it'\''s'`, or Codex's `"rg -g '"'!*.md'"'"`. In `"..."` a backslash
/// escapes `"`, `\`, `$` and `` ` ``.
fn word(s: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match c {
            '\'' => loop {
                match chars.next()? {
                    '\'' => break,
                    c => out.push(c),
                }
            },
            '"' => loop {
                match chars.next()? {
                    '"' => break,
                    '\\' => match chars.next()? {
                        e @ ('"' | '\\' | '$' | '`') => out.push(e),
                        '\n' => {}
                        e => {
                            out.push('\\');
                            out.push(e);
                        }
                    },
                    c => out.push(c),
                }
            },
            '\\' => out.push(chars.next()?),
            c if c.is_whitespace() => return None,
            c => out.push(c),
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::unwrap_shell;

    #[test]
    fn unwraps_shells() {
        assert_eq!(
            unwrap_shell("/usr/bin/zsh -lc 'python3 -m unittest'"),
            "python3 -m unittest"
        );
        assert_eq!(
            unwrap_shell("bash -c \"echo \\\"hi\\\" \\$HOME\""),
            "echo \"hi\" $HOME"
        );
        assert_eq!(unwrap_shell("sh -c 'it'\\''s'"), "it's");
        assert_eq!(
            unwrap_shell("/usr/bin/zsh -lc \"python3 -c 'print(1)'\""),
            "python3 -c 'print(1)'"
        );
        // Codex's quoting of a `'` in a double-quoted script.
        assert_eq!(
            unwrap_shell(r#"/usr/bin/zsh -lc "rg --files -g '"'!**/.git/**'"'""#),
            "rg --files -g '!**/.git/**'"
        );
        // Bare pieces and escapes join the quoted ones into one word.
        assert_eq!(unwrap_shell("sh -c 'a'b\"c\""), "abc");
        assert_eq!(unwrap_shell("sh -c 'a'\\ b"), "a b");
        // In double quotes a backslash keeps other characters, and drops a newline.
        assert_eq!(unwrap_shell("bash -c \"a\\nb\\\nc\""), "a\\nbc");
    }

    #[test]
    fn leaves_other_commands() {
        for c in [
            "cargo test",
            "bash script.sh",
            "zsh -lc 'unterminated",
            "zsh -lc 'a' 'b'",
            "zsh -lc 'a'\t'b'",
            "bash -c \"echo",
            "bash -c \"a\\",
            "sh -c 'a'\\",
            "zsh --login -c 'x'",
        ] {
            assert_eq!(unwrap_shell(c), c);
        }
    }
}
