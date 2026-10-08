//! Where an extension's contributions show: its own rule first, then each
//! contribution's.

use std::path::Path;
use std::sync::OnceLock;

use serde::Serialize;

use crate::js;
use crate::manifest::{Activation, Contribution};
use crate::pack::InstalledPack;

/// The session a rule is read against.
#[derive(Debug, Clone)]
pub struct Subject<'a> {
    pub worktree: &'a Path,
    pub agent: &'a str,
    /// `darwin`, `linux` or `win32`, as Node names this machine.
    pub platform: &'a str,
    /// The origin remote's host, lowercase; `None` when it could not be read,
    /// which widens a rule naming one rather than hiding the contribution.
    pub remote_host: Option<&'a str>,
}

/// What an extension shows on one session, by contribution id.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Shown {
    pub active: bool,
    pub panes: Vec<String>,
    pub footers: Vec<String>,
    pub link_handlers: Vec<String>,
}

/// This machine's platform as Node names it.
pub fn platform() -> &'static str {
    match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    }
}

/// Whether any of `packs` has a rule naming a remote host, so the remote
/// is read only when something will look at it.
pub fn names_remote_host(packs: &[InstalledPack]) -> bool {
    packs.iter().any(|pack| {
        let rules = pack.contributions().into_iter().flat_map(|c| c.rules());
        std::iter::once(pack.activates.as_ref())
            .chain(rules)
            .any(|rule| rule.is_some_and(|r| !r.remote_host.is_empty()))
    })
}

fn contains_any(worktree: &Path, paths: &[String]) -> bool {
    paths.iter().any(|entry| {
        let escapes = entry.is_empty()
            || entry.starts_with(['/', '\\'])
            || Path::new(entry).is_absolute()
            || entry.split(['/', '\\']).any(|s| s == "..");
        !escapes && worktree.join(entry).exists()
    })
}

/// Whether `rule` holds for `subject`; no rule always holds.
pub fn matches(rule: Option<&Activation>, subject: &Subject<'_>) -> bool {
    let Some(rule) = rule else { return true };
    if !rule.workspace_contains.is_empty()
        && !contains_any(subject.worktree, &rule.workspace_contains)
    {
        return false;
    }
    if !rule.agent.is_empty() && !rule.agent.iter().any(|a| a == subject.agent) {
        return false;
    }
    if !rule.platform.is_empty() && !rule.platform.iter().any(|p| p == subject.platform) {
        return false;
    }
    if let (false, Some(host)) = (rule.remote_host.is_empty(), subject.remote_host) {
        if !rule.remote_host.iter().any(|n| n.to_lowercase() == host) {
            return false;
        }
    }
    true
}

fn shown<'a>(
    contributions: impl Iterator<Item = &'a Contribution>,
    subject: &Subject<'_>,
) -> Vec<String> {
    contributions
        .filter(|c| matches(c.when.as_ref(), subject))
        .map(|c| c.id.clone())
        .collect()
}

/// What `pack` shows on `subject`; nothing for a connector or when its own
/// rule does not hold.
pub fn shown_on(pack: &InstalledPack, subject: &Subject<'_>) -> Shown {
    if !pack.is_extension() || !matches(pack.activates.as_ref(), subject) {
        return Shown::default();
    }
    let Some(c) = pack.contributions() else {
        return Shown {
            active: true,
            ..Shown::default()
        };
    };
    Shown {
        active: true,
        panes: shown(c.panes().iter().map(|p| &p.base), subject),
        footers: shown(c.footers().iter().map(|f| &f.base), subject),
        link_handlers: shown(c.link_handlers().iter().map(|l| &l.base), subject),
    }
}

/// The host `git remote get-url origin` names: the scp form's host, or the
/// URL's; lowercase, `None` when there is none.
pub fn remote_host_of(url: &str) -> Option<String> {
    static SCP: OnceLock<regress::Regex> = OnceLock::new();
    let url = js::trim(url);
    if url.is_empty() {
        return None;
    }
    let scp = SCP.get_or_init(|| js::regex(r"^(?:[^@/\s]+@)?([A-Za-z0-9._-]+):(?!\/)"));
    if let Some(m) = scp.find(url) {
        let host = m.group(1).map(|r| &url[r])?;
        return Some(host.to_lowercase());
    }
    url_host(url)
}

/// The host of a URL with an authority, as `new URL(url).hostname`.
fn url_host(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let valid_scheme = scheme
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    if !valid_scheme {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = if host_port.starts_with('[') {
        host_port.split_inclusive(']').next().unwrap_or_default()
    } else {
        host_port.split(':').next().unwrap_or_default()
    };
    (!host.is_empty()).then(|| host.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::{tests::install, PackStore};
    use serde_json::json;

    fn rule(v: serde_json::Value) -> Activation {
        crate::manifest::activation(Some(&v)).unwrap()
    }

    fn subject<'a>(worktree: &'a Path, remote: Option<&'a str>) -> Subject<'a> {
        Subject {
            worktree,
            agent: "claude",
            platform: "linux",
            remote_host: remote,
        }
    }

    #[test]
    fn reads_workspace_contains_as_a_path_really_there() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "").unwrap();
        let s = subject(dir.path(), None);
        assert!(matches(
            Some(&rule(json!({ "workspaceContains": ["x", "Cargo.toml"] }))),
            &s
        ));
        assert!(!matches(
            Some(&rule(json!({ "workspaceContains": ["package.json"] }))),
            &s
        ));
        let mut escaping = rule(json!({ "workspaceContains": ["x"] }));
        escaping.workspace_contains = vec!["..\\Cargo.toml".into(), String::new(), "/etc".into()];
        assert!(!matches(Some(&escaping), &s));
    }

    #[test]
    fn matches_agent_platform_and_remote() {
        let dir = Path::new("/");
        let s = subject(dir, Some("github.com"));
        assert!(matches(
            Some(&rule(json!({ "agent": ["codex", "claude"] }))),
            &s
        ));
        assert!(!matches(Some(&rule(json!({ "agent": ["codex"] }))), &s));
        assert!(!matches(Some(&rule(json!({ "platform": ["darwin"] }))), &s));
        assert!(matches(
            Some(&rule(json!({ "remoteHost": ["GitHub.com"] }))),
            &s
        ));
        assert!(!matches(
            Some(&rule(json!({ "remoteHost": ["gitlab.com"] }))),
            &s
        ));
        assert!(matches(
            Some(&rule(json!({ "remoteHost": ["gitlab.com"] }))),
            &subject(dir, None)
        ));
        assert!(matches(None, &s));
    }

    #[test]
    fn shows_what_each_rule_lets_through() {
        let root = tempfile::tempdir().unwrap();
        install(
            root.path(),
            "x",
            "1.0.0",
            &json!({ "id": "x", "name": "X", "kind": "extension", "contributes": {
                "panes": [{ "id": "p", "web": "web/a.html" }, { "id": "q", "command": ["q"], "when": { "agent": ["codex"] } }],
                "footers": [{ "id": "f", "every": 5, "when": { "remoteHost": ["github.com"] } }],
                "linkHandlers": [{ "id": "l", "pattern": "a" }]
            } }),
        );
        install(
            root.path(),
            "y",
            "1.0.0",
            &json!({ "id": "y", "name": "Y", "kind": "extension", "activates": { "agent": ["codex"] },
                     "contributes": { "panes": [{ "id": "p", "web": "web/a.html" }] } }),
        );
        install(
            root.path(),
            "c",
            "1.0.0",
            &json!({ "id": "c", "name": "C", "actions": [{ "type": "a" }] }),
        );
        let store = PackStore::new(root.path());
        let packs = store.list();
        assert!(names_remote_host(&packs));
        let s = subject(Path::new("/"), Some("gitlab.com"));
        let shown: Vec<Shown> = packs.iter().map(|p| shown_on(p, &s)).collect();
        assert_eq!(shown[0], Shown::default());
        assert_eq!(
            shown[1],
            Shown {
                active: true,
                panes: vec!["p".into()],
                footers: vec![],
                link_handlers: vec!["l".into()]
            }
        );
        assert_eq!(shown[2], Shown::default());
        assert!(!names_remote_host(&packs[2..]));
    }

    #[test]
    fn reads_the_host_a_remote_names() {
        assert_eq!(
            remote_host_of("git@GitHub.com:a/b.git\n").as_deref(),
            Some("github.com")
        );
        assert_eq!(
            remote_host_of("host.example:repo").as_deref(),
            Some("host.example")
        );
        assert_eq!(
            remote_host_of("https://u@GitLab.com:8443/a/b").as_deref(),
            Some("gitlab.com")
        );
        assert_eq!(
            remote_host_of("ssh://git@host.x/a").as_deref(),
            Some("host.x")
        );
        assert_eq!(remote_host_of("/srv/repo.git"), None);
        assert_eq!(remote_host_of(""), None);
    }
}
