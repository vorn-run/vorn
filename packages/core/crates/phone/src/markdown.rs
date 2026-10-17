//! The marks a phone keyboard lacks, applied to a description as it is written.

/// The key bar over a description: what each key is called and what it applies.
pub const KEYS: [(&str, &str); 8] = [
    ("H1", "h1"),
    ("B", "bold"),
    ("I", "italic"),
    ("</>", "code"),
    ("• list", "bullet"),
    ("☐ task", "checkbox"),
    ("link", "link"),
    ("“", "quote"),
];

fn wrap(key: &str) -> Option<(&'static str, &'static str)> {
    Some(match key {
        "bold" => ("**", "**"),
        "italic" => ("*", "*"),
        "code" => ("`", "`"),
        "link" => ("[", "](url)"),
        _ => return None,
    })
}

fn prefix(key: &str) -> Option<&'static str> {
    Some(match key {
        "h1" => "# ",
        "bullet" => "- ",
        "checkbox" => "- [ ] ",
        "quote" => "> ",
        _ => return None,
    })
}

/// Applies `key` to `text` with `start..end` selected, offsets in chars; the new text and selection.
///
/// A mark wraps the selection, the caret between the marks when nothing is selected; a line mark
/// starts the caret's line, or comes off it when it is already there.
pub fn key(text: &str, start: usize, end: usize, key: &str) -> (String, usize, usize) {
    let chars: Vec<char> = text.chars().collect();
    let start = start.min(chars.len());
    let end = end.clamp(start, chars.len());
    let s = |r: std::ops::Range<usize>| chars[r].iter().collect::<String>();
    let len = |m: &str| m.chars().count();
    if let Some((open, close)) = wrap(key) {
        let chosen = s(start..end);
        let out = format!(
            "{}{open}{chosen}{close}{}",
            s(0..start),
            s(end..chars.len())
        );
        let caret = if chosen.is_empty() {
            start + len(open)
        } else {
            start + len(open) + len(&chosen) + len(close)
        };
        return (out, caret, caret);
    }
    let Some(prefix) = prefix(key) else {
        return (text.to_owned(), start, end);
    };
    let from = chars[..start]
        .iter()
        .rposition(|c| *c == '\n')
        .map_or(0, |i| i + 1);
    let line = s(from..chars.len());
    let n = len(prefix);
    if let Some(rest) = line.strip_prefix(prefix) {
        let out = format!("{}{rest}", s(0..from));
        let back = |at: usize| at.saturating_sub(n).max(from);
        (out, back(start), back(end))
    } else {
        let out = format!("{}{prefix}{line}", s(0..from));
        (out, start + n, end + n)
    }
}

#[cfg(test)]
mod tests {
    use super::key;

    #[test]
    fn marks_wrap_and_leave_the_caret_where_typing_goes() {
        assert_eq!(key("a b", 2, 3, "bold"), ("a **b**".into(), 7, 7));
        assert_eq!(key("ab", 1, 1, "code"), ("a``b".into(), 2, 2));
        assert_eq!(key("go", 0, 2, "link"), ("[go](url)".into(), 9, 9));
    }

    #[test]
    fn line_marks_toggle_on_the_caret_line() {
        assert_eq!(key("one\ntwo", 5, 5, "bullet"), ("one\n- two".into(), 7, 7));
        assert_eq!(key("one\n- two", 7, 7, "bullet"), ("one\ntwo".into(), 5, 5));
        assert_eq!(key("\nx", 0, 0, "h1"), ("# \nx".into(), 2, 2));
    }

    #[test]
    fn offsets_count_chars_not_bytes() {
        assert_eq!(key("é x", 2, 3, "italic"), ("é *x*".into(), 5, 5));
    }
}
