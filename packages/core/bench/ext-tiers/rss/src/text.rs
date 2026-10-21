//! Entities, whitespace and markup as text, as the connector's `text.ts` reads them.

use std::sync::LazyLock;

use regex::Regex;

/// The longest summary an item keeps, in characters.
pub const SUMMARY_LENGTH: usize = 2000;

const LATIN1: &str = "Agrave Aacute Acirc Atilde Auml Aring AElig Ccedil Egrave Eacute Ecirc Euml Igrave Iacute Icirc Iuml ETH Ntilde Ograve Oacute Ocirc Otilde Ouml times Oslash Ugrave Uacute Ucirc Uuml Yacute THORN szlig agrave aacute acirc atilde auml aring aelig ccedil egrave eacute ecirc euml igrave iacute icirc iuml eth ntilde ograve oacute ocirc otilde ouml divide oslash ugrave uacute ucirc uuml yacute thorn yuml";

const NAMED: [(&str, char); 20] = [
    ("amp", '&'),
    ("lt", '<'),
    ("gt", '>'),
    ("quot", '"'),
    ("apos", '\''),
    ("nbsp", '\u{a0}'),
    ("rsquo", '’'),
    ("lsquo", '‘'),
    ("rdquo", '”'),
    ("ldquo", '“'),
    ("hellip", '…'),
    ("mdash", '—'),
    ("ndash", '–'),
    ("copy", '©'),
    ("reg", '®'),
    ("trade", '™'),
    ("laquo", '«'),
    ("raquo", '»'),
    ("bull", '•'),
    ("middot", '·'),
];

/// The only entity names ever expanded; a name a DOCTYPE declares is left as written.
fn entity(name: &str) -> Option<char> {
    let named = |n: &str| {
        NAMED.iter().find(|(k, _)| *k == n).map(|(_, c)| *c).or_else(|| {
            LATIN1
                .split(' ')
                .position(|k| k == n)
                .and_then(|i| char::from_u32(192 + u32::try_from(i).ok()?))
        })
    };
    named(name).or_else(|| named(&name.to_lowercase()))
}

fn code_point(digits: &str, radix: u32) -> Option<char> {
    u32::from_str_radix(digits, radix)
        .ok()
        .filter(|&c| c > 0)
        .and_then(char::from_u32)
}

static ENTITY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"&(#[xX][0-9a-fA-F]+|#[0-9]+|[A-Za-z][A-Za-z0-9]*);").expect("a valid pattern")
});

/// Numeric references and the fixed table of names decoded; anything else, a stray `&` included, stays as written.
pub fn decode_entities(value: &str) -> String {
    if !value.contains('&') {
        return value.to_owned();
    }
    ENTITY
        .replace_all(value, |c: &regex::Captures<'_>| {
            let name = &c[1];
            let decoded = match name.strip_prefix('#') {
                Some(n) => match n.strip_prefix(['x', 'X']) {
                    Some(hex) => code_point(hex, 16),
                    None => code_point(n, 10),
                },
                None => entity(name),
            };
            decoded.map_or_else(|| c[0].to_owned(), String::from)
        })
        .into_owned()
}

static SPACES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new("[ \t\u{a0}]+").expect("a valid pattern"));

/// Lines trimmed and runs of spaces collapsed, empty lines dropped.
pub fn tidy(value: &str) -> String {
    let lines: Vec<String> = value
        .split("\r\n")
        .flat_map(|l| l.split(['\n', '\r']))
        .map(|line| js_trim(&SPACES.replace_all(line, " ")).to_owned())
        .filter(|line| !line.is_empty())
        .collect();
    lines.join("\n")
}

static SCRIPT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<script\b.*?</script\s*>").expect("a valid pattern"));
static STYLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<style\b.*?</style\s*>").expect("a valid pattern"));
static WHITESPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").expect("a valid pattern"));
static BREAK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)<(br|hr)\b[^>]*>").expect("a valid pattern"));
static BLOCK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)</?(p|h[1-6]|li|blockquote|pre|div|ul|ol|figure|figcaption|table|tr)\b[^>]*>")
        .expect("a valid pattern")
});
static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").expect("a valid pattern"));

/// Markup as plain text: whitespace collapsed as HTML does, one line per block, scripts and styles dropped, entities decoded.
pub fn plain_text(html: &str) -> String {
    // The connector drops both in one pattern with a backreference, which this regex engine lacks.
    let text = SCRIPT.replace_all(html, "");
    let text = STYLE.replace_all(&text, "");
    let text = WHITESPACE.replace_all(&text, " ");
    let text = BREAK.replace_all(&text, "\n");
    let text = BLOCK.replace_all(&text, "\n");
    let text = TAG.replace_all(&text, "");
    tidy(&decode_entities(&text))
}

/// Markup as one line of text, for titles and names.
pub fn inline_text(html: &str) -> String {
    plain_text(html).replace('\n', " ")
}

pub fn one_line(value: &str) -> String {
    tidy(value).replace('\n', " ")
}

pub fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Text cut to `max` characters, the last one an ellipsis when anything was cut.
pub fn cut(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_owned();
    }
    let kept: String = value.chars().take(max - 1).collect();
    format!("{kept}…")
}

/// What JavaScript's `trim` drops: Unicode white space and the byte-order mark.
pub fn is_js_space(c: char) -> bool {
    c.is_whitespace() || c == '\u{feff}'
}

pub fn js_trim(s: &str) -> &str {
    s.trim_matches(is_js_space)
}

pub fn js_trim_start(s: &str) -> &str {
    s.trim_start_matches(is_js_space)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_numbers_and_the_fixed_names_and_leaves_the_rest() {
        assert_eq!(
            decode_entities("a &amp; b &#39;c&#x27; &Eacute; &AMP; &bogus; &#0; & d"),
            "a & b 'c' É & &bogus; &#0; & d"
        );
    }

    #[test]
    fn markup_reads_as_lines_of_text() {
        assert_eq!(
            plain_text("<p>One <b>two</b></p><script>x()</script><br>three&nbsp; four"),
            "One two\nthree four"
        );
        assert_eq!(inline_text("<h1>A</h1><p>B</p>"), "A B");
    }

    #[test]
    fn a_cut_ends_in_an_ellipsis() {
        assert_eq!(cut("abcdef", 4), "abc…");
        assert_eq!(cut("abc", 4), "abc");
    }
}
