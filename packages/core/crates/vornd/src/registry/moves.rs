//! A worktree's branch renamed, or the worktree moved, by vornd for a
//! client's call (`git:renameWorktreeBranch`, `git:renameWorktree`): the
//! sessions in it take the new branch or path and name, as the server's
//! `updateSessionsForWorktree` sets them, and each is told whole, marked
//! `moved`, for the server to tell its clients.

use serde_json::json;

use super::{Registry, RegistryError, Value};

/// What changed about a worktree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorktreeMove {
    /// Its branch, as the client named it: the server keeps the name
    /// untrimmed in the record, though git was given it trimmed.
    Branch(String),
    /// It was moved to `path`, and is now called `name`.
    Path { path: String, name: String },
}

impl WorktreeMove {
    fn apply(
        &self,
        branch: &mut Option<String>,
        worktree_path: &mut Option<String>,
        worktree_name: &mut Option<String>,
    ) {
        match self {
            WorktreeMove::Branch(b) => *branch = Some(b.clone()),
            WorktreeMove::Path { path, name } => {
                *worktree_path = Some(path.clone());
                *worktree_name = Some(name.clone());
            }
        }
    }
}

impl Registry {
    /// Sets what `moved` says on every terminal, then every headless agent,
    /// whose worktree is `path`, in the order the server holds them. Each
    /// is told, changed or not, as the server tells each.
    pub fn move_worktree(
        &mut self,
        path: &str,
        moved: &WorktreeMove,
    ) -> Result<Vec<Value>, RegistryError> {
        if !self.decides() {
            let call = match moved {
                WorktreeMove::Branch(_) => "git:renameWorktreeBranch",
                WorktreeMove::Path { .. } => "git:renameWorktree",
            };
            return Err(RegistryError::NotDeciding { call });
        }
        let here = |p: &Option<String>| p.as_deref() == Some(path);
        let mut terminals = Vec::new();
        for row in &mut self.terminals.rows {
            let r = &mut row.record;
            if here(&r.worktree_path) {
                moved.apply(&mut r.branch, &mut r.worktree_path, &mut r.worktree_name);
                terminals.push(r.id.clone());
            }
        }
        let mut headless = Vec::new();
        for row in &mut self.headless.rows {
            let r = &mut row.record;
            if here(&r.worktree_path) {
                moved.apply(&mut r.branch, &mut r.worktree_path, &mut r.worktree_name);
                headless.push(r.id.clone());
            }
        }
        let mut notes = Vec::with_capacity(terminals.len() + headless.len());
        for id in terminals {
            notes.extend(self.native_upsert(&id, json!({ "moved": true })));
        }
        for id in headless {
            notes.extend(self.native_upsert_headless(&id, json!({ "moved": true })));
        }
        Ok(notes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{Change, Gen};

    const OLD: &str = "/w/old-1a2b3c4d";

    fn registry() -> Registry {
        let terminal = |id: &str, wt: Option<&str>| {
            json!({
                "id": id, "agentType": "claude", "projectName": "p", "projectPath": "/p",
                "status": "running", "createdAt": 1, "pid": 0,
                "worktreePath": wt, "worktreeName": "old", "branch": "main",
            })
        };
        let headless = |id: &str, wt: &str| {
            json!({
                "id": id, "pid": 7, "agentType": "claude", "projectName": "p", "projectPath": "/p",
                "status": "running", "startedAt": 1, "worktreePath": wt,
            })
        };
        let snapshot = json!({
            "op": "snapshot",
            "terminals": [terminal("b", Some(OLD)), terminal("x", None), terminal("a", Some(OLD))],
            "headless": [headless("h", OLD), headless("o", "/w/other")],
        });
        let mut r = Registry::new(Gen::draw());
        r.apply(Change::try_from(&snapshot).unwrap());
        r
    }

    /// Each note's kind and record, after checking it is marked as a move.
    fn told(notes: &[Value]) -> Vec<(String, Value)> {
        notes
            .iter()
            .map(|n| {
                assert_eq!((&n["native"], &n["moved"]), (&json!(true), &json!(true)));
                (n["kind"].as_str().unwrap().to_owned(), n["record"].clone())
            })
            .collect()
    }

    #[test]
    fn moves_every_session_in_the_worktree_and_tells_each() {
        let mut r = registry();
        let moved = WorktreeMove::Path {
            path: "/w/new-1a2b3c4d".into(),
            name: "new".into(),
        };
        assert!(matches!(
            r.move_worktree(OLD, &moved),
            Err(RegistryError::NotDeciding { .. })
        ));
        r.decide_statuses();
        let rev = r.rev.0;
        let told_now = told(&r.move_worktree(OLD, &moved).unwrap());
        let ids: Vec<_> = told_now
            .iter()
            .map(|(k, rec)| format!("{k}:{}", rec["id"].as_str().unwrap()))
            .collect();
        assert_eq!(ids, ["terminal:b", "terminal:a", "headless:h"]);
        for (_, rec) in &told_now {
            assert_eq!(rec["worktreePath"], "/w/new-1a2b3c4d");
            assert_eq!(rec["worktreeName"], "new");
        }
        assert_eq!(told_now[0].1["branch"], "main");
        assert_eq!(r.rev.0, rev + 3);
        assert_eq!(r.terminals.get("x").unwrap().record.worktree_path, None);

        // Renamed again: the sessions are found at their new path.
        let branch = WorktreeMove::Branch(" feature ".into());
        let told_now = told(&r.move_worktree("/w/new-1a2b3c4d", &branch).unwrap());
        assert_eq!(told_now.len(), 3);
        assert!(told_now.iter().all(|(_, rec)| rec["branch"] == " feature "));
        assert!(r.move_worktree(OLD, &branch).unwrap().is_empty());
    }
}
