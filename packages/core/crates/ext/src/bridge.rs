//! The bridge: every call an extension makes back into Vorn, in either tier.
//!
//! [`Gate`] is the one place the manifest is enforced: the hosts `net.fetch`
//! may reach, the methods `host.call` may use, and the credential added to a
//! request. A component reaches it through its imports, a quick extension's
//! service through stdio; both get the same answers. What lies behind the gate
//! (the network, git, the vault) is a [`World`], so checks can record and
//! replay it.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::manifest::{AuthRung, Manifest};
use crate::ExtError;

/// An HTTP request as the WIT's `request` carries it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FetchRequest {
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    #[serde(default)]
    pub body: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FetchResponse {
    pub status: u16,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    pub body: String,
}

/// What sits behind the gate. vornd's is live; checks record and replay one.
pub trait World: Send + Sync {
    /// Sends a request the gate already allowed and signed.
    fn fetch(&self, req: &FetchRequest) -> Result<FetchResponse, ExtError>;
    /// Answers a core method the gate already allowed, for `project`.
    fn call(&self, method: &str, args: &Value, project: &str) -> Result<Value, ExtError>;
    /// The credential for `ext` per its auth rungs; `None` when no rung has one.
    fn credential(&self, ext: &Manifest) -> Result<Option<String>, ExtError>;
}

/// Core methods a guest may call, and the permission each needs.
pub const METHODS: &[(&str, &str)] = &[("git.branch", "git.read"), ("git.remote", "git.read")];

/// One extension's view of the world, held to its manifest.
pub struct Gate<'a> {
    pub manifest: &'a Manifest,
    pub world: &'a dyn World,
    pub project: &'a str,
}

impl Gate<'_> {
    /// `net.fetch`: https only, to a host the manifest names, signed with the
    /// extension's credential. A guest's own `authorization` header is dropped,
    /// so what reaches a host is always what the install sheet showed.
    pub fn fetch(&self, req: &FetchRequest) -> Result<FetchResponse, ExtError> {
        let host = https_host(&req.url)
            .ok_or_else(|| ExtError::new("denied", format!("{} is not an https URL", req.url)))?;
        if !self.manifest.network.iter().any(|h| h == host) {
            return Err(ExtError::new(
                "denied",
                format!("{host} is not in the manifest's network list"),
            ));
        }
        let mut signed = FetchRequest {
            method: req.method.clone(),
            url: req.url.clone(),
            headers: req
                .headers
                .iter()
                .filter(|(n, _)| !n.eq_ignore_ascii_case("authorization"))
                .cloned()
                .collect(),
            body: req.body.clone(),
        };
        if let Some(token) = self.world.credential(self.manifest)? {
            if let Some((name, value)) = self
                .manifest
                .auth
                .first()
                .map(AuthRung::header)
                .and_then(|h| h.split_once(':'))
            {
                signed.headers.push((
                    name.trim().to_owned(),
                    value.trim().replace("{token}", &token),
                ));
            }
        }
        self.world.fetch(&signed)
    }

    /// `host.call`: only the methods [`METHODS`] lists, under their permission.
    pub fn call(&self, method: &str, args: &Value) -> Result<Value, ExtError> {
        let Some((_, permission)) = METHODS.iter().find(|(m, _)| *m == method) else {
            return Err(ExtError::new(
                "unknown-method",
                format!("Vorn has no method {method}"),
            ));
        };
        if !self.manifest.allows(permission) {
            return Err(ExtError::new(
                "denied",
                format!("{method} needs the {permission} permission"),
            ));
        }
        self.world.call(method, args, self.project)
    }
}

/// The host of an `https://` URL, without a port; `None` for anything else,
/// or a URL carrying credentials of its own.
pub fn https_host(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("https://")?;
    let authority = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
    if authority.contains('@') || authority.is_empty() {
        return None;
    }
    Some(authority.split(':').next().unwrap_or(authority))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Seen(Mutex<Vec<FetchRequest>>);

    impl World for Seen {
        fn fetch(&self, req: &FetchRequest) -> Result<FetchResponse, ExtError> {
            self.0.lock().unwrap().push(req.clone());
            Ok(FetchResponse {
                status: 200,
                headers: vec![],
                body: "{}".into(),
            })
        }
        fn call(&self, _: &str, _: &Value, project: &str) -> Result<Value, ExtError> {
            Ok(Value::String(project.into()))
        }
        fn credential(&self, _: &Manifest) -> Result<Option<String>, ExtError> {
            Ok(Some("t0ken".into()))
        }
    }

    fn manifest() -> Manifest {
        Manifest::parse(
            r#"id = "x"
name = "X"
version = "1"
api = "vorn:extension@0.9"
permissions = ["git.read"]
network = ["api.github.com"]
runtime = { component = "x.cwasm" }
auth = [{ rung = "vault", field = "token", header = "Authorization: Bearer {token}" }]
"#,
        )
        .unwrap()
    }

    fn get(url: &str) -> FetchRequest {
        FetchRequest {
            method: "GET".into(),
            url: url.into(),
            headers: vec![("Authorization".into(), "Bearer stolen".into())],
            body: None,
        }
    }

    #[test]
    fn signs_a_request_to_a_declared_host_with_the_credential_not_the_guests() {
        let world = Seen::default();
        let m = manifest();
        let gate = Gate {
            manifest: &m,
            world: &world,
            project: "/p",
        };
        gate.fetch(&get("https://api.github.com/repos/a/b"))
            .unwrap();
        let sent = world.0.lock().unwrap();
        assert_eq!(
            sent[0].headers,
            [("Authorization".to_owned(), "Bearer t0ken".to_owned())]
        );
    }

    #[test]
    fn refuses_hosts_the_manifest_does_not_name() {
        let world = Seen::default();
        let m = manifest();
        let gate = Gate {
            manifest: &m,
            world: &world,
            project: "/p",
        };
        for url in [
            "https://evil.example/x",
            "http://api.github.com/x",
            "https://api.github.com@evil.example/x",
            "https://api.github.com.evil.example/",
        ] {
            assert_eq!(gate.fetch(&get(url)).unwrap_err().code, "denied", "{url}");
        }
        assert!(world.0.lock().unwrap().is_empty());
    }

    #[test]
    fn holds_core_calls_to_the_permissions() {
        let world = Seen::default();
        let mut m = manifest();
        let gate = Gate {
            manifest: &m,
            world: &world,
            project: "/p",
        };
        assert_eq!(gate.call("git.branch", &Value::Null).unwrap(), "/p");
        assert_eq!(
            gate.call("fs.read", &Value::Null).unwrap_err().code,
            "unknown-method"
        );
        m.permissions.clear();
        let gate = Gate {
            manifest: &m,
            world: &world,
            project: "/p",
        };
        assert_eq!(
            gate.call("git.branch", &Value::Null).unwrap_err().code,
            "denied"
        );
    }

    #[test]
    fn reads_the_host_of_an_https_url() {
        assert_eq!(
            https_host("https://api.github.com:443/x?y"),
            Some("api.github.com")
        );
        assert_eq!(https_host("https://a.b"), Some("a.b"));
        assert_eq!(https_host("https:///x"), None);
    }
}
