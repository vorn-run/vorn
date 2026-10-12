//! The description as `RichMarkdownEditor` shows it at rest: markdown read
//! into the blocks and inline marks its editor draws.

/// A run of text with the marks on it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inline {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub strike: bool,
    pub link: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Heading(u8, Vec<Inline>),
    Para(Vec<Inline>),
    /// A task item, its source line and whether it is checked.
    Task {
        line: usize,
        checked: bool,
        text: Vec<Inline>,
    },
    Bullet(Vec<Inline>),
    Ordered(u32, Vec<Inline>),
    Quote(Vec<Inline>),
    Code(String),
    Rule,
}

impl Block {
    /// The block's CSS margins (top, bottom) in px, from `editor-theme.css`.
    pub fn margins(&self) -> (f32, f32) {
        match self {
            Block::Heading(1 | 2, _) => (12.0, 4.0),
            Block::Heading(_, _) => (8.0, 4.0),
            Block::Code(_) | Block::Quote(_) => (6.0, 6.0),
            Block::Rule => (14.0, 14.0),
            _ => (2.0, 2.0),
        }
    }
}

/// The space between two blocks: their margins collapse to the larger.
pub fn gap(a: &Block, b: &Block) -> f32 {
    a.margins().1.max(b.margins().0)
}

/// Reads `src` into blocks; a blank line only ends a paragraph.
pub fn blocks(src: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut lines = src.split('\n').enumerate().peekable();
    while let Some((i, raw)) = lines.next() {
        let line = raw.trim_end();
        let t = line.trim_start();
        if t.is_empty() {
            continue;
        }
        if t.starts_with("```") {
            let mut code = Vec::new();
            for (_, l) in lines.by_ref() {
                if l.trim_start().starts_with("```") {
                    break;
                }
                code.push(l);
            }
            out.push(Block::Code(code.join("\n")));
            continue;
        }
        if matches!(t, "---" | "***" | "___") {
            out.push(Block::Rule);
            continue;
        }
        let hashes = t.chars().take_while(|&c| c == '#').count();
        if (1..=6).contains(&hashes) && t[hashes..].starts_with(' ') {
            out.push(Block::Heading(hashes as u8, inline(t[hashes..].trim())));
            continue;
        }
        if let Some(rest) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
            let task = rest
                .strip_prefix("[ ] ")
                .map(|r| (false, r))
                .or_else(|| rest.strip_prefix("[x] ").map(|r| (true, r)))
                .or_else(|| rest.strip_prefix("[X] ").map(|r| (true, r)))
                .or_else(|| (rest == "[ ]").then_some((false, "")))
                .or_else(|| matches!(rest, "[x]" | "[X]").then_some((true, "")));
            out.push(match task {
                Some((checked, r)) => Block::Task {
                    line: i,
                    checked,
                    text: inline(r),
                },
                None => Block::Bullet(inline(rest)),
            });
            continue;
        }
        let digits = t.chars().take_while(char::is_ascii_digit).count();
        if digits > 0 && t[digits..].starts_with(". ") {
            let n = t[..digits].parse().unwrap_or(1);
            out.push(Block::Ordered(n, inline(&t[digits + 2..])));
            continue;
        }
        if let Some(rest) = t.strip_prefix('>') {
            out.push(Block::Quote(inline(rest.trim_start())));
            continue;
        }
        // A paragraph runs until a blank line or another block starts.
        let mut para = t.to_owned();
        while let Some((_, next)) = lines.peek() {
            let n = next.trim();
            if n.is_empty() || starts_block(n) {
                break;
            }
            para.push(' ');
            para.push_str(n);
            lines.next();
        }
        out.push(Block::Para(inline(&para)));
    }
    out
}

fn starts_block(t: &str) -> bool {
    t.starts_with('#')
        || t.starts_with("- ")
        || t.starts_with("* ")
        || t.starts_with('>')
        || t.starts_with("```")
        || matches!(t, "---" | "***" | "___")
        || {
            let d = t.chars().take_while(char::is_ascii_digit).count();
            d > 0 && t[d..].starts_with(". ")
        }
}

/// Reaches one of an [`Inline`]'s marks.
type Flag = fn(&mut Inline) -> &mut bool;

/// The inline marks: `**bold**`, `*italic*`, `` `code` ``, `~~strike~~`
/// and `[links](url)`.
pub fn inline(src: &str) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::new();
    let mut cur = Inline::default();
    let mut rest = src;
    let push = |out: &mut Vec<Inline>, run: &Inline, s: &str| {
        if s.is_empty() {
            return;
        }
        match out.last_mut() {
            Some(last)
                if (last.bold, last.italic, last.code, last.strike, last.link)
                    == (run.bold, run.italic, run.code, run.strike, run.link) =>
            {
                last.text.push_str(s)
            }
            _ => out.push(Inline {
                text: s.to_owned(),
                ..run.clone()
            }),
        }
    };
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix('`') {
            if let Some(end) = r.find('`') {
                let run = Inline {
                    code: true,
                    ..cur.clone()
                };
                push(&mut out, &run, &r[..end]);
                rest = &r[end + 1..];
                continue;
            }
        }
        if let Some(r) = rest.strip_prefix('[') {
            if let Some(close) = r.find("](") {
                if let Some(end) = r[close + 2..].find(')') {
                    for i in inline(&r[..close]) {
                        let run = Inline { link: true, ..i };
                        push(&mut out, &run, &run.text);
                    }
                    rest = &r[close + 2 + end + 1..];
                    continue;
                }
            }
        }
        let toggles: [(&str, Flag); 5] = [
            ("**", |i| &mut i.bold),
            ("__", |i| &mut i.bold),
            ("~~", |i| &mut i.strike),
            ("*", |i| &mut i.italic),
            ("_", |i| &mut i.italic),
        ];
        let hit = toggles.iter().find(|(m, _)| {
            rest.starts_with(m) && {
                let opening = !*flag_of(&cur, m);
                // An opener needs a closer later; a closer always closes.
                !opening || rest[m.len()..].contains(m)
            }
        });
        if let Some((m, flag)) = hit {
            let f = flag(&mut cur);
            *f = !*f;
            rest = &rest[m.len()..];
            continue;
        }
        let next = rest
            .char_indices()
            .skip(1)
            .find(|(_, c)| matches!(c, '`' | '[' | '*' | '_' | '~'))
            .map_or(rest.len(), |(i, _)| i);
        push(&mut out, &cur, &rest[..next]);
        rest = &rest[next..];
    }
    out
}

fn flag_of<'a>(i: &'a Inline, mark: &str) -> &'a bool {
    match mark {
        "**" | "__" => &i.bold,
        "~~" => &i.strike,
        _ => &i.italic,
    }
}

/// The description with the task item on `line` checked or cleared.
pub fn toggle_task(src: &str, line: usize) -> String {
    src.split('\n')
        .enumerate()
        .map(|(i, l)| {
            if i != line {
                return l.to_owned();
            }
            if let Some(at) = l.find("[ ]") {
                format!("{}[x]{}", &l[..at], &l[at + 3..])
            } else if let Some(at) = l.find("[x]").or_else(|| l.find("[X]")) {
                format!("{}[ ]{}", &l[..at], &l[at + 3..])
            } else {
                l.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(text: &str) -> Vec<Inline> {
        vec![Inline {
            text: text.into(),
            ..Inline::default()
        }]
    }

    #[test]
    fn the_task_template_reads_as_headings_paragraphs_and_task_items() {
        let src = crate::tasks::TEMPLATE;
        let b = blocks(src);
        assert_eq!(b[0], Block::Heading(2, plain("Description")));
        assert!(matches!(b[1], Block::Para(_)));
        assert_eq!(b[2], Block::Heading(2, plain("Acceptance Criteria")));
        assert!(matches!(b[3], Block::Task { checked: false, .. }));
        assert!(matches!(b[4], Block::Task { checked: false, .. }));
        assert_eq!(b.len(), 7);
        assert_eq!(gap(&b[1], &b[2]), 12.0);
        assert_eq!(gap(&b[0], &b[1]), 4.0);
    }

    #[test]
    fn inline_marks_split_the_text_into_runs() {
        let r = inline("a **b** *c* `d` [e](http://x) ~~f~~ 2*3");
        let texts: Vec<_> = r.iter().map(|i| i.text.as_str()).collect();
        assert_eq!(
            texts,
            ["a ", "b", " ", "c", " ", "d", " ", "e", " ", "f", " 2*3"]
        );
        assert!(r[1].bold && r[3].italic && r[5].code && r[7].link && r[9].strike);
    }

    #[test]
    fn a_task_line_toggles_between_checked_and_open() {
        let src = "x\n- [ ] one\n- [x] two";
        assert_eq!(toggle_task(src, 1), "x\n- [x] one\n- [x] two");
        assert_eq!(toggle_task(src, 2), "x\n- [ ] one\n- [ ] two");
        let b = blocks(src);
        assert_eq!(
            b[1],
            Block::Task {
                line: 1,
                checked: false,
                text: plain("one")
            }
        );
    }
}
