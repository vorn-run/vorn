//! What a local shell is launched with so it reports command blocks
//! (`shell-integration`): the environment and arguments for bash, zsh, fish,
//! PowerShell and cmd, and which shell a session runs when none is chosen.
//!
//! bash, zsh and fish read init files ("shims") written into a directory of
//! their own under the temporary directory ([`shim_root`]). Every shell
//! sources them, so they are written only into a directory this user alone
//! can change, and each file is replaced whole by a rename: a shell starting
//! meanwhile reads the old file or the new one, never half of one. A file
//! already as this build writes it is left alone. PowerShell and cmd need no
//! files: their integration rides the command line and the `PROMPT`
//! variable, built here.

use std::fmt;
use std::path::Path;

use data_encoding::BASE64;

use super::env::lookup;
use super::Platform;
use crate::{js, paths};

/// A shell family with an integration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShellFamily {
    Zsh,
    Bash,
    Fish,
    PowerShell,
    Cmd,
}

impl ShellFamily {
    /// The family a shell's path names (`detectShellFamily`), or `None` for a
    /// shell that runs untouched.
    pub fn of(path: &str) -> Option<ShellFamily> {
        let name = path.replace('\\', "/");
        let name = name.rsplit('/').next().unwrap_or("").to_lowercase();
        match name.strip_suffix(".exe").unwrap_or(&name) {
            "zsh" => Some(ShellFamily::Zsh),
            "bash" => Some(ShellFamily::Bash),
            "fish" => Some(ShellFamily::Fish),
            "pwsh" | "powershell" => Some(ShellFamily::PowerShell),
            "cmd" => Some(ShellFamily::Cmd),
            _ => None,
        }
    }

    /// The `ShellFamily` name.
    pub fn id(self) -> &'static str {
        match self {
            ShellFamily::Zsh => "zsh",
            ShellFamily::Bash => "bash",
            ShellFamily::Fish => "fish",
            ShellFamily::PowerShell => "powershell",
            ShellFamily::Cmd => "cmd",
        }
    }

    /// The files written for it, relative to its shim directory, with their
    /// contents; `None` for a family that needs none. bash and fish draw the
    /// prompt differently with the minimal prompt.
    fn shim(self, minimal_prompt: bool) -> Option<&'static [(&'static str, &'static str)]> {
        match (self, minimal_prompt) {
            (ShellFamily::Zsh, _) => Some(ZSH_SHIM),
            (ShellFamily::Bash, true) => Some(&[(
                "vorn-bashrc",
                include_str!("../../shims/bash/minimal.bashrc"),
            )]),
            (ShellFamily::Bash, false) => {
                Some(&[("vorn-bashrc", include_str!("../../shims/bash/own.bashrc"))])
            }
            (ShellFamily::Fish, true) => {
                Some(&[(FISH_FILE, include_str!("../../shims/fish/minimal.fish"))])
            }
            (ShellFamily::Fish, false) => {
                Some(&[(FISH_FILE, include_str!("../../shims/fish/own.fish"))])
            }
            (ShellFamily::PowerShell | ShellFamily::Cmd, _) => None,
        }
    }
}

/// zsh reads its startup files from `ZDOTDIR`, and each of these hands on to the person's own.
const ZSH_SHIM: &[(&str, &str)] = &[
    (".zshenv", include_str!("../../shims/zsh/.zshenv")),
    (".zprofile", include_str!("../../shims/zsh/.zprofile")),
    (".zshrc", include_str!("../../shims/zsh/.zshrc")),
];

/// fish reads vendor configuration from every directory on `XDG_DATA_DIRS`.
const FISH_FILE: &str = "fish/vendor_conf.d/vorn.fish";

/// How to launch a shell with its integration (`ShellSetup`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShellSetup {
    /// Added to the session's environment.
    pub env: Vec<(String, String)>,
    /// Launch arguments in place of the defaults, when there are any: bash
    /// and PowerShell can only be told on their command line.
    pub args: Option<Vec<String>>,
}

/// What a shell's integration is set up with.
#[derive(Clone, Copy, Debug)]
pub struct ShellContext<'a> {
    /// Replace the shell's own prompt with nothing, so blocks own the layout.
    pub minimal_prompt: bool,
    /// The safe environment the session starts from: its `ZDOTDIR`,
    /// `XDG_DATA_DIRS` and `PROMPT` are kept behind the integration's own.
    pub env: &'a [(String, String)],
    /// The home directory, the user's zsh directory when `ZDOTDIR` is unset.
    pub home: &'a str,
    /// Where the shims are written ([`shim_root`]).
    pub shim_root: &'a str,
}

/// Why the shims cannot be written.
#[derive(Debug)]
pub enum ShimError {
    /// A file or directory could not be written.
    Unwritable { path: String, error: std::io::Error },
    /// The directory is not one only this user can change.
    NotOwned(String),
}

impl fmt::Display for ShimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShimError::Unwritable { path, error } => write!(f, "cannot write shim {path}: {error}"),
            ShimError::NotOwned(dir) => write!(
                f,
                "refusing to use shim directory not exclusively owned by this user: {dir}"
            ),
        }
    }
}

impl std::error::Error for ShimError {}

/// The setup for `shell` (`getShellIntegration` with the shell given): an
/// empty one for a shell with no integration, which then runs exactly as it
/// would have.
pub fn shell_setup(shell: &str, cx: &ShellContext<'_>) -> Result<ShellSetup, ShimError> {
    let Some(family) = ShellFamily::of(shell) else {
        return Ok(ShellSetup::default());
    };
    let minimal = if cx.minimal_prompt { "1" } else { "0" };
    let pair = |k: &str, v: &str| (k.to_owned(), v.to_owned());
    let setup = match family {
        ShellFamily::Zsh => {
            let dir = write_shim(family, cx)?;
            let user = lookup(cx.env, "ZDOTDIR").unwrap_or(cx.home);
            ShellSetup {
                env: vec![
                    pair("ZDOTDIR", &dir),
                    pair("VORN_USER_ZDOTDIR", user),
                    pair("VORN_MINIMAL_PROMPT", minimal),
                    // The gap between blocks rides along with the minimal prompt.
                    pair("VORN_BLOCK_GAP", if cx.minimal_prompt { "1" } else { "" }),
                ],
                args: None,
            }
        }
        ShellFamily::Bash => {
            let dir = write_shim(family, cx)?;
            ShellSetup {
                env: vec![pair("VORN_MINIMAL_PROMPT", minimal)],
                // Long options first: bash 3.2 rejects `-i --rcfile`.
                args: Some(vec![
                    "--rcfile".to_owned(),
                    format!("{dir}/vorn-bashrc"),
                    "-i".to_owned(),
                ]),
            }
        }
        ShellFamily::Fish => {
            let dir = write_shim(family, cx)?;
            // Prepended, never replacing: other vendors' files stay visible.
            let data_dirs = match lookup(cx.env, "XDG_DATA_DIRS").unwrap_or("") {
                "" => format!("{dir}:/usr/local/share:/usr/share"),
                existing => format!("{dir}:{existing}"),
            };
            ShellSetup {
                env: vec![
                    pair("XDG_DATA_DIRS", &data_dirs),
                    pair("VORN_MINIMAL_PROMPT", minimal),
                ],
                args: None,
            }
        }
        ShellFamily::PowerShell => ShellSetup {
            env: vec![pair("VORN_MINIMAL_PROMPT", minimal)],
            // -EncodedCommand takes UTF-16LE, and is no script file, so the
            // execution policy does not apply to it.
            args: Some(vec![
                "-NoExit".to_owned(),
                "-EncodedCommand".to_owned(),
                encode_utf16le_base64(&powershell_script(cx.minimal_prompt)),
            ]),
        },
        ShellFamily::Cmd => {
            // Wrapping what the person had; `$P$G` is cmd's own default.
            let visible = if cx.minimal_prompt {
                ""
            } else {
                match lookup(cx.env, "PROMPT").unwrap_or("") {
                    "" => "$P$G",
                    existing => existing,
                }
            };
            ShellSetup {
                env: vec![pair(
                    "PROMPT",
                    &format!("{CMD_MARKERS_BEFORE}{visible}{CMD_MARKERS_AFTER}"),
                )],
                args: None,
            }
        }
    };
    Ok(setup)
}

/// The family's shim directory, with its files as this build writes them,
/// in a directory only this user can change.
fn write_shim(family: ShellFamily, cx: &ShellContext<'_>) -> Result<String, ShimError> {
    let files = family
        .shim(cx.minimal_prompt)
        .expect("only families with shim files");
    let dir = paths::join(cx.shim_root, family.id());
    private_dir(&dir)?;
    if !exclusively_owned(Path::new(&dir)) {
        return Err(ShimError::NotOwned(dir));
    }
    for (name, contents) in files {
        let path = paths::join(&dir, name);
        // Read through no link: a link here is not a file this build wrote.
        let current = std::fs::symlink_metadata(&path)
            .ok()
            .filter(std::fs::Metadata::is_file)
            .and_then(|_| std::fs::read(&path).ok());
        if current.as_deref() == Some(contents.as_bytes()) {
            continue;
        }
        if let Some(parent) = Path::new(&path).parent() {
            private_dir(&parent.to_string_lossy())?;
        }
        replace(&path, contents.as_bytes()).map_err(|error| ShimError::Unwritable {
            path: path.clone(),
            error,
        })?;
    }
    Ok(dir)
}

/// Makes `dir` and its parents, readable and writable by this user only.
fn private_dir(dir: &str) -> Result<(), ShimError> {
    let made = {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder.create(dir)
    };
    // The mode is masked by the umask, and ignored for a directory there already.
    #[cfg(unix)]
    let made = made.and_then(|()| {
        use std::os::unix::fs::PermissionsExt;
        let meta = std::fs::symlink_metadata(dir)?;
        if meta.is_dir()
            && meta.permissions().mode() & 0o077 != 0
            && exclusively_owned(Path::new(dir))
        {
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
        Ok(())
    });
    made.map_err(|error| ShimError::Unwritable {
        path: dir.to_owned(),
        error,
    })
}

/// Writes `contents` beside `path` and renames it over: the entry is
/// replaced, never followed, and a reader sees one whole file or the other.
fn replace(path: &str, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let fresh = format!("{path}.{}.tmp", std::process::id());
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let _ = std::fs::remove_file(&fresh);
    let written = options
        .open(&fresh)
        .and_then(|mut f| f.write_all(contents).and_then(|()| f.sync_all()))
        .and_then(|()| std::fs::rename(&fresh, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&fresh);
    }
    written
}

/// Whether `dir` is a directory, owned by this user, writable by nobody else.
#[cfg(unix)]
fn exclusively_owned(dir: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Ok(meta) = std::fs::symlink_metadata(dir) else {
        return false;
    };
    // SAFETY: getuid has no preconditions and cannot fail.
    let uid = unsafe { libc::getuid() };
    meta.is_dir() && meta.uid() == uid && meta.mode() & 0o022 == 0
}

#[cfg(not(unix))]
fn exclusively_owned(dir: &Path) -> bool {
    std::fs::symlink_metadata(dir).is_ok_and(|m| m.is_dir())
}

/// Where the shims are written: `vorn-shell-integration` in the temporary
/// directory Node's `os.tmpdir()` names, read from the environment through `var`.
pub fn shim_root(platform: Platform, var: impl Fn(&str) -> Option<String>) -> String {
    let set = |name: &str| var(name).filter(|v| !v.is_empty());
    let tmp = match platform {
        Platform::Posix => {
            let dir = set("TMPDIR")
                .or_else(|| set("TMP"))
                .or_else(|| set("TEMP"))
                .unwrap_or_else(|| "/tmp".to_owned());
            match dir.strip_suffix('/') {
                Some(trimmed) if dir.len() > 1 => trimmed.to_owned(),
                _ => dir,
            }
        }
        Platform::Windows => {
            let dir = set("TEMP").or_else(|| set("TMP")).unwrap_or_else(|| {
                let root = set("SystemRoot").or_else(|| set("windir"));
                format!("{}\\temp", root.unwrap_or_default())
            });
            match dir.strip_suffix('\\') {
                Some(trimmed) if dir.len() > 1 && !dir.ends_with(":\\") => trimmed.to_owned(),
                _ => dir,
            }
        }
    };
    paths::join(&tmp, "vorn-shell-integration")
}

/// The shell a session runs (`getDefaultShell`): the configured one when
/// set; else on Windows PowerShell 7 where it is on PATH, then the Windows
/// PowerShell every install has, and cmd only if neither is there; else
/// `$SHELL`, else zsh. `var` reads the server's own environment.
pub fn default_shell(
    configured: Option<&str>,
    platform: Platform,
    var: impl Fn(&str) -> Option<String>,
) -> String {
    if let Some(chosen) = configured.map(js::trim).filter(|c| !c.is_empty()) {
        return chosen.to_owned();
    }
    let set = |name: &str| var(name).filter(|v| !v.is_empty());
    if platform == Platform::Posix {
        return set("SHELL").unwrap_or_else(|| "/bin/zsh".to_owned());
    }
    let path = var("PATH").unwrap_or_default();
    // `path.delimiter` and `path.join` are the host's.
    let delimiter = if cfg!(windows) { ';' } else { ':' };
    for dir in path.split(delimiter).filter(|d| !d.is_empty()) {
        let candidate = paths::join(dir, "pwsh.exe");
        if Path::new(&candidate).exists() {
            return candidate;
        }
    }
    let root = set("SystemRoot")
        .or_else(|| set("windir"))
        .unwrap_or_else(|| "C:\\Windows".to_owned());
    let windows = paths::join(
        &paths::join(
            &paths::join(&paths::join(&root, "System32"), "WindowsPowerShell"),
            "v1.0",
        ),
        "powershell.exe",
    );
    if Path::new(&windows).exists() {
        return windows;
    }
    set("COMSPEC").unwrap_or_else(|| "cmd.exe".to_owned())
}

/// The arguments a shell starts with when its integration sets none
/// (`getShellArgs`): a login shell, except on Windows.
pub fn default_shell_args(platform: Platform) -> &'static [&'static str] {
    match platform {
        Platform::Posix => &["-l"],
        Platform::Windows => &[],
    }
}

/// `$e` is ESC, `$P` the current path, `$e\` the string terminator. D comes
/// first, closing the command that just finished, before A opens the next.
const CMD_MARKERS_BEFORE: &str = "$e]133;D$e\\$e]5522;cwd;$P$e\\$e]133;A$e\\";
const CMD_MARKERS_AFTER: &str = "$e]133;B$e\\";

/// The PowerShell integration: a `prompt` that reports the last command one
/// prompt late, there being no pre-execution hook, with its duration taken
/// from the history entry.
fn powershell_script(minimal_prompt: bool) -> String {
    const HEAD: &str = r#"
$Global:__VornLastHistoryId = -1
$Global:__VornOriginalPrompt = $function:prompt

function Global:__VornExitCode {
  if ($? -eq $True) { return 0 }
  $h = Get-History -Count 1
  if ($Error[0].InvocationInfo.HistoryId -eq $h.Id) { return -1 }
  if ($null -eq $LastExitCode) { return 1 }
  return $LastExitCode
}

function Global:prompt {
  $code = __VornExitCode
  $h = Get-History -Count 1
  $out = ''
  if ($Global:__VornLastHistoryId -ne -1 -and $h -and $h.Id -ne $Global:__VornLastHistoryId) {
    $bytes = [Text.Encoding]::UTF8.GetBytes($h.CommandLine)
    $out += "`e]5522;cmd;$([Convert]::ToBase64String($bytes))`a"
    $ms = [int]($h.EndExecutionTime - $h.StartExecutionTime).TotalMilliseconds
    $out += "`e]5522;dur;$ms`a"
    $out += "`e]133;D;$code`a"
"#;
    const MIDDLE: &str = r#"  }
  $out += "`e]5522;cwd;$($executionContext.SessionState.Path.CurrentLocation)`a"
  $out += "`e]133;A`a"
"#;
    const MINIMAL: &str = r#"  # Nothing in place of the prompt: the input bar already shows a caret and
  # each command is its own block heading. The string is still non-empty
  # because of the sequences, so PowerShell does not fall back to "PS>"."#;
    const ORIGINAL: &str = "  $out += [string](& $Global:__VornOriginalPrompt)";
    const TAIL: &str = r#"
  $out += "`e]133;B`a"
  if ($h) { $Global:__VornLastHistoryId = $h.Id }
  return $out
}
"#;
    let mut script = String::with_capacity(HEAD.len() + MIDDLE.len() + MINIMAL.len() + 64);
    script.push_str(HEAD);
    if minimal_prompt {
        script.push_str("    $out += \"\\n\"\n");
    }
    script.push_str(MIDDLE);
    script.push_str(if minimal_prompt { MINIMAL } else { ORIGINAL });
    script.push_str(TAIL);
    script
}

fn encode_utf16le_base64(text: &str) -> String {
    let bytes: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
    BASE64.encode(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    fn cx<'a>(env: &'a [(String, String)], minimal: bool, root: &'a str) -> ShellContext<'a> {
        ShellContext {
            minimal_prompt: minimal,
            env,
            home: "/home/me",
            shim_root: root,
        }
    }

    #[test]
    fn knows_a_shell_by_its_file_name() {
        assert_eq!(ShellFamily::of("/usr/bin/zsh"), Some(ShellFamily::Zsh));
        assert_eq!(
            ShellFamily::of("C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\PowerShell.EXE"),
            Some(ShellFamily::PowerShell)
        );
        assert_eq!(ShellFamily::of("/bin/sh"), None);
    }

    #[test]
    fn leaves_a_shell_it_cannot_integrate_alone() {
        let e = env(&[]);
        assert_eq!(
            shell_setup("/bin/sh", &cx(&e, true, "/nonexistent")).unwrap(),
            ShellSetup::default()
        );
    }

    #[test]
    fn wraps_the_cmd_prompt_unless_minimal() {
        let e = env(&[("PROMPT", "$N$G")]);
        let setup = shell_setup("cmd.exe", &cx(&e, false, "/x")).unwrap();
        assert_eq!(
            setup.env,
            env(&[(
                "PROMPT",
                "$e]133;D$e\\$e]5522;cwd;$P$e\\$e]133;A$e\\$N$G$e]133;B$e\\"
            )])
        );
        let none = env(&[]);
        let setup = shell_setup("cmd", &cx(&none, false, "/x")).unwrap();
        assert!(setup.env[0].1.contains("$P$G$e]133;B"));
        let setup = shell_setup("cmd", &cx(&e, true, "/x")).unwrap();
        assert!(setup.env[0].1.ends_with("$e]133;A$e\\$e]133;B$e\\"));
    }

    #[test]
    fn hands_powershell_its_script_as_utf16() {
        let e = env(&[]);
        let setup = shell_setup("pwsh", &cx(&e, true, "/x")).unwrap();
        let args = setup.args.unwrap();
        assert_eq!(args[..2], ["-NoExit", "-EncodedCommand"]);
        let bytes = BASE64.decode(args[2].as_bytes()).unwrap();
        let units: Vec<u16> = bytes
            .chunks(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        let script = String::from_utf16(&units).unwrap();
        assert_eq!(script, powershell_script(true));
        assert!(script.contains("$out += \"\\n\"\n  }"));
        assert!(!powershell_script(false).contains("\"\\n\""));
    }

    #[test]
    fn writes_each_shim_once_and_whole() {
        let root = tempfile::tempdir().unwrap();
        let root_str = root.path().to_str().unwrap();
        let e = env(&[]);
        let setup = shell_setup("bash", &cx(&e, true, root_str)).unwrap();
        let rc = root.path().join("bash").join("vorn-bashrc");
        assert_eq!(setup.args.unwrap()[1], rc.to_str().unwrap());
        assert_eq!(
            std::fs::read_to_string(&rc).unwrap(),
            include_str!("../../shims/bash/minimal.bashrc")
        );
        // Another prompt setting is another file; a changed one is put back.
        shell_setup("bash", &cx(&e, false, root_str)).unwrap();
        assert_eq!(
            std::fs::read_to_string(&rc).unwrap(),
            include_str!("../../shims/bash/own.bashrc")
        );
        std::fs::write(&rc, "echo changed\n").unwrap();
        shell_setup("bash", &cx(&e, false, root_str)).unwrap();
        assert_eq!(
            std::fs::read_to_string(&rc).unwrap(),
            include_str!("../../shims/bash/own.bashrc")
        );
        shell_setup("fish", &cx(&e, true, root_str)).unwrap();
        assert!(root.path().join("fish").join(FISH_FILE).is_file());
        // Nothing left beside them.
        let left: Vec<_> = std::fs::read_dir(root.path().join("bash"))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(left, ["vorn-bashrc"]);
    }

    #[cfg(unix)]
    #[test]
    fn replaces_a_link_rather_than_writing_through_it() {
        let root = tempfile::tempdir().unwrap();
        let root_str = root.path().to_str().unwrap();
        let elsewhere = root.path().join("elsewhere");
        std::fs::write(&elsewhere, "mine\n").unwrap();
        let e = env(&[]);
        shell_setup("bash", &cx(&e, true, root_str)).unwrap();
        let rc = root.path().join("bash").join("vorn-bashrc");
        std::fs::remove_file(&rc).unwrap();
        std::os::unix::fs::symlink(&elsewhere, &rc).unwrap();
        shell_setup("bash", &cx(&e, true, root_str)).unwrap();
        assert_eq!(std::fs::read_to_string(&elsewhere).unwrap(), "mine\n");
        assert!(std::fs::symlink_metadata(&rc).unwrap().is_file());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_a_shim_directory_somebody_else_can_change() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let root_str = root.path().to_str().unwrap();
        let dir = root.path().join("zsh");
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
        let e = env(&[]);
        assert!(matches!(
            shell_setup("zsh", &cx(&e, true, root_str)),
            Err(ShimError::NotOwned(_))
        ));
        assert!(!dir.join(".zshrc").exists());
    }

    /// The SHA-256 of each file's name and contents, each followed by a NUL.
    fn digest(files: &[(&str, &str)]) -> String {
        use sha2::{Digest, Sha256};
        let mut hash = Sha256::new();
        for (name, contents) in files {
            hash.update(name.as_bytes());
            hash.update([0]);
            hash.update(contents.as_bytes());
            hash.update([0]);
        }
        data_encoding::HEXLOWER.encode(&hash.finalize())
    }

    #[test]
    fn writes_the_shims_the_server_wrote_byte_for_byte() {
        // The digests of the files `packages/server/src/shell-integration` wrote, minimal prompt first.
        let known = [
            (
                ShellFamily::Zsh,
                true,
                "2b134ab77ca78b34c72a666059948c30b7c1326334d41b077e3df6e7cde74470",
            ),
            (
                ShellFamily::Zsh,
                false,
                "2b134ab77ca78b34c72a666059948c30b7c1326334d41b077e3df6e7cde74470",
            ),
            (
                ShellFamily::Bash,
                true,
                "6f280e8e47fc35ce63af67cd177c59fb770d7f6b0abdf35fef0939a90738bb64",
            ),
            (
                ShellFamily::Bash,
                false,
                "85f7e687049046a51313e2ae0751e141314b7efb03538c2cf972b8c10013a2c2",
            ),
            (
                ShellFamily::Fish,
                true,
                "3cd25184386f51024f8377607acd46f6943ea9760ec6839406944d98d38545df",
            ),
            (
                ShellFamily::Fish,
                false,
                "3ad0e51310ded684c526c5c52bd16da196368098e44b25c2f6ff93369177b65d",
            ),
        ];
        for (family, minimal, want) in known {
            assert_eq!(
                digest(family.shim(minimal).unwrap()),
                want,
                "{family:?} {minimal}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn refuses_a_directory_others_can_write() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("zsh");
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(!exclusively_owned(&dir));
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(exclusively_owned(&dir));
    }

    fn root_with(platform: Platform, vars: &'static [(&'static str, &'static str)]) -> String {
        shim_root(platform, move |n| {
            vars.iter()
                .find(|(k, _)| *k == n)
                .map(|(_, v)| (*v).to_owned())
        })
    }

    // The directory is joined as Node's `path.join` joins on the host, so
    // each platform's rules are checked on that platform.
    #[cfg(windows)]
    #[test]
    fn finds_the_temporary_directory_as_node_does() {
        let windows = |vars| root_with(Platform::Windows, vars);
        assert_eq!(
            windows(&[("TEMP", "C:\\Users\\a\\Temp\\"), ("TMP", "D:\\x")]),
            "C:\\Users\\a\\Temp\\vorn-shell-integration"
        );
        assert_eq!(
            windows(&[("SystemRoot", "C:\\Windows")]),
            "C:\\Windows\\temp\\vorn-shell-integration"
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn finds_the_temporary_directory_as_node_does() {
        let posix = |vars| root_with(Platform::Posix, vars);
        assert_eq!(posix(&[]), "/tmp/vorn-shell-integration");
        assert_eq!(
            posix(&[("TMPDIR", "/var/t/"), ("TMP", "/x")]),
            "/var/t/vorn-shell-integration"
        );
        assert_eq!(
            posix(&[("TMPDIR", ""), ("TEMP", "/e")]),
            "/e/vorn-shell-integration"
        );
    }

    #[test]
    fn picks_the_configured_shell_then_the_platform_default() {
        let none = |_: &str| None;
        assert_eq!(
            default_shell(Some("  /bin/fish "), Platform::Posix, none),
            "/bin/fish"
        );
        assert_eq!(default_shell(Some(" "), Platform::Posix, none), "/bin/zsh");
        assert_eq!(
            default_shell(None, Platform::Posix, |n| (n == "SHELL")
                .then(|| "/bin/bash".into())),
            "/bin/bash"
        );
        let comspec = |n: &str| match n {
            "SystemRoot" => Some("/nonexistent".into()),
            "COMSPEC" => Some("C:\\cmd.exe".into()),
            _ => None,
        };
        assert_eq!(
            default_shell(None, Platform::Windows, comspec),
            "C:\\cmd.exe"
        );
    }
}
