//! The app's updater. It reads the release feed for the person's channel,
//! downloads this platform's asset, checks it, and installs it when the
//! person restarts:
//!
//! - macOS swaps the new `Vorn.app` in for the running one and reopens it;
//!   vornd keeps running, and the new app replaces it with its own build
//!   while the session holder keeps the terminals (`launch::retire_older`).
//! - Windows runs the installer silently, which reopens the app.
//! - Linux replaces the AppImage and reopens it; a .deb install is told
//!   where the new package is.
//!
//! The feed is `vorn-update.json`, published with every release:
//!
//! ```json
//! { "version": "0.9.0", "releaseDate": "…",
//!   "assets": [{ "key": "mac-arm64", "name": "…", "url": "…", "sha256": "…", "size": 1 }] }
//! ```
//!
//! Stable reads the latest release's feed; beta the newest release that
//! has one, prerelease or not. `VORN_UPDATE_FEED` names a feed to read
//! instead, for a test.
//!
//! `Vorn update` runs the same steps without a window ([`update_now`]).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::client::rpc::Wake;
use crate::settings::UpdateStatus;

/// A feed to read instead of the release's.
pub const FEED_ENV: &str = "VORN_UPDATE_FEED";
/// The feed's file name among a release's assets.
pub const FEED_ASSET: &str = "vorn-update.json";
const REPO: &str = "vorn-run/vorn";
/// Where a person can fetch a release by hand.
pub const RELEASES: &str = "https://github.com/vorn-run/vorn/releases";
/// How often the app checks on its own.
pub const CHECK_EVERY: Duration = Duration::from_secs(4 * 60 * 60);
/// The first check waits for the window to settle.
const FIRST_CHECK: Duration = Duration::from_secs(10);

/// Restarting for an update ends the sessions: Windows cannot pass a
/// terminal between processes.
pub const ENDS_SESSIONS: bool = cfg!(windows);

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Feed {
    pub version: String,
    #[serde(default, rename = "releaseDate")]
    pub release_date: Option<String>,
    pub assets: Vec<Asset>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Asset {
    pub key: String,
    pub name: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

/// How this copy of the app was installed, which decides how it updates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Install {
    /// `Vorn.app`, replaced whole.
    Mac(PathBuf),
    /// The NSIS install, updated by running the new installer.
    Windows,
    /// An AppImage, replaced in place.
    AppImage(PathBuf),
    /// A .deb package: the person installs the new one.
    Deb,
    /// A build tree: no updates.
    Dev,
}

impl Install {
    /// This process's install.
    pub fn current() -> Install {
        let Ok(exe) = std::env::current_exe() else {
            return Install::Dev;
        };
        let exe = exe.canonicalize().unwrap_or(exe);
        Install::of(&exe, std::env::var_os("APPIMAGE").map(PathBuf::from))
    }

    /// The install an executable at `exe` belongs to; `appimage` is the
    /// AppImage runtime's `$APPIMAGE`.
    pub fn of(exe: &Path, appimage: Option<PathBuf>) -> Install {
        let Some(dir) = exe.parent() else {
            return Install::Dev;
        };
        if cfg!(target_os = "macos") {
            let app = dir.parent().and_then(Path::parent);
            return match app {
                Some(app)
                    if dir.ends_with("Contents/MacOS")
                        && app.extension().is_some_and(|e| e == "app")
                        && dir.join("../Resources/vornd").is_dir() =>
                {
                    Install::Mac(app.to_owned())
                }
                _ => Install::Dev,
            };
        }
        if !dir.join("resources").join("vornd").is_dir() {
            return Install::Dev;
        }
        if cfg!(windows) {
            Install::Windows
        } else if let Some(image) = appimage.filter(|p| p.is_file()) {
            Install::AppImage(image)
        } else {
            Install::Deb
        }
    }

    /// The feed asset this install takes, if it updates itself.
    pub fn key(&self) -> Option<&'static str> {
        let arch = match std::env::consts::ARCH {
            "aarch64" => "arm64",
            "x86_64" => "x64",
            _ => return None,
        };
        match (self, arch) {
            (Install::Mac(_), "arm64") => Some("mac-arm64"),
            (Install::Mac(_), _) => Some("mac-x64"),
            (Install::Windows, "x64") => Some("win-x64"),
            (Install::AppImage(_) | Install::Deb, "x64") => Some("linux-x64"),
            _ => None,
        }
    }
}

/// Whether `candidate` is a newer version than `current`.
pub fn newer(candidate: &str, current: &str) -> bool {
    let parse = |v: &str| semver::Version::parse(v.trim_start_matches('v')).ok();
    match (parse(candidate), parse(current)) {
        (Some(c), Some(now)) => c > now,
        _ => false,
    }
}

/// From GitHub's list of releases, the feed of the newest one that has
/// one, for the beta channel.
pub fn beta_feed_url(releases: &Value) -> Option<String> {
    releases
        .as_array()?
        .iter()
        .filter(|r| r["draft"].as_bool() != Some(true))
        .filter_map(|r| {
            let version =
                semver::Version::parse(r["tag_name"].as_str()?.trim_start_matches('v')).ok()?;
            let url = r["assets"]
                .as_array()?
                .iter()
                .find(|a| a["name"].as_str() == Some(FEED_ASSET))?["browser_download_url"]
                .as_str()?
                .to_owned();
            Some((version, url))
        })
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, url)| url)
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .timeout_connect(Some(Duration::from_secs(30)))
        .max_redirects(10)
        .http_status_as_error(false)
        .user_agent(concat!("Vorn/", env!("CARGO_PKG_VERSION")))
        .build()
        .new_agent()
}

fn get_json(url: &str) -> Result<Value, String> {
    let mut response = agent(Duration::from_secs(60))
        .get(url)
        .header("Accept", "application/json")
        .call()
        .map_err(|e| format!("Could not reach {}: {e}", host(url)))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(format!("{} answered HTTP {status}", host(url)));
    }
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())?;
    serde_json::from_str(&body).map_err(|e| format!("The update feed is not readable: {e}"))
}

fn host(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_else(|| url.to_owned())
}

/// The feed for `channel` ("stable" or "beta").
pub fn fetch_feed(channel: &str) -> Result<Feed, String> {
    let url = match std::env::var(FEED_ENV).ok().filter(|v| !v.is_empty()) {
        Some(url) => url,
        None if channel == "beta" => {
            let releases = get_json(&format!(
                "https://api.github.com/repos/{REPO}/releases?per_page=30"
            ))?;
            beta_feed_url(&releases).ok_or("No release has an update feed yet")?
        }
        None => format!("https://github.com/{REPO}/releases/latest/download/{FEED_ASSET}"),
    };
    serde_json::from_value(get_json(&url)?)
        .map_err(|e| format!("The update feed is not readable: {e}"))
}

/// The update `feed` offers this install, if it is newer than `current`.
pub fn pick(feed: &Feed, key: &str, current: &str) -> Option<Asset> {
    if !newer(&feed.version, current) {
        return None;
    }
    feed.assets.iter().find(|a| a.key == key).cloned()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The file's SHA-256, as lowercase hex.
pub fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut chunk = vec![0u8; 256 * 1024];
    loop {
        let n = file.read(&mut chunk).map_err(|e| e.to_string())?;
        if n == 0 {
            return Ok(hex(&hasher.finalize()));
        }
        hasher.update(&chunk[..n]);
    }
}

/// Downloads `asset` into `dir`, checking its size and SHA-256; `progress`
/// hears the percent done. A file already there that matches is kept.
pub fn download(asset: &Asset, dir: &Path, progress: &dyn Fn(u32)) -> Result<PathBuf, String> {
    let name = Path::new(&asset.name)
        .file_name()
        .ok_or("The update's file name is not a file name")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join(name);
    if path.is_file() && sha256_file(&path)?.eq_ignore_ascii_case(&asset.sha256) {
        return Ok(path);
    }
    let part = path.with_extension("part");
    let mut response = agent(Duration::from_secs(60 * 60))
        .get(&asset.url)
        .call()
        .map_err(|e| format!("Could not download the update: {e}"))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(format!("Downloading the update failed with HTTP {status}"));
    }
    let mut reader = response.body_mut().as_reader();
    let mut out = std::fs::File::create(&part).map_err(|e| format!("{}: {e}", part.display()))?;
    let mut hasher = Sha256::new();
    let mut got = 0u64;
    let mut shown = u32::MAX;
    let mut chunk = vec![0u8; 256 * 1024];
    loop {
        let n = reader.read(&mut chunk).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        got += n as u64;
        if got > asset.size {
            let _ = std::fs::remove_file(&part);
            return Err("The update is larger than the feed says".into());
        }
        hasher.update(&chunk[..n]);
        out.write_all(&chunk[..n]).map_err(|e| e.to_string())?;
        let percent = (got * 100 / asset.size.max(1)) as u32;
        if percent != shown {
            shown = percent;
            progress(percent);
        }
    }
    out.sync_all().map_err(|e| e.to_string())?;
    drop(out);
    let sum = hex(&hasher.finalize());
    if got != asset.size || !sum.eq_ignore_ascii_case(&asset.sha256) {
        let _ = std::fs::remove_file(&part);
        return Err("The update did not match its checksum; it was discarded".into());
    }
    std::fs::rename(&part, &path).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Where downloads wait, under the user's cache.
pub fn cache_dir() -> Option<PathBuf> {
    let var = |k: &str| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    if cfg!(target_os = "macos") {
        return var("HOME").map(|h| h.join("Library/Caches/com.vorn.app/updates"));
    }
    if cfg!(windows) {
        return var("LOCALAPPDATA").map(|d| d.join("vorn-updater"));
    }
    var("XDG_CACHE_HOME")
        .or_else(|| var("HOME").map(|h| h.join(".cache")))
        .map(|d| d.join("vorn-updater"))
}

/// A downloaded update, checked and ready to install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Staged {
    pub version: String,
    /// macOS: the new `Vorn.app`, beside the running one. Windows: the
    /// installer. Linux: the new AppImage, beside the running one.
    pub path: PathBuf,
    pub install: Install,
}

fn run(cmd: &str, args: &[&std::ffi::OsStr]) -> Result<String, String> {
    let out = vorn_spawn::command(cmd)
        .args(args)
        .output()
        .map_err(|e| format!("{cmd}: {e}"))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    if out.status.success() {
        Ok(text)
    } else {
        Err(format!("{cmd} failed: {}", text.trim()))
    }
}

/// The Developer ID team that signed a bundle, if a team did.
pub fn team_id(codesign_details: &str) -> Option<String> {
    codesign_details
        .lines()
        .find_map(|l| l.strip_prefix("TeamIdentifier="))
        .map(str::trim)
        .filter(|t| !t.is_empty() && *t != "not set")
        .map(str::to_owned)
}

fn os(s: &str) -> &std::ffi::OsStr {
    std::ffi::OsStr::new(s)
}

/// Unpacks the macOS zip beside the running app and checks the bundle: a
/// valid signature, the same team as this app's when it has one, and the
/// same bundle identifier.
fn stage_mac(app: &Path, zip: &Path, version: &str) -> Result<PathBuf, String> {
    let parent = app.parent().ok_or("The app has no folder")?;
    let staging = parent.join(format!(".Vorn-update-{version}"));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| {
        format!(
            "Vorn cannot write to {} to update itself ({e}). Download it from {RELEASES}",
            parent.display()
        )
    })?;
    let fail = |e: String| {
        let _ = std::fs::remove_dir_all(&staging);
        e
    };
    run(
        "ditto",
        &[os("-x"), os("-k"), zip.as_os_str(), staging.as_os_str()],
    )
    .map_err(fail)?;
    let new = std::fs::read_dir(&staging)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == "app"))
        .ok_or_else(|| fail("The update has no app in it".into()))?;
    run(
        "codesign",
        &[
            os("--verify"),
            os("--deep"),
            os("--strict"),
            new.as_os_str(),
        ],
    )
    .map_err(|e| fail(format!("The update's signature is not valid: {e}")))?;
    let details = |p: &Path| run("codesign", &[os("-dv"), os("--verbose=2"), p.as_os_str()]);
    if let Some(ours) = details(app).ok().as_deref().and_then(team_id) {
        let theirs = details(&new).ok().as_deref().and_then(team_id);
        if theirs.as_deref() != Some(ours.as_str()) {
            return Err(fail(
                "The update is not signed by the same developer as this app".into(),
            ));
        }
    }
    let bundle_id = |p: &Path| {
        run(
            "plutil",
            &[
                os("-extract"),
                os("CFBundleIdentifier"),
                os("raw"),
                os("-o"),
                os("-"),
                p.join("Contents/Info.plist").as_os_str(),
            ],
        )
        .map(|s| s.trim().to_owned())
    };
    if bundle_id(&new).map_err(fail)? != bundle_id(app).unwrap_or_else(|_| "com.vorn.app".into()) {
        return Err(fail("The update is a different app".into()));
    }
    Ok(new)
}

/// Windows: an installer signed by the same publisher when this app is signed.
fn check_windows(installer: &Path) -> Result<(), String> {
    let signer = |p: &Path| -> Option<String> {
        let script = format!(
            "$s = Get-AuthenticodeSignature -LiteralPath '{}'; if ($s.Status -eq 'Valid') {{ $s.SignerCertificate.Subject }}",
            p.display().to_string().replace('\'', "''")
        );
        run(
            "powershell",
            &[
                os("-NoProfile"),
                os("-NonInteractive"),
                os("-Command"),
                os(&script),
            ],
        )
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
    };
    let Some(ours) = std::env::current_exe().ok().and_then(|e| signer(&e)) else {
        return Ok(());
    };
    if signer(installer).as_deref() == Some(ours.as_str()) {
        Ok(())
    } else {
        Err("The update is not signed by the same publisher as this app".into())
    }
}

/// Linux: the new AppImage, executable, beside the running one.
fn stage_appimage(image: &Path, file: &Path, version: &str) -> Result<PathBuf, String> {
    let dir = image.parent().ok_or("The AppImage has no folder")?;
    let staged = dir.join(format!(".Vorn-{version}.AppImage"));
    std::fs::copy(file, &staged).map_err(|e| {
        format!(
            "Vorn cannot write to {} to update itself ({e}). Download it from {RELEASES}",
            dir.display()
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
    }
    Ok(staged)
}

/// Checks a downloaded `file` and readies it for `install`.
pub fn stage(install: &Install, file: &Path, version: &str) -> Result<Staged, String> {
    let path = match install {
        Install::Mac(app) => stage_mac(app, file, version)?,
        Install::Windows => {
            check_windows(file)?;
            file.to_owned()
        }
        Install::AppImage(image) => stage_appimage(image, file, version)?,
        Install::Deb | Install::Dev => return Err("This install does not update itself".into()),
    };
    Ok(Staged {
        version: version.to_owned(),
        path,
        install: install.clone(),
    })
}

/// Swaps `new` in for `current`, atomically where the system can.
#[cfg(target_os = "macos")]
fn swap(new: &Path, current: &Path) -> Result<(), String> {
    use std::os::unix::ffi::OsStrExt;
    let c = |p: &Path| std::ffi::CString::new(p.as_os_str().as_bytes()).map_err(|e| e.to_string());
    let (from, to) = (c(new)?, c(current)?);
    // SAFETY: both are valid NUL-terminated paths that outlive the call.
    let done = unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_SWAP) };
    if done == 0 {
        Ok(())
    } else {
        Err(format!(
            "Could not replace {}: {}",
            current.display(),
            std::io::Error::last_os_error()
        ))
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn swap(new: &Path, current: &Path) -> Result<(), String> {
    std::fs::rename(new, current)
        .map_err(|e| format!("Could not replace {}: {e}", current.display()))
}

/// Starts `command` so it outlives this process.
fn spawn_detached(command: &mut Command) -> Result<(), String> {
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use vorn_spawn::Hidden;
        command.hidden_with(vorn_spawn::DETACHED_PROCESS | vorn_spawn::CREATE_NEW_PROCESS_GROUP);
    }
    command.spawn().map(drop).map_err(|e| e.to_string())
}

/// A shell that waits for this process to end, then runs `then`.
#[cfg(unix)]
fn after_exit(then: &str, args: &[&Path]) -> Command {
    let mut command = vorn_spawn::command("/bin/sh");
    command
        .arg("-c")
        .arg(format!(
            "while kill -0 \"$0\" 2>/dev/null; do sleep 0.2; done; {then}"
        ))
        .arg(std::process::id().to_string())
        .args(args);
    command
}

/// Installs `staged` and starts the new app once this one exits, on
/// `data_dir` when given; the caller exits next. vornd is left running.
pub fn install(staged: &Staged, data_dir: Option<&Path>) -> Result<(), String> {
    match &staged.install {
        #[cfg(unix)]
        Install::Mac(app) => {
            swap(&staged.path, app)?;
            // The old app is now at the staged path, in the staging folder.
            let staging = staged.path.parent().unwrap_or(&staged.path);
            // `open` drops the environment, so the data directory goes as an argument.
            let mut command = match data_dir {
                Some(dir) => after_exit(
                    "rm -rf \"$1\"; open \"$2\" --args --data-dir \"$3\"",
                    &[staging, app.as_path(), dir],
                ),
                None => after_exit("rm -rf \"$1\"; open \"$2\"", &[staging, app.as_path()]),
            };
            spawn_detached(&mut command)
        }
        Install::Windows => {
            let mut command = vorn_spawn::command(&staged.path);
            command.args(["/S", "--updated", "--force-run"]);
            // The installer starts the app with its own environment, so the data directory goes there.
            if let Some(dir) = data_dir {
                command.env(crate::client::endpoint::DATA_DIR_ENV, dir);
            }
            spawn_detached(&mut command)
        }
        #[cfg(unix)]
        Install::AppImage(image) => {
            swap(&staged.path, image)?;
            let mut command = match data_dir {
                Some(dir) => after_exit("exec \"$1\" --data-dir \"$2\"", &[image.as_path(), dir]),
                None => after_exit("exec \"$1\"", &[image.as_path()]),
            };
            spawn_detached(&mut command)
        }
        _ => Err("This install does not update itself".into()),
    }
}

/// The channel `Vorn update` reads when none is named: a prerelease build
/// follows betas, as the person who installed one chose.
pub fn default_channel(current: &str) -> &'static str {
    match semver::Version::parse(current.trim_start_matches('v')) {
        Ok(v) if !v.pre.is_empty() => "beta",
        _ => "stable",
    }
}

/// What `Vorn update` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing newer on the channel.
    UpToDate,
    /// A newer release this install cannot apply itself (a .deb).
    Manual { version: String },
    /// Installed; the new app starts once this process exits.
    Installed { version: String },
}

/// Checks `channel` now and, when a newer release is there, downloads,
/// checks, stages and installs it as a restart from the window would, then
/// returns so the caller exits. `say` hears each step.
pub fn update_now(
    channel: &str,
    data_dir: Option<&Path>,
    say: &dyn Fn(&str),
) -> Result<Outcome, String> {
    let current = env!("CARGO_PKG_VERSION");
    let this = Install::current();
    let key = this
        .key()
        .ok_or("This copy of Vorn is not an installed app, so it does not update itself")?;
    say(&format!("Vorn {current}: checking the {channel} channel"));
    let feed = fetch_feed(channel)?;
    let Some(asset) = pick(&feed, key, current) else {
        return Ok(Outcome::UpToDate);
    };
    let version = feed.version;
    if this == Install::Deb {
        return Ok(Outcome::Manual { version });
    }
    let dir = cache_dir().ok_or("No cache folder to download into")?;
    say(&format!("Downloading {}", asset.name));
    let file = download(&asset, &dir, &|_| {})?;
    say(&format!("Checking Vorn {version}"));
    let staged = stage(&this, &file, &version)?;
    say(&format!("Installing Vorn {version}"));
    install(&staged, data_dir)?;
    Ok(Outcome::Installed { version })
}

/// What the person last chose, read from the config each frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prefs {
    pub channel: String,
    pub auto_download: bool,
}

impl Prefs {
    pub fn from_config(config: &Value) -> Prefs {
        let d = config.get("defaults");
        Prefs {
            channel: d
                .and_then(|d| d.get("updateChannel"))
                .and_then(Value::as_str)
                .filter(|c| *c == "beta")
                .unwrap_or("stable")
                .to_owned(),
            auto_download: d
                .and_then(|d| d.get("updateAutoDownload"))
                .and_then(Value::as_bool)
                .unwrap_or(true),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Job {
    Check,
    Download,
}

struct Inner {
    status: UpdateStatus,
    prefs: Option<Prefs>,
    job: Option<Job>,
    offered: Option<Asset>,
    offered_version: String,
    staged: Option<Staged>,
}

/// The updater's thread and what it has found, shared with the window.
pub struct Updater {
    install: Install,
    inner: Mutex<Inner>,
    signal: Condvar,
    wake: Wake,
}

impl Updater {
    /// Starts checking in the background when this install updates itself.
    pub fn start(wake: Wake) -> Arc<Updater> {
        let install = Install::current();
        let forced = std::env::var_os(FEED_ENV).is_some_and(|v| !v.is_empty());
        let status = match &install {
            Install::Dev if !forced => UpdateStatus::Unsupported,
            _ => UpdateStatus::Idle { last_checked: None },
        };
        let updater = Arc::new(Updater {
            install,
            inner: Mutex::new(Inner {
                status,
                prefs: None,
                job: None,
                offered: None,
                offered_version: String::new(),
                staged: None,
            }),
            signal: Condvar::new(),
            wake,
        });
        if updater.status() != UpdateStatus::Unsupported {
            let worker = Arc::clone(&updater);
            let _ = std::thread::Builder::new()
                .name("updater".into())
                .spawn(move || worker.run());
        }
        updater
    }

    pub fn status(&self) -> UpdateStatus {
        self.lock().status.clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn set(&self, status: UpdateStatus) {
        self.lock().status = status;
        (self.wake)();
    }

    /// The person's choices; a new channel checks again.
    pub fn set_prefs(&self, prefs: Prefs) {
        let mut inner = self.lock();
        if inner.prefs.as_ref() == Some(&prefs) {
            return;
        }
        // The first check waits for FIRST_CHECK; a new channel checks now.
        let channel_changed = inner
            .prefs
            .as_ref()
            .is_some_and(|p| p.channel != prefs.channel);
        inner.prefs = Some(prefs);
        if channel_changed {
            inner.job.get_or_insert(Job::Check);
            inner.offered = None;
        }
        self.signal.notify_all();
    }

    /// Check now.
    pub fn check(&self) {
        self.queue(Job::Check);
    }

    /// Download what the last check found.
    pub fn download(&self) {
        self.queue(Job::Download);
    }

    fn queue(&self, job: Job) {
        let mut inner = self.lock();
        if matches!(
            inner.status,
            UpdateStatus::Unsupported | UpdateStatus::Checking | UpdateStatus::Downloading { .. }
        ) {
            return;
        }
        inner.job = Some(job);
        self.signal.notify_all();
    }

    /// Installs the ready update, to restart on `data_dir`; the caller
    /// exits once this returns Ok.
    pub fn install(&self, data_dir: Option<&Path>) -> Result<(), String> {
        let staged = self.lock().staged.clone().ok_or("No update is ready")?;
        install(&staged, data_dir).inspect_err(|e| {
            self.set(UpdateStatus::Error { message: e.clone() });
        })
    }

    fn run(&self) {
        let mut next = Instant::now() + FIRST_CHECK;
        loop {
            let job = {
                let mut inner = self.lock();
                loop {
                    if inner.prefs.is_some() {
                        if let Some(job) = inner.job.take() {
                            break job;
                        }
                        if Instant::now() >= next {
                            break Job::Check;
                        }
                    }
                    let wait = next.saturating_duration_since(Instant::now());
                    inner = self
                        .signal
                        .wait_timeout(inner, wait.max(Duration::from_millis(50)))
                        .map(|(g, _)| g)
                        .unwrap_or_else(|e| e.into_inner().0);
                }
            };
            match job {
                Job::Check => {
                    next = Instant::now() + CHECK_EVERY;
                    self.run_check();
                }
                Job::Download => self.run_download(),
            }
        }
    }

    fn run_check(&self) {
        let Some(prefs) = self.lock().prefs.clone() else {
            return;
        };
        if self.lock().staged.is_some() {
            // Ready stays ready; a newer release waits for the restart.
            return;
        }
        self.set(UpdateStatus::Checking);
        let found = fetch_feed(&prefs.channel).map(|feed| {
            let asset = self
                .install
                .key()
                .and_then(|k| pick(&feed, k, env!("CARGO_PKG_VERSION")));
            (feed.version, asset)
        });
        match found {
            Err(message) => self.set(UpdateStatus::Error { message }),
            Ok((_, None)) => self.set(UpdateStatus::Idle {
                last_checked: Some(crate::state::now_ms()),
            }),
            Ok((version, Some(_))) if matches!(self.install, Install::Deb) => {
                self.set(UpdateStatus::Manual { version });
            }
            Ok((version, Some(asset))) => {
                {
                    let mut inner = self.lock();
                    inner.offered = Some(asset);
                    inner.offered_version = version.clone();
                }
                if prefs.auto_download {
                    self.run_download();
                } else {
                    self.set(UpdateStatus::Available { version });
                }
            }
        }
    }

    fn run_download(&self) {
        let (asset, version) = {
            let inner = self.lock();
            match &inner.offered {
                Some(a) => (a.clone(), inner.offered_version.clone()),
                None => return,
            }
        };
        let Some(dir) = cache_dir() else {
            return self.set(UpdateStatus::Error {
                message: "No cache folder to download into".into(),
            });
        };
        self.set(UpdateStatus::Downloading {
            version: version.clone(),
            percent: 0,
        });
        let progress = |percent| {
            self.set(UpdateStatus::Downloading {
                version: version.clone(),
                percent,
            })
        };
        let staged = download(&asset, &dir, &progress)
            .and_then(|file| stage(&self.install, &file, &version));
        match staged {
            Ok(staged) => {
                self.lock().staged = Some(staged);
                self.set(UpdateStatus::Ready { version });
            }
            Err(message) => self.set(UpdateStatus::Error { message }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn compares_versions_with_prereleases() {
        assert!(newer("0.8.0-beta.9", "0.8.0-beta.8"));
        assert!(newer("0.8.0", "0.8.0-beta.9"));
        assert!(newer("v0.9.0", "0.8.0"));
        assert!(!newer("0.8.0-beta.8", "0.8.0-beta.8"));
        assert!(!newer("0.7.4", "0.8.0-beta.8"));
        assert!(!newer("junk", "0.8.0"));
    }

    #[test]
    fn beta_takes_the_newest_release_with_a_feed() {
        let feed = |v: &str| json!({ "name": FEED_ASSET, "browser_download_url": format!("https://x/{v}/{FEED_ASSET}") });
        let releases = json!([
            { "tag_name": "v0.8.0-beta.9", "draft": false, "assets": [] },
            { "tag_name": "v0.9.0", "draft": true, "assets": [feed("0.9.0")] },
            { "tag_name": "v0.8.0-beta.8", "draft": false, "assets": [feed("0.8.0-beta.8")] },
            { "tag_name": "v0.8.0-beta.10", "draft": false, "assets": [feed("0.8.0-beta.10")] },
            { "tag_name": "v0.7.0", "draft": false, "assets": [feed("0.7.0")] }
        ]);
        assert_eq!(
            beta_feed_url(&releases).as_deref(),
            Some("https://x/0.8.0-beta.10/vorn-update.json")
        );
        assert_eq!(beta_feed_url(&json!([])), None);
    }

    #[test]
    fn picks_this_platforms_asset_only_when_newer() {
        let feed: Feed = serde_json::from_value(json!({
            "version": "0.8.0-beta.9",
            "releaseDate": "2026-01-01T00:00:00Z",
            "assets": [
                { "key": "mac-arm64", "name": "a.zip", "url": "u", "sha256": "ab", "size": 2 },
                { "key": "win-x64", "name": "b.exe", "url": "u", "sha256": "cd", "size": 2 }
            ]
        }))
        .unwrap();
        assert_eq!(
            pick(&feed, "win-x64", "0.8.0-beta.8").unwrap().name,
            "b.exe"
        );
        assert_eq!(pick(&feed, "linux-x64", "0.8.0-beta.8"), None);
        assert_eq!(pick(&feed, "win-x64", "0.8.0-beta.9"), None);
    }

    #[test]
    fn a_prerelease_build_follows_betas() {
        assert_eq!(default_channel("0.8.0-beta.9"), "beta");
        assert_eq!(default_channel("0.8.0"), "stable");
        assert_eq!(default_channel("junk"), "stable");
    }

    #[test]
    fn reads_the_team_from_codesign() {
        let out = "Executable=/A.app/Contents/MacOS/Vorn\nIdentifier=com.vorn.app\nTeamIdentifier=ABCDE12345\n";
        assert_eq!(team_id(out).as_deref(), Some("ABCDE12345"));
        assert_eq!(team_id("TeamIdentifier=not set\n"), None);
        assert_eq!(team_id(""), None);
    }

    #[test]
    fn reads_the_channel_and_auto_download() {
        assert_eq!(
            Prefs::from_config(&json!({})),
            Prefs {
                channel: "stable".into(),
                auto_download: true
            }
        );
        let p = Prefs::from_config(
            &json!({ "defaults": { "updateChannel": "beta", "updateAutoDownload": false } }),
        );
        assert_eq!(p.channel, "beta");
        assert!(!p.auto_download);
    }

    #[test]
    fn tells_a_build_tree_from_an_install() {
        let d = std::env::temp_dir().join(format!("vorn-update-install-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        assert_eq!(Install::of(&d.join("vorn-app"), None), Install::Dev);
        if cfg!(target_os = "macos") {
            let app = d.join("Vorn.app");
            std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
            std::fs::create_dir_all(app.join("Contents/Resources/vornd")).unwrap();
            assert_eq!(
                Install::of(&app.join("Contents/MacOS/Vorn"), None),
                Install::Mac(app.clone())
            );
        } else {
            std::fs::create_dir_all(d.join("resources/vornd")).unwrap();
            let image = d.join("Vorn.AppImage");
            std::fs::write(&image, "").unwrap();
            let got = Install::of(&d.join("vorn"), Some(image.clone()));
            if cfg!(windows) {
                assert_eq!(got, Install::Windows);
            } else {
                assert_eq!(got, Install::AppImage(image));
                assert_eq!(Install::of(&d.join("vorn"), None), Install::Deb);
            }
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn downloads_and_checks_the_sum() {
        let body = b"the new app".to_vec();
        let sum = hex(&Sha256::digest(&body));
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let served = body.clone();
        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut s, _) = listener.accept().unwrap();
                let mut buf = [0u8; 1024];
                let _ = s.read(&mut buf);
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    served.len()
                );
                s.write_all(head.as_bytes()).unwrap();
                s.write_all(&served).unwrap();
            }
        });
        let dir = std::env::temp_dir().join(format!("vorn-update-dl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let asset = Asset {
            key: "mac-arm64".into(),
            name: "Vorn.zip".into(),
            url: format!("http://127.0.0.1:{port}/Vorn.zip"),
            sha256: sum,
            size: body.len() as u64,
        };
        let seen = Mutex::new(Vec::new());
        let got = download(&asset, &dir, &|p| seen.lock().unwrap().push(p)).unwrap();
        assert_eq!(std::fs::read(&got).unwrap(), body);
        assert_eq!(seen.lock().unwrap().last(), Some(&100));

        let bad = Asset {
            name: "Other.zip".into(),
            sha256: "00".repeat(32),
            ..asset
        };
        let err = download(&bad, &dir, &|_| {}).unwrap_err();
        assert!(err.contains("checksum"), "{err}");
        assert!(!dir.join("Other.zip").exists());
        server.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
