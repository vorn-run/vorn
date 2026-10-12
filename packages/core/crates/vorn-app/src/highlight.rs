//! Syntax colour for the editor (`shiki.ts`): the language a file name
//! implies, and the colours of its tokens in today's vitesse-dark theme.
//!
//! Today's app runs Shiki's TextMate grammars. This is a lexer per family
//! of languages instead, matched to Shiki per token class (comments,
//! strings, numbers, keywords, storage words, constants, types, functions,
//! properties, punctuation) rather than per scope; the fixtures under
//! `tests/fixtures/highlight` hold Shiki's own output and the test says how
//! much of it this agrees with.

use std::ops::Range;

/// vitesse-dark's colours, as `0xRRGGBB` and alpha.
mod theme {
    pub const TEXT: (u32, u8) = (0xdbd7ca, 0xee);
    pub const COMMENT: (u32, u8) = (0x758575, 0xdd);
    pub const STRING: (u32, u8) = (0xc98a7d, 0xff);
    pub const QUOTE: (u32, u8) = (0xc98a7d, 0x77);
    pub const NUMBER: (u32, u8) = (0x4c9a91, 0xff);
    pub const KEYWORD: (u32, u8) = (0x4d9375, 0xff);
    pub const STORAGE: (u32, u8) = (0xcb7676, 0xff);
    pub const CONSTANT: (u32, u8) = (0xc99076, 0xff);
    pub const FUNCTION: (u32, u8) = (0x80a665, 0xff);
    pub const VARIABLE: (u32, u8) = (0xbd976a, 0xff);
    pub const PROPERTY: (u32, u8) = (0xb8a965, 0xff);
    pub const TYPE: (u32, u8) = (0x5da994, 0xff);
    pub const PUNCT: (u32, u8) = (0x666666, 0xff);
    pub const LIST: (u32, u8) = (0xd4976c, 0xff);
}

/// A token's colour as `0xRRGGBB` and alpha.
pub type Color = (u32, u8);

/// `EXT_TO_LANG` and `FILENAME_TO_LANG`: the Shiki language id for a file.
pub fn lang_for(name: &str) -> Option<&'static str> {
    let lower = name.to_lowercase();
    let by_name = match lower.as_str() {
        "dockerfile" => Some("dockerfile"),
        "makefile" => Some("makefile"),
        ".gitignore" => Some("gitignore"),
        ".env" => Some("dotenv"),
        _ => None,
    };
    if by_name.is_some() {
        return by_name;
    }
    let (_, ext) = lower.rsplit_once('.')?;
    Some(match ext {
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "tsx",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "jsx",
        "json" => "json",
        "jsonc" => "jsonc",
        "json5" => "json5",
        "html" | "htm" => "html",
        "vue" => "vue",
        "svelte" => "svelte",
        "css" => "css",
        "scss" => "scss",
        "sass" => "sass",
        "less" => "less",
        "md" => "markdown",
        "mdx" => "mdx",
        "py" | "pyi" => "python",
        "rs" => "rust",
        "go" => "go",
        "java" => "java",
        "kt" => "kotlin",
        "swift" => "swift",
        "rb" => "ruby",
        "php" => "php",
        "lua" => "lua",
        "zig" => "zig",
        "c" | "h" => "c",
        "cpp" | "cc" | "hpp" | "cxx" => "cpp",
        "cs" => "csharp",
        "sh" | "bash" | "zsh" => "bash",
        "fish" => "fish",
        "sql" => "sql",
        "graphql" | "gql" => "graphql",
        "yml" | "yaml" => "yaml",
        "toml" => "toml",
        "ini" => "ini",
        "xml" | "svg" => "xml",
        "dockerfile" => "dockerfile",
        "makefile" => "makefile",
        "r" => "r",
        "dart" => "dart",
        "ex" | "exs" => "elixir",
        "prisma" => "prisma",
        "tf" => "hcl",
        "ps1" => "powershell",
        "bat" => "batch",
        _ => return None,
    })
}

/// How one family of languages is lexed.
#[derive(Default)]
struct Spec {
    line_comments: &'static [&'static str],
    block_comment: Option<(&'static str, &'static str)>,
    quotes: &'static [char],
    /// `"""` and `'''` strings.
    triple: bool,
    /// Keywords in vitesse's green (`keyword`).
    control: &'static [&'static str],
    /// Keywords in its red (`storage`, `keyword.operator`).
    storage: &'static [&'static str],
    /// Constants in green (`true`, `None`).
    green: &'static [&'static str],
    /// Constants in red (`null`, `nil`).
    red: &'static [&'static str],
    /// `this`, `self`.
    selfs: &'static [&'static str],
    types: &'static [&'static str],
    builtins: &'static [&'static str],
    /// A capitalised name is a type.
    upper_types: bool,
    /// A name with nothing else to say is a variable, not plain text.
    variables: bool,
    /// `name:` at a line's start, or `"name":`, is a key.
    keys: bool,
    /// `macro!` is a function, as Rust's are.
    bang_macros: bool,
    /// A name before `(` is a function call.
    calls: bool,
    /// Operators in vitesse's red, longest tried first.
    red_ops: &'static [&'static str],
    /// Rust: `a::` names a module (green), `.field` is plain, `#[..]` is an
    /// attribute.
    rust: bool,
    /// The colour of a primitive type name.
    type_color: Option<Color>,
}

const TS_CONTROL: &[&str] = &[
    "import", "export", "from", "return", "if", "else", "for", "while", "do", "switch", "case",
    "break", "continue", "try", "catch", "finally", "throw", "await", "yield", "default", "as",
    "async", "with",
];
const TS_STORAGE: &[&str] = &[
    "const",
    "let",
    "var",
    "function",
    "class",
    "extends",
    "implements",
    "interface",
    "type",
    "enum",
    "public",
    "private",
    "protected",
    "static",
    "readonly",
    "get",
    "set",
    "declare",
    "abstract",
    "namespace",
    "of",
    "in",
    "instanceof",
    "new",
    "typeof",
    "keyof",
    "delete",
];
const TS_OPS: &[&str] = &[
    "&&", "||", "=>", "??", "!", "?", "+", "-", "*", "/", "%", "...", "|",
];
const TS_TYPES: &[&str] = &[
    "number", "string", "boolean", "void", "any", "unknown", "never", "object", "symbol", "bigint",
];

const RUST_CONTROL: &[&str] = &[
    "use", "pub", "fn", "if", "else", "for", "while", "loop", "match", "return", "impl", "mod",
    "crate", "where", "as", "in", "break", "continue", "async", "await", "move", "unsafe", "dyn",
    "extern", "super",
];
const RUST_STORAGE: &[&str] = &[
    "let", "mut", "const", "static", "struct", "enum", "trait", "type", "ref",
];
const RUST_OPS: &[&str] = &[
    "::", "->", "=>", "==", "!=", "<=", ">=", "&&", "||", "+=", "-=", "..=", "..", ".", ":", "?",
    "!", "&", "|", "+", "-", "*", "/", "%",
];
const RUST_TYPES: &[&str] = &[
    "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize", "f32",
    "f64", "bool", "char", "str",
];

const PY_CONTROL: &[&str] = &[
    "import", "from", "return", "if", "elif", "else", "for", "while", "in", "try", "except",
    "finally", "raise", "with", "as", "pass", "break", "continue", "yield", "await", "async",
    "global", "nonlocal", "assert", "del", "lambda",
];
const PY_STORAGE: &[&str] = &["def", "class", "is", "and", "or", "not"];
const PY_BUILTINS: &[&str] = &[
    "print",
    "range",
    "len",
    "int",
    "str",
    "float",
    "bool",
    "list",
    "dict",
    "set",
    "tuple",
    "open",
    "super",
    "isinstance",
    "enumerate",
    "zip",
    "map",
    "filter",
    "sorted",
    "min",
    "max",
    "sum",
    "any",
    "all",
    "type",
    "object",
];

const GO_CONTROL: &[&str] = &[
    "package",
    "import",
    "func",
    "var",
    "const",
    "type",
    "struct",
    "interface",
    "return",
    "if",
    "else",
    "for",
    "range",
    "switch",
    "case",
    "default",
    "break",
    "continue",
    "go",
    "defer",
    "select",
    "chan",
    "map",
    "fallthrough",
    "goto",
];
const GO_TYPES: &[&str] = &[
    "int", "int8", "int16", "int32", "int64", "uint", "uint8", "uint16", "uint32", "uint64",
    "float32", "float64", "string", "bool", "byte", "rune", "error", "any",
];

const SH_CONTROL: &[&str] = &[
    "if", "then", "else", "elif", "fi", "for", "in", "do", "done", "while", "until", "case",
    "esac", "function", "return", "select",
];
const SH_STORAGE: &[&str] = &["export", "local", "declare", "readonly", "unset", "|"];
const SH_BUILTINS: &[&str] = &[
    "echo", "cd", "printf", "source", "exit", "set", "eval", "exec", "read", "test", "pwd",
    "shift", "alias", "trap",
];

const C_CONTROL: &[&str] = &[
    "if",
    "else",
    "for",
    "while",
    "do",
    "switch",
    "case",
    "break",
    "continue",
    "return",
    "goto",
    "default",
    "try",
    "catch",
    "throw",
    "finally",
    "import",
    "package",
    "using",
    "namespace",
    "include",
    "define",
    "new",
    "func",
    "fun",
    "when",
    "guard",
    "defer",
    "in",
    "is",
    "then",
    "end",
    "elseif",
    "local",
    "require",
    "module",
    "begin",
    "rescue",
    "ensure",
    "unless",
    "yield",
    "await",
    "async",
    "pub",
    "fn",
    "use",
    "comptime",
    "try",
    "orelse",
];
const C_STORAGE: &[&str] = &[
    "class",
    "struct",
    "enum",
    "interface",
    "extends",
    "implements",
    "public",
    "private",
    "protected",
    "static",
    "final",
    "const",
    "var",
    "let",
    "val",
    "def",
    "void",
    "abstract",
    "override",
    "virtual",
    "extern",
    "typedef",
    "union",
    "sealed",
    "data",
    "object",
    "open",
    "internal",
    "readonly",
    "volatile",
    "inline",
    "function",
    "mut",
];
const C_OPS: &[&str] = &[
    "&&", "||", "->", "=>", "==", "!=", "!", "+", "-", "*", "/", "%",
];
const C_TYPES: &[&str] = &[
    "int", "long", "short", "char", "float", "double", "bool", "boolean", "byte", "string",
    "String", "unsigned", "signed", "size_t", "auto", "Int", "Double", "Float", "Bool", "u8",
    "u32", "usize", "i32", "i64", "f64", "dynamic",
];

const TRUE_FALSE: &[&str] = &["true", "false"];

fn spec(lang: &str) -> Option<Spec> {
    let c_like = Spec {
        line_comments: &["//"],
        block_comment: Some(("/*", "*/")),
        quotes: &['"', '\''],
        control: C_CONTROL,
        storage: C_STORAGE,
        green: TRUE_FALSE,
        red: &["null", "nil", "NULL", "nullptr", "None"],
        selfs: &["this", "self", "super"],
        types: C_TYPES,
        upper_types: true,
        variables: true,
        calls: true,
        red_ops: C_OPS,
        ..Spec::default()
    };
    Some(match lang {
        "typescript" | "tsx" | "javascript" | "jsx" | "vue" | "svelte" => Spec {
            line_comments: &["//"],
            block_comment: Some(("/*", "*/")),
            quotes: &['"', '\'', '`'],
            control: TS_CONTROL,
            storage: TS_STORAGE,
            green: TRUE_FALSE,
            red: &["null", "undefined"],
            selfs: &["this", "super"],
            types: TS_TYPES,
            upper_types: true,
            variables: true,
            keys: true,
            calls: true,
            red_ops: TS_OPS,
            ..Spec::default()
        },
        "rust" => Spec {
            line_comments: &["//"],
            block_comment: Some(("/*", "*/")),
            quotes: &['"'],
            control: RUST_CONTROL,
            storage: RUST_STORAGE,
            green: TRUE_FALSE,
            selfs: &["self", "Self"],
            types: RUST_TYPES,
            upper_types: true,
            variables: true,
            bang_macros: true,
            calls: true,
            red_ops: RUST_OPS,
            rust: true,
            ..Spec::default()
        },
        "python" => Spec {
            line_comments: &["#"],
            quotes: &['"', '\''],
            triple: true,
            control: PY_CONTROL,
            storage: PY_STORAGE,
            green: &["True", "False", "None"],
            selfs: &[],
            builtins: PY_BUILTINS,
            red_ops: &["->"],
            ..Spec::default()
        },
        "go" => Spec {
            control: GO_CONTROL,
            storage: &[],
            red: &["nil"],
            types: GO_TYPES,
            type_color: Some(theme::STORAGE),
            selfs: &[],
            upper_types: false,
            red_ops: &["&&", "||", ":=", "!", "+", "-", "*", "/", "%"],
            ..c_like
        },
        "json" | "jsonc" | "json5" => Spec {
            line_comments: &["//"],
            block_comment: Some(("/*", "*/")),
            quotes: &['"', '\''],
            green: &["true", "false", "null"],
            keys: true,
            ..Spec::default()
        },
        "yaml" => return Some(Spec::default()),
        "toml" | "ini" | "dotenv" => Spec {
            line_comments: &["#", ";"],
            quotes: &['"', '\''],
            green: &["true", "false", "null", "on", "off", "yes", "no"],
            keys: true,
            ..Spec::default()
        },
        "bash" | "fish" | "powershell" | "batch" | "makefile" | "dockerfile" | "gitignore" => {
            return Some(Spec::default())
        }
        "css" | "scss" | "sass" | "less" => Spec {
            line_comments: &["//"],
            block_comment: Some(("/*", "*/")),
            quotes: &['"', '\''],
            keys: true,
            ..Spec::default()
        },
        "markdown" | "mdx" => return None,
        "html" | "xml" => Spec {
            block_comment: Some(("<!--", "-->")),
            quotes: &['"', '\''],
            ..Spec::default()
        },
        "sql" => Spec {
            line_comments: &["--"],
            block_comment: Some(("/*", "*/")),
            quotes: &['\'', '"'],
            control: &[
                "select", "from", "where", "insert", "into", "values", "update", "delete",
                "create", "table", "join", "on", "group", "by", "order", "limit", "as", "and",
                "or", "not", "null", "SELECT", "FROM", "WHERE", "INSERT", "INTO", "VALUES",
                "UPDATE", "DELETE", "CREATE", "TABLE", "JOIN", "ON", "GROUP", "BY", "ORDER",
                "LIMIT", "AS", "AND", "OR", "NOT", "NULL",
            ],
            ..Spec::default()
        },
        "ruby" | "elixir" | "r" => Spec {
            line_comments: &["#"],
            block_comment: None,
            ..c_like
        },
        "lua" => Spec {
            line_comments: &["--"],
            block_comment: Some(("--[[", "]]")),
            ..c_like
        },
        _ => c_like,
    })
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || c == '$'
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// What a lexer pass yields: colours by byte range, in order.
struct Out {
    spans: Vec<(Range<usize>, Color)>,
}

impl Out {
    fn push(&mut self, r: Range<usize>, c: Color) {
        if r.is_empty() || c == theme::TEXT {
            return;
        }
        // Neighbours of one colour are one span.
        if let Some((last, lc)) = self.spans.last_mut() {
            if *lc == c && last.end == r.start {
                last.end = r.end;
                return;
            }
        }
        self.spans.push((r, c));
    }
}

/// The colours of `code` as `lang` (a Shiki id), by byte range: what is
/// left out is the theme's text colour. Empty for a language with no lexer.
pub fn highlight(code: &str, lang: &str) -> Vec<(Range<usize>, Color)> {
    match lang {
        "markdown" | "mdx" => return markdown(code),
        "yaml" => return yaml(code),
        "bash" | "fish" | "powershell" | "batch" | "makefile" | "dockerfile" | "gitignore" => {
            return shell(code)
        }
        _ => {}
    }
    let Some(sp) = spec(lang) else {
        return Vec::new();
    };
    let markup = lang == "html" || lang == "xml" || lang == "vue" || lang == "svelte";
    let mut out = Out { spans: Vec::new() };
    let b = code.as_bytes();
    let mut i = 0;
    // The last word drawn, and whether a line has begun.
    let mut prev_word: &str = "";
    let mut line_start = true;
    let mut in_tag = false;
    while i < b.len() {
        let rest = &code[i..];
        let Some(ch) = rest.chars().next() else { break };
        if ch == '\n' {
            line_start = true;
            i += 1;
            continue;
        }
        if ch.is_whitespace() {
            i += ch.len_utf8();
            continue;
        }
        if sp.rust && rest.starts_with("#[") || sp.rust && rest.starts_with("#![") {
            let lead = if rest.starts_with("#!") { 3 } else { 2 };
            out.push(i..i + lead, theme::PUNCT);
            let close = rest.find(']').map_or(code.len(), |e| i + e);
            let name_end = rest[lead..]
                .find(|c: char| !is_ident(c))
                .map_or(close, |e| i + lead + e);
            i = name_end;
            continue;
        }
        if let Some((open, close)) = sp.block_comment {
            if rest.starts_with(open) {
                let end = rest[open.len()..]
                    .find(close)
                    .map_or(code.len(), |e| i + open.len() + e + close.len());
                out.push(i..end, theme::COMMENT);
                i = end;
                continue;
            }
        }
        if let Some(lc) = sp.line_comments.iter().find(|lc| rest.starts_with(**lc)) {
            // `#` in a colour or a URL is not a comment.
            let is_comment = *lc != "#" || line_start || code[..i].ends_with(char::is_whitespace);
            if is_comment {
                let end = rest.find('\n').map_or(code.len(), |e| i + e);
                out.push(i..end, theme::COMMENT);
                i = end;
                continue;
            }
        }
        if markup && (rest.starts_with('<') || (in_tag && rest.starts_with('>'))) {
            let close = rest.starts_with("</");
            let lead = if close { 2 } else { 1 };
            out.push(i..i + lead, theme::PUNCT);
            if rest.starts_with('>') {
                in_tag = false;
                i += 1;
                continue;
            }
            let name_end = rest[lead..]
                .find(|c: char| !(is_ident(c) || c == '-' || c == ':' || c == '.'))
                .map_or(rest.len(), |e| e + lead);
            out.push(i + lead..i + name_end, theme::KEYWORD);
            in_tag = true;
            i += name_end;
            line_start = false;
            continue;
        }
        if sp.triple && (rest.starts_with("\"\"\"") || rest.starts_with("'''")) {
            let q = &rest[..3];
            let end = rest[3..].find(q).map_or(code.len(), |e| i + 3 + e + 3);
            out.push(i..i + 3, theme::QUOTE);
            out.push(i + 3..end.saturating_sub(3).max(i + 3), theme::STRING);
            out.push(end.saturating_sub(3).max(i + 3)..end, theme::QUOTE);
            i = end;
            line_start = false;
            continue;
        }
        if sp.quotes.contains(&ch) {
            let end = string_end(code, i, ch);
            let key = sp.keys && is_key(code, end);
            quoted(code, i..end, ch, key, &mut out);
            i = end;
            line_start = false;
            continue;
        }
        if ch.is_ascii_digit() {
            let end = rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '_'))
                .map_or(code.len(), |e| i + e);
            out.push(i..end, theme::NUMBER);
            i = end;
            line_start = false;
            continue;
        }
        if is_ident_start(ch) || (markup && in_tag && ch.is_alphabetic()) {
            let mut end = rest
                .find(|c: char| !(is_ident(c) || (markup && c == '-')))
                .map_or(code.len(), |e| i + e);
            let word = &code[i..end];
            let next = code[end..].trim_start_matches([' ', '\t']);
            let color = if markup {
                if in_tag {
                    theme::VARIABLE
                } else {
                    theme::TEXT
                }
            } else if sp.bang_macros && next.starts_with('!') && !next.starts_with("!=") {
                end += code[end..].find('!').map_or(0, |e| e + 1);
                theme::FUNCTION
            } else if sp.rust && next.starts_with("::") {
                theme::FUNCTION
            } else if sp.rust
                && code[..i].ends_with('.')
                && !code[..i].ends_with("..")
                && !next.starts_with('(')
            {
                theme::TEXT
            } else {
                word_color(&sp, word, prev_word, next, line_start)
            };
            out.push(i..end, color);
            prev_word = word;
            i = end;
            line_start = false;
            continue;
        }
        // Operators the language colours as keywords, longest first.
        if let Some(op) = sp
            .red_ops
            .iter()
            .filter(|w| rest.starts_with(**w))
            .max_by_key(|w| w.len())
        {
            out.push(i..i + op.len(), theme::STORAGE);
            i += op.len();
            line_start = false;
            continue;
        }
        let color = if sp.keys && ch == '-' && line_start {
            theme::PUNCT
        } else if ch.is_ascii_punctuation() {
            theme::PUNCT
        } else {
            theme::TEXT
        };
        out.push(i..i + ch.len_utf8(), color);
        i += ch.len_utf8();
        line_start = false;
    }
    out.spans
}

/// Where the string opened by the quote at `i` ends: past its closing
/// quote, or at the line's end for an unclosed one (but a template, which
/// runs on).
fn string_end(code: &str, i: usize, q: char) -> usize {
    let mut esc = false;
    for (k, c) in code[i + 1..].char_indices() {
        if esc {
            esc = false;
            continue;
        }
        match c {
            '\\' => esc = true,
            '\n' if q != '`' => return i + 1 + k,
            c if c == q => return i + 1 + k + c.len_utf8(),
            _ => {}
        }
    }
    code.len()
}

/// A quoted string: its quote marks faint, a key's body in property
/// colour, a template's `${}` in green.
fn quoted(code: &str, r: Range<usize>, q: char, key: bool, out: &mut Out) {
    let closed = r.end > r.start + 1 && code[..r.end].ends_with(q);
    let inner_end = if closed { r.end - 1 } else { r.end };
    let (body, mark) = if key {
        (theme::PROPERTY, (theme::PROPERTY.0, 0x77))
    } else {
        (theme::STRING, theme::QUOTE)
    };
    out.push(r.start..r.start + 1, mark);
    if key {
        out.push(r.start + 1..inner_end, body);
    } else {
        template(code, r.start + 1..inner_end, q == '`', out);
    }
    if closed {
        out.push(inner_end..r.end, mark);
    }
}

/// A string followed by `:` is an object key.
fn is_key(code: &str, end: usize) -> bool {
    code[end..].trim_start_matches([' ', '\t']).starts_with(':')
}

/// A string's body, with a template's `${` and `}` in keyword green.
fn template(code: &str, r: Range<usize>, tpl: bool, out: &mut Out) {
    if !tpl {
        out.push(r, theme::STRING);
        return;
    }
    let mut at = r.start;
    while at < r.end {
        match code[at..r.end].find("${") {
            Some(o) => {
                out.push(at..at + o, theme::STRING);
                let open = at + o;
                out.push(open..open + 2, theme::KEYWORD);
                let close = code[open..r.end].find('}').map_or(r.end, |c| open + c);
                out.push(open + 2..close, theme::STRING);
                if close < r.end {
                    out.push(close..close + 1, theme::KEYWORD);
                }
                at = (close + 1).min(r.end);
            }
            None => {
                out.push(at..r.end, theme::STRING);
                break;
            }
        }
    }
}

fn word_color(sp: &Spec, word: &str, prev: &str, next: &str, line_start: bool) -> Color {
    if sp.control.contains(&word) {
        return theme::KEYWORD;
    }
    if sp.storage.contains(&word) {
        return theme::STORAGE;
    }
    if sp.green.contains(&word) {
        return theme::KEYWORD;
    }
    if sp.red.contains(&word) {
        return theme::STORAGE;
    }
    if sp.selfs.contains(&word) {
        return theme::CONSTANT;
    }
    if sp.keys && next.starts_with(':') && !next.starts_with("::") && (line_start || !sp.variables)
    {
        return theme::PROPERTY;
    }
    if sp.types.contains(&word) {
        return sp.type_color.unwrap_or(theme::TYPE);
    }
    if sp.builtins.contains(&word) {
        return theme::PROPERTY;
    }
    if matches!(prev, "function" | "def" | "fn" | "func" | "fun") {
        return theme::FUNCTION;
    }
    if matches!(
        prev,
        "class" | "struct" | "interface" | "enum" | "trait" | "type" | "package"
    ) {
        return theme::TYPE;
    }
    if matches!(prev, "extends" | "implements") {
        return theme::FUNCTION;
    }
    if sp.calls && next.starts_with('(') {
        return if sp.upper_types && word.starts_with(char::is_uppercase) && prev != "new" {
            theme::TYPE
        } else {
            theme::FUNCTION
        };
    }
    if sp.upper_types && word.starts_with(char::is_uppercase) {
        if word.len() > 1 && word.chars().all(|c| c.is_uppercase() || c == '_') {
            return theme::CONSTANT;
        }
        return theme::TYPE;
    }
    if sp.variables {
        theme::VARIABLE
    } else {
        theme::TEXT
    }
}

/// YAML: keys, plain values as strings, numbers and booleans, flow
/// collections, comments, and block scalars (`|`, `>`) as strings.
fn yaml(code: &str) -> Vec<(Range<usize>, Color)> {
    let mut out = Out { spans: Vec::new() };
    let mut at = 0;
    let mut block: Option<usize> = None;
    for line in code.split_inclusive('\n') {
        let body = line.trim_end_matches(['\n', '\r']);
        let indent = body.len() - body.trim_start().len();
        if let Some(min) = block {
            if body.trim().is_empty() || indent > min {
                out.push(at + indent..at + body.len(), theme::STRING);
                at += line.len();
                continue;
            }
            block = None;
        }
        let mut k = indent;
        let t = |k: usize| &body[k..];
        if t(k).starts_with('#') {
            out.push(at + k..at + body.len(), theme::COMMENT);
            at += line.len();
            continue;
        }
        while t(k).starts_with("- ") || t(k) == "-" {
            out.push(at + k..at + k + 1, theme::PUNCT);
            k += 1;
            k += t(k).len() - t(k).trim_start().len();
        }
        if let Some(colon) = key_end(t(k)) {
            out.push(at + k..at + k + colon, theme::PROPERTY);
            out.push(at + k + colon..at + k + colon + 1, theme::PUNCT);
            k += colon + 1;
            k += t(k).len() - t(k).trim_start().len();
        }
        let v = t(k);
        let value_end = v.find(" #").map_or(v.len(), |c| v[..c].trim_end().len());
        let value = &v[..value_end];
        if value.starts_with('|') || value.starts_with('>') {
            out.push(at + k..at + k + value.len(), theme::PUNCT);
            block = Some(indent);
        } else if value.starts_with('{') || value.starts_with('[') {
            flow(value, at + k, &mut out);
        } else if !value.is_empty() {
            out.push(at + k..at + k + value.len(), scalar(value));
        }
        if let Some(c) = v.find(" #") {
            out.push(at + k + c + 1..at + body.len(), theme::COMMENT);
        }
        at += line.len();
    }
    out.spans
}

/// Where a `key:` ends, when the line starts with one.
fn key_end(t: &str) -> Option<usize> {
    if t.starts_with(['"', '\'', '{', '[', '#']) {
        return None;
    }
    let c = t.find(':')?;
    let after = &t[c + 1..];
    (after.is_empty() || after.starts_with(' ')).then_some(c)
}

/// A plain YAML scalar's colour.
fn scalar(v: &str) -> Color {
    let first = v.chars().next().unwrap_or(' ');
    if matches!(v, "true" | "false" | "null" | "~" | "yes" | "no") {
        theme::KEYWORD
    } else if first.is_ascii_digit() && v.parse::<f64>().is_ok() {
        theme::NUMBER
    } else {
        theme::STRING
    }
}

/// `{ a: b, c: [d] }` inside a YAML value.
fn flow(v: &str, base: usize, out: &mut Out) {
    let mut word = 0;
    let flush = |start: usize, end: usize, out: &mut Out| {
        let w = v[start..end].trim();
        if w.is_empty() {
            return;
        }
        let s = start + (v[start..end].len() - v[start..end].trim_start().len());
        let c = if v[end..].starts_with(':') {
            theme::PROPERTY
        } else {
            scalar(w)
        };
        out.push(base + s..base + s + w.len(), c);
    };
    for (k, ch) in v.char_indices() {
        if matches!(ch, '{' | '}' | '[' | ']' | ',' | ':') {
            flush(word, k, out);
            out.push(base + k..base + k + 1, theme::PUNCT);
            word = k + 1;
        }
    }
    flush(word, v.len(), out);
}

/// Shell: comments, strings, keywords, the command in each position, its
/// flags and arguments, and `name=` assignments.
fn shell(code: &str) -> Vec<(Range<usize>, Color)> {
    let mut out = Out { spans: Vec::new() };
    let b = code.as_bytes();
    let mut i = 0;
    let mut command = true;
    while i < b.len() {
        let rest = &code[i..];
        let Some(ch) = rest.chars().next() else { break };
        if ch == '\n' || ch == ';' {
            if ch == ';' {
                out.push(i..i + 1, theme::PUNCT);
            }
            command = true;
            i += 1;
            continue;
        }
        if ch.is_whitespace() {
            i += ch.len_utf8();
            continue;
        }
        if ch == '#' && (i == 0 || code[..i].ends_with(char::is_whitespace)) {
            let end = rest.find('\n').map_or(code.len(), |e| i + e);
            out.push(i..end, theme::COMMENT);
            i = end;
            continue;
        }
        if ch == '"' || ch == '\'' {
            let end = string_end(code, i, ch);
            quoted(code, i..end, ch, false, &mut out);
            i = end;
            command = false;
            continue;
        }
        if let Some(op) = ["&&", "||", "|", "&"]
            .iter()
            .find(|op| rest.starts_with(**op))
        {
            let c = if *op == "|" {
                theme::STORAGE
            } else {
                theme::PUNCT
            };
            out.push(i..i + op.len(), c);
            i += op.len();
            command = true;
            continue;
        }
        let end = rest
            .find(|c: char| c.is_whitespace() || matches!(c, ';' | '|' | '&' | '"' | '\''))
            .map_or(code.len(), |e| i + e)
            .max(i + ch.len_utf8());
        let word = &code[i..end];
        if let Some(eq) = word.find('=').filter(|_| command) {
            out.push(i..i + eq, theme::VARIABLE);
            out.push(i + eq..i + eq + 1, theme::PUNCT);
            out.push(i + eq + 1..end, theme::STRING);
        } else if SH_CONTROL.contains(&word) {
            out.push(i..end, theme::KEYWORD);
            command = true;
            i = end;
            continue;
        } else if SH_STORAGE.contains(&word) {
            out.push(i..end, theme::STORAGE);
        } else if word
            .chars()
            .all(|c| matches!(c, '[' | ']' | '(' | ')' | '{' | '}'))
        {
            out.push(i..end, theme::PUNCT);
        } else if command {
            let c = if SH_BUILTINS.contains(&word) {
                theme::PROPERTY
            } else {
                theme::FUNCTION
            };
            out.push(i..end, c);
            command = false;
        } else if word.starts_with('-') {
            out.push(i..end, theme::CONSTANT);
        } else if word.starts_with('$') {
            out.push(i..end, theme::TEXT);
        } else if word.chars().all(|c| c.is_ascii_digit()) {
            out.push(i..end, theme::NUMBER);
        } else {
            out.push(i..end, theme::STRING);
        }
        i = end;
    }
    out.spans
}

/// Markdown: headings green behind a grey `#`, list marks orange, inline
/// code and emphasis marks grey, link text as a string.
fn markdown(code: &str) -> Vec<(Range<usize>, Color)> {
    let mut out = Out { spans: Vec::new() };
    let mut at = 0;
    for line in code.split_inclusive('\n') {
        let body = line.trim_end_matches('\n');
        let indent = body.len() - body.trim_start().len();
        let t = &body[indent..];
        let hashes = t.bytes().take_while(|&c| c == b'#').count();
        if hashes > 0 && t[hashes..].starts_with(' ') {
            out.push(at + indent..at + indent + hashes, theme::PUNCT);
            out.push(at + indent + hashes..at + body.len(), theme::KEYWORD);
        } else {
            let mut start = indent;
            if (t.starts_with("- ") || t.starts_with("* ") || t.starts_with("+ ")) && !t.is_empty()
            {
                out.push(at + indent..at + indent + 1, theme::LIST);
                start += 1;
            }
            for (k, c) in body[start..].char_indices() {
                if matches!(c, '`' | '*' | '_' | '[' | ']' | '(' | ')') {
                    let p = at + start + k;
                    out.push(p..p + 1, theme::PUNCT);
                }
            }
        }
        at += line.len();
    }
    out.spans
}

#[cfg(test)]
mod tests {
    use super::*;

    fn color_of(code: &str, lang: &str, word: &str) -> Color {
        let at = code.find(word).expect(word);
        highlight(code, lang)
            .into_iter()
            .find(|(r, _)| r.start <= at && at < r.end)
            .map_or(theme::TEXT, |(_, c)| c)
    }

    #[test]
    fn languages_come_from_names_as_shiki_maps_them() {
        assert_eq!(lang_for("app.ts"), Some("typescript"));
        assert_eq!(lang_for("Dockerfile"), Some("dockerfile"));
        assert_eq!(lang_for("x.RS"), Some("rust"));
        assert_eq!(lang_for("notes"), None);
        assert_eq!(lang_for("a.unknownext"), None);
    }

    #[test]
    fn typescript_tokens_take_vitesse_colours() {
        let code = "import { greet } from './lib'\n// hi\nconst n = 42\nexport function main(): void { console.log(greet(n)) }\n";
        let ts = |w| color_of(code, "typescript", w);
        assert_eq!(ts("import"), theme::KEYWORD);
        assert_eq!(ts("// hi"), theme::COMMENT);
        assert_eq!(ts("const"), theme::STORAGE);
        assert_eq!(ts("42"), theme::NUMBER);
        assert_eq!(ts("'./lib'"), theme::QUOTE);
        assert_eq!(ts("./lib"), theme::STRING);
        assert_eq!(ts("main"), theme::FUNCTION);
        assert_eq!(ts("void"), theme::TYPE);
        assert_eq!(ts("console"), theme::VARIABLE);
        assert_eq!(ts("log"), theme::FUNCTION);
    }

    #[test]
    fn strings_survive_escapes_and_unclosed_ends() {
        let code = "const a = 'it\\'s'\nconst b = 'open\nconst c = 1";
        assert_eq!(color_of(code, "typescript", "it"), theme::STRING);
        assert_eq!(color_of(code, "typescript", "c ="), theme::VARIABLE);
    }

    #[test]
    fn spans_are_ordered_and_inside_the_text() {
        let code = "fn main() { let s = \"é\"; } // ü\n";
        let spans = highlight(code, "rust");
        assert!(spans.windows(2).all(|w| w[0].0.end <= w[1].0.start));
        assert!(spans.iter().all(|(r, _)| code.is_char_boundary(r.start)
            && code.is_char_boundary(r.end)
            && r.end <= code.len()));
    }

    #[test]
    fn markdown_headings_and_marks() {
        let code = "# Acme\n- item with `code`\n";
        assert_eq!(color_of(code, "markdown", "Acme"), theme::KEYWORD);
        assert_eq!(color_of(code, "markdown", "- "), theme::LIST);
        assert_eq!(color_of(code, "markdown", "`"), theme::PUNCT);
    }
}
