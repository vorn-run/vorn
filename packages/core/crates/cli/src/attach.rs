//! `vorn attach [session]` and `vorn` alone in a terminal: Vorn's screens in
//! this terminal, against the running server. Detaching leaves every session
//! running in vornd.

use std::path::PathBuf;

use crate::args::ClientArgs;
use crate::client::{CommandError, Context};
use crate::exit::ExitCode;
use crate::output::Io;
use crate::target::{self, Candidate, Miss};

pub const ATTACH_USAGE: &str = "Usage
  vorn attach [<session>] [--data-dir <path>]

Opens Vorn in this terminal, full window on one session: its id (or the start
of one), its name, or its number in `vorn session list`. With no session, the
most recent one.

Keys
  Alt+B         Show or hide the sidebar
  Alt+1..9      Jump to a session
  Alt+[ Alt+]   Previous or next session
  Ctrl+\\        Detach; sessions keep running
";

/// What `vorn` opens in this terminal once the server is up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tui {
    /// The server's data directory.
    pub data_dir: PathBuf,
    /// The session to open full window; none opens the sessions screen.
    pub focus: Option<String>,
}

/// What a command line asks for: an exit code now, or the TUI.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Exit(ExitCode),
    Tui(Tui),
}

/// The session `given` names by the rules in [`crate::target`]; none picks the newest terminal.
fn pick(sessions: &[Candidate], given: Option<&str>) -> Result<String, CommandError> {
    let Some(given) = given.filter(|g| !g.is_empty()) else {
        return sessions
            .iter()
            .filter(|s| !s.headless)
            .max_by(|a, b| a.created.total_cmp(&b.created))
            .map(|s| s.id.clone())
            .ok_or_else(|| {
                CommandError::Refused(
                    "no sessions running. Start one with: vorn session start".into(),
                )
            });
    };
    let found = match target::resolve(given, sessions) {
        Ok(found) => found,
        Err(Miss::None) => {
            return Err(CommandError::Refused(format!(
                "no session matches \"{given}\""
            )))
        }
        Err(Miss::Ambiguous(found)) => {
            return Err(CommandError::Ambiguous {
                given: given.to_owned(),
                found,
            })
        }
    };
    if found.headless {
        return Err(CommandError::Refused(format!(
            "{} is a headless session, which has no terminal to attach to",
            crate::output::short_id(&found.id)
        )));
    }
    Ok(found.id.clone())
}

/// `vorn attach [session]`, or `vorn` alone when `bare`.
pub async fn run(argv: &[String], io: &mut dyn Io, bare: bool, tty: bool) -> Outcome {
    let args = match ClientArgs::parse(argv) {
        Ok(args) => args,
        Err(err) => {
            io.write_err(&format!("vorn: {err}\n"));
            return Outcome::Exit(ExitCode::Usage);
        }
    };
    if args.help {
        io.write(ATTACH_USAGE);
        return Outcome::Exit(ExitCode::Ok);
    }
    if !tty {
        io.write_err("vorn: attach needs a terminal on stdin and stdout\n");
        return Outcome::Exit(ExitCode::Usage);
    }
    let given = args.positionals.get(1).cloned();
    let mut ctx = Context::new(io, args);
    if !ctx.server().await {
        return Outcome::Exit(ExitCode::Unreachable);
    }
    let data_dir = ctx
        .args
        .data_dir
        .as_deref()
        .map_or_else(|| ctx.rpc.data_dir().path().to_owned(), PathBuf::from);
    if bare {
        return Outcome::Tui(Tui {
            data_dir,
            focus: None,
        });
    }
    let listed = match crate::session::candidates(&ctx).await {
        Ok(listed) => listed,
        Err(err) => return Outcome::Exit(ctx.failed("could not list sessions", err)),
    };
    match pick(&listed, given.as_deref()) {
        Ok(id) => Outcome::Tui(Tui {
            data_dir,
            focus: Some(id),
        }),
        Err(CommandError::Refused(message)) => {
            ctx.io.write_err(&format!("vorn: {message}\n"));
            Outcome::Exit(ExitCode::Failure)
        }
        Err(err) => Outcome::Exit(ctx.error("could not attach", err)),
    }
}

/// Runs the TUI until it is detached.
pub fn open(tui: &Tui) -> ExitCode {
    let opts = vorn_app::tui::Options {
        focus: tui.focus.clone(),
    };
    match vorn_app::tui::run(&tui.data_dir, &opts) {
        Ok(()) => ExitCode::Ok,
        Err(err) => {
            eprintln!("vorn: {err}");
            ExitCode::Failure
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(id: &str, name: &str, created: f64, headless: bool) -> Candidate {
        Candidate {
            id: id.into(),
            label: name.into(),
            agent: String::new(),
            project: String::new(),
            headless,
            created,
        }
    }

    #[test]
    fn picks_by_the_shared_rules_or_the_newest() {
        let all = [
            s("aaaa1111", "build", 10.0, false),
            s("bbbb2222", "", 30.0, false),
            s("cccc3333", "", 50.0, true),
        ];
        assert_eq!(pick(&all, None).unwrap(), "bbbb2222");
        assert_eq!(pick(&all, Some("aaaa")).unwrap(), "aaaa1111");
        assert_eq!(pick(&all, Some("2")).unwrap(), "bbbb2222");
        assert_eq!(pick(&all, Some("Build")).unwrap(), "aaaa1111");
        assert!(pick(&all, Some("3"))
            .unwrap_err()
            .to_string()
            .contains("headless"));
        assert!(pick(&all, Some("zz"))
            .unwrap_err()
            .to_string()
            .contains("no session"));
        assert!(matches!(
            pick(&all, Some("a")),
            Ok(_) | Err(CommandError::Ambiguous { .. })
        ));
        assert!(pick(&[], None).is_err());
    }
}
