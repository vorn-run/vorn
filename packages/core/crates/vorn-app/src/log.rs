//! A shell's log, the terminal-bound half of shell blocks: its finished
//! commands cut from its lines at each prompt and paired, newest with newest,
//! with the commands the Shell blocks extension lists from the shell's marks.
//! This stays in the client because each cut reads the screen the client
//! mirrors, again whenever that screen changes; the extension says what ran.

use std::collections::HashMap;
use std::sync::Arc;
use web_time::Instant;

use vorn_tree::Command;

use crate::grid::Lines;
use crate::view::PaneView;

/// Finished commands a log keeps, as the desktop's.
pub const MAX_BLOCKS: usize = 100;
/// Prompts cut at most: as many as the extension keeps commands for.
const MAX_PROMPTS: usize = 400;
/// An idle prompt still needs its own row plus somewhere to type.
pub const MIN_LIVE_ROWS: u16 = 2;
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
    /// What the extension last listed of the shell's commands, oldest first.
    pub commands: Vec<Command>,
    /// When the command running now was first seen here, by its start.
    started: Option<(u64, Instant)>,
    /// The generation of the tree `commands` came from.
    pub generation: u64,
    pub blocks: Vec<Block>,
    /// The screen rows the live command takes: `(first, count)`.
    pub live: (u16, u16),
    pub lines: Lines,
    cache: HashMap<(u32, u64, u64), Arc<PaneView>>,
}

/// Whether `session` shows its finished commands as blocks over a live
/// region: an integrated shell with Shell blocks on, outside a full-screen program.
pub fn split<'a>(
    points: &crate::points::Points,
    session: &crate::client::Session,
    shell: Option<&'a Shell>,
) -> Option<&'a Shell> {
    if !crate::prompt::blocks_on(points) {
        return None;
    }
    shell.filter(|sh| session.is_shell() && !session.ended() && sh.integrated() && !sh.lines.alt)
}

impl Shell {
    /// Takes the commands the extension lists, seen at `now`.
    pub fn listed(&mut self, commands: Vec<Command>, now: Instant) {
        let running = commands.last().filter(|c| c.running()).map(|c| c.at);
        self.started = match (running, self.started) {
            (Some(at), Some((seen, since))) if at == seen => Some((seen, since)),
            (Some(at), _) => Some((at, now)),
            (None, _) => None,
        };
        self.commands = commands;
    }

    /// The command running now, if one is.
    pub fn running(&self) -> Option<Running> {
        let c = self.commands.last().filter(|c| c.running())?;
        Some(Running {
            command: c.command.clone(),
            since: self.started.map(|(_, at)| at)?,
        })
    }

    /// Whether the shell is known to report its commands.
    pub fn integrated(&self) -> bool {
        !self.commands.is_empty() || !self.lines.prompts.is_empty()
    }

    /// The entry each prompt in `lines` belongs to, by index.
    fn pair(&self, lines: &Lines) -> Vec<Option<usize>> {
        let (n, m) = (lines.prompts.len() as i64, self.commands.len() as i64);
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
        let first = n.saturating_sub(MAX_PROMPTS);
        for (w, p) in lines.prompts.windows(2).zip(&paired).skip(first) {
            let (start, end) = (w[0], w[1]);
            let entry = p.and_then(|j| self.commands.get(j));
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
                exit_code: entry.and_then(|e| e.exit),
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

/// A duration as `0.3s`, `4.2s`, `42s` and up.
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

/// The last two parts of a path.
pub fn shorten_cwd(cwd: &str) -> String {
    let parts: Vec<&str> = cwd.split('/').filter(|p| !p.is_empty()).collect();
    match parts.len() {
        0 => "/".to_owned(),
        1 | 2 => format!("/{}", parts.join("/")),
        n => parts[n - 2..].join("/"),
    }
}

/// Jump by command: how far back to scroll for the prompt before (`up`) or
/// after the line at the top of the view. `prompts` are absolute prompt lines,
/// oldest first; `top` is the line the view starts at and `live` the live
/// screen's first. Answers rows above the live screen, 0 for the live screen
/// itself, and `None` when no prompt lies that way.
pub fn prompt_jump(prompts: &[u64], top: u64, live: u64, up: bool) -> Option<u64> {
    if up {
        let target = prompts.iter().rev().find(|&&p| p < top)?;
        return Some(live.saturating_sub(*target));
    }
    Some(match prompts.iter().find(|&&p| p > top) {
        Some(&p) if p < live => live - p,
        _ => 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jumps_to_the_prompt_before_or_after_the_top_of_the_view() {
        let prompts = [10, 40, 90, 130];
        // From the live screen at 120: up to 90, then 40, then 10, then nothing.
        assert_eq!(prompt_jump(&prompts, 120, 120, true), Some(30));
        assert_eq!(prompt_jump(&prompts, 90, 120, true), Some(80));
        assert_eq!(prompt_jump(&prompts, 40, 120, true), Some(110));
        assert_eq!(prompt_jump(&prompts, 10, 120, true), None);
        // Down from 10 is 40; past the last one in history is the live screen.
        assert_eq!(prompt_jump(&prompts, 10, 120, false), Some(80));
        assert_eq!(prompt_jump(&prompts, 90, 120, false), Some(0));
        assert_eq!(prompt_jump(&[], 50, 120, false), Some(0));
        assert_eq!(prompt_jump(&[], 50, 120, true), None);
    }
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

    fn cmd(command: Option<&str>, started: bool, ended: bool) -> Command {
        Command {
            command: command.map(str::to_owned),
            cwd: Some("/a/b/c".into()),
            started,
            ended,
            exit: ended.then_some(1),
            duration_ms: ended.then_some(250),
            at: 100,
        }
    }

    #[test]
    fn finished_commands_become_blocks_and_empty_prompts_do_not() {
        let now = Instant::now();
        let mut sh = Shell::default();
        sh.listed(vec![cmd(Some("ls"), true, false)], now);
        assert_eq!(sh.running().map(|r| r.command), Some(Some("ls".into())));
        let later = now + std::time::Duration::from_secs(1);
        sh.listed(vec![cmd(Some("ls"), true, false)], later);
        assert_eq!(sh.running().map(|r| r.since), Some(now), "seen once");
        // ls ended; Enter on an empty line; the prompt now.
        sh.listed(
            vec![
                cmd(Some("ls"), true, true),
                cmd(None, false, false),
                cmd(None, false, false),
            ],
            later,
        );
        assert_eq!(sh.running(), None);
        let l = lines(&[0, 3, 4], 0, 4);
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
        let mut make = cmd(Some("make"), true, true);
        make.exit = Some(0);
        sh.listed(vec![make, cmd(None, false, false)], now);
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
