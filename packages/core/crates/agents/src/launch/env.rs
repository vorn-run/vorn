//! Which variables reach what a session runs (`filterEnv`, `getSafeEnv`,
//! `getLaunchEnv`), as rules over a given environment. Where that
//! environment comes from (the login shell's answer, or the host's own until
//! it has one) is the host's business.

/// Names stripped whatever the configuration says: the markers an agent CLI
/// leaves to describe the session it is inside (`NEVER_BORROWED_ENV`), and
/// the desktop's launch credential (`BOOTSTRAP_ENV_VAR`).
pub const STRIP_KEYS: &[&str] = &["CLAUDECODE"];
pub const STRIP_PREFIXES: &[&str] = &["CLAUDE_CODE_", "SECRET_VORN_BOOTSTRAP_TOKEN"];

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

/// `normalizePassthrough`: the names trimmed and uppercased, blanks dropped.
pub fn normalize_passthrough<S: AsRef<str>>(names: &[S]) -> Vec<String> {
    names
        .iter()
        .map(|k| k.as_ref().trim().to_uppercase())
        .filter(|k| !k.is_empty())
        .collect()
}

/// Whether a name is stripped no matter who asks
/// (`isAbsolutelyStrippedEnvName`). Compared uppercased: Windows names are
/// case-insensitive, and this list is meant to be absolute.
pub fn is_stripped(name: &str) -> bool {
    let upper = name.to_uppercase();
    STRIP_KEYS.contains(&upper.as_str()) || STRIP_PREFIXES.iter().any(|p| upper.starts_with(p))
}

/// `filterEnv`: `passthrough`, normalized names, lets those credential-shaped
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

/// `getSafeEnv` over `source`: for what the person never asked about (git,
/// detection, helpers), so nothing is passed through.
pub fn safe_env(source: impl IntoIterator<Item = (String, String)>) -> Env {
    filter_env(source, &[])
}

/// `getLaunchEnv` over `source`: for an agent terminal, a headless agent or
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
    fn drops_credentials_and_agent_markers_whatever_their_case() {
        let kept = safe_env(pairs(&[
            ("PATH", "/bin"),
            ("github_token_extra", "x"),
            ("SECRET_VORN_BOOTSTRAP_TOKEN", "x"),
            ("ClaudeCode", "1"),
            ("CLAUDE_CODE_SSE_PORT", "1"),
            ("ANTHROPIC_BASE_URL", "u"),
            ("ANTHROPIC_API_KEY", "k"),
        ]));
        assert_eq!(names(&kept), ["PATH", "ANTHROPIC_BASE_URL"]);
    }

    #[test]
    fn launches_with_what_was_passed_through_and_the_data_dir() {
        let source = pairs(&[
            ("VORN_DATA_DIR", "old"),
            ("anthropic_api_key", "k"),
            ("CLAUDECODE", "1"),
            ("PATH", "/bin"),
        ]);
        let passthrough = [" anthropic_api_key ".to_owned(), "claudecode".to_owned()];
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
