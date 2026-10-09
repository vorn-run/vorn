//! The two command-line grammars: the one for commands that talk to a running
//! server, and the one for `vorn server`.
//!
//! Both are parsed the way `node:util`'s `parseArgs` parses them in strict
//! mode, with the same messages, because the TypeScript command is built on it
//! and a script that matched its errors keeps matching ours: `--lines=200` and
//! `--lines 200` both work, `--` ends the options, `-h` may be grouped, and the
//! first bad option in order is the one reported.

use std::collections::BTreeMap;

/// Whether an option takes a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    String,
    Boolean,
}

/// One option a grammar accepts.
#[derive(Clone, Copy, Debug)]
pub struct OptionSpec {
    pub name: &'static str,
    pub kind: Kind,
    pub short: Option<char>,
    /// Repeatable: every value is kept, in order.
    pub multiple: bool,
}

const fn string(name: &'static str) -> OptionSpec {
    OptionSpec {
        name,
        kind: Kind::String,
        short: None,
        multiple: false,
    }
}

const fn boolean(name: &'static str) -> OptionSpec {
    OptionSpec {
        name,
        kind: Kind::Boolean,
        short: None,
        multiple: false,
    }
}

const HELP: OptionSpec = OptionSpec {
    name: "help",
    kind: Kind::Boolean,
    short: Some('h'),
    multiple: false,
};

/// The options the client commands take (`client-args.ts`).
pub const CLIENT_OPTIONS: &[OptionSpec] = &[
    string("agent"),
    string("prompt"),
    string("project"),
    string("path"),
    string("name"),
    string("branch"),
    string("workflow"),
    OptionSpec {
        name: "input",
        kind: Kind::String,
        short: None,
        multiple: true,
    },
    string("data-dir"),
    string("lines"),
    string("limit"),
    string("timeout"),
    boolean("headless"),
    boolean("worktree"),
    boolean("recent"),
    boolean("raw"),
    boolean("json"),
    HELP,
];

/// The options `vorn server` takes (`server-args.ts`).
pub const SERVER_OPTIONS: &[OptionSpec] = &[
    string("host"),
    string("port"),
    string("data-dir"),
    string("name"),
    HELP,
];

/// A command line that does not parse, worded as `parseArgs` or the grammar
/// words it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArgsError(pub String);

impl std::fmt::Display for ArgsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ArgsError {}

/// What one option was given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Given {
    Flag,
    Value(String),
    Values(Vec<String>),
}

/// A parsed command line: options by name, and everything else in order.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Parsed {
    pub values: BTreeMap<&'static str, Given>,
    pub positionals: Vec<String>,
}

impl Parsed {
    /// A string option's value, if it was given one.
    pub fn str(&self, name: &str) -> Option<&str> {
        match self.values.get(name) {
            Some(Given::Value(v)) => Some(v),
            _ => None,
        }
    }

    pub fn flag(&self, name: &str) -> bool {
        matches!(self.values.get(name), Some(Given::Flag))
    }

    pub fn many(&self, name: &str) -> &[String] {
        match self.values.get(name) {
            Some(Given::Values(v)) => v,
            _ => &[],
        }
    }
}

/// One token of a command line, as `parseArgs` splits it.
enum Token {
    Option {
        /// The long name, or the short letter when no option has it.
        name: String,
        /// What was typed, for messages: `--agent`, `-h`.
        raw: String,
        value: Option<String>,
        inline: bool,
    },
    Positional(String),
}

fn find(options: &[OptionSpec], name: &str) -> Option<OptionSpec> {
    options.iter().copied().find(|o| o.name == name)
}

/// The long name a short letter stands for, or the letter itself.
fn long_for_short(options: &[OptionSpec], short: char) -> String {
    options
        .iter()
        .find(|o| o.short == Some(short))
        .map_or_else(|| short.to_string(), |o| o.name.to_owned())
}

fn takes_value(options: &[OptionSpec], name: &str) -> bool {
    find(options, name).is_some_and(|o| o.kind == Kind::String)
}

/// Splits `argv` into tokens. Never fails: validation happens per token, in
/// order, as `parseArgs` does it.
fn tokenize(options: &[OptionSpec], argv: &[String]) -> Vec<Token> {
    let mut tokens = Vec::with_capacity(argv.len());
    let mut remaining: std::collections::VecDeque<String> = argv.iter().cloned().collect();

    while let Some(arg) = remaining.pop_front() {
        let chars: Vec<char> = arg.chars().collect();

        if arg == "--" {
            tokens.extend(remaining.drain(..).map(Token::Positional));
            break;
        }

        // `-x`
        if chars.len() == 2 && chars[0] == '-' && chars[1] != '-' {
            let name = long_for_short(options, chars[1]);
            let value = if takes_value(options, &name) {
                remaining.pop_front()
            } else {
                None
            };
            tokens.push(Token::Option {
                name,
                raw: arg,
                value,
                inline: false,
            });
            continue;
        }

        // `-abc`: a group of short options, the last of which may take the rest.
        if chars.len() > 2
            && chars[0] == '-'
            && chars[1] != '-'
            && !takes_value(options, &long_for_short(options, chars[1]))
        {
            let mut expanded = Vec::new();
            for (index, &c) in chars.iter().enumerate().skip(1) {
                let name = long_for_short(options, c);
                if !takes_value(options, &name) || index == chars.len() - 1 {
                    expanded.push(format!("-{c}"));
                } else {
                    let rest: String = chars[index + 1..].iter().collect();
                    expanded.push(format!("-{c}{rest}"));
                    break;
                }
            }
            for item in expanded.into_iter().rev() {
                remaining.push_front(item);
            }
            continue;
        }

        // `-fVALUE`, for a short option that takes a value.
        if chars.len() > 2 && chars[0] == '-' && chars[1] != '-' {
            let name = long_for_short(options, chars[1]);
            let value: String = chars[2..].iter().collect();
            tokens.push(Token::Option {
                name,
                raw: format!("-{}", chars[1]),
                value: Some(value),
                inline: true,
            });
            continue;
        }

        // `--name`, or `--name=value` (an `=` from the third character on).
        if chars.len() > 2 && arg.starts_with("--") {
            let equals = arg[2..]
                .char_indices()
                .skip(1)
                .find(|&(_, c)| c == '=')
                .map(|(at, _)| at + 2);
            match equals {
                None => {
                    let name = arg[2..].to_owned();
                    let value = if takes_value(options, &name) {
                        remaining.pop_front()
                    } else {
                        None
                    };
                    tokens.push(Token::Option {
                        name,
                        raw: arg,
                        value,
                        inline: false,
                    });
                }
                Some(at) => {
                    let name = arg[2..at].to_owned();
                    tokens.push(Token::Option {
                        raw: format!("--{name}"),
                        name,
                        value: Some(arg[at + 1..].to_owned()),
                        inline: true,
                    });
                }
            }
            continue;
        }

        tokens.push(Token::Positional(arg));
    }
    tokens
}

/// How an option is named in a message: `-h, --help` or `--agent`.
fn short_and_long(spec: &OptionSpec) -> String {
    match spec.short {
        Some(short) => format!("-{short}, --{}", spec.name),
        None => format!("--{}", spec.name),
    }
}

/// Parses `argv` against `options`, strictly, with positionals allowed.
pub fn parse(options: &[OptionSpec], argv: &[String]) -> Result<Parsed, ArgsError> {
    let mut parsed = Parsed::default();
    for token in tokenize(options, argv) {
        match token {
            Token::Positional(value) => parsed.positionals.push(value),
            Token::Option {
                name,
                raw,
                value,
                inline,
            } => {
                let Some(spec) = find(options, &name) else {
                    return Err(ArgsError(format!(
                        "Unknown option '{raw}'. To specify a positional argument starting with a '-', place it at the end of the command after '--', as in '-- \"{raw}\""
                    )));
                };
                match (spec.kind, value) {
                    (Kind::String, None) => {
                        return Err(ArgsError(format!(
                            "Option '{} <value>' argument missing",
                            short_and_long(&spec)
                        )))
                    }
                    (Kind::Boolean, Some(_)) => {
                        return Err(ArgsError(format!(
                            "Option '{}' does not take an argument",
                            short_and_long(&spec)
                        )))
                    }
                    (Kind::Boolean, None) => {
                        parsed.values.insert(spec.name, Given::Flag);
                    }
                    (Kind::String, Some(value)) => {
                        // A value that looks like an option was probably meant as one.
                        if !inline && value.chars().count() > 1 && value.starts_with('-') {
                            return Err(ArgsError(format!(
                                "Option '{raw}' argument is ambiguous.\nDid you forget to specify the option argument for '{raw}'?\nTo specify an option argument starting with a dash use '{raw}=-XYZ'."
                            )));
                        }
                        if spec.multiple {
                            match parsed
                                .values
                                .entry(spec.name)
                                .or_insert(Given::Values(Vec::new()))
                            {
                                Given::Values(all) => all.push(value),
                                other => *other = Given::Values(vec![value]),
                            }
                        } else {
                            parsed.values.insert(spec.name, Given::Value(value));
                        }
                    }
                }
            }
        }
    }
    Ok(parsed)
}

/// `Number.parseInt(raw, 10)`: leading whitespace, a sign, then digits, and
/// whatever follows ignored. `None` is `NaN`.
pub fn parse_int(raw: &str) -> Option<f64> {
    let trimmed = raw.trim_start_matches(is_js_whitespace);
    let (negative, digits) = match trimmed.as_bytes().first() {
        Some(b'-') => (true, &trimmed[1..]),
        Some(b'+') => (false, &trimmed[1..]),
        _ => (false, trimmed),
    };
    let end = digits
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(digits.len());
    if end == 0 {
        return None;
    }
    // Through f64 as JavaScript does, so twenty digits read as it reads them.
    let value: f64 = digits[..end].parse().ok()?;
    Some(if negative { -value } else { value })
}

/// What `String.prototype.trim` and `parseInt` skip.
pub fn is_js_whitespace(c: char) -> bool {
    matches!(
        c,
        '\u{9}' | '\u{a}' | '\u{b}' | '\u{c}' | '\u{d}' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

/// `String.prototype.trim`.
pub fn js_trim(s: &str) -> &str {
    s.trim_matches(is_js_whitespace)
}

/// The client grammar, parsed and checked (`ClientArgs`).
#[derive(Debug, Default, Clone)]
pub struct ClientArgs {
    /// The noun, its verb, and their operands: `session`, `start`, an id.
    pub positionals: Vec<String>,
    pub agent: Option<String>,
    pub prompt: Option<String>,
    pub project: Option<String>,
    pub path: Option<String>,
    pub name: Option<String>,
    pub branch: Option<String>,
    pub workflow: Option<String>,
    /// `--input key=value`, in the order given; a repeated key keeps its last value.
    pub inputs: Option<serde_json::Map<String, serde_json::Value>>,
    pub data_dir: Option<String>,
    pub lines: Option<f64>,
    pub limit: Option<f64>,
    pub timeout_ms: Option<f64>,
    pub headless: bool,
    pub worktree: bool,
    pub recent: bool,
    pub raw: bool,
    pub json: bool,
    pub help: bool,
}

/// A count, a line budget, a millisecond ceiling: all of them positive integers.
fn positive_int(raw: Option<&str>, flag: &str) -> Result<Option<f64>, ArgsError> {
    let Some(raw) = raw else { return Ok(None) };
    match parse_int(raw) {
        Some(value) if value.is_finite() && value > 0.0 => Ok(Some(value)),
        _ => Err(ArgsError(format!(
            "{flag} must be a positive number, got \"{raw}\""
        ))),
    }
}

impl ClientArgs {
    /// Parses a client command line (`parseClientArgs`), checks in the order it does.
    pub fn parse(argv: &[String]) -> Result<ClientArgs, ArgsError> {
        let parsed = parse(CLIENT_OPTIONS, argv)?;

        let inputs = if parsed.many("input").is_empty() {
            None
        } else {
            let mut pairs = serde_json::Map::new();
            for entry in parsed.many("input") {
                match entry.find('=') {
                    Some(at) if at > 0 => {
                        pairs.insert(
                            entry[..at].to_owned(),
                            serde_json::Value::String(entry[at + 1..].to_owned()),
                        );
                    }
                    _ => {
                        return Err(ArgsError(format!(
                            "--input wants key=value, got \"{entry}\""
                        )))
                    }
                }
            }
            Some(pairs)
        };

        let owned = |name: &str| parsed.str(name).map(str::to_owned);

        // An empty path is not a path: it would resolve to wherever the command was typed.
        let data_dir = owned("data-dir");
        if data_dir.as_deref().is_some_and(|d| js_trim(d).is_empty()) {
            return Err(ArgsError("--data-dir needs a directory".into()));
        }

        Ok(ClientArgs {
            agent: owned("agent"),
            prompt: owned("prompt"),
            project: owned("project"),
            path: owned("path"),
            name: owned("name"),
            branch: owned("branch"),
            workflow: owned("workflow"),
            inputs,
            data_dir,
            lines: positive_int(parsed.str("lines"), "--lines")?,
            limit: positive_int(parsed.str("limit"), "--limit")?,
            timeout_ms: positive_int(parsed.str("timeout"), "--timeout")?,
            headless: parsed.flag("headless"),
            worktree: parsed.flag("worktree"),
            recent: parsed.flag("recent"),
            raw: parsed.flag("raw"),
            json: parsed.flag("json"),
            help: parsed.flag("help"),
            positionals: parsed.positionals,
        })
    }
}

/// The server grammar, parsed and checked (`ServerArgs`).
#[derive(Debug, Default, Clone)]
pub struct ServerArgs {
    pub host: Option<String>,
    pub port: Option<f64>,
    pub data_dir: Option<String>,
    /// Label for `token create`.
    pub name: Option<String>,
    pub help: bool,
    /// Everything that is not an option: `serve`, `token`, `create`, an id.
    pub positionals: Vec<String>,
}

impl ServerArgs {
    /// Parses a server command line (`parseServerArgs`).
    pub fn parse(argv: &[String]) -> Result<ServerArgs, ArgsError> {
        let parsed = parse(SERVER_OPTIONS, argv)?;
        let port = match parsed.str("port") {
            None => None,
            Some(raw) => match parse_int(raw) {
                Some(port) if port.is_finite() => Some(port),
                _ => return Err(ArgsError(format!("--port must be a number, got \"{raw}\""))),
            },
        };
        Ok(ServerArgs {
            host: parsed.str("host").map(str::to_owned),
            port,
            data_dir: parsed.str("data-dir").map(str::to_owned),
            name: parsed.str("name").map(str::to_owned),
            help: parsed.flag("help"),
            positionals: parsed.positionals,
        })
    }
}

/// Whether an option of either grammar takes a value, so finding the command
/// in `vorn --data-dir /tmp session list` skips the directory.
pub fn takes_value_anywhere(name: &str) -> bool {
    takes_value(SERVER_OPTIONS, name) || takes_value(CLIENT_OPTIONS, name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn err(items: &[&str]) -> String {
        parse(CLIENT_OPTIONS, &argv(items)).unwrap_err().0
    }

    #[test]
    fn reports_what_node_reports() {
        assert_eq!(
            err(&["--foo"]),
            "Unknown option '--foo'. To specify a positional argument starting with a '-', place it at the end of the command after '--', as in '-- \"--foo\""
        );
        assert!(err(&["--foo=bar"]).starts_with("Unknown option '--foo'."));
        assert!(err(&["-x"]).starts_with("Unknown option '-x'."));
        assert!(err(&["-hx"]).starts_with("Unknown option '-x'."));
        assert!(err(&["-agent"]).starts_with("Unknown option '-a'."));
        assert!(err(&["-h=1"]).starts_with("Unknown option '-='."));
        assert!(err(&["--=x"]).starts_with("Unknown option '--=x'."));
        assert_eq!(
            err(&["--agent"]),
            "Option '--agent <value>' argument missing"
        );
        assert_eq!(
            err(&["--json=1"]),
            "Option '--json' does not take an argument"
        );
        assert_eq!(
            err(&["--json="]),
            "Option '--json' does not take an argument"
        );
        assert_eq!(
            err(&["--help=1"]),
            "Option '-h, --help' does not take an argument"
        );
        assert_eq!(
            err(&["--agent", "--json"]),
            "Option '--agent' argument is ambiguous.\nDid you forget to specify the option argument for '--agent'?\nTo specify an option argument starting with a dash use '--agent=-XYZ'."
        );
        assert!(err(&["--agent", "--"]).contains("is ambiguous"));
    }

    #[test]
    fn reports_the_first_bad_option_in_order() {
        assert!(err(&["--json=1", "--nope"]).contains("does not take"));
        assert!(err(&["--nope", "--json=1"]).starts_with("Unknown option '--nope'"));
    }

    #[test]
    fn takes_values_inline_and_apart() {
        let p = parse(
            CLIENT_OPTIONS,
            &argv(&[
                "session",
                "--agent=-x",
                "--lines",
                "3",
                "-h",
                "--",
                "--json",
            ]),
        )
        .unwrap();
        assert_eq!(p.str("agent"), Some("-x"));
        assert_eq!(p.str("lines"), Some("3"));
        assert!(p.flag("help"));
        assert!(!p.flag("json"));
        assert_eq!(p.positionals, argv(&["session", "--json"]));

        let p = parse(CLIENT_OPTIONS, &argv(&["--agent", "-", "-", "-hh"])).unwrap();
        assert_eq!(p.str("agent"), Some("-"));
        assert_eq!(p.positionals, argv(&["-"]));

        let p = parse(CLIENT_OPTIONS, &argv(&["--agent=a", "--agent", "b"])).unwrap();
        assert_eq!(p.str("agent"), Some("b"));

        let p = parse(CLIENT_OPTIONS, &argv(&["--input", "a=1", "--input=b=2"])).unwrap();
        assert_eq!(p.many("input"), argv(&["a=1", "b=2"]).as_slice());
    }

    #[test]
    fn parses_integers_as_parse_int_does() {
        assert_eq!(parse_int("12abc"), Some(12.0));
        assert_eq!(parse_int("  +5"), Some(5.0));
        assert_eq!(parse_int("1e3"), Some(1.0));
        assert_eq!(parse_int("0x10"), Some(0.0));
        assert_eq!(parse_int("abc"), None);
        assert_eq!(parse_int("-"), None);
        assert_eq!(parse_int("-3"), Some(-3.0));
    }

    #[test]
    fn checks_client_values_in_the_typescript_order() {
        let e = ClientArgs::parse(&argv(&["--lines", "0", "--input", "x"])).unwrap_err();
        assert_eq!(e.0, "--input wants key=value, got \"x\"");
        let e = ClientArgs::parse(&argv(&["--lines", "0", "--data-dir="])).unwrap_err();
        assert_eq!(e.0, "--data-dir needs a directory");
        let e = ClientArgs::parse(&argv(&["--timeout=-0", "--lines", "x"])).unwrap_err();
        assert_eq!(e.0, "--lines must be a positive number, got \"x\"");
        let e = ClientArgs::parse(&argv(&["--input", "=v"])).unwrap_err();
        assert_eq!(e.0, "--input wants key=value, got \"=v\"");

        let ok = ClientArgs::parse(&argv(&["--input", "pr=4=2", "--input", "pr=5"])).unwrap();
        assert_eq!(
            serde_json::Value::Object(ok.inputs.unwrap()).to_string(),
            r#"{"pr":"5"}"#
        );
    }

    #[test]
    fn refuses_a_port_that_is_not_a_number() {
        let e = ServerArgs::parse(&argv(&["serve", "--port=abc"])).unwrap_err();
        assert_eq!(e.0, "--port must be a number, got \"abc\"");
        let ok = ServerArgs::parse(&argv(&["serve", "--port=-3"])).unwrap();
        assert_eq!(ok.port, Some(-3.0));
    }
}
