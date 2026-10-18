//! What a shell said about each prompt, from its OSC 133 marks: the command
//! it ran, where, how it ended and how long it took. One [`Command`] per
//! prompt, oldest first, as the client pairs them with the prompts on screen.

use vorn_extension_sdk::tree::Command;
use vorn_extension_sdk::Value;

/// Prompts remembered: more than the log shows, since empty ones count.
pub const MAX_COMMANDS: usize = 400;

/// One of the shell's reports about a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mark {
    Prompt,
    Start(Option<String>),
    End {
        exit: Option<i32>,
        duration_ms: Option<u64>,
    },
}

/// A `session.mark` event: its session, the mark, when (epoch ms) and the shell's folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marked {
    pub session: String,
    pub mark: Mark,
    pub at: u64,
    pub cwd: Option<String>,
}

impl Marked {
    pub fn read(v: &Value) -> Option<Marked> {
        let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
        let mark = match v.get("phase")?.as_str()? {
            "prompt" => Mark::Prompt,
            "start" => Mark::Start(text("command")),
            "end" => Mark::End {
                exit: v
                    .get("exitCode")
                    .and_then(Value::as_i64)
                    .and_then(|c| i32::try_from(c).ok()),
                duration_ms: v.get("durationMs").and_then(Value::as_u64),
            },
            _ => return None,
        };
        Some(Marked {
            session: text("sessionId")?,
            mark,
            at: v.get("at").and_then(Value::as_u64).unwrap_or(0),
            cwd: text("cwd").filter(|c| !c.is_empty()),
        })
    }
}

/// Folds `mark`, reported at `at` with the shell in `cwd`, into `commands`.
pub fn fold(commands: &mut Vec<Command>, mark: Mark, at: u64, cwd: Option<&str>) {
    match mark {
        Mark::Prompt => {
            commands.push(Command {
                cwd: cwd.map(str::to_owned),
                ..Command::default()
            });
            if commands.len() > MAX_COMMANDS {
                commands.drain(..commands.len() - MAX_COMMANDS);
            }
        }
        Mark::Start(command) => {
            // A second start for one command is another integration also marking; it changes nothing.
            if commands.last().is_some_and(Command::running) {
                return;
            }
            if commands.last().is_none_or(|c| c.started) {
                commands.push(Command::default());
            }
            let Some(c) = commands.last_mut() else {
                return;
            };
            c.started = true;
            c.command = command.filter(|t| !t.trim().is_empty());
            c.at = at;
            // The prompt's folder, by now surely reported.
            if let Some(d) = cwd {
                c.cwd = Some(d.to_owned());
            }
        }
        Mark::End { exit, duration_ms } => {
            let Some(c) = commands.last_mut().filter(|c| !c.ended) else {
                return;
            };
            // A shell with no hook before a command reports it only once it is over.
            if !c.started {
                c.started = true;
                c.at = at;
            }
            c.ended = true;
            c.exit = Some(exit.unwrap_or(0));
            c.duration_ms = Some(duration_ms.unwrap_or(at.saturating_sub(c.at)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vorn_extension_sdk::json;

    #[test]
    fn marks_read_from_the_event() {
        let start = json!({"sessionId": "s1", "phase": "start", "command": "ls", "at": 5, "cwd": "/a"});
        assert_eq!(
            Marked::read(&start),
            Some(Marked {
                session: "s1".into(),
                mark: Mark::Start(Some("ls".into())),
                at: 5,
                cwd: Some("/a".into()),
            })
        );
        let end = json!({"sessionId": "s1", "phase": "end", "exitCode": 2, "durationMs": 40, "at": 9});
        assert_eq!(
            Marked::read(&end).map(|m| m.mark),
            Some(Mark::End {
                exit: Some(2),
                duration_ms: Some(40)
            })
        );
        assert_eq!(Marked::read(&json!({"sessionId": "s1", "phase": "?"})), None);
        assert_eq!(Marked::read(&json!({"phase": "prompt"})), None);
    }

    #[test]
    fn a_prompt_then_a_command_then_its_end() {
        let mut c = Vec::new();
        fold(&mut c, Mark::Prompt, 0, Some("/a/b"));
        fold(&mut c, Mark::Start(Some("ls".into())), 100, None);
        assert_eq!(c.last().and_then(|x| x.command.as_deref()), Some("ls"));
        assert!(c[0].running());
        fold(
            &mut c,
            Mark::End {
                exit: Some(1),
                duration_ms: None,
            },
            350,
            None,
        );
        assert!(!c[0].running());
        assert_eq!((c[0].exit, c[0].duration_ms), (Some(1), Some(250)));
        assert_eq!(c[0].cwd.as_deref(), Some("/a/b"));
        // Enter on an empty line: a prompt that never starts.
        fold(&mut c, Mark::Prompt, 400, Some("/a/b"));
        fold(&mut c, Mark::Prompt, 500, Some("/a/b"));
        assert_eq!(c.len(), 3);
        assert!(!c[1].started);
    }

    #[test]
    fn a_second_start_and_an_end_without_a_start() {
        let mut c = Vec::new();
        fold(&mut c, Mark::Prompt, 0, None);
        fold(&mut c, Mark::Start(Some("make".into())), 10, Some("/p"));
        fold(&mut c, Mark::Start(Some("make again".into())), 20, None);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].command.as_deref(), Some("make"));
        assert_eq!(c[0].cwd.as_deref(), Some("/p"));
        fold(&mut c, Mark::Prompt, 30, None);
        fold(
            &mut c,
            Mark::End {
                exit: None,
                duration_ms: None,
            },
            90,
            None,
        );
        assert_eq!((c[1].started, c[1].exit, c[1].duration_ms), (true, Some(0), Some(0)));
        // A start with no prompt before it still makes a command of its own.
        fold(&mut c, Mark::Start(Some("ls".into())), 100, None);
        assert_eq!(c.len(), 3);
    }

    #[test]
    fn only_the_newest_prompts_are_kept() {
        let mut c = Vec::new();
        for i in 0..(MAX_COMMANDS + 10) {
            fold(&mut c, Mark::Prompt, i as u64, None);
        }
        assert_eq!(c.len(), MAX_COMMANDS);
    }
}
