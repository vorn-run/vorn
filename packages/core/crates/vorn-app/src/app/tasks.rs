//! What the Tasks view's controls and keys do, and the answers to the
//! calls they make, as `TaskBoardView`, `TaskDetailPanel`, `AddTaskDialog`
//! and `CommitDialog` do them.

use std::time::{Duration, Instant};

use serde_json::{json, Value};
use vorn_protocol::TerminalSession;

use super::{list, App, Call, Src, MOD};
use crate::act::Act;
use crate::client::rpc::decode;
use crate::client::RpcError;
use crate::runs::{self, RunAct};
use crate::screen::tasks::{
    branch_id, card_id, column_id, comment_id, commit_id, desc_id, image_id, link_id, slash_open,
    title_id,
};
use crate::screen::MainView;
use crate::state::Dialog;
use crate::tasks::{
    self, Comment, Commit, Diff, DiffFile, Form, Mark, TaskAct, TaskForm, STATUSES,
};
use crate::ui::{decode_png, mods, Event as UiEvent, Input, Ui};

/// How long the detail panel waits after an edit before saving it.
const AUTOSAVE: Duration = Duration::from_millis(500);

/// A task call whose answer changes the screen.
pub(super) enum TaskCall {
    /// `workflowRun:listByTask` for the task.
    Runs(String),
    /// `git:diffFull` of the directory.
    Diff(String),
    /// `task:imageGetPath` of the file name.
    ImagePath(String),
    /// `task:imageSave` into the form, under the task id.
    ImageSaved(Form, String),
    Committed {
        cwd: String,
        push: bool,
    },
    Pushed,
    /// A session started for a task (put in progress when set), or for a
    /// run's step.
    Session {
        task: Option<String>,
        toast: Option<&'static str>,
    },
}

impl App {
    pub(super) fn task_answered(&mut self, call: TaskCall, answer: Result<Value, RpcError>) {
        let t = &mut self.st.tasks;
        match call {
            TaskCall::Runs(id) => {
                if t.selected.as_deref() == Some(id.as_str()) {
                    t.runs = list(answer);
                }
            }
            TaskCall::Diff(cwd) => {
                if t.diff.cwd != cwd {
                    return;
                }
                t.diff.loading = false;
                t.diff.files = answer
                    .ok()
                    .and_then(|v| v.get("files").and_then(Value::as_array).cloned())
                    .unwrap_or_default()
                    .iter()
                    .filter_map(DiffFile::from_value)
                    .collect();
                t.diff.file = None;
                t.diff.comments.clear();
                t.diff.commenting = None;
            }
            TaskCall::ImagePath(name) => {
                let Ok(Value::String(path)) = answer else {
                    return;
                };
                // Only PNG thumbnails decode here; other formats keep their tile.
                if let Some((w, h, rgba)) =
                    std::fs::read(&path).ok().and_then(|b| decode_png(&b).ok())
                {
                    t.decoded.push((name.clone(), w, h, rgba));
                }
                t.image_paths.insert(name, path);
            }
            TaskCall::ImageSaved(form, id) => match answer {
                Ok(Value::String(name)) => {
                    let f = t.form(form);
                    if f.id != id {
                        return;
                    }
                    f.images.push(name.clone());
                    f.attaching = false;
                    self.touch(form);
                    self.call(
                        Call::Task(TaskCall::ImagePath(name.clone())),
                        "task:imageGetPath",
                        json!({ "taskId": id, "filename": name }),
                    );
                }
                Ok(_) => {}
                Err(e) => self.st.toast(e.to_string(), true),
            },
            TaskCall::Committed { cwd, push } => match committed(answer, "Commit failed") {
                Err(e) => {
                    t.commit.loading = false;
                    t.commit.error = Some(e);
                }
                Ok(()) if push => self.call(Call::Task(TaskCall::Pushed), "git:push", json!(cwd)),
                Ok(()) => self.commit_done("Changes committed"),
            },
            TaskCall::Pushed => match committed(answer, "") {
                Err(e) => {
                    t.commit.loading = false;
                    t.commit.error = Some(format!("Committed, but push failed: {e}"));
                }
                Ok(()) => self.commit_done("Committed and pushed"),
            },
            TaskCall::Session { task, toast } => {
                let info = match answer.and_then(decode::<TerminalSession>) {
                    Ok(info) => info,
                    Err(e) => return self.st.toast(e.to_string(), true),
                };
                if let Some(id) = task {
                    let agent = info.agent_type.to_string();
                    let wt = info.worktree_path.clone();
                    let sid = info.id.clone();
                    self.change_tasks(None, |c, now| {
                        tasks::start(c, &id, &sid, &agent, wt.as_deref(), now)
                    });
                }
                self.st.selected = Some(info.id.clone());
                self.st.focused = Some(info.id.clone());
                self.st.view = MainView::Sessions;
                self.store.upsert(info);
                if let Some(text) = toast {
                    self.st.toast(text, false);
                }
            }
        }
    }

    fn commit_done(&mut self, text: &str) {
        self.st.tasks.commit.loading = false;
        self.st.dialog = None;
        self.st.toast(text, false);
        self.fetch_diff();
    }

    /// Saves the config with the tasks `f` changed, toasting `text`.
    fn change_tasks(&mut self, text: Option<&str>, f: impl FnOnce(&mut Value, &str) -> bool) {
        let mut config = self.store.config.clone();
        if !f(&mut config, &tasks::now_iso()) {
            return;
        }
        self.save_config(config, Call::Quiet);
        if let Some(text) = text {
            self.st.toast(text, false);
        }
    }

    fn task(&self, id: &str) -> Option<&vorn_protocol::TaskConfig> {
        self.store.tasks.iter().find(|t| t.id == id)
    }

    fn project_path(&self, name: &str) -> Option<String> {
        self.store
            .projects
            .iter()
            .find(|p| p.name == name)
            .map(|p| p.path.clone())
    }

    /// The project a new task starts in.
    fn default_project(&self) -> String {
        self.st
            .project
            .clone()
            .or_else(|| self.store.projects.first().map(|p| p.name.clone()))
            .unwrap_or_default()
    }

    /// `StatusPicker`'s change; moving into progress says nothing.
    fn set_status(&mut self, id: &str, status: &str) {
        let toast = (status != "in_progress").then(|| tasks::status_toast(status));
        self.change_tasks(toast, |c, now| tasks::set_status(c, id, status, now));
    }

    /// Opens a task, or the create form, in the detail panel.
    fn open_task(&mut self, ui: &mut Ui, id: String) {
        self.flush_task(ui);
        let new = id == "new";
        let form = match self.task(&id) {
            Some(t) => {
                ui.set_edit_text(title_id(Form::Detail), &t.title);
                ui.set_edit_text(desc_id(Form::Detail), &t.description);
                tasks::form_of(t)
            }
            None if new => {
                ui.set_edit_text(title_id(Form::Detail), "");
                ui.set_edit_text(desc_id(Form::Detail), tasks::TEMPLATE);
                TaskForm {
                    id: tasks::uuid(),
                    status: "todo".into(),
                    project: self.default_project(),
                    ..TaskForm::default()
                }
            }
            None => return,
        };
        ui.set_edit_text(branch_id(), &form.branch);
        ui.set_edit_text(image_id(Form::Detail), "");
        let images = form.images.clone();
        let tid = form.id.clone();
        let t = &mut self.st.tasks;
        t.detail_form = form;
        t.selected = Some(id.clone());
        t.confirm_delete = None;
        t.runs.clear();
        t.diff = Diff::default();
        t.diff_key = None;
        t.dirty = None;
        t.slash_closed = None;
        if new {
            ui.set_focus(Some(title_id(Form::Detail)));
            return;
        }
        self.call(
            Call::Task(TaskCall::Runs(id.clone())),
            "workflowRun:listByTask",
            json!({ "taskId": id, "limit": 20 }),
        );
        self.fetch_images(&tid, &images);
    }

    fn fetch_images(&mut self, task: &str, images: &[String]) {
        for name in images {
            if self.st.tasks.image_paths.contains_key(name) {
                continue;
            }
            self.call(
                Call::Task(TaskCall::ImagePath(name.clone())),
                "task:imageGetPath",
                json!({ "taskId": task, "filename": name }),
            );
        }
    }

    /// Where the selected task's changes are, while it is in progress or in
    /// review and the Changes section is open.
    fn diff_cwd(&self) -> Option<String> {
        if !self.st.tasks.diff_open {
            return None;
        }
        let t = self
            .st
            .tasks
            .selected
            .as_deref()
            .and_then(|id| self.task(id))?;
        if !matches!(t.status.to_string().as_str(), "in_review" | "in_progress") {
            return None;
        }
        t.worktree_path
            .clone()
            .or_else(|| self.project_path(&t.project_name))
            .filter(|c| !c.is_empty())
    }

    /// `fetchDiff`: the selected task's uncommitted changes.
    fn fetch_diff(&mut self) {
        let Some(cwd) = self.diff_cwd() else { return };
        self.st.tasks.diff.cwd = cwd.clone();
        self.st.tasks.diff.loading = true;
        self.call(
            Call::Task(TaskCall::Diff(cwd.clone())),
            "git:diffFull",
            json!(cwd),
        );
    }

    /// Saves the detail panel's pending edits now.
    fn flush_task(&mut self, ui: &Ui) {
        if self.st.tasks.dirty.take().is_none() {
            return;
        }
        let mut f = self.st.tasks.detail_form.clone();
        if !f.editing {
            return;
        }
        f.branch = ui.edit_text(branch_id()).to_owned();
        let title = ui.edit_text(title_id(Form::Detail)).to_owned();
        let desc = ui.edit_text(desc_id(Form::Detail)).to_owned();
        self.change_tasks(None, |c, now| {
            tasks::patch(c, &f.id, now, |t| tasks::apply_form(t, &f, &title, &desc))
        });
    }

    /// Each frame: thumbnails to the GPU and the debounced autosave.
    pub(super) fn tasks_tick(&mut self, ui: &mut Ui) {
        // The diff follows the task and its directory, as the panel's `diffKey`.
        let key = self.diff_cwd().map(|cwd| {
            let id = self.st.tasks.selected.as_deref().unwrap_or("");
            format!("{id}|{cwd}")
        });
        if key != self.st.tasks.diff_key {
            let fetch = key.is_some();
            self.st.tasks.diff_key = key;
            self.st.tasks.diff = Diff::default();
            if fetch {
                self.fetch_diff();
            }
        }
        for (name, w, h, rgba) in std::mem::take(&mut self.st.tasks.decoded) {
            if let Some(img) = ui.image(w, h, &rgba) {
                self.st.tasks.thumbs.insert(name, img);
            }
        }
        let editing = [title_id(Form::Detail), desc_id(Form::Detail), branch_id()]
            .into_iter()
            .any(|id| ui.focused(id));
        if self
            .st
            .tasks
            .dirty
            .is_some_and(|t| !editing || t.elapsed() >= AUTOSAVE)
        {
            self.flush_task(ui);
        }
        if self
            .st
            .tasks
            .copied
            .is_some_and(|t| t.elapsed() >= Duration::from_secs(2))
        {
            self.st.tasks.copied = None;
        }
    }

    pub(super) fn close_task_dialog(&mut self, ui: &mut Ui) {
        self.st.dialog = None;
        self.st.tasks.dialog_form = TaskForm::default();
        ui.set_focus(None);
    }

    fn new_task(&mut self, ui: &mut Ui, status: String) {
        self.st.tasks.dialog_form = TaskForm {
            id: tasks::uuid(),
            status,
            project: self.default_project(),
            ..TaskForm::default()
        };
        ui.set_edit_text(title_id(Form::Dialog), "");
        ui.set_edit_text(desc_id(Form::Dialog), tasks::TEMPLATE);
        ui.set_edit_text(image_id(Form::Dialog), "");
        ui.set_focus(Some(title_id(Form::Dialog)));
        self.st.tasks.slash_closed = None;
        self.st.dialog = Some(Dialog::AddTask);
    }

    /// The form's title and description, when it can be saved.
    fn form_text(&self, ui: &Ui, form: Form) -> Option<(String, String)> {
        let title = ui.edit_text(title_id(form)).trim().to_owned();
        let desc = ui.edit_text(desc_id(form)).trim().to_owned();
        let ok = !title.is_empty()
            && !desc.is_empty()
            && !self.st.tasks.form_ref(form).project.is_empty();
        ok.then_some((title, desc))
    }

    fn submit_dialog(&mut self, ui: &mut Ui) {
        let Some((title, desc)) = self.form_text(ui, Form::Dialog) else {
            return;
        };
        let f = self.st.tasks.dialog_form.clone();
        if f.editing {
            self.change_tasks(Some("Task updated"), |c, now| {
                tasks::patch(c, &f.id, now, |t| {
                    tasks::apply_form(t, &f, &title, &desc);
                    t.insert("status".into(), json!(f.status));
                })
            });
        } else {
            self.change_tasks(Some("Task created"), |c, now| {
                tasks::add(c, &f, &title, &desc, now);
                true
            });
        }
        self.close_task_dialog(ui);
    }

    fn create_from_panel(&mut self, ui: &mut Ui) {
        let Some((title, desc)) = self.form_text(ui, Form::Detail) else {
            return;
        };
        let mut f = self.st.tasks.detail_form.clone();
        f.branch = ui.edit_text(branch_id()).to_owned();
        f.status = "todo".into();
        self.change_tasks(Some("Task created"), |c, now| {
            tasks::add(c, &f, &title, &desc, now);
            true
        });
        self.open_task(ui, f.id);
    }

    /// An edit of the detail form, saved after the debounce.
    fn touch(&mut self, form: Form) {
        if form == Form::Detail && self.st.tasks.detail_form.editing {
            self.st.tasks.dirty = Some(Instant::now());
        }
    }

    /// Replaces the description of `form` and puts the caret at `caret`.
    fn set_desc(&mut self, ui: &mut Ui, form: Form, text: &str, caret: usize) {
        let id = desc_id(form);
        ui.set_edit_text(id, text);
        ui.edit_mut(id).move_to(caret, false);
        ui.set_focus(Some(id));
        self.touch(form);
    }

    /// `terminal:create`'s params resuming the task's agent session.
    fn resume_task(&self, t: &vorn_protocol::TaskConfig) -> Option<Value> {
        let agent = t.assigned_agent.as_ref()?.to_string();
        let session = t
            .agent_session_id
            .as_ref()
            .filter(|_| tasks::resumable(&agent))?;
        let mut params = json!({
            "agentType": agent,
            "projectName": t.project_name,
            "projectPath": self.project_path(&t.project_name)?,
            "resumeSessionId": session,
        });
        if let Some(b) = &t.branch {
            params["branch"] = json!(b);
        }
        if let Some(w) = t.use_worktree {
            params["useWorktree"] = json!(w);
        }
        Some(params)
    }

    /// `getOpenSessionHandler`: the live session focused, else the agent's
    /// session resumed.
    fn open_session(&mut self, ui: &mut Ui, id: &str) {
        let Some(t) = self.task(id) else { return };
        let live = t
            .assigned_session_id
            .clone()
            .filter(|s| self.store.session(s).is_some_and(|x| x.exited.is_none()));
        if let Some(sid) = live {
            self.run(ui, Act::Expand(sid), Src::default());
            return;
        }
        let Some(params) = self.resume_task(t) else {
            return;
        };
        let start = (t.status.to_string() == "in_progress").then(|| t.id.clone());
        self.call(
            Call::Task(TaskCall::Session {
                task: start,
                toast: None,
            }),
            "terminal:create",
            params,
        );
    }

    /// `handleSendFeedback`: into the agent's session when it can resume,
    /// else onto the clipboard.
    fn send_feedback(&mut self, ui: &mut Ui) {
        let comments = std::mem::take(&mut self.st.tasks.diff.comments);
        self.st.tasks.diff.commenting = None;
        let Some(t) = self
            .st
            .tasks
            .selected
            .as_deref()
            .and_then(|id| self.task(id))
        else {
            return;
        };
        if comments.is_empty() {
            return;
        }
        let feedback = tasks::feedback(&comments);
        let Some(mut params) = self.resume_task(t) else {
            ui.clipboard().set(&feedback);
            self.st.toast("Review feedback copied to clipboard", false);
            return;
        };
        params["initialPrompt"] = json!(format!(
            "# Review Feedback for: {}\n\n**Project:** {}\n**Task ID:** {}\n\n{feedback}\n\nPlease address the review feedback above and update the task when done.",
            t.title, t.project_name, t.id
        ));
        let task = Some(t.id.clone());
        self.call(
            Call::Task(TaskCall::Session {
                task,
                toast: Some("Feedback sent to agent"),
            }),
            "terminal:create",
            params,
        );
    }

    pub(super) fn run_task(&mut self, ui: &mut Ui, act: TaskAct) {
        match act {
            TaskAct::Open(id) | TaskAct::Edit(id) => self.open_task(ui, id),
            TaskAct::CloseDetail => {
                self.flush_task(ui);
                self.st.tasks.selected = None;
                ui.set_focus(None);
            }
            TaskAct::New(status) => self.new_task(ui, status),
            TaskAct::CloseDialog => self.close_task_dialog(ui),
            TaskAct::Expand => {
                self.close_task_dialog(ui);
                self.open_task(ui, "new".into());
            }
            TaskAct::Submit => self.submit_dialog(ui),
            TaskAct::Create => self.create_from_panel(ui),
            TaskAct::Filter(f) => self.st.tasks.filter = f,
            TaskAct::Source(s) => self.st.tasks.source = s,
            TaskAct::ToggleArchived => self.st.tasks.archived = !self.st.tasks.archived,
            TaskAct::Mode(m) => {
                self.st.tasks.mode = m;
                let mut config = self.store.config.clone();
                if !config.get("defaults").is_some_and(Value::is_object) {
                    config["defaults"] = json!({});
                }
                config["defaults"]["taskViewMode"] = json!(m.key());
                self.save_config(config, Call::Quiet);
            }
            TaskAct::ToggleSection(s) => {
                if !self.st.tasks.collapsed.remove(&s) {
                    self.st.tasks.collapsed.insert(s);
                }
            }
            TaskAct::SetStatus(Form::Dialog, s) => self.st.tasks.dialog_form.status = s,
            TaskAct::SetStatus(Form::Detail, s) => {
                let f = &self.st.tasks.detail_form;
                if f.editing {
                    let id = f.id.clone();
                    self.set_status(&id, &s);
                }
            }
            TaskAct::PickProject(form, p) => {
                self.st.tasks.form(form).project = p;
                self.touch(form);
            }
            TaskAct::PickAgent(form, a) => {
                self.st.tasks.form(form).agent = a;
                self.touch(form);
            }
            TaskAct::ToggleWorktree(form) => {
                let f = self.st.tasks.form(form);
                f.worktree = !f.worktree;
                self.touch(form);
            }
            TaskAct::Complete(id) => self.set_status(&id, "done"),
            TaskAct::Cancel(id) => self.set_status(&id, "cancelled"),
            TaskAct::Reopen(id) => self.set_status(&id, "todo"),
            TaskAct::Archive(id) => self.change_tasks(Some("Task archived"), |c, now| {
                tasks::set_archived(c, &id, true, now)
            }),
            TaskAct::Unarchive(id) => self.change_tasks(Some("Task unarchived"), |c, now| {
                tasks::set_archived(c, &id, false, now)
            }),
            TaskAct::AskDelete(id) => self.st.tasks.confirm_delete = (!id.is_empty()).then_some(id),
            TaskAct::Delete(id) => {
                self.st.tasks.confirm_delete = None;
                if self.st.tasks.selected.as_deref() == Some(id.as_str()) {
                    self.st.tasks.selected = None;
                    self.st.tasks.dirty = None;
                }
                self.change_tasks(Some("Task deleted"), |c, _| tasks::remove(c, &id));
                self.call(Call::Quiet, "task:imageCleanup", json!(id));
            }
            TaskAct::ReviewDiff(id) => {
                self.st.tasks.diff_open = true;
                self.open_task(ui, id);
            }
            TaskAct::OpenSession(id) => self.open_session(ui, &id),
            TaskAct::AttachImages(form) => {
                self.st.tasks.form(form).attaching = true;
                ui.set_edit_text(image_id(form), "");
                ui.set_focus(Some(image_id(form)));
            }
            TaskAct::RemoveImage(form, name) => {
                let f = self.st.tasks.form(form);
                f.images.retain(|i| *i != name);
                let id = f.id.clone();
                self.touch(form);
                self.call(
                    Call::Quiet,
                    "task:imageDelete",
                    json!({ "taskId": id, "filename": name }),
                );
            }
            TaskAct::Slash(form, index) => {
                let Some(edit) = ui.edit(desc_id(form)) else {
                    return;
                };
                let (text, caret) = (edit.text().to_owned(), edit.caret());
                let Some((start, _)) = tasks::slash_query(&text, caret) else {
                    return;
                };
                let (text, caret) = tasks::apply_slash(&text, start, caret, index);
                self.st.tasks.slash = 0;
                self.set_desc(ui, form, &text, caret);
            }
            TaskAct::Format(form, mark) => {
                let Some(edit) = ui.edit(desc_id(form)) else {
                    return;
                };
                let (text, range) = (edit.text().to_owned(), edit.selection());
                let (text, caret) = tasks::apply_mark(&text, range, mark);
                self.set_desc(ui, form, &text, caret);
            }
            TaskAct::StartLink(form) => {
                let Some(range) = ui.edit(desc_id(form)).map(|e| e.selection()) else {
                    return;
                };
                if let Some(r) = ui.ime_area() {
                    self.st.tasks.link_pos = (r.x, r.y);
                }
                self.st.tasks.linking = Some((form, range));
                ui.set_edit_text(link_id(), "");
                ui.set_focus(Some(link_id()));
            }
            TaskAct::ApplyLink => {
                let Some((form, range)) = self.st.tasks.linking.take() else {
                    return;
                };
                let url = ui.edit_text(link_id()).trim().to_owned();
                let text = ui.edit_text(desc_id(form)).to_owned();
                if url.is_empty() || range.end > text.len() {
                    ui.set_focus(Some(desc_id(form)));
                    return;
                }
                let (text, caret) = tasks::link(&text, range, &url);
                self.set_desc(ui, form, &text, caret);
            }
            TaskAct::CopyOutput => {
                ui.clipboard().set(&self.st.tasks.output);
                self.st.tasks.copied = Some(Instant::now());
            }
            TaskAct::Drop(id, to) => self.drop_task(&id, &to),
            TaskAct::ToggleRuns => self.st.tasks.runs_open = !self.st.tasks.runs_open,
            TaskAct::ToggleDiff => self.st.tasks.diff_open = !self.st.tasks.diff_open,
            TaskAct::RefreshDiff => self.fetch_diff(),
            TaskAct::PickFile(f) => self.st.tasks.diff.file = f,
            TaskAct::Line(path, index) => {
                let d = &mut self.st.tasks.diff;
                if d.commenting
                    .as_ref()
                    .is_some_and(|(p, i, _)| *p == path && *i == index)
                {
                    d.commenting = None;
                    return;
                }
                let content = d
                    .files
                    .iter()
                    .find(|f| f.path == path)
                    .and_then(|f| f.diff.split('\n').nth(index))
                    .unwrap_or("")
                    .to_owned();
                d.commenting = Some((path, index, content));
                ui.set_edit_text(comment_id(), "");
                ui.set_focus(Some(comment_id()));
            }
            TaskAct::AddComment => {
                let text = ui.edit_text(comment_id()).trim().to_owned();
                if text.is_empty() {
                    return;
                }
                let d = &mut self.st.tasks.diff;
                let Some((file, line, content)) = d.commenting.take() else {
                    return;
                };
                d.comments.push(Comment {
                    file,
                    line,
                    content,
                    text,
                });
                ui.set_focus(None);
            }
            TaskAct::RemoveComment(i) => {
                let c = &mut self.st.tasks.diff.comments;
                if i < c.len() {
                    c.remove(i);
                }
            }
            TaskAct::ClearComments => self.st.tasks.diff.comments.clear(),
            TaskAct::SendFeedback => self.send_feedback(ui),
            TaskAct::CancelComment => {
                self.st.tasks.diff.commenting = None;
                ui.set_focus(None);
            }
            TaskAct::OpenCommit => {
                self.st.tasks.commit = Commit::default();
                ui.set_edit_text(commit_id(), "");
                ui.set_focus(Some(commit_id()));
                self.st.dialog = Some(Dialog::Commit);
            }
            TaskAct::ToggleUnstaged => {
                self.st.tasks.commit.unstaged = !self.st.tasks.commit.unstaged;
            }
            TaskAct::CommitPush(p) => self.st.tasks.commit.push = p,
            TaskAct::Commit => {
                let message = ui.edit_text(commit_id()).trim().to_owned();
                let cm = &mut self.st.tasks.commit;
                if message.is_empty() || cm.loading {
                    return;
                }
                cm.loading = true;
                cm.error = None;
                let params = json!({
                    "cwd": self.st.tasks.diff.cwd,
                    "message": message,
                    "includeUnstaged": cm.unstaged,
                });
                let call = TaskCall::Committed {
                    cwd: self.st.tasks.diff.cwd.clone(),
                    push: cm.push,
                };
                self.call(Call::Task(call), "git:commit", params);
            }
            TaskAct::ToggleProjects => {
                self.st.tasks.projects_folded = !self.st.tasks.projects_folded;
            }
        }
    }

    /// `handleKanbanDrop`.
    fn drop_task(&mut self, id: &str, to: &str) {
        let Some(from) = self.task(id).map(|t| t.status.to_string()) else {
            return;
        };
        let Some(status) = tasks::drop_status(&from, to) else {
            return;
        };
        let toast = match status {
            "done" => Some("Task completed"),
            "cancelled" => Some("Task cancelled"),
            _ => None,
        };
        self.change_tasks(toast, |c, now| tasks::set_status(c, id, status, now));
    }

    /// The run `key` names: what vornd last reported, else the list the
    /// panel loaded.
    pub(super) fn find_run(&self, key: &str) -> Option<Value> {
        self.store.runs.get(key).cloned().or_else(|| {
            self.st
                .tasks
                .runs
                .iter()
                .find(|r| runs::key(r) == key)
                .cloned()
                .or_else(|| self.find_wf_run(key))
        })
    }

    fn run_nodes(&self, run: &Value) -> Value {
        let wid = runs::s(run, "workflowId").unwrap_or("");
        self.store
            .workflows
            .iter()
            .find(|w| w.id == wid)
            .and_then(|w| serde_json::to_value(&w.nodes).ok())
            .unwrap_or(Value::Null)
    }

    pub(super) fn run_run(&mut self, act: RunAct) {
        match act {
            RunAct::Toggle(key) => {
                let Some(run) = self.find_run(&key) else {
                    return;
                };
                let open = self.st.runs.is_open(&run);
                self.st.runs.open.insert(key, !open);
            }
            RunAct::ToggleStep(key, nid) => {
                let Some(run) = self.find_run(&key) else {
                    return;
                };
                let current = self.st.runs.step(&run).map(str::to_owned);
                let next = (current.as_deref() != Some(nid.as_str())).then_some(nid);
                self.st.runs.step.insert(key, next);
            }
            RunAct::Stop(key) => {
                self.call(Call::Quiet, "workflow:stopRun", json!({ "runId": key }));
            }
            RunAct::Rerun(key) => {
                self.call(Call::Quiet, "workflow:rerun", json!({ "runId": key }));
            }
            RunAct::Retry(key) => {
                self.call(Call::Quiet, "workflow:retryRun", json!({ "runId": key }));
            }
            RunAct::Output(key, nid) => {
                let Some(run) = self.find_run(&key) else {
                    return;
                };
                let logs = runs::states(&run)
                    .find(|n| runs::s(n, "nodeId") == Some(nid.as_str()))
                    .and_then(|n| runs::s(n, "logs"))
                    .unwrap_or("")
                    .to_owned();
                self.st.tasks.output = logs;
                self.st.tasks.copied = None;
                self.st.dialog = Some(Dialog::Output);
            }
            RunAct::OpenReview(key, nid) => {
                let token = self.find_run(&key).and_then(|r| {
                    runs::states(&r)
                        .find(|n| runs::s(n, "nodeId") == Some(nid.as_str()))
                        .and_then(|n| runs::s(n, "viewToken"))
                        .map(str::to_owned)
                });
                let Some(t) = token else {
                    self.st.toast("The review page could not be opened.", true);
                    return;
                };
                open_url(&format!(
                    "http://127.0.0.1:{}/gate-view/{}/{}?t={}",
                    self.port,
                    runs::encode(&key),
                    runs::encode(&nid),
                    runs::encode(&t)
                ));
            }
            RunAct::Resume(key, nid) => {
                let Some(run) = self.find_run(&key) else {
                    return;
                };
                let nodes = self.run_nodes(&run);
                let tasks: Vec<Value> = self
                    .store
                    .tasks
                    .iter()
                    .filter_map(|t| serde_json::to_value(t).ok())
                    .collect();
                if let Some(params) = runs::resume_params(&run, &nodes, &nid, &tasks) {
                    let call = TaskCall::Session {
                        task: None,
                        toast: None,
                    };
                    self.call(Call::Task(call), "terminal:create", params);
                }
            }
        }
    }

    /// A run vornd reported: the panel's list again when it touches the
    /// selected task.
    pub(super) fn task_run_updated(&mut self, run: &Value) {
        let Some(id) = self.st.tasks.selected.clone().filter(|s| s != "new") else {
            return;
        };
        let touches = runs::s(run, "triggerTaskId") == Some(id.as_str())
            || runs::states(run).any(|n| runs::s(n, "taskId") == Some(id.as_str()))
            || self
                .st
                .tasks
                .runs
                .iter()
                .any(|r| runs::key(r) == runs::key(run));
        if touches {
            self.call(
                Call::Task(TaskCall::Runs(id.clone())),
                "workflowRun:listByTask",
                json!({ "taskId": id, "limit": 20 }),
            );
        }
    }

    /// The Tasks view's own events: a kanban card dragged and dropped, and
    /// the fields that submit or autosave. True when one was used.
    pub(super) fn task_event(&mut self, ui: &mut Ui, event: &UiEvent) -> bool {
        match *event {
            UiEvent::Move { id, x, y, .. } => {
                let Some(task) = self.store.tasks.iter().find(|t| card_id(&t.id) == id) else {
                    return false;
                };
                let over = STATUSES
                    .iter()
                    .find(|s| {
                        ui.rect_of(column_id(s.key))
                            .is_some_and(|r| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h)
                    })
                    .map(|s| s.key.to_owned());
                self.st.tasks.dragging = Some((task.id.clone(), over));
                true
            }
            UiEvent::Drop { id, .. } => {
                let Some((task, over)) = self.st.tasks.dragging.take() else {
                    return false;
                };
                if card_id(&task) == id {
                    if let Some(to) = over {
                        self.drop_task(&task, &to);
                    }
                }
                true
            }
            UiEvent::Submit(id) => {
                let image = [Form::Dialog, Form::Detail]
                    .into_iter()
                    .find(|&f| image_id(f) == id);
                if let Some(form) = image {
                    let path = ui.edit_text(id).trim().to_owned();
                    ui.set_edit_text(id, "");
                    if path.is_empty() {
                        self.st.tasks.form(form).attaching = false;
                        ui.set_focus(None);
                        return true;
                    }
                    let tid = self.st.tasks.form_ref(form).id.clone();
                    self.call(
                        Call::Task(TaskCall::ImageSaved(form, tid.clone())),
                        "task:imageSave",
                        json!({ "taskId": tid, "sourcePath": path }),
                    );
                } else if id == link_id() {
                    self.run_task(ui, TaskAct::ApplyLink);
                } else if id == title_id(Form::Dialog) {
                    self.submit_dialog(ui);
                } else if id != title_id(Form::Detail) && id != branch_id() {
                    return false;
                }
                true
            }
            UiEvent::Changed(id) => {
                if [title_id(Form::Detail), desc_id(Form::Detail), branch_id()].contains(&id) {
                    self.touch(Form::Detail);
                }
                if [desc_id(Form::Dialog), desc_id(Form::Detail)].contains(&id) {
                    self.st.tasks.slash = 0;
                }
                false
            }
            _ => false,
        }
    }

    /// Keys the task editors take before the UI: the slash menu's, a list
    /// carried on by Enter, the marks' shortcuts, and the comment's, link's
    /// and commit message's Mod+Enter and Escape.
    pub(super) fn task_key(&mut self, ui: &mut Ui, input: &Input) -> bool {
        let Input::Key { code, mods: m, .. } = input else {
            return false;
        };
        let held = *m & (mods::CTRL | mods::SUPER | mods::ALT | mods::SHIFT);
        let (plain, with_mod) = (held == 0, held == MOD);
        if let Some((form, start, matches)) = slash_open(ui, self.st.tasks.slash_closed) {
            let n = matches.len();
            let sel = self.st.tasks.slash.min(n - 1);
            match code.as_str() {
                "ArrowDown" if plain => self.st.tasks.slash = (sel + 1) % n,
                "ArrowUp" if plain => self.st.tasks.slash = (sel + n - 1) % n,
                "Enter" | "Tab" if plain => self.run_task(ui, TaskAct::Slash(form, matches[sel])),
                "Escape" => self.st.tasks.slash_closed = Some(start),
                _ => return false,
            }
            return true;
        }
        let focus = ui.focus();
        let editor = [Form::Dialog, Form::Detail]
            .into_iter()
            .find(|&f| focus == Some(desc_id(f)));
        if let Some(form) = editor {
            match code.as_str() {
                "Enter" if plain => {
                    let Some(edit) = ui.edit(desc_id(form)) else {
                        return false;
                    };
                    if !edit.selection().is_empty() {
                        return false;
                    }
                    let (text, caret) = tasks::newline(edit.text(), edit.caret());
                    self.set_desc(ui, form, &text, caret);
                    self.st.tasks.slash_closed = None;
                }
                "KeyB" if with_mod => self.run_task(ui, TaskAct::Format(form, Mark::Bold)),
                "KeyI" if with_mod => self.run_task(ui, TaskAct::Format(form, Mark::Italic)),
                _ => return false,
            }
            return true;
        }
        if focus == Some(comment_id()) {
            match code.as_str() {
                "Enter" if with_mod => self.run_task(ui, TaskAct::AddComment),
                "Escape" => self.run_task(ui, TaskAct::CancelComment),
                _ => return false,
            }
            return true;
        }
        if focus == Some(link_id()) && code == "Escape" {
            if let Some((form, _)) = self.st.tasks.linking.take() {
                ui.set_focus(Some(desc_id(form)));
            }
            return true;
        }
        if focus == Some(commit_id()) && code == "Enter" && with_mod {
            self.run_task(ui, TaskAct::Commit);
            return true;
        }
        false
    }
}

/// A `GitCommitResult`: ok, or why not (`fallback` when it does not say).
fn committed(answer: Result<Value, RpcError>, fallback: &str) -> Result<(), String> {
    let v = answer.map_err(|e| e.to_string())?;
    if v.get("success").and_then(Value::as_bool) == Some(true) {
        return Ok(());
    }
    Err(v
        .get("error")
        .and_then(Value::as_str)
        .filter(|e| !e.is_empty())
        .unwrap_or(fallback)
        .to_owned())
}

/// Opens a web or mail link in the system's handler.
pub(super) fn open_url(url: &str) {
    let web = ["https://", "http://", "mailto:"]
        .iter()
        .any(|p| url.starts_with(p));
    if !web {
        return;
    }
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    // spawn-visible: the system opener shows the link; nothing to do without one.
    let _ = std::process::Command::new(opener).arg(url).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commit_results_say_why_they_failed() {
        assert_eq!(committed(Ok(json!({ "success": true })), "x"), Ok(()));
        assert_eq!(
            committed(Ok(json!({ "success": false })), "Commit failed"),
            Err("Commit failed".to_owned())
        );
        assert_eq!(
            committed(Ok(json!({ "success": false, "error": "no remote" })), ""),
            Err("no remote".to_owned())
        );
    }
}
