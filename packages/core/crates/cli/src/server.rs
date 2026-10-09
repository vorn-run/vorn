//! `vorn server serve|token`, and the bare `serve` and `token` that
//! `vorn-server` was always called with.
//!
//! The token commands work on the database file directly, through
//! `vorn-store`, and need no running server: a device token is a row whose
//! secret is kept only as its SHA-256, and whose plaintext,
//! `vorn_<id>_<base64url secret>`, is printed once and never again.
//!
//! `serve` runs the Node server, which is still the one that serves.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use vorn_store::{Store, StoreOptions};

use crate::args::ServerArgs;
use crate::exit::ExitCode;
use crate::output::Io;

pub const SERVER_USAGE: &str = "vorn server: run a Vorn server without the desktop app

Usage
  vorn server serve [options]              Start the server
  vorn server token create --name <name>   Mint a device token
  vorn server token list                   List device tokens
  vorn server token revoke <id>            Revoke a device token

Options
  --port <port>       Port to listen on (default: chosen by the OS)
  --data-dir <path>   Where the database lives (default ~/.vorn)
  -h, --help          Show this message

A server sharing ~/.vorn with a desktop app on the same machine shares one
database. Pass --data-dir to keep them apart.
";

const TOKEN_PREFIX: &str = "vorn";
const SECRET_BYTES: usize = 32;

/// Why a token command could not touch the database.
#[derive(Debug)]
pub enum TokenError {
    Store(vorn_store::Error),
    Io(std::io::Error),
    Random(getrandom::Error),
    /// The database has no owner to mint for.
    NoOwner,
    /// A debug build kept off the default data directory.
    Refused(String),
}

impl std::fmt::Display for TokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TokenError::Store(err) => write!(f, "{err}"),
            TokenError::Io(err) => write!(f, "{err}"),
            TokenError::Random(err) => write!(f, "no randomness for a secret: {err}"),
            TokenError::NoOwner => {
                f.write_str("No owner user found. The database may not have been migrated.")
            }
            TokenError::Refused(why) => f.write_str(why),
        }
    }
}

impl std::error::Error for TokenError {}

impl From<vorn_store::Error> for TokenError {
    fn from(err: vorn_store::Error) -> Self {
        TokenError::Store(err)
    }
}

/// The database a token command works on: `--data-dir`, else `~/.vorn`.
///
/// `VORN_DATA_DIR` is deliberately not read: it says where a *running*
/// server is, and the TypeScript token commands never consulted it either.
fn database_dir(flag: Option<&str>) -> PathBuf {
    flag.map(PathBuf::from)
        .unwrap_or_else(|| crate::rpc::home_dir().join(".vorn"))
}

/// What the store needs to create or migrate the file. The seeded workflows
/// are left to the server, which adds any that are missing when it opens it.
fn store_options() -> StoreOptions {
    let owner = std::env::var(if cfg!(windows) { "USERNAME" } else { "USER" })
        .ok()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "owner".into());
    StoreOptions {
        default_shell: String::new(),
        default_agent_commands: serde_json::Map::new(),
        default_workspace: serde_json::from_value(serde_json::json!({
            "id": "personal",
            "name": "Personal",
            "icon": "User",
            "iconColor": "#6b7280",
            "order": 0
        }))
        .expect("the default workspace is a valid WorkspaceConfig"),
        owner_name: owner,
        seed_workflows: Vec::new(),
    }
}

/// Opens (creating and migrating if need be) the database in `dir`.
fn open(dir: &Path) -> Result<Store, TokenError> {
    crate::launch::refuse_default(dir).map_err(TokenError::Refused)?;
    if !dir.exists() {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder.create(dir).map_err(TokenError::Io)?;
    }
    let (store, _) = Store::open(&dir.join("vorn.db"), store_options())?;
    Ok(store)
}

/// A device token, minted: its row and the only copy of its plaintext.
#[derive(Debug)]
pub struct Minted {
    pub id: String,
    pub name: String,
    pub plaintext: String,
}

/// Mints a token for the database's owner. Only the secret's hash reaches the
/// database.
pub fn mint_owner_token(store: &Store, name: &str) -> Result<Minted, TokenError> {
    let owner = store.db_get_owner_user()?.ok_or(TokenError::NoOwner)?;
    let id = uuid::Uuid::new_v4().to_string();
    let mut secret = [0u8; SECRET_BYTES];
    getrandom::fill(&mut secret).map_err(TokenError::Random)?;
    let secret = data_encoding::BASE64URL_NOPAD.encode(&secret);
    let hash = hex(&Sha256::digest(secret.as_bytes()));

    let token: vorn_protocol::NewDeviceToken = serde_json::from_value(serde_json::json!({
        "id": id,
        "userId": owner.id,
        "name": name,
        "createdAt": crate::time::now_iso(),
        "tokenHash": hash,
    }))
    .map_err(|err| TokenError::Store(vorn_store::Error::Json(err)))?;
    store.db_insert_device_token(&token)?;
    Ok(Minted {
        plaintext: format!("{TOKEN_PREFIX}_{id}_{secret}"),
        id,
        name: name.to_owned(),
    })
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
}

/// The one place a token's plaintext is ever printed.
fn print_minted(io: &mut dyn Io, lead: &str, plaintext: &str) {
    io.write(&format!(
        "{lead}\n\n  {plaintext}\n\nThis is the only time it is shown. Store it now.\n"
    ));
}

fn token_command(args: &ServerArgs, io: &mut dyn Io) -> Result<ExitCode, TokenError> {
    let sub = args.positionals.get(1).map(String::as_str);
    let dir = database_dir(args.data_dir.as_deref());
    match sub {
        Some("create") => {
            let Some(name) = args.name.as_deref().filter(|n| !n.is_empty()) else {
                io.write_err("vorn: token create requires --name <name>\n");
                return Ok(ExitCode::Usage);
            };
            let minted = mint_owner_token(&open(&dir)?, name)?;
            print_minted(
                io,
                &format!("Created token \"{}\" ({})", minted.name, minted.id),
                &minted.plaintext,
            );
            Ok(ExitCode::Ok)
        }
        Some("list") => {
            let tokens = open(&dir)?.db_list_device_tokens()?;
            if tokens.is_empty() {
                io.write("No device tokens.\n");
                return Ok(ExitCode::Ok);
            }
            for t in tokens {
                let state = if t.revoked_at.as_deref().is_some_and(|r| !r.is_empty()) {
                    "revoked"
                } else {
                    "active"
                };
                let seen = t.last_seen_at.as_deref().unwrap_or("never");
                io.write(&format!(
                    "{}  {state:<7}  last seen {seen}  {}\n",
                    t.id, t.name
                ));
            }
            Ok(ExitCode::Ok)
        }
        Some("revoke") => {
            let Some(id) = args.positionals.get(2).filter(|id| !id.is_empty()) else {
                io.write_err("vorn: token revoke requires a token id\n");
                return Ok(ExitCode::Usage);
            };
            if !open(&dir)?.db_revoke_device_token(id, &crate::time::now_iso())? {
                io.write_err(&format!("vorn: no active token with id {id}\n"));
                return Ok(ExitCode::Failure);
            }
            io.write(&format!("Revoked {id}\n"));
            Ok(ExitCode::Ok)
        }
        other => {
            io.write_err(&format!(
                "vorn: unknown token command \"{}\". Try: create, list, revoke\n",
                other.unwrap_or("")
            ));
            Ok(ExitCode::Usage)
        }
    }
}

/// Runs vornd as the server for the data directory, in the foreground, and
/// passes its exit code through. A data directory with no device token gets
/// one first, shown on a terminal only: a log is no place for a secret.
fn serve(args: &ServerArgs, io: &mut dyn Io) -> ExitCode {
    let dir = database_dir(args.data_dir.as_deref());
    if let Err(why) = crate::launch::refuse_default(&dir) {
        io.write_err(&format!("vorn: {why}\n"));
        return ExitCode::Failure;
    }
    let server = match crate::launch::locate() {
        Ok(server) => server,
        Err(err) => {
            io.write_err(&format!("vorn: {err}\n"));
            return ExitCode::Failure;
        }
    };
    match open(&dir).and_then(|store| Ok((store.db_has_device_tokens()?, store))) {
        Ok((false, store)) if io.is_tty() => match mint_owner_token(&store, "first-run") {
            Ok(minted) => {
                print_minted(
                    io,
                    "\nNo device tokens existed, so one was created for this server:",
                    &minted.plaintext,
                );
                io.write("Manage tokens with: vorn server token list\n\n");
            }
            Err(err) => io.write_err(&format!("vorn: could not mint a token: {err}\n")),
        },
        Ok((false, _)) => io.write(
            "\nNo device tokens exist. Mint one with: vorn server token create --name <name>\n",
        ),
        Ok((true, _)) => {}
        Err(err) => {
            io.write_err(&format!("vorn: {err}\n"));
            return ExitCode::Failure;
        }
    }
    io.write(&format!("Starting the Vorn server for {}\n", dir.display()));
    let port = args
        .port
        .filter(|p| p.fract() == 0.0 && (0.0..=65535.0).contains(p))
        .map(|p| p as u16);
    let mut command = server.command(&dir, port, args.host.as_deref());

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Becomes the server, so signals and the exit code are its own.
        let err = command.exec();
        io.write_err(&format!("vorn: could not start the server: {err}\n"));
        ExitCode::Failure
    }
    #[cfg(not(unix))]
    match command.status() {
        Ok(status) => ExitCode::Passed(status.code().unwrap_or(1)),
        Err(err) => {
            io.write_err(&format!("vorn: could not start the server: {err}\n"));
            ExitCode::Failure
        }
    }
}

/// `vorn server ...`, with the command line after `server` (or the whole
/// line, for the bare `serve` and `token`).
pub fn run(argv: &[String], io: &mut dyn Io) -> ExitCode {
    let args = match ServerArgs::parse(argv) {
        Ok(args) => args,
        Err(err) => {
            io.write_err(&format!("vorn: {err}\n"));
            return ExitCode::Usage;
        }
    };

    let command = args.positionals.first().map(String::as_str);
    if args.help || command == Some("help") {
        io.write(SERVER_USAGE);
        return ExitCode::Ok;
    }
    match command.filter(|c| !c.is_empty()) {
        None => {
            io.write_err(SERVER_USAGE);
            ExitCode::Usage
        }
        Some("serve") => serve(&args, io),
        Some("token") => match token_command(&args, io) {
            Ok(code) => code,
            Err(err) => {
                io.write_err(&format!("vorn: {err}\n"));
                ExitCode::Failure
            }
        },
        Some(other) => {
            io.write_err(&format!(
                "vorn: unknown server command \"{other}\". Try: serve, token, help\n"
            ));
            ExitCode::Usage
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::Captured;

    fn argv(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn mints_lists_and_revokes_a_token() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("data");
        let dir_arg = dir.to_string_lossy().into_owned();

        let mut io = Captured::default();
        let code = run(
            &argv(&[
                "token",
                "create",
                "--name",
                "iPhone",
                "--data-dir",
                &dir_arg,
            ]),
            &mut io,
        );
        assert_eq!(code, ExitCode::Ok, "{}", io.err);
        let plaintext = io
            .out
            .lines()
            .find_map(|l| l.trim().strip_prefix("vorn_"))
            .expect("the plaintext is printed")
            .to_owned();
        let (id, secret) = plaintext.split_once('_').unwrap();
        assert_eq!(id.len(), 36);
        assert_eq!(secret.len(), 43);

        // Only the hash is kept, and it is the hash of the secret printed.
        let store = open(&dir).unwrap();
        let row = store.db_get_device_token_secret(id).unwrap().unwrap();
        assert_eq!(row.token_hash, hex(&Sha256::digest(secret.as_bytes())));
        drop(store);

        let mut io = Captured::default();
        run(&argv(&["token", "list", "--data-dir", &dir_arg]), &mut io);
        assert_eq!(io.out, format!("{id}  active   last seen never  iPhone\n"));

        let mut io = Captured::default();
        assert_eq!(
            run(
                &argv(&["token", "revoke", id, "--data-dir", &dir_arg]),
                &mut io
            ),
            ExitCode::Ok
        );
        let mut io = Captured::default();
        assert_eq!(
            run(
                &argv(&["token", "revoke", id, "--data-dir", &dir_arg]),
                &mut io
            ),
            ExitCode::Failure
        );
        assert_eq!(io.err, format!("vorn: no active token with id {id}\n"));

        let mut io = Captured::default();
        run(&argv(&["token", "list", "--data-dir", &dir_arg]), &mut io);
        assert!(io.out.contains("  revoked  last seen never  iPhone"));
    }

    #[test]
    fn hashes_as_sha256_hex() {
        assert_eq!(
            hex(&Sha256::digest(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
