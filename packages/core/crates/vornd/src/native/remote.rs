//! Where a call's path is, and how to reach it there: on this machine, or
//! in a project on a remote host, logged in to over ssh as the server's
//! `sshExec` does (`vorn_remote::Login`).

use std::path::PathBuf;

use vorn_git::repo::Git;
use vorn_store::Placement;

use super::Native;

/// Where a path is: here, or on the remote host with this login.
#[derive(Clone, Debug)]
pub enum Place {
    Local,
    Remote {
        host: String,
        login: vorn_remote::Login,
    },
}

impl Place {
    /// The git that runs there.
    pub fn git(&self, native: &Native) -> Git {
        Git {
            bin: native.env.git_bin(),
            env: native.env.get(),
            ssh: match self {
                Place::Local => None,
                Place::Remote { login, .. } => Some(login.clone()),
            },
        }
    }

    /// What the changes to a repository there take turns under.
    pub fn turn(&self, path: &str) -> PathBuf {
        match self {
            Place::Local => PathBuf::from(path),
            Place::Remote { host, .. } => PathBuf::from(format!("{host}:{path}")),
        }
    }

    pub fn login(&self) -> Option<&vorn_remote::Login> {
        match self {
            Place::Local => None,
            Place::Remote { login, .. } => Some(login),
        }
    }
}

impl Native {
    /// How remote host `id` is logged in to; `None` when there is no such host.
    pub(crate) fn login(&self, id: &str) -> Option<vorn_remote::Login> {
        let db = self.db.get()?;
        let host = vorn_store::remote_host(db, id).ok()??;
        let host = vorn_remote::Host {
            hostname: host.hostname,
            user: host.user,
            port: host.port,
            ssh_key_path: host.ssh_key_path,
            ssh_options: host.ssh_options,
        };
        Some(vorn_remote::Login::new(&host, self.env.get()))
    }

    fn place_of(&self, placement: Option<Placement>) -> Place {
        match placement {
            Some(Placement::Remote(id)) => match self.login(&id) {
                Some(login) => Place::Remote { host: id, login },
                None => Place::Local,
            },
            _ => Place::Local,
        }
    }

    /// Where the project at exactly `path` is (`resolveRemoteHost`).
    pub(crate) fn project_place(&self, path: &str) -> Place {
        self.place_of(self.hosts().map(|h| h.for_project(path)))
    }

    /// Where any path is, a worktree's included (`resolveRemoteHostByPath`).
    pub(crate) fn path_place(&self, path: &str) -> Place {
        self.place_of(self.hosts().map(|h| h.for_path(path)))
    }

    /// The host a call names by id (`remoteHostId`), when it names one.
    pub(crate) fn host_place(&self, id: &str) -> Place {
        self.place_of(Some(Placement::Remote(id.to_owned())))
    }
}
