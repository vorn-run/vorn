//! The sandbox a program extension starts in, on by default where the
//! machine has one: Seatbelt (`sandbox-exec`) on macOS, Landlock on Linux.
//!
//! A sandboxed program reads its own folder (its manifest and code, never
//! written), reads and writes its data folder ([`DATA_ENV`]) and the
//! projects it is handed, reads the runtime it was started with, and reads nothing else
//! of the person's home or Vorn's data. On macOS it reaches the network only
//! through the ports it is given: the proxy that lets it through to the hosts
//! its manifest declares ([`crate::netproxy`]) and vornd's own. Landlock
//! holds files, not every kind of socket, so on Linux its network is not
//! limited, and Vorn says so. Everything it asks Vorn for still goes through
//! the gate.
//!
//! A person can turn it off per extension; the switch is a file beside the
//! package's pointer, so removing the extension removes it too. Where the
//! machine has no sandbox (Windows, a Linux kernel without Landlock) programs
//! run unsandboxed and say so.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use crate::child::Launch;
use crate::netproxy::{NetProxy, ProxyGrant};

#[cfg(target_os = "linux")]
pub mod landlock;

/// The variable naming the folder an extension's program writes its own
/// files to, outside the folder its code is in.
pub const DATA_ENV: &str = "VORN_EXTENSION_DATA";

/// Where the switch is kept, beside `current.json`.
pub const SETTINGS_FILE: &str = "sandbox.json";

/// The program that applies a profile on macOS.
pub const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

/// The flag that makes vornd confine itself and run a program (Linux):
/// `vornd --vorn-confine <rules> -- <program> <args>...`.
pub const CONFINE_FLAG: &str = "--vorn-confine";

/// The sandbox this machine offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Seatbelt,
    Landlock,
    /// None: programs run unsandboxed.
    None,
}

impl Kind {
    /// What this machine has, asked of the OS once a call.
    pub fn here() -> Kind {
        if cfg!(target_os = "macos") && Path::new(SANDBOX_EXEC).is_file() {
            return Kind::Seatbelt;
        }
        #[cfg(target_os = "linux")]
        if landlock::abi().is_some() {
            return Kind::Landlock;
        }
        Kind::None
    }

    /// The word a person reads for it.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Seatbelt => "sandboxed",
            Kind::Landlock => "files-only",
            Kind::None => "unsandboxed",
        }
    }

    /// Whether it holds a program to the network it is given, every socket
    /// kind and host included; Landlock holds files and TCP ports only.
    pub fn limits_network(self) -> bool {
        self == Kind::Seatbelt
    }
}

/// Whether programs can be confined here: the OS has a sandbox and, where
/// confining takes a helper (Linux), there is one (`confine`).
pub fn available(confine: Option<&Path>) -> bool {
    match Kind::here() {
        Kind::Seatbelt => true,
        Kind::Landlock => confine.is_some(),
        Kind::None => false,
    }
}

/// Whether the extension kept at `home` (`<root>/<id>`) runs sandboxed: yes
/// unless the person turned it off.
pub fn is_on(home: &Path) -> bool {
    fs::read_to_string(home.join(SETTINGS_FILE))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.get("on").and_then(serde_json::Value::as_bool))
        .unwrap_or(true)
}

/// Turns the sandbox on (the default) or off for the extension kept at `home`.
pub fn set(home: &Path, on: bool) -> Result<(), String> {
    if !home.is_dir() {
        return Err(format!("{} is not an installed extension", home.display()));
    }
    let path = home.join(SETTINGS_FILE);
    if on {
        return match fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(format!("cannot turn the sandbox on: {e}"))
            }
            _ => Ok(()),
        };
    }
    fs::write(&path, "{\"on\":false}\n").map_err(|e| format!("cannot turn the sandbox off: {e}"))
}

/// What a sandboxed program may reach.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reach {
    /// Its own folder, its manifest and code: read only.
    pub own: PathBuf,
    /// Its data folder: read and written.
    pub work: Option<PathBuf>,
    /// Projects it works in: read and written.
    pub write: Vec<PathBuf>,
    /// Folders it only reads, beyond the system's: its runtime.
    pub read: Vec<PathBuf>,
    /// Folders it may not read even where the system's are open (macOS):
    /// the person's home and Vorn's data. `own`, `write` and `read` stay open
    /// inside them.
    pub hidden: Vec<PathBuf>,
    /// The person's home, which a runtime folder never widens to.
    pub home: Option<PathBuf>,
    /// Vorn's data folder, where every extension's own folder is.
    pub data: Option<PathBuf>,
    /// Loopback ports it may connect to: the network proxy's and vornd's.
    pub ports: Vec<u16>,
}

impl Reach {
    /// The reach of a program kept in `own`: hidden from `home` and `data`.
    pub fn of(own: &Path, home: Option<&Path>, data: Option<&Path>) -> Reach {
        Reach {
            own: real(own),
            hidden: home.into_iter().chain(data).map(real).collect(),
            home: home.map(real),
            data: data.map(real),
            ..Reach::default()
        }
    }
}

/// `p` through its symlinks, as both sandboxes match real paths.
fn real(p: &Path) -> PathBuf {
    fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// The folder a runtime lives in, for a program at `program`: the prefix a
/// `bin/` sits in (`~/.nvm/versions/node/v22`), else the program's own
/// folder; never the home folder or anything above it.
pub fn runtime_root(program: &Path, home: Option<&Path>) -> Option<PathBuf> {
    let program = real(program);
    let parent = program.parent()?;
    let prefix = parent
        .file_name()
        .is_some_and(|n| n == "bin")
        .then(|| parent.parent())
        .flatten();
    let too_wide = |p: &Path| p.parent().is_none() || home.is_some_and(|h| h.starts_with(p));
    prefix
        .filter(|p| !too_wide(p))
        .or(Some(parent).filter(|p| !too_wide(p)))
        .map(Path::to_path_buf)
}

/// How this Vorn confines the programs it starts: made once by the host
/// (vornd) and shared by every launcher, so all of them sandbox alike.
#[derive(Debug, Clone, Default)]
pub struct Confiner {
    /// The helper Linux confines through: vornd itself.
    pub confine: Option<PathBuf>,
    /// The person's home, hidden from every program.
    pub home: Option<PathBuf>,
    /// Vorn's data folder, hidden as well.
    pub data: Option<PathBuf>,
    /// The way out to the hosts a program declares.
    pub proxy: Option<Arc<NetProxy>>,
    /// The projects programs may work in, kept current by the host.
    pub projects: Arc<RwLock<Vec<PathBuf>>>,
    /// One way through the proxy per extension, shared by all its programs.
    pub grants: Arc<std::sync::Mutex<std::collections::HashMap<String, ProxyGrant>>>,
}

impl Confiner {
    /// Whether programs can be confined here.
    pub fn available(&self) -> bool {
        available(self.confine.as_deref())
    }

    /// The projects programs may work in from now on.
    pub fn set_projects(&self, projects: Vec<PathBuf>) {
        let real: Vec<PathBuf> = projects.iter().map(|p| real(p)).collect();
        *self.projects.write().unwrap_or_else(|e| e.into_inner()) = real;
    }

    /// Where extension `id`'s data folder is: under Vorn's data, else the
    /// system's temporary folder (a test, the command line).
    fn work_path(&self, id: &str) -> PathBuf {
        let root = match &self.data {
            Some(data) => data.join("extension-data"),
            None => std::env::temp_dir().join("vorn-extension-data"),
        };
        root.join(id)
    }

    /// Extension `id`'s data folder, made if it is not there.
    pub fn work_dir(&self, id: &str) -> Option<PathBuf> {
        let dir = self.work_path(id);
        fs::create_dir_all(&dir).ok()?;
        Some(real(&dir))
    }

    /// Removes extension `id`'s data folder, as it is removed.
    pub fn forget_data(&self, id: &str) -> std::io::Result<()> {
        match fs::remove_dir_all(self.work_path(id)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    /// `launch` told where extension `id` writes its own files, whether or
    /// not it runs sandboxed.
    pub fn name_data(&self, id: &str, launch: &mut Launch) {
        if let Some(dir) = self.work_dir(id) {
            set_env(launch, DATA_ENV, &dir.to_string_lossy());
        }
    }

    /// The reach of extension `id`'s program kept in `own`; with
    /// `projects`, the projects too, but never one that holds Vorn's data or
    /// sits inside it ([`Confiner::closed_projects`]).
    pub fn reach(&self, own: &Path, id: &str, projects: bool) -> Reach {
        let mut reach = Reach::of(own, self.home.as_deref(), self.data.as_deref());
        reach.work = self.work_dir(id);
        if projects {
            let closed = self.closed_projects();
            reach.write = self
                .projects
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .filter(|p| !closed.contains(p))
                .cloned()
                .collect();
        }
        reach
    }

    /// The projects no sandboxed program is given, read or write: each one
    /// that holds Vorn's data (the home folder, say) or sits inside it.
    pub fn closed_projects(&self) -> Vec<PathBuf> {
        let Some(data) = self.data.as_deref().map(real) else {
            return Vec::new();
        };
        self.projects
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|p| inside(&data, p) || inside(p, &data))
            .cloned()
            .collect()
    }

    /// A way through the proxy for `owner` to `hosts`, put into `launch` and
    /// `reach`; held for as long as the program runs.
    pub fn grant(
        &self,
        owner: &str,
        hosts: Vec<String>,
        launch: &mut Launch,
        reach: &mut Reach,
    ) -> Option<ProxyGrant> {
        if hosts.is_empty() {
            return None;
        }
        let grant = match self.proxy.as_ref()?.grant(owner, hosts) {
            Ok(g) => g,
            Err(err) => {
                tracing::warn!("[extensions] {owner} runs without a network: {err}");
                return None;
            }
        };
        put(&grant, launch, reach);
        Some(grant)
    }

    /// The extension `owner`'s way through the proxy to `hosts`, put into
    /// `launch` and `reach`: made on its first program, then shared by every
    /// program it starts until [`Confiner::revoke`], so a new one never cuts
    /// off one still running.
    pub fn share(&self, owner: &str, hosts: Vec<String>, launch: &mut Launch, reach: &mut Reach) {
        if hosts.is_empty() {
            return;
        }
        let mut grants = self.grants.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(grant) = grants.get(owner) {
            put(grant, launch, reach);
            return;
        }
        if let Some(grant) = self.grant(owner, hosts, launch, reach) {
            grants.insert(owner.to_owned(), grant);
        }
    }

    /// Takes `owner`'s shared way through the proxy back, as it is removed
    /// or changed; whether it had one.
    pub fn revoke(&self, owner: &str) -> bool {
        self.grants
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(owner)
            .is_some()
    }

    /// `launch` confined to `reach`, or `None` where this machine cannot confine it.
    pub fn wrap(&self, launch: Launch, reach: &Reach) -> Option<Launch> {
        wrap(launch, reach, self.confine.as_deref())
    }
}

/// `launch`'s `key` set to `value`, replacing any it had.
pub fn set_env(launch: &mut Launch, key: &str, value: &str) {
    launch.env.retain(|(k, _)| k != key);
    launch.env.push((key.to_owned(), value.to_owned()));
}

/// `grant`'s variables into `launch`, and its port into `reach`.
fn put(grant: &ProxyGrant, launch: &mut Launch, reach: &mut Reach) {
    for (k, v) in grant.env() {
        launch.env.retain(|(key, _)| *key != k);
        launch.env.push((k, v));
    }
    if !reach.ports.contains(&grant.port()) {
        reach.ports.push(grant.port());
    }
}

/// Whether `program` may be run outside the sandbox to say where its runtime
/// is: only when nothing a sandboxed program can write could have put it
/// there (its own folder, its projects, Vorn's data).
fn may_ask_where(program: &Path, reach: &Reach) -> bool {
    // Through the nearest folder's links that is there, as a file may not be yet.
    let program = program
        .ancestors()
        .find_map(|a| {
            let rest = program.strip_prefix(a).ok()?;
            fs::canonicalize(a).ok().map(|r| r.join(rest))
        })
        .unwrap_or_else(|| program.to_path_buf());
    let writable = std::iter::once(&reach.own)
        .chain(&reach.work)
        .chain(&reach.write)
        .chain(reach.data.as_ref());
    !writable.into_iter().any(|w| inside(&program, w))
}

/// Whether `path` is `dir` or under it, ignoring case where the file system
/// does (macOS and Windows): `/Users/me/.VORN` is `/Users/me/.vorn` there.
fn inside(path: &Path, dir: &Path) -> bool {
    if !cfg!(any(target_os = "macos", windows)) {
        return path.starts_with(dir);
    }
    let lower = |p: &Path| PathBuf::from(p.to_string_lossy().to_lowercase());
    lower(path).starts_with(lower(dir))
}

/// The question each runtime answers with its own executable's path.
fn asks_where(program: &Path) -> Option<&'static [&'static str]> {
    let name = program.file_name()?.to_str()?;
    Some(match name {
        "node" | "nodejs" | "bun" => &["-p", "process.execPath"],
        "python" | "python3" => &["-c", "import sys; print(sys.executable)"],
        "deno" => &["eval", "console.log(Deno.execPath())"],
        _ => return None,
    })
}

/// Runtimes already asked where they are, by the program started.
static WHERE: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<PathBuf, PathBuf>>> =
    std::sync::LazyLock::new(Default::default);

/// The executable a runtime at `program` really is, when it is one Vorn
/// can ask (node, bun, deno, python): a shim (asdf, mise, volta) answers
/// with the version it would run. Asked once per program, outside the
/// sandbox, within a few seconds; `None` when it does not say.
pub fn real_runtime(program: &Path, env: &[(String, String)]) -> Option<PathBuf> {
    let question = asks_where(program)?;
    let mut known = WHERE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(real) = known.get(program) {
        return Some(real.clone());
    }
    // From home, not the extension's folder: a shim reads version files where it starts.
    let home = env
        .iter()
        .find(|(k, _)| k == "HOME")
        .map_or_else(|| PathBuf::from("/"), |(_, h)| PathBuf::from(h));
    let mut child = vorn_spawn::command(program)
        .args(question)
        .current_dir(if home.is_dir() {
            home
        } else {
            PathBuf::from("/")
        })
        .env_clear()
        // Only what a shim needs to find its version; never the extension's own variables.
        .envs(
            env.iter()
                .filter(|(k, _)| k == "PATH" || k == "HOME")
                .map(|(k, v)| (k, v)),
        )
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut out = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take()?, &mut out).ok()?;
    let real = PathBuf::from(out.trim());
    if !real.is_absolute() || !real.is_file() {
        return None;
    }
    known.insert(program.to_path_buf(), real.clone());
    Some(real)
}

/// SBPL string literal for `s`.
fn quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

fn subpaths(paths: &[&PathBuf]) -> String {
    paths
        .iter()
        .map(|p| format!("\n  (subpath {})", quoted(&p.to_string_lossy())))
        .collect()
}

/// The Seatbelt profile for `reach`. Later rules win, so each deny comes
/// before the allows carved out of it.
pub fn profile(reach: &Reach) -> String {
    let ports: String = reach
        .ports
        .iter()
        .map(|p| format!("(allow network-outbound (remote tcp \"localhost:{p}\"))\n"))
        .collect();
    let writable: Vec<&PathBuf> = reach.work.iter().chain(&reach.write).collect();
    let readable: Vec<&PathBuf> = std::iter::once(&reach.own)
        .chain(writable.iter().copied())
        .chain(&reach.read)
        .collect();
    let hidden: Vec<&PathBuf> = reach.hidden.iter().collect();
    let mut out = format!(
        "(version 1)
(allow default)
(deny network*)
{ports}(deny file-write*)
(allow file-write*{}
  (literal \"/dev/null\")
  (literal \"/dev/zero\")
  (literal \"/dev/dtracehelper\")
  (regex #\"^/dev/fd/\")
  (regex #\"^/dev/tty\"))
",
        subpaths(&writable)
    );
    if !hidden.is_empty() {
        out.push_str(&format!(
            "(deny file-read-data{})\n(allow file-read-data{})\n",
            subpaths(&hidden),
            subpaths(&readable)
        ));
    }
    out
}

/// `launch` confined to `reach`, or `None` where this machine cannot
/// confine it (see [`available`]); `confine` is the helper Linux needs.
pub fn wrap(launch: Launch, reach: &Reach, confine: Option<&Path>) -> Option<Launch> {
    let program = crate::child::on_path(&launch.program, &launch.env, &launch.cwd)
        .unwrap_or_else(|| launch.program.clone());
    // A version manager's shim reads its own files under home: start the runtime it names instead.
    let program = if may_ask_where(&program, reach) {
        real_runtime(&program, &launch.env).unwrap_or(program)
    } else {
        program
    };
    let mut reach = reach.clone();
    if let Some(root) = runtime_root(&program, reach.home.as_deref()) {
        if !reach.read.contains(&root) {
            reach.read.push(root);
        }
    }
    // Its own temporary files, inside the one folder it writes besides its projects.
    let mut launch = launch;
    if let Some(work) = &reach.work {
        set_env(&mut launch, DATA_ENV, &work.to_string_lossy());
    }
    let tmp = reach.work.as_ref().map(|w| w.join(".tmp"));
    if let Some(tmp) = tmp.filter(|t| fs::create_dir_all(t).is_ok()) {
        launch.env.retain(|(k, _)| k != "TMPDIR");
        launch
            .env
            .push(("TMPDIR".into(), tmp.to_string_lossy().into_owned()));
    }
    let mut args = Vec::with_capacity(launch.args.len() + 4);
    let helper = match Kind::here() {
        Kind::Seatbelt => {
            args.push("-p".to_owned());
            args.push(profile(&reach));
            PathBuf::from(SANDBOX_EXEC)
        }
        Kind::Landlock => {
            args.push(CONFINE_FLAG.to_owned());
            args.push(rules(&reach));
            args.push("--".to_owned());
            confine?.to_path_buf()
        }
        Kind::None => return None,
    };
    args.push(program.to_string_lossy().into_owned());
    args.extend(launch.args);
    Some(Launch {
        program: helper,
        args,
        ..launch
    })
}

/// `reach` as the confine helper reads it: JSON of what to open.
fn rules(reach: &Reach) -> String {
    let text = |ps: &[&PathBuf]| -> Vec<String> {
        ps.iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect()
    };
    let writable: Vec<&PathBuf> = reach.work.iter().chain(&reach.write).collect();
    let readable: Vec<&PathBuf> = std::iter::once(&reach.own).chain(&reach.read).collect();
    serde_json::json!({
        "write": text(&writable),
        "read": text(&readable),
        "ports": reach.ports,
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reach() -> Reach {
        Reach {
            own: "/data/connectors/x/1.0.0".into(),
            work: Some("/data/extension-data/x".into()),
            write: vec!["/work/app".into()],
            read: vec!["/rt/node".into()],
            hidden: vec!["/home/me".into(), "/data".into()],
            home: Some("/home/me".into()),
            data: Some("/data".into()),
            ports: vec![4242, 50091],
        }
    }

    #[test]
    fn hides_home_and_data_but_opens_its_own_folder_projects_and_runtime() {
        let p = profile(&reach());
        assert!(p.contains("(deny network*)"));
        assert!(p.contains("(allow network-outbound (remote tcp \"localhost:4242\"))"));
        assert!(p.contains("(allow network-outbound (remote tcp \"localhost:50091\"))"));
        let deny = p.find("(deny file-read-data").unwrap();
        let allow = p.find("(allow file-read-data").unwrap();
        assert!(deny < allow, "an allow after the deny wins:\n{p}");
        assert!(p[deny..allow].contains("(subpath \"/home/me\")"));
        for open in [
            "/data/connectors/x/1.0.0",
            "/data/extension-data/x",
            "/work/app",
            "/rt/node",
        ] {
            assert!(
                p[allow..].contains(&format!("(subpath \"{open}\")")),
                "{open}:\n{p}"
            );
        }
        let writes = &p[p.find("(allow file-write*").unwrap()..deny];
        assert!(writes.contains("/work/app") && writes.contains("/data/extension-data/x"));
        // Its manifest and code are read, never written.
        assert!(!writes.contains("/rt/node") && !writes.contains("/data/connectors/x/1.0.0"));
        let rules: serde_json::Value = serde_json::from_str(&rules(&reach())).unwrap();
        assert!(!rules["write"].to_string().contains("connectors/x"));
        assert!(rules["read"].to_string().contains("connectors/x"));
    }

    #[cfg(unix)]
    #[test]
    fn a_version_managers_shim_starts_the_runtime_it_names() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let home = real(root.path());
        let real_node = home.join("installs/node/22/bin/node");
        std::fs::create_dir_all(real_node.parent().unwrap()).unwrap();
        std::fs::write(&real_node, "").unwrap();
        let shim = home.join(".asdf/shims/node");
        std::fs::create_dir_all(shim.parent().unwrap()).unwrap();
        // It answers only when asked from home: from the extension's folder it would read version files planted there.
        std::fs::write(
            &shim,
            format!(
                "#!/bin/sh\n[ \"$1\" = -p ] && [ \"$(pwd -P)\" = \"{}\" ] && [ -z \"$VORN_EXTENSION_TOKEN\" ] && echo {}\n",
                home.display(),
                real_node.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
        let env = vec![
            ("PATH".to_owned(), "/usr/bin:/bin".to_owned()),
            ("HOME".to_owned(), home.to_string_lossy().into_owned()),
            ("VORN_EXTENSION_TOKEN".to_owned(), "secret".to_owned()),
        ];
        assert_eq!(real_runtime(&shim, &env), Some(real_node.clone()));
        // Its runtime folder is where the version lives, not the shims or the home folder.
        assert_eq!(
            runtime_root(&real_node, Some(&home)),
            Some(home.join("installs/node/22"))
        );
        assert_eq!(real_runtime(Path::new("/bin/sh"), &env), None);
    }

    #[tokio::test]
    async fn every_program_of_an_extension_shares_one_way_out() {
        let confiner = Confiner {
            proxy: Some(NetProxy::start().await.unwrap()),
            ..Confiner::default()
        };
        let launch = || Launch {
            program: "/bin/sh".into(),
            args: vec![],
            cwd: "/".into(),
            env: vec![],
        };
        let proxy_of = |l: &Launch| {
            l.env
                .iter()
                .find(|(k, _)| k == "HTTPS_PROXY")
                .map(|(_, v)| v.clone())
        };
        let hosts = || vec!["api.example.com".to_owned()];
        let (mut first, mut r1) = (launch(), Reach::default());
        confiner.share("x.y", hosts(), &mut first, &mut r1);
        let (mut second, mut r2) = (launch(), Reach::default());
        confiner.share("x.y", hosts(), &mut second, &mut r2);
        // The second program starts with the first's grant, which stays good.
        assert!(proxy_of(&first).is_some());
        assert_eq!(proxy_of(&first), proxy_of(&second));
        assert_eq!(r1.ports, r2.ports);
        assert_eq!(confiner.grants.lock().unwrap().len(), 1);
        confiner.revoke("x.y");
        let (mut third, mut r3) = (launch(), Reach::default());
        confiner.share("x.y", hosts(), &mut third, &mut r3);
        assert_ne!(proxy_of(&first), proxy_of(&third));
        let (mut none, mut r4) = (launch(), Reach::default());
        confiner.share("x.z", vec![], &mut none, &mut r4);
        assert!(proxy_of(&none).is_none() && r4.ports.is_empty());
    }

    #[test]
    fn never_runs_unconfined_a_program_a_sandboxed_one_could_have_written() {
        let data = tempfile::tempdir().unwrap();
        let own = data.path().join("connectors/x/1.0.0");
        let project = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(own.join("bin")).unwrap();
        let mut reach = Reach::of(&own, None, Some(data.path()));
        reach.write.push(real(project.path()));
        assert!(!may_ask_where(&own.join("bin/node"), &reach));
        assert!(!may_ask_where(&project.path().join("node"), &reach));
        assert!(!may_ask_where(
            &data.path().join("connectors/other/node"),
            &reach
        ));
        assert!(may_ask_where(Path::new("/usr/bin/python3"), &reach));
        // A file system that ignores case does here too.
        if cfg!(any(target_os = "macos", windows)) {
            let upper = PathBuf::from(own.to_string_lossy().to_uppercase()).join("bin/node");
            assert!(!may_ask_where(&upper, &reach));
        }
    }

    #[test]
    fn a_removed_extensions_data_is_gone_for_the_next_with_its_id() {
        let data = tempfile::tempdir().unwrap();
        let confiner = Confiner {
            data: Some(data.path().to_path_buf()),
            ..Confiner::default()
        };
        let dir = confiner.work_dir("x.y").unwrap();
        std::fs::write(dir.join("kept"), "left behind").unwrap();
        confiner.forget_data("x.y").unwrap();
        confiner.forget_data("x.y").unwrap();
        let again = confiner.work_dir("x.y").unwrap();
        assert_eq!(std::fs::read_dir(again).unwrap().count(), 0);
    }

    #[test]
    fn a_project_holding_vorns_data_is_not_given_at_all() {
        let home = tempfile::tempdir().unwrap();
        let data = home.path().join("a/b/.vorn");
        let beside = home.path().join("work/app");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::create_dir_all(&beside).unwrap();
        let confiner = Confiner {
            home: Some(home.path().to_path_buf()),
            data: Some(data.clone()),
            ..Confiner::default()
        };
        std::fs::create_dir_all(data.join("x")).unwrap();
        confiner.set_projects(vec![
            home.path().to_path_buf(),
            beside.clone(),
            data.join("x"),
        ]);
        assert_eq!(
            confiner.closed_projects(),
            [real(home.path()), real(&data).join("x")]
        );
        let reach = confiner.reach(&home.path().join("own"), "t.x", true);
        assert_eq!(reach.write, [real(&beside)]);
    }

    #[test]
    fn quotes_what_it_names() {
        let r = Reach {
            work: Some("/a \"b\"/c".into()),
            ..Reach::default()
        };
        assert!(profile(&r).contains("(subpath \"/a \\\"b\\\"/c\")"));
        assert!(
            !profile(&r).contains("file-read-data"),
            "nothing hidden, nothing denied"
        );
    }

    #[test]
    fn a_runtime_root_is_its_prefix_but_never_home() {
        let home = Path::new("/home/me");
        assert_eq!(
            runtime_root(Path::new("/home/me/.nvm/v22/bin/node"), Some(home)),
            Some(PathBuf::from("/home/me/.nvm/v22"))
        );
        assert_eq!(
            runtime_root(Path::new("/home/me/bin/tool"), Some(home)),
            Some(PathBuf::from("/home/me/bin"))
        );
        assert_eq!(runtime_root(Path::new("/home/me/tool"), Some(home)), None);
        assert_eq!(runtime_root(Path::new("/tool"), Some(home)), None);
    }

    #[test]
    fn wraps_the_program_where_the_machine_can_and_keeps_its_place() {
        let launch = Launch {
            program: "/bin/sh".into(),
            args: vec!["main.sh".into()],
            cwd: "/x".into(),
            env: vec![("A".into(), "1".into())],
        };
        let own = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let mut r = Reach::of(own.path(), None, None);
        r.work = Some(real(work.path()));
        let Some(wrapped) = wrap(launch, &r, None) else {
            assert!(!available(None));
            return;
        };
        assert_eq!(wrapped.program, PathBuf::from(SANDBOX_EXEC));
        assert_eq!(wrapped.args[0], "-p");
        assert_eq!(&wrapped.args[2..], ["/bin/sh", "main.sh"]);
        assert_eq!(wrapped.cwd, PathBuf::from("/x"));
        assert!(wrapped.env.contains(&("A".to_owned(), "1".to_owned())));
        let tmp = wrapped.env.iter().find(|(k, _)| k == "TMPDIR").unwrap();
        assert!(Path::new(&tmp.1).starts_with(real(work.path())));
        let data = wrapped.env.iter().find(|(k, _)| k == DATA_ENV).unwrap();
        assert_eq!(Path::new(&data.1), real(work.path()));
    }

    #[test]
    fn the_switch_is_on_until_a_person_turns_it_off() {
        let home = tempfile::tempdir().unwrap();
        assert!(is_on(home.path()));
        set(home.path(), false).unwrap();
        assert!(!is_on(home.path()));
        set(home.path(), true).unwrap();
        assert!(is_on(home.path()));
        set(home.path(), true).unwrap();
        assert!(set(&home.path().join("gone"), false).is_err());
    }

    #[test]
    fn names_where_programs_run() {
        assert_eq!(Kind::None.name(), "unsandboxed");
        assert_eq!(Kind::Seatbelt.name(), "sandboxed");
        assert_eq!(Kind::Landlock.name(), "files-only");
        assert!(Kind::Seatbelt.limits_network() && !Kind::Landlock.limits_network());
        let rules: serde_json::Value = serde_json::from_str(&rules(&reach())).unwrap();
        assert_eq!(rules["write"][1], "/work/app");
        assert_eq!(rules["read"][0], "/data/connectors/x/1.0.0");
        assert_eq!(rules["read"][1], "/rt/node");
        assert_eq!(rules["ports"][0], 4242);
    }

    /// A program whose project holds Vorn's data reads neither that data nor the project.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_program_cannot_open_a_project_holding_vorns_data() {
        if Kind::here() != Kind::Seatbelt {
            eprintln!("skipped: no sandbox-exec");
            return;
        }
        let home = tempfile::tempdir().unwrap();
        let data = home.path().join("deep/er/.vorn");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("token"), "s3cret").unwrap();
        std::fs::write(home.path().join("notes"), "mine").unwrap();
        let own = data.join("connectors/t.x/1.0.0");
        std::fs::create_dir_all(&own).unwrap();
        let confiner = Confiner {
            home: Some(home.path().to_path_buf()),
            data: Some(data.clone()),
            ..Confiner::default()
        };
        confiner.set_projects(vec![home.path().to_path_buf()]);
        let reach = confiner.reach(&own, "t.x", true);
        let run = |script: String| {
            let launch = Launch {
                program: "/bin/sh".into(),
                args: vec!["-c".into(), script],
                cwd: own.clone(),
                env: vec![("PATH".into(), "/usr/bin:/bin".into())],
            };
            let w = wrap(launch, &reach, None).unwrap();
            vorn_spawn::command(&w.program)
                .args(&w.args)
                .current_dir(&w.cwd)
                .env_clear()
                .envs(w.env.iter().map(|(k, v)| (k, v)))
                .output()
                .unwrap()
                .status
                .success()
        };
        let (token, notes) = (real(&data).join("token"), real(home.path()).join("notes"));
        assert!(!run(format!("cat {}", token.display())));
        assert!(!run(format!("cat {}", notes.display())));
        assert!(!run(format!(
            "echo x > {}",
            real(home.path()).join("written").display()
        )));
        // Its own data folder, under Vorn's, is still its own.
        assert!(run("echo x > \"$VORN_EXTENSION_DATA/ok\"".to_owned()));
    }

    /// The profile does what it says, on a machine that has Seatbelt.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_sandboxed_program_cannot_read_a_hidden_folder_or_write_outside_its_own() {
        if Kind::here() != Kind::Seatbelt {
            eprintln!("skipped: no sandbox-exec");
            return;
        }
        let home = tempfile::tempdir().unwrap();
        let own = home.path().join("own");
        std::fs::create_dir(&own).unwrap();
        std::fs::write(home.path().join("secret"), "s3cret").unwrap();
        std::fs::write(own.join("mine"), "mine").unwrap();
        let work = home.path().join("data");
        std::fs::create_dir(&work).unwrap();
        let mut reach = Reach::of(&own, Some(home.path()), None);
        reach.work = Some(real(&work));
        let run = |script: &str| {
            let launch = Launch {
                program: "/bin/sh".into(),
                args: vec!["-c".into(), script.into()],
                cwd: own.clone(),
                env: vec![("PATH".into(), "/usr/bin:/bin".into())],
            };
            let w = wrap(launch, &reach, None).unwrap();
            let out = vorn_spawn::command(&w.program)
                .args(&w.args)
                .current_dir(&w.cwd)
                .env_clear()
                .envs(w.env.iter().map(|(k, v)| (k, v)))
                .output()
                .unwrap();
            (
                out.status.success(),
                String::from_utf8_lossy(&out.stdout).into_owned(),
            )
        };
        let secret = real(&home.path().join("secret"));
        assert!(!run(&format!("cat {}", secret.display())).0);
        assert_eq!(run("cat mine"), (true, "mine".to_owned()));
        // Its own folder holds its manifest and code: read, never written.
        assert!(!run("echo x > written").0);
        assert!(!run("echo x >> mine").0);
        assert!(
            run(
                "echo x > \"$VORN_EXTENSION_DATA/written\" && cat \"$VORN_EXTENSION_DATA/written\""
            )
            .0
        );
        assert!(!run(&format!("echo x > {}/outside", real(home.path()).display())).0);
    }
}
