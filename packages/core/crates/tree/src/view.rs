//! Card views: the cards a view arranges, what each session is doing, how big each is drawn, and the sidebar's selection.

use std::borrow::Cow;
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Output this long apart is one burst: a card prints again only after this much quiet.
pub const QUIET_MS: u64 = 2_000;
/// A session that has not printed yet reads as started for this long.
pub const STARTED_MS: u64 = 10_000;
/// The most characters a card's last line keeps.
pub const MAX_LAST_LINE: usize = 200;

/// How big a view draws a session's card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CardSize {
    /// The card as the grid draws it.
    Full,
    /// The terminal drawn smaller, clipped to what fits; its session keeps its size.
    Mini,
    /// One line: status, title and the last line printed.
    Line,
    /// Whatever a view's own tree draws for it; never a terminal.
    Tile,
}

impl CardSize {
    pub const ALL: [CardSize; 4] = [
        CardSize::Full,
        CardSize::Mini,
        CardSize::Line,
        CardSize::Tile,
    ];

    pub fn key(self) -> &'static str {
        match self {
            CardSize::Full => "full",
            CardSize::Mini => "mini",
            CardSize::Line => "line",
            CardSize::Tile => "tile",
        }
    }

    pub fn parse(key: &str) -> Option<CardSize> {
        CardSize::ALL.into_iter().find(|s| s.key() == key)
    }
}

/// What a session is doing, as a view's columns and card sizes read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CardStatus {
    /// New, and nothing printed yet.
    Started,
    /// At work: printing, or its agent says it runs.
    Printing,
    /// Waiting for a person.
    Waiting,
    /// Its program ended, failed, or its agent went quiet after working.
    Finished,
    /// Nothing done yet, or nothing for a while.
    Idle,
}

impl CardStatus {
    pub const ALL: [CardStatus; 5] = [
        CardStatus::Started,
        CardStatus::Printing,
        CardStatus::Waiting,
        CardStatus::Finished,
        CardStatus::Idle,
    ];

    pub fn key(self) -> &'static str {
        match self {
            CardStatus::Started => "started",
            CardStatus::Printing => "printing",
            CardStatus::Waiting => "waiting",
            CardStatus::Finished => "finished",
            CardStatus::Idle => "idle",
        }
    }

    pub fn parse(key: &str) -> Option<CardStatus> {
        CardStatus::ALL.into_iter().find(|s| s.key() == key)
    }

    /// What a column of this status is headed.
    pub fn label(self) -> &'static str {
        match self {
            CardStatus::Started => "Started",
            CardStatus::Printing => "Running",
            CardStatus::Waiting => "Waiting",
            CardStatus::Finished => "Done",
            CardStatus::Idle => "Idle",
        }
    }

    /// A card's status from its agent status, whether it ended, and when it started and last printed (epoch ms).
    pub fn of(status: &str, exited: bool, created: u64, printed: Option<u64>, now: u64) -> Self {
        let recent = |at: u64| now.saturating_sub(at) < QUIET_MS;
        match status {
            "waiting" => CardStatus::Waiting,
            _ if exited => CardStatus::Finished,
            "error" => CardStatus::Finished,
            _ if printed.is_some_and(recent) => CardStatus::Printing,
            "running" if printed.is_some() => CardStatus::Printing,
            _ if printed.is_none() && now.saturating_sub(created) < STARTED_MS => {
                CardStatus::Started
            }
            "running" => CardStatus::Printing,
            _ if printed.is_some() => CardStatus::Finished,
            _ => CardStatus::Idle,
        }
    }
}

/// One session as a view sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Card {
    pub id: String,
    pub title: String,
    pub agent: String,
    pub status: CardStatus,
    /// The heading the view's grouping puts it under.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// The last line it printed, as [`last_line`] reads it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_line: Option<String>,
    /// When it last printed, in epoch ms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub printed_at: Option<u64>,
}

/// What the sidebar has selected, which a view follows.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Selection {
    /// A group heading, by its key.
    Group(String),
    Session(String),
}

/// Which session prints a new burst: once after [`QUIET_MS`] of quiet, not per chunk.
#[derive(Debug, Default)]
pub struct Bursts {
    last: HashMap<String, u64>,
}

impl Bursts {
    /// Session `id` printed at `now` (epoch ms); whether that starts a burst.
    pub fn printed(&mut self, id: &str, now: u64) -> bool {
        match self.last.get_mut(id) {
            Some(at) => {
                let gap = now.saturating_sub(*at);
                *at = now;
                gap >= QUIET_MS
            }
            None => {
                self.last.insert(id.to_owned(), now);
                true
            }
        }
    }

    /// When session `id` last printed, in epoch ms.
    pub fn at(&self, id: &str) -> Option<u64> {
        self.last.get(id).copied()
    }

    pub fn forget(&mut self, id: &str) {
        self.last.remove(id);
    }
}

/// What an agent status moving from `from` to `to` says it did; output never moves it.
pub fn moved(from: Option<&str>, to: &str) -> Option<CardStatus> {
    match (from, to) {
        (_, "waiting") => Some(CardStatus::Waiting),
        (Some("running" | "waiting"), "idle") | (_, "error") => Some(CardStatus::Finished),
        _ => None,
    }
}

/// The last non-blank line of `lines`, escapes and controls stripped, cut to [`MAX_LAST_LINE`] characters.
pub fn last_line<'a, I>(lines: I) -> Option<String>
where
    I: IntoIterator<Item = &'a str>,
    I::IntoIter: DoubleEndedIterator,
{
    lines.into_iter().rev().find_map(|line| {
        let plain = plain(line);
        let kept = plain.trim();
        if kept.is_empty() {
            return None;
        }
        Some(match kept.char_indices().nth(MAX_LAST_LINE) {
            Some((cut, _)) => kept[..cut].to_owned(),
            None => kept.to_owned(),
        })
    })
}

/// `line` without its escape sequences (CSI, OSC, DCS and two-byte ones) or control characters.
fn plain(line: &str) -> Cow<'_, str> {
    if !line.chars().any(char::is_control) {
        return Cow::Borrowed(line);
    }
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\u{1b}' {
            if !ch.is_control() {
                out.push(ch);
            } else if ch == '\t' {
                out.push(' ');
            }
            continue;
        }
        match chars.next() {
            // CSI: parameters and intermediates, then one final byte.
            Some('[') => {
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            // OSC, DCS, APC, PM, SOS: up to BEL or ST.
            Some(']' | 'P' | '_' | '^' | 'X') => {
                while let Some(c) = chars.next() {
                    if c == '\u{7}' {
                        break;
                    }
                    if c == '\u{1b}' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_line_skips_blanks_and_strips_escapes() {
        let lines = ["first", "\u{1b}[32mok\u{1b}[0m  done\t", "   ", ""];
        assert_eq!(last_line(lines).as_deref(), Some("ok  done"));
        let osc = ["\u{1b}]0;title\u{7}$ ls\u{1b}]8;;http://x\u{1b}\\link\u{1b}]8;;\u{1b}\\"];
        assert_eq!(last_line(osc).as_deref(), Some("$ lslink"));
        assert_eq!(last_line(["", " \u{1b}[K "]), None);
        assert_eq!(last_line(Vec::<&str>::new()), None);
    }

    #[test]
    fn the_last_line_is_cut_at_a_character_not_a_byte() {
        let long = "é".repeat(MAX_LAST_LINE + 50);
        let kept = last_line([long.as_str()]).unwrap();
        assert_eq!(kept.chars().count(), MAX_LAST_LINE);
        // A sequence cut off at the end of the line takes nothing after it.
        assert_eq!(last_line(["ok\u{1b}[3"]).as_deref(), Some("ok"));
    }

    #[test]
    fn one_burst_prints_once_until_it_goes_quiet() {
        let mut b = Bursts::default();
        assert!(b.printed("a", 1_000));
        assert!(!b.printed("a", 1_900));
        assert!(!b.printed("a", 3_800));
        assert!(b.printed("a", 3_800 + QUIET_MS));
        assert!(b.printed("b", 3_800), "each session has its own bursts");
        b.forget("a");
        assert_eq!(b.at("a"), None);
        assert!(b.printed("a", 9_000));
    }

    #[test]
    fn output_never_moves_a_status_but_waiting_and_finishing_do() {
        assert_eq!(moved(Some("idle"), "waiting"), Some(CardStatus::Waiting));
        assert_eq!(moved(Some("running"), "idle"), Some(CardStatus::Finished));
        assert_eq!(moved(None, "error"), Some(CardStatus::Finished));
        assert_eq!(moved(Some("idle"), "running"), None);
        assert_eq!(moved(None, "idle"), None);
    }

    #[test]
    fn a_card_reads_its_status_from_its_session() {
        let now = 100_000;
        let of = |status, exited, created, printed| {
            CardStatus::of(status, exited, created, printed, now)
        };
        assert_eq!(of("waiting", false, 0, Some(now)), CardStatus::Waiting);
        assert_eq!(of("idle", true, 0, Some(now)), CardStatus::Finished);
        assert_eq!(of("idle", false, now - 1_000, None), CardStatus::Started);
        assert_eq!(of("idle", false, 0, Some(now - 500)), CardStatus::Printing);
        assert_eq!(of("running", false, 0, Some(0)), CardStatus::Printing);
        assert_eq!(of("idle", false, 0, Some(0)), CardStatus::Finished);
        assert_eq!(of("idle", false, 0, None), CardStatus::Idle);
        assert_eq!(of("error", false, 0, None), CardStatus::Finished);
    }

    #[test]
    fn sizes_statuses_and_selections_round_trip() {
        for s in CardSize::ALL {
            assert_eq!(CardSize::parse(s.key()), Some(s));
        }
        for s in CardStatus::ALL {
            assert_eq!(CardStatus::parse(s.key()), Some(s));
        }
        let sel = Selection::Group("g".into());
        let v = serde_json::to_value(&sel).unwrap();
        assert_eq!(v, serde_json::json!({ "group": "g" }));
        assert_eq!(serde_json::from_value::<Selection>(v).unwrap(), sel);
        let card = Card {
            id: "s".into(),
            title: "t".into(),
            agent: "shell".into(),
            status: CardStatus::Printing,
            group: None,
            last_line: Some("ok".into()),
            printed_at: Some(5),
        };
        let v = serde_json::to_value(&card).unwrap();
        assert_eq!(v["lastLine"], "ok");
        assert_eq!(v["printedAt"], 5);
        assert_eq!(serde_json::from_value::<Card>(v).unwrap(), card);
    }
}
