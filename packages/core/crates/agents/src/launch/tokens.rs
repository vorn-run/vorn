//! Reading a launch line well enough to remove a session selector from it,
//! and refusing to when that cannot be done safely (`launch-tokens`).
//!
//! A configured command is a string the launch line interpolates unescaped:
//! `npx -y @anthropic-ai/claude-code` is a working configuration. So there is
//! no argument array to inspect, only a line, and it has to be read as one.
//!
//! The point is to leave exactly one selector on the line. Too cautious ships
//! two, which starts the wrong session; too confident mangles somebody's
//! command, which starts nothing. So anything this cannot reason about is
//! handed back untouched: an operator, an unterminated quote, a
//! substitution, a trailing backslash. `-r<id>` joined into one token and
//! clustered shorts like `-ir` are left alone too, since splitting them
//! needs to know which letters take values.
//!
//! The server reads the line in UTF-16 code units and this reads it in bytes.
//! Every character either one stops at is ASCII, which UTF-8 never uses
//! inside a longer character, so both split every line at the same places;
//! only the offsets are counted in different units.

use crate::Agent;

/// One word of a line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token<'a> {
    /// As it appeared, quotes and all.
    pub raw: &'a str,
    /// With quoting removed: what the shell would pass along.
    pub value: String,
    /// Byte offsets into the line, so a splice can put back everything else.
    pub start: usize,
    pub end: usize,
}

/// Bytes that make a line more than one simple command.
fn refused(b: u8) -> bool {
    matches!(b, b'|' | b'&' | b';' | b'<' | b'>' | b'(' | b')' | b'\n')
}

fn blank(b: u8) -> bool {
    b == b' ' || b == b'\t'
}

/// The line split into words, or `None` for every line this must not touch
/// (`tokenize`). `None` is not a failure to work around; it is the answer.
pub fn tokenize(line: &str) -> Option<Vec<Token<'_>>> {
    let b = line.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && blank(b[i]) {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        let start = i;
        // Bytes, pushed only between ASCII boundaries, so always UTF-8.
        let mut value: Vec<u8> = Vec::new();
        while i < b.len() {
            let ch = b[i];
            if blank(ch) {
                break;
            }
            if refused(ch) || ch == b'`' || (ch == b'$' && b.get(i + 1) == Some(&b'(')) {
                return None;
            }
            match ch {
                b'\'' => {
                    let close = i + 1 + memchr(b'\'', &b[i + 1..])?;
                    value.extend_from_slice(&b[i + 1..close]);
                    i = close + 1;
                }
                b'"' => {
                    i += 1;
                    let mut closed = false;
                    while i < b.len() {
                        let c = b[i];
                        if c == b'"' {
                            i += 1;
                            closed = true;
                            break;
                        }
                        if c == b'`' || (c == b'$' && b.get(i + 1) == Some(&b'(')) {
                            return None;
                        }
                        if c == b'\\' {
                            if let Some(&next @ (b'"' | b'\\' | b'$' | b'`')) = b.get(i + 1) {
                                value.push(next);
                                i += 2;
                                continue;
                            }
                        }
                        value.push(c);
                        i += 1;
                    }
                    if !closed {
                        return None;
                    }
                }
                b'\\' => {
                    // A trailing backslash continues the line, so this is not
                    // the whole command.
                    let next = line[i + 1..].chars().next()?;
                    let width = next.len_utf8();
                    value.extend_from_slice(&b[i + 1..i + 1 + width]);
                    i += 1 + width;
                }
                _ => {
                    value.push(ch);
                    i += 1;
                }
            }
        }
        tokens.push(Token {
            raw: &line[start..i],
            value: String::from_utf8(value).expect("split only at ASCII bytes"),
            start,
            end: i,
        });
    }
    Some(tokens)
}

fn memchr(needle: u8, hay: &[u8]) -> Option<usize> {
    hay.iter().position(|&c| c == needle)
}

/// A byte range of the line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Span {
    start: usize,
    end: usize,
}

impl From<&Token<'_>> for Span {
    fn from(t: &Token<'_>) -> Span {
        Span {
            start: t.start,
            end: t.end,
        }
    }
}

/// The flag shapes that can be proven to select a session, split by whether
/// the flag takes a value: that decides whether the token after it belongs
/// to the flag or to the person who wrote the line.
struct Selectors {
    /// `--flag value` and `--flag=value`.
    with_value: &'static [&'static str],
    /// No value; whatever follows is not theirs.
    bare: &'static [&'static str],
}

fn selectors(agent: Agent) -> Option<Selectors> {
    match agent {
        Agent::Claude => Some(Selectors {
            with_value: &["--resume", "-r", "--session-id"],
            bare: &["--continue", "-c"],
        }),
        Agent::Copilot => Some(Selectors {
            with_value: &["--resume", "--session-id"],
            bare: &[],
        }),
        Agent::OpenCode => Some(Selectors {
            with_value: &["--session", "-s"],
            bare: &[],
        }),
        Agent::Codex | Agent::Gemini => None,
    }
}

/// Whether a token could be a flag's value rather than the next flag.
fn is_value(token: Option<&Token<'_>>) -> bool {
    token.is_some_and(|t| !t.value.starts_with('-'))
}

/// A flag and, when it takes one and the next token is not itself a flag,
/// its value. A bare `--resume` is the interactive picker: still a selector,
/// and what follows it is somebody else's argument.
fn flag_spans(tokens: &[Token<'_>], at: usize, sel: &Selectors) -> Option<Vec<Span>> {
    let token = &tokens[at];
    for name in sel.with_value {
        if token.value == *name {
            let next = tokens.get(at + 1);
            return Some(match next {
                Some(next) if is_value(Some(next)) => vec![token.into(), next.into()],
                _ => vec![token.into()],
            });
        }
        if token
            .value
            .strip_prefix(name)
            .is_some_and(|rest| rest.starts_with('='))
        {
            return Some(vec![token.into()]);
        }
    }
    sel.bare
        .contains(&token.value.as_str())
        .then(|| vec![token.into()])
}

/// The spans on the line that select a session; empty when none can be
/// proven. `args_from` is the byte offset where the configured command ends,
/// known exactly because the caller composed the line.
fn selector_spans(line: &str, agent: Agent, args_from: usize) -> Vec<Span> {
    let Some(tokens) = tokenize(line) else {
        return Vec::new();
    };
    let args: Vec<Token<'_>> = tokens
        .into_iter()
        .filter(|t| t.start >= args_from)
        .collect();
    let mut found = Vec::new();

    if agent == Agent::Codex {
        // A subcommand, and only in the first argument position: a `resume`
        // further along is somebody's value.
        if let Some(first) = args.first().filter(|t| t.value == "resume") {
            found.push(first.into());
            if let Some(next) = args
                .get(1)
                .filter(|n| n.value == "--last" || !n.value.starts_with('-'))
            {
                found.push(next.into());
            }
        }
        return found;
    }

    let Some(sel) = selectors(agent) else {
        return found;
    };
    // Past a bare `--` everything is positional: a `--resume` there is a
    // prompt, not a flag.
    let end = args
        .iter()
        .position(|t| t.value == "--")
        .unwrap_or(args.len());
    let scannable = &args[..end];
    let mut at = 0;
    while at < scannable.len() {
        if let Some(spans) = flag_spans(scannable, at, &sel) {
            at += spans.len();
            found.extend(spans);
        } else {
            at += 1;
        }
    }
    found
}

/// The line with its session selectors removed, or unchanged when none could
/// be proven (`stripSessionSelectors`). Rebuilt by copying the gaps out of
/// the original, so every byte outside a proven span survives as it was.
pub fn strip_session_selectors(line: &str, agent: Agent, args_from: usize) -> String {
    let mut spans = selector_spans(line, agent, args_from);
    if spans.is_empty() {
        return line.to_owned();
    }
    spans.sort_by_key(|s| s.start);
    let b = line.as_bytes();
    let mut out = String::with_capacity(line.len());
    let mut at = 0;
    for one in spans {
        // The blanks in front go too, or removing a flag from the middle
        // leaves a double space and removing the last a trailing one.
        let mut from = one.start;
        while from > at && blank(b[from - 1]) {
            from -= 1;
        }
        out.push_str(&line[at..from]);
        at = one.end;
    }
    out.push_str(&line[at..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(line: &str) -> Option<Vec<String>> {
        tokenize(line).map(|t| t.into_iter().map(|t| t.value).collect())
    }

    #[test]
    fn keeps_quoting_out_of_the_value_and_offsets_on_the_line() {
        let tokens = tokenize(r#"claude --model "gpt 4""#).unwrap();
        let raw: Vec<&str> = tokens.iter().map(|t| t.raw).collect();
        assert_eq!(raw, ["claude", "--model", "\"gpt 4\""]);
        assert_eq!(
            values(r#"claude --model "gpt 4""#).unwrap(),
            ["claude", "--model", "gpt 4"]
        );
    }

    #[test]
    fn escapes_a_character_whole_whatever_its_width() {
        assert_eq!(values("a\\é\\😀b").unwrap(), ["aé😀b"]);
        assert_eq!(values(r#""\x\"""#).unwrap(), ["\\x\""]);
    }

    #[test]
    fn refuses_what_it_cannot_reason_about() {
        for line in [
            "a | b",
            "a&&b",
            "a;b",
            "a > b",
            "a(b",
            "a\nb",
            "a $(b)",
            "a `b`",
            "\"a $(b)\"",
            "\"a `b`\"",
            "'a",
            "\"a",
            "a \\",
        ] {
            assert_eq!(values(line), None, "{line:?}");
        }
        // A `$` not opening a substitution, and `\r`, are only characters.
        assert_eq!(values("$HOME a\rb").unwrap(), ["$HOME", "a\rb"]);
    }

    #[test]
    fn offsets_count_bytes() {
        let line = "é --resume x";
        assert_eq!(strip_session_selectors(line, Agent::Claude, "é".len()), "é");
    }
}
