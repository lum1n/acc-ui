//! Links in agent text that an app can make tappable: files (with a line),
//! pull requests, commits and web pages.
//!
//! Found, in this order (an earlier match wins where they overlap):
//! 1. markdown links, `[label](target)`: a URL, or a path (`#L12` or `:12`
//!    gives the line);
//! 2. bare `http(s)://` URLs;
//! 3. `path:line` (or `path:line:col`) where the path has an extension;
//! 4. absolute paths under the session's root.
//!
//! GitHub pull request and commit URLs are told apart from other pages.
//! Ranges are byte offsets into the text.

use serde::Serialize;

/// One link in a text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Link {
    /// Byte range of the whole match in the text.
    pub start: usize,
    pub end: usize,
    pub kind: LinkKind,
    /// The URL, or the file's path (relative to the root when under it).
    pub target: String,
    /// What a markdown link shows; `None` for bare links.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// `owner/name`, for pull requests and commits.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// The pull request's number.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number: Option<u64>,
    /// The commit's hash.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    File,
    Pr,
    Commit,
    Url,
}

/// The links in `text`, in order. `root` is the session's working
/// directory: paths under it come back relative, and bare absolute paths
/// are only taken when under it.
pub fn detect_links(text: &str, root: Option<&str>) -> Vec<Link> {
    let root = root.map(|r| r.trim_end_matches('/')).filter(|r| !r.is_empty());
    let mut found: Vec<Link> = Vec::new();
    let add = |link: Link, found: &mut Vec<Link>| {
        if !found.iter().any(|l| link.start < l.end && l.start < link.end) {
            found.push(link);
        }
    };
    for l in markdown_links(text, root) {
        add(l, &mut found);
    }
    for (start, end) in urls(text) {
        add(url_link(start, end, &text[start..end], None), &mut found);
    }
    for l in path_lines(text, root) {
        add(l, &mut found);
    }
    if let Some(r) = root {
        for l in root_paths(text, r) {
            add(l, &mut found);
        }
    }
    found.sort_by_key(|l| l.start);
    found
}

fn markdown_links(text: &str, root: Option<&str>) -> Vec<Link> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(open) = text[i..].find('[').map(|o| o + i) {
        i = open + 1;
        let Some(close) = text[open..].find("](").map(|c| c + open) else {
            break;
        };
        let label = &text[open + 1..close];
        if label.contains('\n') || label.contains('[') {
            continue;
        }
        let Some(end) = text[close + 2..].find(')').map(|e| e + close + 2) else {
            continue;
        };
        let target = text[close + 2..end].trim();
        if target.is_empty() || target.contains(char::is_whitespace) {
            continue;
        }
        let label = Some(label.to_owned());
        let link = if target.starts_with("http://") || target.starts_with("https://") {
            url_link(open, end + 1, target, label)
        } else if target.contains("://") || target.starts_with("mailto:") || target.starts_with('#') {
            continue;
        } else {
            let (path, line) = split_line(target);
            file_link(open, end + 1, path, line, root, label)
        };
        out.push(link);
        i = end + 1;
    }
    out
}

/// `path#L12`, `path:12` or `path:12:3` into the path and line.
fn split_line(target: &str) -> (&str, Option<u32>) {
    let number = |s: &str| {
        (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse().ok())
            .flatten()
    };
    if let Some((p, frag)) = target.split_once("#L") {
        let n: String = frag.chars().take_while(char::is_ascii_digit).collect();
        return (p, number(&n));
    }
    let Some((rest, last)) = target.rsplit_once(':') else {
        return (target, None);
    };
    let Some(last) = number(last) else {
        return (target, None);
    };
    match rest.rsplit_once(':') {
        Some((p, line)) if number(line).is_some() => (p, number(line)),
        _ => (rest, Some(last)),
    }
}

fn file_link(
    start: usize,
    end: usize,
    path: &str,
    line: Option<u32>,
    root: Option<&str>,
    label: Option<String>,
) -> Link {
    let target = root
        .and_then(|r| path.strip_prefix(r))
        .and_then(|rest| rest.strip_prefix('/'))
        .filter(|rest| !rest.is_empty())
        .unwrap_or(path)
        .to_owned();
    Link {
        start,
        end,
        kind: LinkKind::File,
        target,
        label,
        line,
        repo: None,
        number: None,
        sha: None,
    }
}

fn url_link(start: usize, end: usize, url: &str, label: Option<String>) -> Link {
    let mut link = Link {
        start,
        end,
        kind: LinkKind::Url,
        target: url.to_owned(),
        label,
        line: None,
        repo: None,
        number: None,
        sha: None,
    };
    let rest = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("http://github.com/"));
    if let Some(rest) = rest {
        let parts: Vec<&str> = rest.split(['/', '?', '#']).collect();
        if let [owner, name, what, id, ..] = parts.as_slice() {
            let repo = Some(format!("{owner}/{name}"));
            match *what {
                "pull" => {
                    if let Ok(n) = id.parse() {
                        link.kind = LinkKind::Pr;
                        link.repo = repo;
                        link.number = Some(n);
                    }
                }
                "commit" if id.len() >= 7 && id.chars().all(|c| c.is_ascii_hexdigit()) => {
                    link.kind = LinkKind::Commit;
                    link.repo = repo;
                    link.sha = Some((*id).to_owned());
                }
                _ => {}
            }
        }
    }
    link
}

/// Byte ranges of bare URLs, without trailing punctuation.
fn urls(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(s) = ["https://", "http://"]
        .iter()
        .filter_map(|p| text[i..].find(p).map(|o| o + i))
        .min()
    {
        let mut end = text[s..]
            .find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '`' | '|'))
            .map_or(text.len(), |e| e + s);
        while end > s && text[..end].ends_with(['.', ',', ';', ':', '!', '?', ')', ']', '\'', '*', '_']) {
            // Keep a closing paren that has its opening one in the URL.
            if text[..end].ends_with(')')
                && text[s..end].matches('(').count() >= text[s..end].matches(')').count()
            {
                break;
            }
            end -= 1;
        }
        if end > s + 8 {
            out.push((s, end));
        }
        i = end.max(s + 1);
    }
    out
}

fn is_path_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | '~' | '@' | '+')
}

/// Path-like tokens: (start, end) of runs of path characters.
fn tokens(text: &str) -> impl Iterator<Item = (usize, usize)> + '_ {
    let mut chars = text.char_indices().peekable();
    std::iter::from_fn(move || {
        while let Some(&(_, c)) = chars.peek() {
            if is_path_char(c) {
                break;
            }
            chars.next();
        }
        let (start, _) = *chars.peek()?;
        let mut end = start;
        while let Some(&(i, c)) = chars.peek() {
            if !is_path_char(c) {
                break;
            }
            end = i + c.len_utf8();
            chars.next();
        }
        Some((start, end))
    })
}

fn has_extension(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name.rsplit_once('.') {
        Some((stem, ext)) => {
            !stem.is_empty()
                && (1..=8).contains(&ext.len())
                && ext.chars().all(|c| c.is_ascii_alphanumeric())
                && ext.chars().any(|c| c.is_ascii_alphabetic())
        }
        None => false,
    }
}

/// `src/a.rs:12` and `src/a.rs:12:4`.
fn path_lines(text: &str, root: Option<&str>) -> Vec<Link> {
    let mut out = Vec::new();
    for (start, end) in tokens(text) {
        let path = text[start..end].trim_end_matches('.');
        let after = &text[start + path.len()..];
        if !has_extension(path) || !after.starts_with(':') {
            continue;
        }
        let digits: String = after[1..].chars().take_while(char::is_ascii_digit).collect();
        let Ok(line) = digits.parse::<u32>() else {
            continue;
        };
        let mut stop = start + path.len() + 1 + digits.len();
        let col: String = text[stop..].strip_prefix(':').map_or(String::new(), |r| {
            r.chars().take_while(char::is_ascii_digit).collect()
        });
        if !col.is_empty() {
            stop += 1 + col.len();
        }
        out.push(file_link(start, stop, path, Some(line), root, None));
    }
    out
}

/// Absolute paths under the root, without a line.
fn root_paths(text: &str, root: &str) -> Vec<Link> {
    tokens(text)
        .filter_map(|(start, end)| {
            let path = text[start..end].trim_end_matches(['.', ',']);
            let under = path
                .strip_prefix(root)
                .is_some_and(|r| r.starts_with('/') && r.len() > 1);
            under.then(|| file_link(start, start + path.len(), path, None, Some(root), None))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{LinkKind, detect_links, split_line};

    #[test]
    fn lines() {
        assert_eq!(split_line("a.rs:12"), ("a.rs", Some(12)));
        assert_eq!(split_line("a.rs:12:3"), ("a.rs", Some(12)));
        assert_eq!(split_line("/w/a.rs#L7-L9"), ("/w/a.rs", Some(7)));
        assert_eq!(split_line("a.rs"), ("a.rs", None));
    }

    #[test]
    fn finds_each_kind() {
        let text = "Wrote [README.md](/work/README.md); see src/lib.rs:42:7 and /work/inventory.py. \
                    PR https://github.com/lum1n/acc-ui/pull/3, commit https://github.com/lum1n/acc-ui/commit/b871600abc, \
                    docs (https://example.com/a_(b)).";
        let links = detect_links(text, Some("/work/"));
        let got: Vec<_> = links
            .iter()
            .map(|l| (l.kind, l.target.as_str(), l.line, &text[l.start..l.end]))
            .collect();
        assert_eq!(
            got,
            [
                (LinkKind::File, "README.md", None, "[README.md](/work/README.md)"),
                (LinkKind::File, "src/lib.rs", Some(42), "src/lib.rs:42:7"),
                (LinkKind::File, "inventory.py", None, "/work/inventory.py"),
                (
                    LinkKind::Pr,
                    "https://github.com/lum1n/acc-ui/pull/3",
                    None,
                    "https://github.com/lum1n/acc-ui/pull/3"
                ),
                (
                    LinkKind::Commit,
                    "https://github.com/lum1n/acc-ui/commit/b871600abc",
                    None,
                    "https://github.com/lum1n/acc-ui/commit/b871600abc"
                ),
                (
                    LinkKind::Url,
                    "https://example.com/a_(b)",
                    None,
                    "https://example.com/a_(b)"
                ),
            ]
        );
        assert_eq!(links[3].number, Some(3));
        assert_eq!(links[3].repo.as_deref(), Some("lum1n/acc-ui"));
        assert_eq!(links[4].sha.as_deref(), Some("b871600abc"));
    }

    #[test]
    fn leaves_lookalikes() {
        for t in ["version 1.2:3", "at 10:30", "e.g.: this", "/etc/passwd", "a.b.c"] {
            assert!(detect_links(t, Some("/work")).is_empty(), "{t}");
        }
    }
}
