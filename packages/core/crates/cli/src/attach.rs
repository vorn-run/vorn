//! `vorn attach [session]` and `vorn` alone in a terminal: Vorn's screens in
//! this terminal, against the running server. Detaching leaves every session
//! running in vornd.

use std::path::PathBuf;

use serde_json::Value;

use crate::args::ClientArgs;
use crate::client::{CommandError, Context};
use crate::exit::ExitCode;
use crate::js::field;
use crate::output::Io;

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

struct Listed {
    id: String,
    name: String,
    created: f64,
    terminal: bool,
}

fn listed(v: &Value, terminal: bool) -> Listed {
    let s = |k| field(v, k).and_then(Value::as_str).unwrap_or("").to_owned();
    Listed {
        id: s("id"),
        name: s("displayName"),
        created: field(v, "createdAt").and_then(Value::as_f64).unwrap_or(0.0),
        terminal,
    }
}

/// The session `given` names in `vorn session list`'s order: a whole id, a
/// unique prefix, a 1-based number or a display name. None picks the newest.
fn pick(sessions: &[Listed], given: Option<&str>) -> Result<String, String> {
    let Some(given) = given.filter(|g| !g.is_empty()) else {
        return sessions
            .iter()
            .filter(|s| s.terminal)
            .max_by(|a, b| a.created.total_cmp(&b.created))
            .map(|s| s.id.clone())
            .ok_or_else(|| "no sessions running. Start one with: vorn session start".into());
    };
    let found = if let Some(s) = sessions.iter().find(|s| s.id == given) {
        s
    } else if let Some(s) = given
        .parse::<usize>()
        .ok()
        .filter(|n| given.len() <= 3 && *n >= 1)
        .and_then(|n| sessions.get(n - 1))
    {
        s
    } else {
        let by_id: Vec<&Listed> = sessions
            .iter()
            .filter(|s| s.id.starts_with(given))
            .collect();
        let by_name: Vec<&Listed> = sessions
            .iter()
            .filter(|s| s.name.eq_ignore_ascii_case(given))
            .collect();
        match (by_id.as_slice(), by_name.as_slice()) {
            ([one], _) | ([], [one]) => *one,
            ([], []) => return Err(format!("no session matches \"{given}\"")),
            (many, _) if many.len() > 1 => {
                return Err(format!("\"{given}\" matches {} sessions", many.len()))
            }
            (_, many) => return Err(format!("\"{given}\" names {} sessions", many.len())),
        }
    };
    if !found.terminal {
        return Err(format!(
            "{} is a headless session, which has no terminal to attach to",
            crate::output::short_id(&found.id)
        ));
    }
    Ok(found.id.clone())
}

async fn sessions(ctx: &Context<'_>) -> Result<Vec<Listed>, CommandError> {
    let (terminals, headless) = ctx
        .call_both("terminal:listActive", "headless:list")
        .await
        .map_err(CommandError::Call)?;
    let items = |v: Value| match v {
        Value::Array(v) => v,
        _ => Vec::new(),
    };
    Ok(items(terminals)
        .iter()
        .map(|s| listed(s, true))
        .chain(
            items(headless)
                .iter()
                .filter(|s| field(s, "status").and_then(Value::as_str) == Some("running"))
                .map(|s| listed(s, false)),
        )
        .collect())
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
    let listed = match sessions(&ctx).await {
        Ok(listed) => listed,
        Err(err) => return Outcome::Exit(ctx.failed("could not list sessions", err)),
    };
    match pick(&listed, given.as_deref()) {
        Ok(id) => Outcome::Tui(Tui {
            data_dir,
            focus: Some(id),
        }),
        Err(message) => {
            ctx.io.write_err(&format!("vorn: {message}\n"));
            Outcome::Exit(ExitCode::Failure)
        }
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

    fn s(id: &str, name: &str, created: f64, terminal: bool) -> Listed {
        Listed {
            id: id.into(),
            name: name.into(),
            created,
            terminal,
        }
    }

    #[test]
    fn picks_by_id_prefix_number_name_or_newest() {
        let all = [
            s("aaaa1111", "build", 10.0, true),
            s("bbbb2222", "", 30.0, true),
            s("cccc3333", "", 50.0, false),
        ];
        assert_eq!(pick(&all, None).as_deref(), Ok("bbbb2222"));
        assert_eq!(pick(&all, Some("aaaa")).as_deref(), Ok("aaaa1111"));
        assert_eq!(pick(&all, Some("2")).as_deref(), Ok("bbbb2222"));
        assert_eq!(pick(&all, Some("Build")).as_deref(), Ok("aaaa1111"));
        assert!(pick(&all, Some("3")).unwrap_err().contains("headless"));
        assert!(pick(&all, Some("zz")).unwrap_err().contains("no session"));
        assert!(pick(&[], None).is_err());
    }
}
