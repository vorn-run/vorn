//! How the client commands write.
//!
//! One rule underneath all of it: stdout carries data and nothing else, so a
//! command can be piped without anything having to be stripped out of it.
//! Notices and errors go to stderr.

use serde_json::Value;

use crate::js;

const CSI: &str = "\x1b[";
const RESET: &str = "\x1b[0m";

/// Where a command writes. The binary writes to the process's streams; a test
/// collects what was written.
pub trait Io {
    /// Normal output: data, safe to pipe.
    fn write(&mut self, text: &str);
    /// Errors, notices and confirmations. Never data.
    fn write_err(&mut self, text: &str);
    /// Whether a terminal is watching stdout.
    fn is_tty(&self) -> bool;
}

/// The process's own stdout and stderr.
#[derive(Debug, Default)]
pub struct StdIo;

impl Io for StdIo {
    fn write(&mut self, text: &str) {
        use std::io::Write;
        // A closed pipe (`vorn session list | head -1`) is not an error worth
        // a panic; whatever was not read is simply not wanted.
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(text.as_bytes());
        let _ = out.flush();
    }

    fn write_err(&mut self, text: &str) {
        use std::io::Write;
        let _ = std::io::stderr().lock().write_all(text.as_bytes());
    }

    fn is_tty(&self) -> bool {
        use std::io::IsTerminal;
        std::io::stdout().is_terminal()
    }
}

/// What a command wrote, for tests.
#[derive(Debug, Default)]
pub struct Captured {
    pub out: String,
    pub err: String,
    pub tty: bool,
}

impl Io for Captured {
    fn write(&mut self, text: &str) {
        self.out.push_str(text);
    }

    fn write_err(&mut self, text: &str) {
        self.err.push_str(text);
    }

    fn is_tty(&self) -> bool {
        self.tty
    }
}

/// Whether output must stay plain: piped, redirected, or asked to be.
///
/// `NO_COLOR` is honoured by presence, not value: that is what the convention
/// says, and an empty string is how most shells set it.
pub fn is_plain(is_tty: bool) -> bool {
    !is_tty
        || std::env::var_os("NO_COLOR").is_some()
        || std::env::var_os("TERM").is_some_and(|t| t == "dumb")
}

/// Eight characters of a uuid still identify it, and a row stays readable.
///
/// Only of a uuid: seeded and imported workflows carry names as ids, and
/// cutting `system:default-task-workflow` to `system:d` identifies nothing.
pub fn short_id(id: &str) -> &str {
    let b = id.as_bytes();
    let hex = |range: std::ops::Range<usize>| b[range].iter().all(u8::is_ascii_hexdigit);
    if b.len() >= 14 && hex(0..8) && b[8] == b'-' && hex(9..13) && b[13] == b'-' {
        &id[..8]
    } else {
        id
    }
}

/// Colours a padded cell, given its column.
pub type Paint<'a> = &'a dyn Fn(&str, usize) -> String;

/// Columns padded to their widest cell, never truncated.
///
/// Padding happens before `paint` runs, so colour codes cannot widen a cell
/// and push the columns out of line.
pub fn table(headers: &[&str], rows: &[Vec<String>], paint: Option<Paint<'_>>) -> String {
    let widths: Vec<usize> = headers
        .iter()
        .enumerate()
        .map(|(i, header)| {
            rows.iter()
                .map(|row| row.get(i).map_or(0, |c| js::utf16_len(c)))
                .fold(js::utf16_len(header), usize::max)
        })
        .collect();

    let line = |cells: &[String], colour: bool| -> String {
        let last = cells.len().saturating_sub(1);
        let joined = cells
            .iter()
            .enumerate()
            .map(|(i, cell)| {
                let padded = if i == last {
                    cell.clone()
                } else {
                    js::pad_end(cell, widths.get(i).copied().unwrap_or(0))
                };
                match paint {
                    Some(paint) if colour => paint(&padded, i),
                    _ => padded,
                }
            })
            .collect::<Vec<_>>()
            .join("  ");
        joined
            .trim_end_matches(crate::args::is_js_whitespace)
            .to_owned()
    };

    let header: Vec<String> = headers.iter().map(|h| (*h).to_owned()).collect();
    let mut out = line(&header, false);
    for row in rows {
        out.push('\n');
        out.push_str(&line(row, true));
    }
    out.push('\n');
    out
}

/// Colour carries status and nothing else, and only when a terminal is watching.
pub fn paint_status(text: &str, plain: bool) -> String {
    if plain {
        return text.to_owned();
    }
    let code = match crate::args::js_trim(text) {
        "waiting" => "33m",
        "error" | "cancelled" => "31m",
        "running" | "success" => "32m",
        "idle" | "exited" => "2m",
        _ => return text.to_owned(),
    };
    format!("{CSI}{code}{text}{RESET}")
}

/// `Math.round`, which rounds halves up rather than away from zero.
fn js_round(x: f64) -> f64 {
    (x + 0.5).floor()
}

/// How long ago, in the coarsest unit that still says something.
pub fn time_ago(when: Option<&Value>, now_ms: f64) -> String {
    let then = match when {
        None => return "-".into(),
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(Value::String(s)) => crate::time::parse_iso(s).unwrap_or(f64::NAN),
        Some(_) => f64::NAN,
    };
    if !then.is_finite() {
        return "-".into();
    }
    let seconds = js_round((now_ms - then) / 1000.0).max(0.0);
    if seconds < 60.0 {
        return "just now".into();
    }
    let minutes = js_round(seconds / 60.0);
    if minutes < 60.0 {
        return format!("{}m ago", js::number(minutes));
    }
    let hours = js_round(minutes / 60.0);
    if hours < 48.0 {
        return format!("{}h ago", js::number(hours));
    }
    format!("{}d ago", js::number(js_round(hours / 24.0)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn shortens_uuids_only() {
        assert_eq!(short_id("c3f1a2e8-1111-2222-3333-444455556666"), "c3f1a2e8");
        assert_eq!(short_id("C3F1A2E8-1111-x"), "C3F1A2E8");
        assert_eq!(
            short_id("system:default-task-workflow"),
            "system:default-task-workflow"
        );
        assert_eq!(short_id("abc"), "abc");
    }

    #[test]
    fn pads_columns_and_trims_the_line() {
        let rows = vec![
            vec!["a".to_owned(), "long cell".to_owned(), "x".to_owned()],
            vec!["bbbb".to_owned(), "c".to_owned(), String::new()],
        ];
        assert_eq!(
            table(&["ID", "NAME", "S"], &rows, None),
            "ID    NAME       S\na     long cell  x\nbbbb  c\n"
        );
    }

    #[test]
    fn paints_after_padding() {
        let paint = |cell: &str, column: usize| {
            if column == 0 {
                paint_status(cell, false)
            } else {
                cell.to_owned()
            }
        };
        let rows = vec![vec!["running".to_owned(), "x".to_owned()]];
        let out = table(&["STATUS", "N"], &rows, Some(&paint));
        assert_eq!(out, "STATUS   N\n\x1b[32mrunning\x1b[0m  x\n");
    }

    #[test]
    fn says_how_long_ago_in_coarse_units() {
        let now = 10_000_000_000.0;
        assert_eq!(time_ago(None, now), "-");
        assert_eq!(time_ago(Some(&json!(null)), now), "-");
        assert_eq!(time_ago(Some(&json!("soon")), now), "-");
        assert_eq!(time_ago(Some(&json!(now + 5000.0)), now), "just now");
        assert_eq!(time_ago(Some(&json!(now - 89_000.0)), now), "1m ago");
        assert_eq!(time_ago(Some(&json!(now - 90_000.0)), now), "2m ago");
        assert_eq!(time_ago(Some(&json!(now - 3_600_000.0)), now), "1h ago");
        assert_eq!(
            time_ago(Some(&json!(now - 48.0 * 3_600_000.0)), now),
            "2d ago"
        );
        assert_eq!(
            time_ago(
                Some(&json!(crate::time::iso((now as i64) - 7_200_000))),
                now
            ),
            "2h ago"
        );
    }
}
