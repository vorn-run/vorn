//! A doc artifact's Markdown as the page it is read on (`markdown.ts` and
//! `doc-page.ts`). Text is escaped before inline marks, so Markdown cannot
//! smuggle raw HTML through.

use std::sync::LazyLock;

use regex::Regex;

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("a markdown pattern is valid")
}

/// A JavaScript `.`, which stops at every line terminator, not only `\n`.
const DOT: &str = r"[^\r\n\u{2028}\u{2029}]";

static HEADING: LazyLock<Regex> = LazyLock::new(|| re(&format!(r"^(#{{1,3}}) ({DOT}+)$")));
static RULE: LazyLock<Regex> = LazyLock::new(|| re(r"^---+$"));
static TASK: LazyLock<Regex> = LazyLock::new(|| re(&format!(r"^- \[([ x])\] ({DOT}+)$")));
static BULLET: LazyLock<Regex> = LazyLock::new(|| re(&format!(r"^- {DOT}+")));
static ORDERED: LazyLock<Regex> = LazyLock::new(|| re(&format!(r"^[0-9]+\. {DOT}+")));
static ORDERED_MARK: LazyLock<Regex> = LazyLock::new(|| re(r"^[0-9]+\. "));
static STRONG: LazyLock<Regex> = LazyLock::new(|| re(&format!(r"\*\*({DOT}+?)\*\*")));
static EM: LazyLock<Regex> = LazyLock::new(|| re(&format!(r"\*({DOT}+?)\*")));
static STRIKE: LazyLock<Regex> = LazyLock::new(|| re(&format!(r"~~({DOT}+?)~~")));
static CODE: LazyLock<Regex> = LazyLock::new(|| re(r"`([^`]+)`"));
static LINK: LazyLock<Regex> = LazyLock::new(|| re(r"\[([^\]]+)\]\(([^)]+)\)"));
static SAFE_SCHEME: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^(https?|mailto):"));
static ANY_SCHEME: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^[a-z][a-z0-9+.\-]*:"));

/// `escapeHtml`.
pub fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

fn inline(text: &str) -> String {
    let escaped = escape_html(text);
    let strong = STRONG.replace_all(&escaped, "<strong>$1</strong>");
    let em = EM.replace_all(&strong, "<em>$1</em>");
    let strike = STRIKE.replace_all(&em, "<s>$1</s>");
    let code = CODE.replace_all(&strike, "<code>$1</code>");
    LINK.replace_all(&code, |c: &regex::Captures<'_>| {
        let (label, href) = (&c[1], &c[2]);
        if SAFE_SCHEME.is_match(href) || !ANY_SCHEME.is_match(href) {
            format!("<a href=\"{href}\">{label}</a>")
        } else {
            label.to_owned()
        }
    })
    .into_owned()
}

/// `markdownToHtml`.
pub fn to_html(md: &str) -> String {
    if md.trim().is_empty() {
        return String::new();
    }
    let lines: Vec<&str> = md.split('\n').collect();
    let mut parts: Vec<String> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if let Some(lang) = line.strip_prefix("```") {
            let lang = lang.trim();
            let mut code = Vec::new();
            i += 1;
            while i < lines.len() && !lines[i].starts_with("```") {
                code.push(escape_html(lines[i]));
                i += 1;
            }
            i += 1;
            let attr = if lang.is_empty() {
                String::new()
            } else {
                format!(" class=\"language-{}\"", escape_html(lang))
            };
            parts.push(format!("<pre><code{attr}>{}</code></pre>", code.join("\n")));
            continue;
        }
        if let Some(h) = HEADING.captures(line) {
            let level = h[1].len();
            parts.push(format!("<h{level}>{}</h{level}>", inline(&h[2])));
            i += 1;
            continue;
        }
        if RULE.is_match(line.trim()) {
            parts.push("<hr>".to_owned());
            i += 1;
            continue;
        }
        if line.starts_with("> ") {
            let mut quote = Vec::new();
            while i < lines.len() && lines[i].starts_with("> ") {
                quote.push(inline(&lines[i][2..]));
                i += 1;
            }
            parts.push(format!(
                "<blockquote><p>{}</p></blockquote>",
                quote.join("<br>")
            ));
            continue;
        }
        if TASK.is_match(line) {
            let mut items = String::new();
            while let Some(t) = lines.get(i).and_then(|l| TASK.captures(l)) {
                let done = &t[1] == "x";
                items.push_str(&format!(
                    "<li data-type=\"taskItem\" data-checked=\"{done}\"><label><input type=\"checkbox\"{}><span></span></label><div><p>{}</p></div></li>",
                    if done { " checked" } else { "" },
                    inline(&t[2])
                ));
                i += 1;
            }
            parts.push(format!("<ul data-type=\"taskList\">{items}</ul>"));
            continue;
        }
        if BULLET.is_match(line) {
            let mut items = String::new();
            while i < lines.len() && BULLET.is_match(lines[i]) {
                items.push_str(&format!("<li><p>{}</p></li>", inline(&lines[i][2..])));
                i += 1;
            }
            parts.push(format!("<ul>{items}</ul>"));
            continue;
        }
        if ORDERED.is_match(line) {
            let mut items = String::new();
            while i < lines.len() && ORDERED.is_match(lines[i]) {
                let text = ORDERED_MARK.replace(lines[i], "");
                items.push_str(&format!("<li><p>{}</p></li>", inline(&text)));
                i += 1;
            }
            parts.push(format!("<ol>{items}</ol>"));
            continue;
        }
        if line.trim().is_empty() {
            i += 1;
            continue;
        }
        parts.push(format!("<p>{}</p>", inline(line)));
        i += 1;
    }
    parts.concat()
}

const STYLE: &str = r#"
:root{color-scheme:light dark;--bg:#ffffff;--ink:#1f2328;--muted:#5f6b76;--rule:#d8dee4;--code:#f6f8fa}
@media (prefers-color-scheme:dark){:root{--bg:#101012;--ink:#faf9f7;--muted:rgba(255,255,255,.55);--rule:rgba(255,255,255,.12);--code:#1c1c20}}
body{margin:0;background:var(--bg);color:var(--ink);font:16px/1.65 -apple-system,BlinkMacSystemFont,"Helvetica Neue",sans-serif;padding:40px 24px 80px}
main{max-width:68ch;margin:0 auto}
h1,h2,h3{line-height:1.25;margin:1.6em 0 .5em}
h1{font-size:28px;margin-top:0}h2{font-size:21px}h3{font-size:17px}
p,ul,ol,blockquote,pre{margin:0 0 1em}
a{color:inherit}
blockquote{border-left:3px solid var(--rule);padding-left:14px;color:var(--muted)}
code{font:14px ui-monospace,"SF Mono",Menlo,monospace;background:var(--code);padding:1px 4px;border-radius:3px}
pre{background:var(--code);padding:12px 14px;border-radius:4px;overflow-x:auto}
pre code{padding:0;background:none}
hr{border:0;border-top:1px solid var(--rule);margin:2em 0}
ul[data-type=taskList]{list-style:none;padding-left:0}
ul[data-type=taskList] li{display:flex;gap:8px}
ul[data-type=taskList] p{margin:0}
li p{margin:0}
"#;

/// `renderDocPage`: a doc read as a page, in a plain reading column.
pub fn doc_page(title: &str, markdown: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{}</title><style>{STYLE}</style></head><body><main>{}</main></body></html>",
        escape_html(title),
        to_html(markdown)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_blocks_and_inline_marks() {
        let md = "# Title *x*\n\nPara **b** `c` ~~d~~ [ok](https://a) [bad](javascript:x) [rel](/p)\n\n- one\n- two\n\n1. first\n2. second\n\n- [x] done\n- [ ] open\n\n> quote\n> more\n\n---\n```rust\nlet a = <b>;\n```\n";
        assert_eq!(
            to_html(md),
            "<h1>Title <em>x</em></h1><p>Para <strong>b</strong> <code>c</code> <s>d</s> <a href=\"https://a\">ok</a> bad <a href=\"/p\">rel</a></p><ul><li><p>one</p></li><li><p>two</p></li></ul><ol><li><p>first</p></li><li><p>second</p></li></ol><ul data-type=\"taskList\"><li data-type=\"taskItem\" data-checked=\"true\"><label><input type=\"checkbox\" checked><span></span></label><div><p>done</p></div></li><li data-type=\"taskItem\" data-checked=\"false\"><label><input type=\"checkbox\"><span></span></label><div><p>open</p></div></li></ul><blockquote><p>quote<br>more</p></blockquote><hr><pre><code class=\"language-rust\">let a = &lt;b&gt;;</code></pre>"
        );
        assert_eq!(to_html("  \n"), "");
        assert_eq!(
            escape_html("<a href='x'>&\""),
            "&lt;a href=&#39;x&#39;&gt;&amp;&quot;"
        );
    }

    #[test]
    fn a_doc_page_escapes_its_title() {
        let page = doc_page("<T>", "hi");
        assert!(page.contains("<title>&lt;T&gt;</title>"));
        assert!(page.ends_with("<main><p>hi</p></main></body></html>"));
    }
}
