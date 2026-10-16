//! Screen rules: an agent's status read from what its screen shows, as data.
//!
//! A rule file is TOML, one per agent, with an ordered list of rules. Each
//! rule names a status and the conditions that mean it: text patterns in a
//! region of rows, the cursor's visibility, the window title. The first rule
//! whose conditions all hold decides; when none does, the rules say nothing
//! and the caller falls back on [`crate::Analyzer`]'s patterns.
//!
//! ```toml
//! agent = "example"
//!
//! [[rule]]
//! status = "waiting"
//! rows = [-12, -1]              # the last twelve rows; 0 is the top row
//! any = ['Do you want to proceed\?']
//!
//! [[rule]]
//! status = "working"
//! cursor_rows = [-3, 0]         # the cursor's row and the three above it
//! any = ['(?i)esc to interrupt']
//! none = ['^\s*>\s*$']
//! cursor = "hidden"
//! ```
//!
//! A row is matched as text, its trailing blanks dropped. `any` holds when
//! some row in the region matches one of its patterns, `all` when each of its
//! patterns matches some row, `none` when no row matches any of its patterns.
//! A rule needs at least one of `any`, `all` or `title`.

use std::fmt;

use regex::Regex;
use serde::Deserialize;

use crate::{STATUS_IDLE, STATUS_RUNNING, STATUS_WAITING};

/// What a screen says an agent is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScreenStatus {
    Working,
    Waiting,
    Idle,
}

impl ScreenStatus {
    /// The status effect's code, one of `STATUS_*`.
    pub fn code(self) -> u32 {
        match self {
            ScreenStatus::Working => STATUS_RUNNING,
            ScreenStatus::Waiting => STATUS_WAITING,
            ScreenStatus::Idle => STATUS_IDLE,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ScreenStatus::Working => "working",
            ScreenStatus::Waiting => "waiting",
            ScreenStatus::Idle => "idle",
        }
    }
}

/// Where the cursor is, in screen rows and columns from the top left.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cursor {
    pub row: u16,
    pub col: u16,
    pub visible: bool,
}

/// The part of a screen the rules read.
#[derive(Clone, Copy, Debug)]
pub struct Screen<'a> {
    /// Rows `top..top + rows.len()`, trailing blanks dropped. A row the
    /// rules read outside them is blank.
    pub rows: &'a [String],
    pub top: u16,
    /// The screen's height in rows.
    pub height: u16,
    pub cursor: Cursor,
    pub title: &'a str,
}

impl Screen<'_> {
    fn row(&self, y: u16) -> &str {
        y.checked_sub(self.top)
            .and_then(|i| self.rows.get(usize::from(i)))
            .map_or("", String::as_str)
    }
}

/// A rule file that would not read, with why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleError(pub String);

impl fmt::Display for RuleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RuleError {}

/// Whether a rule wants the cursor shown or hidden.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CursorWant {
    Visible,
    Hidden,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Region {
    /// Screen rows, inclusive; a negative one counts from the bottom.
    Rows(i32, i32),
    /// Rows from the cursor's, inclusive.
    Cursor(i32, i32),
}

impl Region {
    /// The rows it covers on `screen`, inclusive, or `None` when it falls
    /// off the screen.
    fn span(self, height: u16, cursor_row: u16) -> Option<(u16, u16)> {
        let h = i32::from(height);
        let (lo, hi) = match self {
            Region::Rows(a, b) => {
                let at = |n: i32| if n < 0 { h + n } else { n };
                (at(a), at(b))
            }
            Region::Cursor(a, b) => (i32::from(cursor_row) + a, i32::from(cursor_row) + b),
        };
        let (lo, hi) = (lo.max(0), hi.min(h - 1));
        // Both within 0..height by now, so they fit.
        (lo <= hi).then_some((lo as u16, hi as u16))
    }
}

#[derive(Debug)]
struct Rule {
    status: ScreenStatus,
    region: Region,
    any: Vec<Regex>,
    all: Vec<Regex>,
    none: Vec<Regex>,
    cursor: Option<CursorWant>,
    title: Option<Regex>,
}

impl Rule {
    fn matches(&self, s: &Screen<'_>) -> bool {
        match self.cursor {
            Some(CursorWant::Visible) if !s.cursor.visible => return false,
            Some(CursorWant::Hidden) if s.cursor.visible => return false,
            _ => {}
        }
        if self.title.as_ref().is_some_and(|t| !t.is_match(s.title)) {
            return false;
        }
        let rows = || -> Box<dyn Iterator<Item = &str>> {
            match self.region.span(s.height, s.cursor.row) {
                Some((lo, hi)) => Box::new((lo..=hi).map(|y| s.row(y))),
                None => Box::new(std::iter::empty()),
            }
        };
        if !self.any.is_empty() && !rows().any(|r| self.any.iter().any(|p| p.is_match(r))) {
            return false;
        }
        if !self.all.iter().all(|p| rows().any(|r| p.is_match(r))) {
            return false;
        }
        !rows().any(|r| self.none.iter().any(|p| p.is_match(r)))
    }
}

/// One agent's screen rules.
#[derive(Debug)]
pub struct Rules {
    agent: String,
    rules: Vec<Rule>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    agent: String,
    #[serde(default, rename = "rule")]
    rules: Vec<RuleSpec>,
}

/// One rule as written, in a rule file's `[[rule]]` or an agent adapter's
/// `[[agent.status]]`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleSpec {
    pub status: ScreenStatus,
    #[serde(default)]
    pub rows: Option<[i32; 2]>,
    #[serde(default)]
    pub cursor_rows: Option<[i32; 2]>,
    #[serde(default)]
    pub any: Vec<String>,
    #[serde(default)]
    pub all: Vec<String>,
    #[serde(default)]
    pub none: Vec<String>,
    #[serde(default)]
    pub cursor: Option<CursorWant>,
    #[serde(default)]
    pub title: Option<String>,
}

fn compile(n: usize, patterns: &[String]) -> Result<Vec<Regex>, RuleError> {
    patterns
        .iter()
        .map(|p| Regex::new(p).map_err(|e| RuleError(format!("rule {n}: pattern {p:?}: {e}"))))
        .collect()
}

impl Rules {
    /// Reads a rule file.
    pub fn parse(toml_text: &str) -> Result<Rules, RuleError> {
        let raw: RawFile = toml::from_str(toml_text).map_err(|e| RuleError(e.to_string()))?;
        Rules::new(&raw.agent, &raw.rules)
    }

    /// Compiles `specs` for `agent`, in their order.
    pub fn new(agent: &str, specs: &[RuleSpec]) -> Result<Rules, RuleError> {
        if specs.is_empty() {
            return Err(RuleError(format!("{agent}: no rules")));
        }
        let rules = specs
            .iter()
            .cloned()
            .enumerate()
            .map(|(i, r)| {
                let n = i + 1;
                let region = match (r.rows, r.cursor_rows) {
                    (Some(_), Some(_)) => {
                        return Err(RuleError(format!(
                            "rule {n}: both `rows` and `cursor_rows`"
                        )))
                    }
                    (Some([a, b]), None) => Region::Rows(a, b),
                    (None, Some([a, b])) => Region::Cursor(a, b),
                    (None, None) => Region::Rows(0, -1),
                };
                if r.any.is_empty() && r.all.is_empty() && r.title.is_none() {
                    return Err(RuleError(format!(
                        "rule {n}: needs `any`, `all` or `title`"
                    )));
                }
                let title = r
                    .title
                    .map(|t| Regex::new(&t))
                    .transpose()
                    .map_err(|e| RuleError(format!("rule {n}: title: {e}")))?;
                Ok(Rule {
                    status: r.status,
                    region,
                    any: compile(n, &r.any)?,
                    all: compile(n, &r.all)?,
                    none: compile(n, &r.none)?,
                    cursor: r.cursor,
                    title,
                })
            })
            .collect::<Result<_, _>>()?;
        Ok(Rules {
            agent: agent.to_owned(),
            rules,
        })
    }

    /// The agent the file names.
    pub fn agent(&self) -> &str {
        &self.agent
    }

    /// The rows every rule together reads on a screen `height` rows tall
    /// with the cursor on `cursor_row`, inclusive; `None` when they read none.
    pub fn span(&self, height: u16, cursor_row: u16) -> Option<(u16, u16)> {
        self.rules
            .iter()
            .filter(|r| !r.any.is_empty() || !r.all.is_empty() || !r.none.is_empty())
            .filter_map(|r| r.region.span(height, cursor_row))
            .reduce(|(a, b), (c, d)| (a.min(c), b.max(d)))
    }

    /// The status the first rule that holds names, or `None` when none does.
    pub fn status(&self, screen: &Screen<'_>) -> Option<ScreenStatus> {
        self.rules
            .iter()
            .find(|r| r.matches(screen))
            .map(|r| r.status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen<'a>(rows: &'a [String], cursor: Cursor) -> Screen<'a> {
        Screen {
            rows,
            top: 0,
            height: rows.len() as u16,
            cursor,
            title: "",
        }
    }

    fn rows(text: &str) -> Vec<String> {
        text.lines().map(str::to_owned).collect()
    }

    const FILE: &str = r#"
agent = "t"

[[rule]]
status = "waiting"
rows = [-3, -1]
any = ['Proceed\?']

[[rule]]
status = "working"
cursor_rows = [-1, 0]
any = ['busy']
none = ['^done']
cursor = "hidden"

[[rule]]
status = "idle"
all = ['^> ', 'help']
title = '^t$'
"#;

    #[test]
    fn the_first_rule_that_holds_decides() {
        let r = Rules::parse(FILE).unwrap();
        assert_eq!(r.agent(), "t");
        let s = rows("busy\nx\nProceed?\n");
        assert_eq!(
            r.status(&screen(
                &s,
                Cursor {
                    row: 1,
                    col: 0,
                    visible: false
                }
            )),
            Some(ScreenStatus::Waiting)
        );
    }

    #[test]
    fn a_region_reads_only_its_rows() {
        let r = Rules::parse(FILE).unwrap();
        let s = rows("Proceed?\na\nb\nc\n");
        assert_eq!(r.status(&screen(&s, Cursor::default())), None);
    }

    #[test]
    fn cursor_rows_follow_the_cursor_and_its_visibility() {
        let r = Rules::parse(FILE).unwrap();
        let s = rows("busy\nx\n\n\n\n");
        let at = |row, visible| Cursor {
            row,
            col: 0,
            visible,
        };
        assert_eq!(
            r.status(&screen(&s, at(1, false))),
            Some(ScreenStatus::Working)
        );
        assert_eq!(r.status(&screen(&s, at(1, true))), None);
        assert_eq!(r.status(&screen(&s, at(3, false))), None);
        let done = rows("done\nbusy\n\n");
        assert_eq!(r.status(&screen(&done, at(1, false))), None);
    }

    #[test]
    fn all_needs_every_pattern_and_the_title() {
        let r = Rules::parse(FILE).unwrap();
        let s = rows("> \n? for help\n\n\n");
        let mut sc = screen(&s, Cursor::default());
        assert_eq!(r.status(&sc), None);
        sc.title = "t";
        assert_eq!(r.status(&sc), Some(ScreenStatus::Idle));
        let half = rows("> \n\n\n\n");
        let mut sc = screen(&half, Cursor::default());
        sc.title = "t";
        assert_eq!(r.status(&sc), None);
    }

    #[test]
    fn rows_outside_those_read_are_blank() {
        let r = Rules::parse(FILE).unwrap();
        let s = vec!["Proceed?".to_owned()];
        let sc = Screen {
            rows: &s,
            top: 9,
            height: 10,
            cursor: Cursor::default(),
            title: "",
        };
        assert_eq!(r.status(&sc), Some(ScreenStatus::Waiting));
        assert_eq!(r.span(10, 4), Some((0, 9)));
    }

    #[test]
    fn the_span_covers_every_region() {
        let file = r#"
agent = "t"
[[rule]]
status = "idle"
rows = [-2, -1]
any = ['x']
[[rule]]
status = "working"
cursor_rows = [-1, 1]
any = ['y']
"#;
        let r = Rules::parse(file).unwrap();
        assert_eq!(r.span(24, 5), Some((4, 23)));
        assert_eq!(r.span(24, 23), Some((22, 23)));
    }

    #[test]
    fn bad_files_are_refused_with_why() {
        let cases = [
            ("agent = \"t\"", "no rules"),
            (
                "agent = \"t\"\n[[rule]]\nstatus = \"busy\"\nany = ['x']",
                "unknown variant",
            ),
            (
                "agent = \"t\"\n[[rule]]\nstatus = \"idle\"\nany = ['(']",
                "rule 1: pattern",
            ),
            ("agent = \"t\"\n[[rule]]\nstatus = \"idle\"", "needs `any`"),
            (
                "agent = \"t\"\n[[rule]]\nstatus = \"idle\"\nany = ['x']\nrows = [0, 1]\ncursor_rows = [0, 1]",
                "both",
            ),
            (
                "agent = \"t\"\n[[rule]]\nstatus = \"idle\"\nany = ['x']\nrow = [0, 1]",
                "unknown field",
            ),
        ];
        for (text, why) in cases {
            let err = Rules::parse(text).unwrap_err();
            assert!(err.0.contains(why), "{text:?}: {err}");
        }
    }

    #[test]
    fn statuses_carry_their_effect_codes() {
        assert_eq!(ScreenStatus::Working.code(), STATUS_RUNNING);
        assert_eq!(ScreenStatus::Waiting.code(), STATUS_WAITING);
        assert_eq!(ScreenStatus::Idle.code(), STATUS_IDLE);
    }
}
