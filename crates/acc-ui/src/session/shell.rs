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

/// One shell word in single or double quotes, or a bare script.
fn unquote(s: &str) -> Option<String> {
    let first = s.chars().next()?;
    match first {
        '\'' => single(s),
        '"' => double(s),
        _ => Some(s.to_owned()),
    }
}

/// `'...'`, with `'\''` for a quote inside.
fn single(s: &str) -> Option<String> {
    let mut out = String::new();
    let mut rest = s;
    loop {
        rest = rest.strip_prefix('\'')?;
        let end = rest.find('\'')?;
        out.push_str(&rest[..end]);
        rest = &rest[end + 1..];
        if rest.is_empty() {
            return Some(out);
        }
        rest = rest.strip_prefix("\\'")?;
        out.push('\'');
        if rest.is_empty() {
            return Some(out);
        }
    }
}

/// `"..."`, where a backslash escapes `"`, `\`, `$` and `` ` ``.
fn double(s: &str) -> Option<String> {
    let inner = s.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return chars.as_str().is_empty().then_some(out),
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
    }
    None
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
    }

    #[test]
    fn leaves_other_commands() {
        for c in [
            "cargo test",
            "bash script.sh",
            "zsh -lc 'unterminated",
            "zsh -lc 'a' 'b'",
            "zsh --login -c 'x'",
        ] {
            assert_eq!(unwrap_shell(c), c);
        }
    }
}
