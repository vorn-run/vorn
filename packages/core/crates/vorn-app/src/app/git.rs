//! The git controls: diff polling, the right panel, commits, the branch
//! switcher and open-in, with vornd answering the git.

use web_time::Instant;

use serde_json::{json, Value};

use super::{list, App, Call, Src};
use crate::client::rpc::RpcError;
use crate::git::{Branches, CommitOp, DiffOp, GitAct, Ide, Stat};
use crate::screen::git::{branch_filter_id, commit_id};
use crate::state::{Dialog, MenuKind};
use crate::tasks::{self, Comment, Commit, Diff, DiffFile};
use crate::ui::Ui;

/// A git call whose answer changes the screen.
pub(super) enum GitCall {
    Stat(String),
    Branch(String),
    Diff(String),
    Committed { cwd: String, push: bool },
    Pushed,
    Branches(String),
    Remotes,
    Switched { cwd: String, branch: String },
    Ides,
}

pub(super) use crate::git::session_cwd as cwd_of;

/// The answer a commit or push gives: ok, or why not.
fn done(answer: Result<Value, RpcError>, fallback: &str) -> Result<(), String> {
    let v = answer.map_err(|e| e.to_string())?;
    if v.get("success").and_then(Value::as_bool) == Some(true)
        || v.get("ok").and_then(Value::as_bool) == Some(true)
    {
        return Ok(());
    }
    Err(v
        .get("error")
        .and_then(Value::as_str)
        .filter(|e| !e.is_empty())
        .unwrap_or(fallback)
        .to_owned())
}

impl App {
    /// The poll.
    pub(super) fn git_tick(&mut self) {
        let now = Instant::now();
        if self.st.git.due(now) && !self.store.sessions.is_empty() {
            self.st.git.polled = Some(now);
            let live: Vec<(String, String)> = self
                .store
                .sessions
                .iter()
                .filter(|s| !s.ended())
                .map(|s| (s.info.id.clone(), cwd_of(&s.info)))
                .filter(|(_, cwd)| !cwd.is_empty())
                .collect();
            let stale: std::collections::BTreeSet<String> = self
                .store
                .sessions
                .iter()
                .filter_map(|x| {
                    let wt = x.info.worktree_path.as_deref()?;
                    let known = self.st.worktrees.get(&x.info.project_path)?;
                    (!known.iter().any(|w| w.path == wt)).then(|| x.info.project_path.clone())
                })
                .collect();
            for path in stale {
                self.call(
                    Call::Worktrees(path.clone()),
                    "git:listWorktrees",
                    json!(path),
                );
            }
            for (id, cwd) in live {
                self.st.git.polling += 2;
                self.call(
                    Call::Git(GitCall::Stat(id.clone())),
                    "git:diffStat",
                    json!(cwd),
                );
                self.call(Call::Git(GitCall::Branch(id)), "git:getBranch", json!(cwd));
            }
        }
        let unlisted: Vec<String> = self
            .store
            .projects
            .iter()
            .map(|p| p.path.clone())
            .filter(|p| !self.st.worktrees.contains_key(p) && !self.st.git.listed.contains(p))
            .collect();
        for path in unlisted {
            self.st.git.listed.insert(path.clone());
            self.call(
                Call::Worktrees(path.clone()),
                "git:listWorktrees",
                json!(path),
            );
        }
        if !self.st.git.ides_asked {
            self.st.git.ides_asked = true;
            self.call(Call::Git(GitCall::Ides), "ide:detect", Value::Null);
        }
    }

    fn fetch_changes(&mut self) {
        let Some(sid) = self.st.git.panel.clone() else {
            return;
        };
        let Some(cwd) = self.store.session(&sid).map(|s| cwd_of(&s.info)) else {
            return;
        };
        self.st.git.diff.cwd = cwd.clone();
        self.st.git.diff.loading = true;
        self.call(
            Call::Git(GitCall::Diff(cwd.clone())),
            "git:diffFull",
            json!(cwd),
        );
    }

    fn refresh_stat(&mut self) {
        let Some(sid) = self.st.git.panel.clone() else {
            return;
        };
        if let Some(cwd) = self.store.session(&sid).map(|s| cwd_of(&s.info)) {
            self.call(Call::Git(GitCall::Stat(sid)), "git:diffStat", json!(cwd));
        }
    }

    pub(super) fn git_answered(&mut self, call: GitCall, answer: Result<Value, RpcError>) {
        let g = &mut self.st.git;
        match call {
            GitCall::Stat(id) => {
                g.polling = g.polling.saturating_sub(1);
                if let Some(stat) = answer.ok().as_ref().and_then(Stat::from_value) {
                    g.stats.insert(id, stat);
                }
            }
            GitCall::Branch(id) => {
                g.polling = g.polling.saturating_sub(1);
                let branch = answer.ok().and_then(|v| v.as_str().map(str::to_owned));
                if let Some(b) = branch.filter(|b| !b.is_empty()) {
                    if let Some(s) = self.store.sessions.iter_mut().find(|s| s.info.id == id) {
                        if s.info.branch.as_deref() != Some(b.as_str()) {
                            s.info.branch = Some(b);
                        }
                    }
                }
            }
            GitCall::Diff(cwd) => {
                if g.diff.cwd != cwd {
                    return;
                }
                g.diff = Diff {
                    cwd,
                    files: answer
                        .ok()
                        .as_ref()
                        .and_then(|v| v.get("files"))
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(DiffFile::from_value)
                        .collect(),
                    ..Diff::default()
                };
            }
            GitCall::Committed { cwd, push } => match done(answer, "Commit failed") {
                Err(e) => {
                    g.commit.loading = false;
                    g.commit.error = Some(e);
                }
                Ok(()) if push => self.call(Call::Git(GitCall::Pushed), "git:push", json!(cwd)),
                Ok(()) => self.committed("Changes committed"),
            },
            GitCall::Pushed => match done(answer, "") {
                Err(e) => {
                    g.commit.loading = false;
                    g.commit.error = Some(format!("Committed, but push failed: {e}"));
                }
                Ok(()) => self.committed("Committed and pushed"),
            },
            GitCall::Branches(session) => {
                let Some(b) = g.branches.as_mut().filter(|b| b.session == session) else {
                    return;
                };
                b.loading = false;
                let v = answer.unwrap_or(Value::Null);
                b.local = v
                    .get("local")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|x| x.as_str().map(str::to_owned))
                    .collect();
            }
            GitCall::Remotes => {
                let Some(b) = g.branches.as_mut() else { return };
                b.fetching = false;
                let local = b.local.clone();
                b.remote = list(answer)
                    .iter()
                    .filter_map(|x| x.as_str().map(str::to_owned))
                    .filter(|r| !local.contains(r))
                    .collect();
            }
            GitCall::Switched { cwd, branch } => {
                if let Some(b) = g.branches.as_mut() {
                    b.switching = false;
                }
                g.branches = None;
                match done(answer, &format!("Failed to checkout '{branch}'")) {
                    Ok(()) => {
                        for s in &mut self.store.sessions {
                            if cwd_of(&s.info) == cwd {
                                s.info.branch = Some(branch.clone());
                            }
                        }
                        self.st.toast(format!("Switched to '{branch}'"), false);
                    }
                    Err(e) => self.st.toast(e, true),
                }
            }
            GitCall::Ides => {
                g.ides = Some(list(answer).iter().filter_map(Ide::from_value).collect());
            }
        }
    }

    fn committed(&mut self, text: &str) {
        self.st.git.commit.loading = false;
        self.st.dialog = None;
        self.st.toast(text, false);
        self.fetch_changes();
        self.refresh_stat();
    }

    /// Opens the branch switcher for `session`'s directory.
    fn open_branches(&mut self, ui: &mut Ui, session: String, src: Src) {
        let Some(info) = self.store.session(&session).map(|s| s.info.clone()) else {
            return;
        };
        self.st.git.branches = Some(Branches {
            session: session.clone(),
            current: info.branch.clone(),
            loading: true,
            ..Branches::default()
        });
        ui.set_edit_text(branch_filter_id(), "");
        self.call(
            Call::Git(GitCall::Branches(session.clone())),
            "git:listBranches",
            json!(info.project_path),
        );
        self.open_menu(ui, MenuKind::Branch(session), src);
        ui.set_focus(Some(branch_filter_id()));
    }

    pub(super) fn run_git(&mut self, ui: &mut Ui, act: GitAct, src: Src) {
        match act {
            GitAct::Open(session) => {
                self.st.git.panel = Some(session);
                self.st.git.maximized = false;
                self.st.git.diff = Diff::default();
                self.fetch_changes();
            }
            GitAct::Close => {
                self.st.git.panel = None;
                self.st.git.diff = Diff::default();
            }
            GitAct::Maximize => self.st.git.maximized = !self.st.git.maximized,
            GitAct::Refresh => self.fetch_changes(),
            GitAct::Diff(op) => self.diff_op(ui, op),
            GitAct::Commit(op) => self.commit_op(ui, op),
            GitAct::Branches(session) => self.open_branches(ui, session, src),
            GitAct::Switch(branch) => {
                let Some(b) = self.st.git.branches.as_mut() else {
                    return;
                };
                let session = b.session.clone();
                let Some(info) = self.store.session(&session).map(|s| s.info.clone()) else {
                    return;
                };
                if b.switching || info.branch.as_deref() == Some(branch.as_str()) {
                    self.close_menu(ui);
                    self.st.git.branches = None;
                    return;
                }
                b.switching = true;
                self.close_menu(ui);
                let cwd = cwd_of(&info);
                self.call(
                    Call::Git(GitCall::Switched {
                        cwd: cwd.clone(),
                        branch: branch.clone(),
                    }),
                    "git:checkoutBranch",
                    json!({ "cwd": cwd, "branch": branch }),
                );
                self.call(
                    Call::Worktrees(info.project_path.clone()),
                    "git:listWorktrees",
                    json!(info.project_path),
                );
            }
            GitAct::FetchRemotes => {
                let Some(b) = self.st.git.branches.as_mut() else {
                    return;
                };
                if b.fetching {
                    return;
                }
                b.fetching = true;
                let session = b.session.clone();
                if let Some(path) = self
                    .store
                    .session(&session)
                    .map(|s| s.info.project_path.clone())
                {
                    self.call(
                        Call::Git(GitCall::Remotes),
                        "git:listRemoteBranches",
                        json!(path),
                    );
                }
            }
            GitAct::OpenIn(path, ide) => {
                let id = ide.or_else(|| {
                    self.st
                        .git
                        .ides
                        .as_ref()
                        .and_then(|l| l.first())
                        .map(|i| i.id.clone())
                });
                if let Some(id) = id {
                    self.call(
                        Call::Quiet,
                        "ide:open",
                        json!({ "ideId": id, "projectPath": path }),
                    );
                }
            }
            GitAct::OpenInMenu(session) => self.open_menu(ui, MenuKind::OpenIn(session), src),
        }
    }

    fn diff_op(&mut self, ui: &mut Ui, op: DiffOp) {
        let d = &mut self.st.git.diff;
        match op {
            DiffOp::PickFile(f) => d.file = f,
            DiffOp::Line(file, line) => {
                let open = d
                    .commenting
                    .as_ref()
                    .is_some_and(|(f, l, _)| *f == file && *l == line);
                if open {
                    d.commenting = None;
                    ui.set_focus(None);
                    return;
                }
                let content = d
                    .files
                    .iter()
                    .find(|f| f.path == file)
                    .and_then(|f| f.diff.split('\n').nth(line))
                    .unwrap_or("")
                    .to_owned();
                d.commenting = Some((file, line, content));
                let id = crate::screen::git::comment_id();
                ui.set_edit_text(id, "");
                ui.set_focus(Some(id));
            }
            DiffOp::AddComment => {
                let id = crate::screen::git::comment_id();
                let text = ui.edit_text(id).trim().to_owned();
                if text.is_empty() {
                    return;
                }
                if let Some((file, line, content)) = d.commenting.take() {
                    d.comments.push(Comment {
                        file,
                        line,
                        content,
                        text,
                    });
                }
                ui.set_focus(None);
            }
            DiffOp::CancelComment => {
                d.commenting = None;
                ui.set_focus(None);
            }
            DiffOp::RemoveComment(i) => {
                if i < d.comments.len() {
                    d.comments.remove(i);
                }
            }
            DiffOp::ClearComments => d.comments.clear(),
            DiffOp::SendFeedback => {
                let comments = std::mem::take(&mut d.comments);
                d.commenting = None;
                let Some(sid) = self.st.git.panel.clone() else {
                    return;
                };
                if comments.is_empty() {
                    return;
                }
                if let Some(g) = &self.grid {
                    g.paste(&sid, &format!("{}\n", tasks::feedback(&comments)));
                }
            }
        }
    }

    fn commit_op(&mut self, ui: &mut Ui, op: CommitOp) {
        match op {
            CommitOp::Open => {
                self.st.git.commit = Commit::default();
                ui.set_edit_text(commit_id(), "");
                ui.set_focus(Some(commit_id()));
                self.st.dialog = Some(Dialog::GitCommit);
            }
            CommitOp::ToggleUnstaged => {
                self.st.git.commit.unstaged = !self.st.git.commit.unstaged;
            }
            CommitOp::Push(p) => self.st.git.commit.push = p,
            CommitOp::Close => {
                self.st.dialog = None;
                ui.set_focus(None);
            }
            CommitOp::Commit => {
                let message = ui.edit_text(commit_id()).trim().to_owned();
                let cwd = self.st.git.diff.cwd.clone();
                let cm = &mut self.st.git.commit;
                if message.is_empty() || cm.loading || cwd.is_empty() {
                    return;
                }
                cm.loading = true;
                cm.error = None;
                let params = json!({
                    "cwd": cwd,
                    "message": message,
                    "includeUnstaged": cm.unstaged,
                });
                let push = cm.push;
                self.call(
                    Call::Git(GitCall::Committed { cwd, push }),
                    "git:commit",
                    params,
                );
            }
        }
    }

    /// Keys the git surfaces take first: the commit message's Mod+Enter, the
    /// comment box's, and the branch filter's Escape.
    pub(super) fn git_key(&mut self, ui: &mut Ui, code: &str, with_mod: bool) -> bool {
        let focus = ui.focus();
        if self.st.dialog == Some(Dialog::GitCommit) && focus == Some(commit_id()) {
            if code == "Enter" && with_mod {
                self.commit_op(ui, CommitOp::Commit);
                return true;
            }
            if code == "Escape" {
                self.commit_op(ui, CommitOp::Close);
                return true;
            }
        }
        if focus == Some(crate::screen::git::comment_id()) {
            if code == "Enter" && with_mod {
                self.diff_op(ui, DiffOp::AddComment);
                return true;
            }
            if code == "Escape" {
                self.diff_op(ui, DiffOp::CancelComment);
                return true;
            }
        }
        false
    }
}
