//! The analysis against the JavaScript analysis it replaced, recorded as
//! `tests/fixtures/js-reference/analysis.json` at the repository root.
//!
//! Each case feeds its reads in order through one analyzer, as the server fed
//! each flush, and compares the status after every read, the completed lines
//! and the line in progress. The one accepted difference is named
//! [`REDRAWN_LINES`].

use std::collections::HashSet;
use std::path::PathBuf;

use serde_json::Value;
use vorn_analysis::{Analyzer, STATUS_ERROR, STATUS_RUNNING, STATUS_WAITING};

/// The analysis applies a carriage return and an erase in line to the line
/// being built, as a terminal does, so a line a program redrew in place holds
/// what was left on it. The JavaScript stripped those sequences and kept every
/// redraw run together on one line. Such a line is compared by its position,
/// not its text.
const REDRAWN_LINES: &str = "lines-redrawn-in-place-keep-what-is-left";

/// One recorded case.
#[derive(Debug)]
struct Case {
    name: String,
    reads: Vec<String>,
    statuses: Vec<String>,
    lines: Vec<String>,
    partial: String,
}

impl Case {
    fn from_json(value: &Value) -> Result<Self, String> {
        let text = |key: &str| {
            value[key]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("case field `{key}` is not a string: {value}"))
        };
        let texts = |key: &str| {
            value[key]
                .as_array()
                .and_then(|items| {
                    items
                        .iter()
                        .map(|item| item.as_str().map(str::to_owned))
                        .collect::<Option<Vec<_>>>()
                })
                .ok_or_else(|| format!("case field `{key}` is not a list of strings: {value}"))
        };
        Ok(Self {
            name: text("name")?,
            reads: texts("reads")?,
            statuses: texts("statuses")?,
            lines: texts("lines")?,
            partial: text("partial")?,
        })
    }
}

fn load_cases() -> Vec<Case> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../tests/fixtures/js-reference/analysis.json");
    let json = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("reading {}: {err}", path.display()));
    let root: Value = serde_json::from_str(&json)
        .unwrap_or_else(|err| panic!("parsing {}: {err}", path.display()));
    root["cases"]
        .as_array()
        .unwrap_or_else(|| panic!("{} has no `cases` list", path.display()))
        .iter()
        .map(|case| Case::from_json(case).unwrap_or_else(|err| panic!("{err}")))
        .collect()
}

/// The status name the server used for a code, or `None` for "no change",
/// which keeps the previous status.
fn status_name(code: u32) -> Option<&'static str> {
    match code {
        STATUS_RUNNING => Some("running"),
        STATUS_WAITING => Some("waiting"),
        STATUS_ERROR => Some("error"),
        _ => None,
    }
}

/// Whether a line holds a redraw: a carriage return that is not its last
/// character, an erase in line (`CSI K`, `CSI 0..2 K`) or a cursor move to a
/// column (`CSI n G`). The same test as `/\r(?!$)|\x1b\[[0-2]?K|\x1b\[\d*G/`.
fn is_redrawn(line: &str) -> bool {
    let bytes = line.as_bytes();
    let carriage_return = bytes
        .iter()
        .enumerate()
        .any(|(i, &b)| b == b'\r' && i + 1 < bytes.len());
    let csi = bytes.windows(2).enumerate().any(|(i, w)| {
        if w != b"\x1b[" {
            return false;
        }
        let rest = &bytes[i + 2..];
        let erase = matches!(rest, [b'K', ..] | [b'0'..=b'2', b'K', ..]);
        let digits = rest.iter().take_while(|b| b.is_ascii_digit()).count();
        erase || rest.get(digits) == Some(&b'G')
    });
    carriage_return || csi
}

/// The indexes of the output lines a redraw touched, from the raw reads.
fn redrawn_lines(reads: &[String]) -> HashSet<usize> {
    reads
        .concat()
        .split('\n')
        .enumerate()
        .filter(|(_, line)| is_redrawn(line))
        .map(|(i, _)| i)
        .collect()
}

/// Lines with the text of every redrawn one set aside.
fn without_redrawn(lines: &[String], redrawn: &HashSet<usize>) -> Vec<String> {
    lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            if redrawn.contains(&i) {
                format!("<{REDRAWN_LINES}>")
            } else {
                line.clone()
            }
        })
        .collect()
}

/// Runs one case, returning what differed.
fn check(case: &Case) -> Result<(), String> {
    let mut analyzer = Analyzer::new();
    let mut status = "running";
    let statuses: Vec<&str> = case
        .reads
        .iter()
        .map(|read| {
            status = status_name(analyzer.append_str(read, true)).unwrap_or(status);
            status
        })
        .collect();
    if statuses != case.statuses {
        return Err(format!(
            "statuses: got {statuses:?}, want {:?}",
            case.statuses
        ));
    }

    let redrawn = redrawn_lines(&case.reads);
    let got = without_redrawn(&analyzer.output(None), &redrawn);
    let want = without_redrawn(&case.lines, &redrawn);
    if got != want {
        return Err(format!(
            "lines ({REDRAWN_LINES}): got {got:?}, want {want:?}"
        ));
    }

    // The line in progress comes after the last full one.
    if !redrawn.contains(&case.lines.len()) && analyzer.partial() != case.partial {
        return Err(format!(
            "partial: got {:?}, want {:?}",
            analyzer.partial(),
            case.partial
        ));
    }
    Ok(())
}

#[test]
fn analysis_matches_js_reference() {
    let cases = load_cases();
    assert!(!cases.is_empty(), "the fixture has no cases");
    let failures: Vec<String> = cases
        .iter()
        .filter_map(|case| check(case).err().map(|err| format!("{}: {err}", case.name)))
        .collect();
    assert!(
        failures.is_empty(),
        "{} of {} cases differ from the JS reference:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}

#[test]
fn redraw_matcher_follows_its_pattern() {
    // A carriage return only counts before the end of the line.
    assert!(is_redrawn("a\rb"));
    assert!(!is_redrawn("a\r"));
    assert!(is_redrawn("\x1b[K"));
    assert!(is_redrawn("x\x1b[2Ky"));
    assert!(!is_redrawn("\x1b[3K"));
    assert!(is_redrawn("\x1b[G"));
    assert!(is_redrawn("\x1b[12G"));
    assert!(!is_redrawn("\x1b[1;2G"));
    assert!(!is_redrawn("\x1b[31m plain"));
}
