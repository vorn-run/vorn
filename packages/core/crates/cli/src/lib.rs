//! The `vorn` command.
//!
//! Two halves behind one name. `vorn server` works on this machine's server:
//! its device tokens, read and written in the database directly, and `serve`,
//! which runs the Node server. The rest talks to a server that is already
//! running, starting one first if there is none, and every one of those verbs
//! is an RPC the app has always had. `vorn mcp` relays an agent's MCP over
//! stdio to vornd.
//!
//! Commands, flags, help text, output, messages and exit codes are the
//! TypeScript command's (`packages/server/src/cli.ts`), so scripts written
//! against one work against the other; `tests/native-cli.test.ts` runs both
//! and compares them.
//!
//! [`run`] takes its argv and its output sinks as arguments and returns an
//! exit code instead of writing to the process and exiting, so every command
//! is assertable.

pub mod args;
pub mod client;
pub mod exit;
pub mod js;
pub mod launch;
pub mod mcp;
pub mod output;
pub mod paths;
pub mod rpc;
pub mod server;
pub mod session;
pub mod time;
pub mod workflow;

use args::{takes_value_anywhere, ClientArgs};
use exit::ExitCode;
use output::Io;

/// What `vorn --version` prints: the app's version, which this crate follows.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub const USAGE: &str = "vorn: Vorn from the command line

Usage
  vorn session start --agent <agent> [--prompt <text>]   Start an agent session
  vorn session list [--recent] [--json]                  What is running
  vorn session logs|send|kill <id>                       Read, steer, stop one
  vorn workflow list [--json]                            Workflows
  vorn workflow run <name> [--input k=v]                 Start one
  vorn workflow stop <run>                               Stop a run
  vorn workflow runs [--workflow <name>] [--json]        Their runs
  vorn server serve|token                                Run a server, or its tokens
  vorn mcp                                               Vorn's MCP server, over stdio
  vorn --help | --version

Options common to the commands that talk to a server
  --json              Machine-readable output on stdout
  --data-dir <path>   Reach a server started with the same flag
  --timeout <ms>      Give up on a call after this long

Running `vorn` with no command opens the app. With no server running, a command
starts one and says so.
";

/// The command, wherever it sits.
///
/// `vorn --data-dir /tmp session list` is how a person writes this; skipping
/// an option's value is what makes the directory not look like a noun.
pub fn find_command(argv: &[String]) -> Option<&str> {
    let mut i = 0;
    while i < argv.len() {
        let token = &argv[i];
        if !token.starts_with('-') {
            return Some(token);
        }
        if token.starts_with("--") && !token.contains('=') && takes_value_anywhere(&token[2..]) {
            i += 1;
        }
        i += 1;
    }
    None
}

const COMMANDS: [&str; 7] = [
    "session", "workflow", "server", "serve", "token", "help", "mcp",
];

/// An option that swallowed the command because its own value was missing:
/// `vorn --data-dir session list`.
fn option_ate_the_command(argv: &[String]) -> Option<(&str, &str)> {
    argv.windows(2).find_map(|pair| {
        let (option, value) = (&pair[0], &pair[1]);
        let eats = option.starts_with("--")
            && !option.contains('=')
            && takes_value_anywhere(&option[2..])
            && COMMANDS.contains(&value.as_str());
        eats.then_some((option.as_str(), value.as_str()))
    })
}

/// The same argv with the command itself taken out, options left where they were.
fn without_command(argv: &[String], command: &str) -> Vec<String> {
    let mut rest = argv.to_vec();
    if let Some(at) = rest.iter().position(|a| a == command) {
        rest.remove(at);
    }
    rest
}

fn eaten(io: &mut dyn Io, argv: &[String]) -> Option<ExitCode> {
    let (option, value) = option_ate_the_command(argv)?;
    io.write_err(&format!(
        "vorn: {option} needs a value; it took \"{value}\" as one\n"
    ));
    Some(ExitCode::Usage)
}

/// The commands that need a server: one grammar, one context, then the noun.
async fn client_command(argv: &[String], io: &mut dyn Io, mcp: bool) -> ExitCode {
    let args = match ClientArgs::parse(argv) {
        Ok(args) => args,
        Err(err) => {
            io.write_err(&format!("vorn: {err}\n"));
            return ExitCode::Usage;
        }
    };
    if mcp {
        return mcp::run(&args, io).await;
    }
    let noun = args.positionals.first().cloned();
    let mut ctx = client::Context::new(io, args);
    if noun.as_deref() == Some("session") {
        session::run(&mut ctx).await
    } else {
        workflow::run(&mut ctx).await
    }
}

/// Runs one command and returns the process's exit code.
pub async fn run(argv: &[String], io: &mut dyn Io) -> ExitCode {
    let Some(command) = find_command(argv) else {
        if argv.iter().any(|a| a == "--version") {
            io.write(&format!("{VERSION}\n"));
            return ExitCode::Ok;
        }
        // Asking for help is a successful invocation; being run with nothing is not.
        if argv.iter().any(|a| a == "--help" || a == "-h") {
            io.write(USAGE);
            return ExitCode::Ok;
        }
        if let Some(code) = eaten(io, argv) {
            return code;
        }
        io.write_err(USAGE);
        return ExitCode::Usage;
    };

    match command {
        "help" => {
            io.write(USAGE);
            ExitCode::Ok
        }
        "server" => server::run(&without_command(argv, command), io),
        // `vorn-server` was called this way for as long as it existed.
        "serve" | "token" => server::run(argv, io),
        "session" | "workflow" => client_command(argv, io, false).await,
        "mcp" => client_command(&without_command(argv, command), io, true).await,
        _ => {
            if let Some(code) = eaten(io, argv) {
                return code;
            }
            io.write_err(&format!(
                "vorn: unknown command \"{command}\". Try: session, workflow, server, help\n"
            ));
            ExitCode::Usage
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use output::Captured;

    fn argv(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn run_sync(items: &[&str]) -> (i32, Captured) {
        let mut io = Captured::default();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let code = rt.block_on(run(&argv(items), &mut io)).code();
        (code, io)
    }

    #[test]
    fn finds_the_command_after_options_and_their_values() {
        assert_eq!(
            find_command(&argv(&["--data-dir", "/tmp", "session", "list"])),
            Some("session")
        );
        assert_eq!(
            find_command(&argv(&["--data-dir=/tmp", "token"])),
            Some("token")
        );
        assert_eq!(find_command(&argv(&["--json", "-h"])), None);
    }

    #[test]
    fn reports_usage_and_dispatch_errors_as_the_typescript_does() {
        let (code, io) = run_sync(&[]);
        assert_eq!((code, io.out.as_str()), (2, ""));
        assert_eq!(io.err, USAGE);

        let (code, io) = run_sync(&["--help"]);
        assert_eq!((code, io.out.as_str()), (0, USAGE));

        let (code, io) = run_sync(&["--version"]);
        assert_eq!((code, io.out), (0, format!("{VERSION}\n")));

        let (code, io) = run_sync(&["bogus"]);
        assert_eq!(code, 2);
        assert_eq!(
            io.err,
            "vorn: unknown command \"bogus\". Try: session, workflow, server, help\n"
        );

        let (code, io) = run_sync(&["--data-dir", "session", "list"]);
        assert_eq!(code, 2);
        assert_eq!(
            io.err,
            "vorn: --data-dir needs a value; it took \"session\" as one\n"
        );

        let (code, io) = run_sync(&["session", "--nope"]);
        assert_eq!(code, 2);
        assert!(io.err.starts_with("vorn: Unknown option '--nope'."));

        let (code, io) = run_sync(&["session", "dance"]);
        assert_eq!(code, 2);
        assert!(io
            .err
            .starts_with("vorn: unknown session command \"dance\"\n\nUsage"));

        let (code, io) = run_sync(&["server"]);
        assert_eq!((code, io.err.as_str()), (2, server::SERVER_USAGE));

        let (code, io) = run_sync(&["serve", "--port=abc"]);
        assert_eq!(code, 2);
        assert_eq!(io.err, "vorn: --port must be a number, got \"abc\"\n");

        let (code, io) = run_sync(&["token", "create"]);
        assert_eq!(
            (code, io.err.as_str()),
            (2, "vorn: token create requires --name <name>\n")
        );
    }
}
