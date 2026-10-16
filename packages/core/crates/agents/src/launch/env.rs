//! Which variables reach what a session runs, as rules over a given
//! environment. Where that environment comes from (the login shell's answer,
//! or the host's own until it has one) is the host's business.

use std::sync::{PoisonError, RwLock};

/// The desktop's launch credential, stripped whatever
/// the configuration says.
pub const STRIP_PREFIXES: &[&str] = &["SECRET_VORN_BOOTSTRAP_TOKEN"];

/// The markers agent CLIs leave to describe the session they are inside
/// (`NEVER_BORROWED_ENV`), as their adapters declare them (`[agent.env]`):
/// whole names, and prefixes.
static MARKERS: RwLock<(Vec<String>, Vec<String>)> = RwLock::new((Vec::new(), Vec::new()));

/// Strips `names` and every name starting with one of `prefixes` from now on,
/// for every process: a marker of an adapter that is switched off still
/// describes a session Vorn may itself be running inside.
pub fn add_markers(names: &[String], prefixes: &[String]) {
    let mut markers = MARKERS.write().unwrap_or_else(PoisonError::into_inner);
    let (keys, starts) = &mut *markers;
    for (have, add) in [(keys, names), (starts, prefixes)] {
        for name in add {
            let upper = name.to_uppercase();
            if !upper.is_empty() && !have.contains(&upper) {
                have.push(upper);
            }
        }
    }
}

/// Credential-shaped names (`SENSITIVE_ENV_PREFIXES`), dropped unless the
/// person passed one through by name.
pub const SENSITIVE_PREFIXES: &[&str] = &[
    "AWS_SECRET",
    "AWS_SESSION",
    "GITHUB_TOKEN",
    "GH_TOKEN",
    "OPENAI_API",
    "ANTHROPIC_API",
    "GOOGLE_API",
    "STRIPE_",
    "DATABASE_URL",
    "DB_PASSWORD",
    "SECRET_",
    "PRIVATE_KEY",
    "NPM_TOKEN",
    "NODE_AUTH_TOKEN",
];

/// An environment as name and value pairs, in the order they were given.
pub type Env = Vec<(String, String)>;

/// The passthrough names trimmed and uppercased, blanks dropped.
pub fn normalize_passthrough<S: AsRef<str>>(names: &[S]) -> Vec<String> {
    names
        .iter()
        .map(|k| k.as_ref().trim().to_uppercase())
        .filter(|k| !k.is_empty())
        .collect()
}

/// Whether a name is stripped no matter who asks. Compared uppercased: Windows names are
/// case-insensitive, and this list is meant to be absolute.
pub fn is_stripped(name: &str) -> bool {
    let upper = name.to_uppercase();
    if STRIP_PREFIXES.iter().any(|p| upper.starts_with(p)) {
        return true;
    }
    let markers = MARKERS.read().unwrap_or_else(PoisonError::into_inner);
    markers.0.contains(&upper) || markers.1.iter().any(|p| upper.starts_with(p.as_str()))
}

/// `source` filtered: `passthrough`, normalized names, lets those credential-shaped
/// names through, but never one [`is_stripped`] refuses.
pub fn filter_env(
    source: impl IntoIterator<Item = (String, String)>,
    passthrough: &[String],
) -> Env {
    source
        .into_iter()
        .filter(|(key, _)| {
            let upper = key.to_uppercase();
            let sensitive = !passthrough.contains(&upper)
                && SENSITIVE_PREFIXES.iter().any(|p| upper.starts_with(p));
            !(is_stripped(key) || sensitive)
        })
        .collect()
}

/// [`filter_env`] with no passthrough: for what the person never asked about (git,
/// detection, helpers), so nothing is passed through.
pub fn safe_env(source: impl IntoIterator<Item = (String, String)>) -> Env {
    filter_env(source, &[])
}

/// [`filter_env`] over `source` for an agent terminal, a headless agent or
/// a workflow script, the only launches that forward the configured
/// passthrough names; with `VORN_DATA_DIR` naming the server's data
/// directory when it has one, where an existing entry keeps its place.
pub fn launch_env(
    source: impl IntoIterator<Item = (String, String)>,
    passthrough: &[String],
    data_dir: Option<&str>,
) -> Env {
    let mut env = filter_env(source, &normalize_passthrough(passthrough));
    if let Some(dir) = data_dir.filter(|d| !d.is_empty()) {
        match env.iter_mut().find(|(k, _)| k == "VORN_DATA_DIR") {
            Some(slot) => dir.clone_into(&mut slot.1),
            None => env.push(("VORN_DATA_DIR".to_owned(), dir.to_owned())),
        }
    }
    env
}

/// The value of `name` in `env`, as an object lookup reads it: the last
/// entry wins.
pub fn lookup<'a>(env: &'a [(String, String)], name: &str) -> Option<&'a str> {
    env.iter()
        .rev()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(list: &[(&str, &str)]) -> Env {
        list.iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    fn names(env: &Env) -> Vec<&str> {
        env.iter().map(|(k, _)| k.as_str()).collect()
    }

    #[test]
    fn markers() {
        add_markers(&["AGENTMARK".to_owned()], &["agent_mark_".to_owned()]);
    }

    #[test]
    fn drops_credentials_and_agent_markers_whatever_their_case() {
        markers();
        let kept = safe_env(pairs(&[
            ("PATH", "/bin"),
            ("github_token_extra", "x"),
            ("SECRET_VORN_BOOTSTRAP_TOKEN", "x"),
            ("AgentMark", "1"),
            ("AGENT_MARK_PORT", "1"),
            ("ANTHROPIC_BASE_URL", "u"),
            ("ANTHROPIC_API_KEY", "k"),
        ]));
        assert_eq!(names(&kept), ["PATH", "ANTHROPIC_BASE_URL"]);
    }

    #[test]
    fn launches_with_what_was_passed_through_and_the_data_dir() {
        markers();
        let source = pairs(&[
            ("VORN_DATA_DIR", "old"),
            ("anthropic_api_key", "k"),
            ("AGENTMARK", "1"),
            ("PATH", "/bin"),
        ]);
        let passthrough = [" anthropic_api_key ".to_owned(), "agentmark".to_owned()];
        let env = launch_env(source, &passthrough, Some("/data"));
        assert_eq!(
            env,
            pairs(&[
                ("VORN_DATA_DIR", "/data"),
                ("anthropic_api_key", "k"),
                ("PATH", "/bin")
            ])
        );
        let env = launch_env(pairs(&[("PATH", "/bin")]), &[], Some(""));
        assert_eq!(names(&env), ["PATH"]);
    }
}
