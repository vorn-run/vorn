//! Command blocks (BlockLog, RunningCommand): a shell's finished commands,
//! cut from its lines at each prompt and titled and timed by the
//! `terminal:command` marks vornd sends as the shell reports them.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use serde_json::Value;

use crate::grid::Lines;
use crate::view::PaneView;

/// Finished commands a log keeps, as the desktop's.
pub const MAX_BLOCKS: usize = 100;
/// An idle prompt still needs its own row plus somewhere to type.
pub const MIN_LIVE_ROWS: u16 = 2;
/// Prompts remembered: more than the log shows, since empty ones count.
const MAX_ENTRIES: usize = 400;

/// One of the shell's reports about a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mark {
    Prompt,
    Start(Option<String>),
    End {
        exit_code: Option<i32>,
        duration_ms: Option<u64>,
    },
}

/// A `terminal:command` note: the session, its mark and when (epoch ms).
pub fn parse(p: &Value) -> Option<(String, Mark, u64)> {
    let id = p.get("id")?.as_str()?.to_owned();
    let at = p.get("at").and_then(Value::as_u64).unwrap_or(0);
    let mark = match p.get("phase")?.as_str()? {
        "prompt" => Mark::Prompt,
        "start" => Mark::Start(p.get("command").and_then(Value::as_str).map(str::to_owned)),
        "end" => Mark::End {
            exit_code: p
                .get("exitCode")
                .and_then(Value::as_i64)
                .and_then(|c| i32::try_from(c).ok()),
            duration_ms: p.get("durationMs").and_then(Value::as_u64),
        },
        _ => return None,
    };
    Some((id, mark, at))
}

/// What the shell said about one prompt.
#[derive(Debug, Clone, Default, PartialEq)]
struct Entry {
    command: Option<String>,
    /// The directory reported at the prompt, where the command runs.
    cwd: Option<String>,
    started: bool,
    start_at: u64,
    since: Option<Instant>,
    ended: bool,
    exit_code: Option<i32>,
    duration_ms: Option<u64>,
}

/// A finished command, ready to draw.
#[derive(Debug, Clone)]
pub struct Block {
    /// Its lines: `(epoch, first, end)`.
    pub key: (u32, u64, u64),
    /// None when the shell did not say; its first row is the title then.
    pub command: Option<String>,
    pub cwd: Option<String>,
    pub exit_code: Option<i32>,
    pub duration_ms: Option<u64>,
    /// The output, or the whole block when there is no command to title it.
    pub view: Arc<PaneView>,
}

impl Block {
    pub fn failed(&self) -> bool {
        self.exit_code.is_some_and(|c| c != 0)
    }

    /// What Copy puts on the clipboard: the command and its output.
    pub fn text(&self) -> String {
        let out = self.view.text();
        match &self.command {
            Some(c) if out.is_empty() => c.clone(),
            Some(c) => format!("{c}\n{out}"),
            None => out,
        }
    }
}

/// The command running now.
#[derive(Debug, Clone, PartialEq)]
pub struct Running {
    pub command: Option<String>,
    pub since: Instant,
}

/// One shell's blocks.
#[derive(Debug, Default)]
pub struct Shell {
    /// The shell has reported a prompt or a command.
    pub seen: bool,
    entries: VecDeque<Entry>,
    pub blocks: Vec<Block>,
    /// The screen rows the live command takes: `(first, count)`.
    pub live: (u16, u16),
    pub lines: Lines,
    cache: HashMap<(u32, u64, u64), Arc<PaneView>>,
}

/// Whether `session` shows its finished commands as blocks over a live
/// region: an integrated shell, outside a full-screen program.
pub fn split<'a>(
    config: &Value,
    session: &crate::client::Session,
    shell: Option<&'a Shell>,
) -> Option<&'a Shell> {
    let on = config
        .pointer("/defaults/domBlockRendering")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    shell.filter(|sh| {
        on && session.is_shell() && !session.ended() && sh.integrated() && !sh.lines.alt
    })
}

impl Shell {
    /// Takes a mark the shell reported at `at` (epoch ms), seen at `now`
    /// with the directory as `cwd`.
    pub fn mark(&mut self, mark: Mark, at: u64, cwd: Option<&str>, now: Instant) {
        self.seen = true;
        match mark {
            Mark::Prompt => {
                self.entries.push_back(Entry {
                    cwd: cwd.filter(|c| !c.is_empty()).map(str::to_owned),
                    ..Entry::default()
                });
                while self.entries.len() > MAX_ENTRIES {
                    self.entries.pop_front();
                }
            }
            Mark::Start(command) => {
                // A second start for one command is another integration also marking; it changes nothing.
                if self.entries.back().is_some_and(|e| e.started && !e.ended) {
                    return;
                }
                if self.entries.back().is_none_or(|e| e.started) {
                    self.entries.push_back(Entry::default());
                }
                let Some(e) = self.entries.back_mut() else {
                    return;
                };
                e.started = true;
                e.command = command.filter(|c| !c.trim().is_empty());
                e.start_at = at;
                e.since = Some(now);
                // The prompt's directory, by now surely reported.
                if let Some(c) = cwd.filter(|c| !c.is_empty()) {
                    e.cwd = Some(c.to_owned());
                }
            }
            Mark::End {
                exit_code,
                duration_ms,
            } => {
                let Some(e) = self.entries.back_mut().filter(|e| !e.ended) else {
                    return;
                };
                // A shell with no hook before a command reports it only once it is over.
                if !e.started {
                    e.started = true;
                    e.start_at = at;
                }
                e.ended = true;
                e.exit_code = Some(exit_code.unwrap_or(0));
                e.duration_ms = Some(duration_ms.unwrap_or(at.saturating_sub(e.start_at)));
            }
        }
    }

    /// The command running now, if one is.
    pub fn running(&self) -> Option<Running> {
        let e = self.entries.back().filter(|e| e.started && !e.ended)?;
        Some(Running {
            command: e.command.clone(),
            since: e.since?,
        })
    }

    /// Whether the shell is known to report its commands.
    pub fn integrated(&self) -> bool {
        self.seen || !self.lines.prompts.is_empty()
    }

    /// The entry each prompt in `lines` belongs to, by index.
    fn pair(&self, lines: &Lines) -> Vec<Option<usize>> {
        let (n, m) = (lines.prompts.len() as i64, self.entries.len() as i64);
        // Both come from the shell's prompt marks, so the newest of each belong together.
        let shift = m - n;
        (0..n)
            .map(|i| {
                let j = i + shift;
                (0..m).contains(&j).then_some(j as usize)
            })
            .collect()
    }

    /// Cuts `lines` into blocks, getting each block's rows from `view`
    /// (lines `from..to`, blank edges trimmed) unless they were cut before.
    pub fn sync(&mut self, lines: Lines, mut view: impl FnMut(u64, u64) -> Option<PaneView>) {
        let paired = self.pair(&lines);
        let mut blocks = Vec::new();
        let mut cache = HashMap::new();
        let n = lines.prompts.len();
        let first = n.saturating_sub(MAX_ENTRIES);
        for (w, p) in lines.prompts.windows(2).zip(&paired).skip(first) {
            let (start, end) = (w[0], w[1]);
            let entry = p.map(|j| &self.entries[j]);
            // A prompt left without a command: Enter on an empty line, or ^C.
            if entry.is_some_and(|e| !e.started) {
                continue;
            }
            let command = entry.and_then(|e| e.command.clone());
            let from = if command.is_some() { start + 1 } else { start };
            let key = (lines.epoch, from, end);
            let v = match self.cache.get(&key) {
                Some(v) => Arc::clone(v),
                None => match view(from, end) {
                    Some(v) => Arc::new(v),
                    None => continue,
                },
            };
            // Nothing said about it and nothing after its own line: the prompt of a command that printed nothing.
            if command.is_none() && v.rows < if entry.is_some() { 1 } else { 2 } {
                continue;
            }
            cache.insert(key, Arc::clone(&v));
            blocks.push(Block {
                key,
                command,
                cwd: entry.and_then(|e| e.cwd.clone()),
                exit_code: entry.and_then(|e| e.exit_code),
                duration_ms: entry.and_then(|e| e.duration_ms),
                view: v,
            });
        }
        if blocks.len() > MAX_BLOCKS {
            blocks.drain(..blocks.len() - MAX_BLOCKS);
        }
        self.cache = cache;
        self.blocks = blocks;
        self.live = live(&lines);
        self.lines = lines;
    }
}

/// The screen rows the live command takes, from its prompt to the cursor:
/// one row at an idle prompt, more as a command draws.
pub fn live(l: &Lines) -> (u16, u16) {
    let rows = l.rows.max(1);
    let start = l.prompts.last().copied().unwrap_or(l.top);
    let first = u16::try_from(start.saturating_sub(l.top))
        .unwrap_or(u16::MAX)
        .min(l.cursor_y);
    let count = (l.cursor_y - first + 1).max(MIN_LIVE_ROWS).min(rows);
    (first.min(rows - count), count)
}

/// `formatDuration`.
pub fn format_duration(ms: u64) -> String {
    if ms < 1000 {
        let tenths = ((ms as f64 / 100.0).round() as u64).max(1);
        return format!("{}s", tenths as f64 / 10.0);
    }
    let s = ms as f64 / 1000.0;
    if s < 10.0 {
        return format!("{s:.1}s");
    }
    if s < 60.0 {
        return format!("{}s", s.round() as u64);
    }
    let m = (s / 60.0).floor();
    format!("{}m {}s", m as u64, (s - m * 60.0).round() as u64)
}

/// `shortenCwd`: the last two parts of a path.
pub fn shorten_cwd(cwd: &str) -> String {
    let parts: Vec<&str> = cwd.split('/').filter(|p| !p.is_empty()).collect();
    match parts.len() {
        0 => "/".to_owned(),
        1 | 2 => format!("/{}", parts.join("/")),
        n => parts[n - 2..].join("/"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn lines(prompts: &[u64], top: u64, cursor_y: u16) -> Lines {
        Lines {
            epoch: 1,
            prompts: prompts.to_vec(),
            top,
            cursor_y,
            rows: 24,
            alt: false,
        }
    }

    fn rows(n: u16) -> PaneView {
        PaneView {
            rows: n,
            ..PaneView::default()
        }
    }

    #[test]
    fn marks_parse_from_the_note() {
        let start = json!({"id": "s1", "phase": "start", "command": "ls", "at": 5});
        assert_eq!(
            parse(&start),
            Some(("s1".into(), Mark::Start(Some("ls".into())), 5))
        );
        let end = json!({"id": "s1", "phase": "end", "exitCode": 2, "durationMs": 40, "at": 9});
        assert_eq!(
            parse(&end),
            Some((
                "s1".into(),
                Mark::End {
                    exit_code: Some(2),
                    duration_ms: Some(40)
                },
                9
            ))
        );
        assert_eq!(parse(&json!({"id": "s1", "phase": "?"})), None);
    }

    #[test]
    fn finished_commands_become_blocks_and_empty_prompts_do_not() {
        let now = Instant::now();
        let mut sh = Shell::default();
        sh.mark(Mark::Prompt, 0, Some("/a/b/c"), now);
        sh.mark(Mark::Start(Some("ls".into())), 100, None, now);
        assert_eq!(sh.running().map(|r| r.command), Some(Some("ls".into())));
        sh.mark(
            Mark::End {
                exit_code: Some(1),
                duration_ms: None,
            },
            350,
            None,
            now,
        );
        assert_eq!(sh.running(), None);
        sh.mark(Mark::Prompt, 400, Some("/a/b/c"), now);
        // Enter on an empty line.
        let l = lines(&[0, 3, 4], 0, 4);
        sh.mark(Mark::Prompt, 500, Some("/a/b/c"), now);
        let mut asked = Vec::new();
        sh.sync(l, |from, to| {
            asked.push((from, to));
            Some(rows((to - from) as u16))
        });
        assert_eq!(asked, vec![(1, 3)]);
        assert_eq!(sh.blocks.len(), 1);
        let b = &sh.blocks[0];
        assert_eq!(b.command.as_deref(), Some("ls"));
        assert_eq!((b.exit_code, b.duration_ms), (Some(1), Some(250)));
        assert!(b.failed());
        assert_eq!(b.cwd.as_deref(), Some("/a/b/c"));
        assert_eq!(sh.live, (4, 2));
    }

    #[test]
    fn a_resize_keeps_titles_by_counting_prompts_from_the_end() {
        let now = Instant::now();
        let mut sh = Shell::default();
        sh.mark(Mark::Prompt, 0, None, now);
        sh.mark(Mark::Start(Some("make".into())), 0, None, now);
        sh.mark(
            Mark::End {
                exit_code: Some(0),
                duration_ms: Some(1200),
            },
            0,
            None,
            now,
        );
        sh.mark(Mark::Prompt, 0, None, now);
        // Reflowed: a new epoch, new numbers, an older prompt from before the app.
        let mut l2 = lines(&[0, 4, 9], 0, 9);
        l2.epoch = 2;
        sh.sync(l2, |from, to| Some(rows((to - from) as u16)));
        let titles: Vec<_> = sh.blocks.iter().map(|b| b.command.clone()).collect();
        assert_eq!(titles, vec![None, Some("make".into())]);
    }

    #[test]
    fn the_live_region_runs_from_the_prompt_to_the_cursor() {
        assert_eq!(live(&lines(&[], 0, 0)), (0, 2));
        assert_eq!(live(&lines(&[5], 0, 9)), (5, 5));
        // The prompt scrolled off: the whole screen.
        assert_eq!(live(&lines(&[5], 40, 23)), (0, 24));
        assert_eq!(live(&lines(&[23], 0, 23)), (22, 2));
    }

    #[test]
    fn durations_and_directories_read_as_the_desktop_writes_them() {
        assert_eq!(format_duration(0), "0.1s");
        assert_eq!(format_duration(340), "0.3s");
        assert_eq!(format_duration(2345), "2.3s");
        assert_eq!(format_duration(12_400), "12s");
        assert_eq!(format_duration(75_000), "1m 15s");
        assert_eq!(shorten_cwd("/"), "/");
        assert_eq!(shorten_cwd("/Users/me"), "/Users/me");
        assert_eq!(shorten_cwd("/Users/me/dev/vorn"), "dev/vorn");
    }
}
