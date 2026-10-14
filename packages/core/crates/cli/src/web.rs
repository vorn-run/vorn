//! `vorn web`: the running server's web client, opened in the browser or,
//! with `--print`, printed. It never starts a server: the address of one
//! started for it would be gone with the command.

use crate::exit::ExitCode;
use crate::output::Io;
use crate::rpc::{read_port, DataDir, PortFile};

const USAGE: &str = "Usage: vorn web [--print] [--data-dir <path>]\n";

/// The web client at `port` on this machine.
pub fn url(port: u16) -> String {
    format!("http://127.0.0.1:{port}/app/")
}

/// Where the server for `dir` serves its web client, or why there is none.
pub fn find(dir: &DataDir) -> Result<String, String> {
    match read_port(dir) {
        PortFile::Port(p) => u16::try_from(p as i64)
            .ok()
            .filter(|p| *p > 0)
            .map(url)
            .ok_or_else(|| format!("{} names no port", dir.port_file().display())),
        PortFile::Missing => {
            Err("No Vorn server is running. Open Vorn, or run `vorn server serve`.".to_owned())
        }
        PortFile::Invalid => Err(format!(
            "{} could not be read. Open Vorn to start its server again.",
            dir.port_file().display()
        )),
    }
}

/// `vorn web [--print] [--data-dir <path>]`, `argv` without the command.
pub fn run(argv: &[String], io: &mut dyn Io) -> ExitCode {
    let mut print = false;
    let mut data_dir: Option<String> = None;
    let mut args = argv.iter();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--print" => print = true,
            "--data-dir" => data_dir = args.next().cloned(),
            "--help" | "-h" => {
                io.write(USAGE);
                return ExitCode::Ok;
            }
            other => match other.strip_prefix("--data-dir=") {
                Some(d) => data_dir = Some(d.to_owned()),
                None => {
                    io.write_err(&format!("vorn web: unknown option \"{other}\"\n{USAGE}"));
                    return ExitCode::Usage;
                }
            },
        }
    }
    let dir = DataDir::resolve(data_dir.as_deref());
    let url = match find(&dir) {
        Ok(url) => url,
        Err(why) => {
            io.write_err(&format!("vorn: {why}\n"));
            return ExitCode::Unreachable;
        }
    };
    io.write(&format!("{url}\n"));
    if print {
        return ExitCode::Ok;
    }
    match open(&url) {
        Ok(()) => ExitCode::Ok,
        Err(e) => {
            io.write_err(&format!(
                "vorn: could not open a browser ({e}); the address is above\n"
            ));
            ExitCode::Failure
        }
    }
}

/// Hands `url` to the system's default browser.
fn open(url: &str) -> std::io::Result<()> {
    let mut command = if cfg!(target_os = "macos") {
        vorn_spawn::command("open")
    } else if cfg!(windows) {
        let mut c = vorn_spawn::command("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    } else {
        vorn_spawn::command("xdg-open")
    };
    let status = command.arg(url).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "the opener exited with {status}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::Captured;

    fn dir(path: &std::path::Path) -> Vec<String> {
        vec![
            "--print".into(),
            "--data-dir".into(),
            path.display().to_string(),
        ]
    }

    #[test]
    fn prints_the_web_client_of_the_server_in_the_data_directory() {
        let d = tempfile::tempdir().unwrap();
        let pid = std::process::id();
        std::fs::write(
            d.path().join("ws-port"),
            format!("{{\"port\": 51324, \"pid\": {pid}}}"),
        )
        .unwrap();
        let mut io = Captured::default();
        assert_eq!(run(&dir(d.path()), &mut io), ExitCode::Ok);
        assert_eq!(io.out, "http://127.0.0.1:51324/app/\n");
    }

    #[test]
    fn says_when_no_server_is_running() {
        let d = tempfile::tempdir().unwrap();
        let mut io = Captured::default();
        assert_eq!(run(&dir(d.path()), &mut io), ExitCode::Unreachable);
        assert!(io.err.contains("No Vorn server is running"), "{}", io.err);
        assert!(io.out.is_empty());
    }

    #[test]
    fn refuses_what_it_does_not_take() {
        let mut io = Captured::default();
        assert_eq!(run(&["--port".into()], &mut io), ExitCode::Usage);
        assert_eq!(url(3), "http://127.0.0.1:3/app/");
    }
}
