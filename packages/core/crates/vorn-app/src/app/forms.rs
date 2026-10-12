//! What the node and workflow forms do: typing in a bound field, a
//! picker's choice, the `{{` dropdown, header rows and the Advanced folds.

use serde_json::{json, Value};

use super::App;
use crate::act::Act;
use crate::runs::s;
use crate::ui::{Event as UiEvent, Id, Input, Ui};
use crate::workflows::{self as wf, Ac, Bind, Conv, WfAct};

/// `CONTEXT_REF`.
const CTX_NAME: &str = "{{context.projectName}}";
const CTX_PATH: &str = "{{context.projectPath}}";
const CTX_CWD: &str = "{{context.cwd}}";
const CTX_BRANCH: &str = "{{context.branch}}";

impl App {
    /// Fields show what the model holds while nobody types in them, so an
    /// undo or a picker's choice reaches them.
    pub(super) fn seed_texts(&self, ui: &mut Ui) {
        for (id, value) in self.acts.texts() {
            if !ui.focused(id) && ui.edit_text(id) != value {
                ui.set_edit_text(id, &value);
            }
        }
    }

    fn write(&mut self, node: Option<&str>, path: &str, v: Option<Value>) {
        let node = node.map(str::to_owned);
        let path = path.to_owned();
        self.edit_graph(|d| match node.as_deref() {
            Some(n) => {
                if let Some(x) = d.node_mut(n) {
                    wf::set_path(x, &path, v);
                }
            }
            None => wf::set_path(&mut d.wf, &path, v),
        });
    }

    /// The text of a bound field, stored.
    pub(super) fn apply_text(&mut self, bind: &Bind, text: &str) {
        let node = bind.node.as_deref();
        if node.is_some() && bind.path == "label" {
            let (n, t) = (node.unwrap_or("").to_owned(), text.to_owned());
            self.edit_graph(|d| wf::relabel(d, &n, &t));
            return;
        }
        let key = (node.unwrap_or("").to_owned(), bind.path.clone());
        let value = match wf::convert(bind.conv, text) {
            Ok(v) => {
                self.st.wf.raw.remove(&key);
                v
            }
            Err(why) => {
                if bind.conv == Conv::Schema {
                    self.st.wf.raw.insert(key, text.to_owned());
                    let _ = why;
                }
                return;
            }
        };
        let path = bind.path.as_str();
        if let Some(rest) = path.strip_prefix("header.") {
            let (Some(i), Some(part)) = rest
                .split_once('.')
                .map(|(i, p)| (i.parse::<usize>().ok(), Some(p)))
                .unwrap_or((None, None))
            else {
                return;
            };
            let Some(n) = node else { return };
            let mut entries = self.headers(n);
            if let Some(e) = entries.get_mut(i) {
                let t = value
                    .as_ref()
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned();
                if part == "name" {
                    e.0 = t;
                } else {
                    e.1 = t;
                }
            }
            self.set_headers(n, entries);
            return;
        }
        if let Some(rest) = path.strip_prefix("input.") {
            let Some((i, prop)) = rest.split_once('.') else {
                return;
            };
            let Ok(i) = i.parse::<usize>() else { return };
            let n = node.unwrap_or("").to_owned();
            let prop = prop.to_owned();
            self.edit_graph(|d| {
                let Some(item) = d
                    .node_mut(&n)
                    .and_then(|x| x.pointer_mut(&format!("/config/inputs/{i}")))
                    .and_then(Value::as_object_mut)
                else {
                    return;
                };
                match value {
                    Some(v) if !(v.as_str() == Some("") && prop != "key" && prop != "label") => {
                        item.insert(prop, v);
                    }
                    _ if prop == "label" => {
                        item.insert(prop, json!(""));
                    }
                    _ => {
                        item.remove(&prop);
                    }
                }
            });
            return;
        }
        // A branch emptied takes the worktree that needed it.
        if path == "config.branch" && value.is_none() {
            self.write(node, "config.useWorktree", None);
        }
        self.write(node, path, value);
    }

    fn headers(&self, node: &str) -> Vec<(String, String)> {
        self.st
            .wf
            .editor
            .as_ref()
            .and_then(|d| d.node(node))
            .and_then(|n| n.pointer("/config/headers"))
            .and_then(Value::as_object)
            .map(|m| {
                m.iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_owned()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// `Object.fromEntries`: a later name wins over an earlier one.
    fn set_headers(&mut self, node: &str, entries: Vec<(String, String)>) {
        let mut m = serde_json::Map::new();
        for (k, v) in entries {
            m.insert(k, json!(v));
        }
        self.write(Some(node), "config.headers", Some(Value::Object(m)));
    }

    /// A picker's choice.
    pub(super) fn choose_value(&mut self, node: Option<&str>, path: &str, value: &str) {
        let Some(n) = node else {
            self.write(
                None,
                path,
                if value.is_empty() {
                    None
                } else {
                    Some(json!(value))
                },
            );
            return;
        };
        let cfg = self
            .st
            .wf
            .editor
            .as_ref()
            .and_then(|d| d.node(n))
            .and_then(|x| x.get("config"))
            .cloned()
            .unwrap_or_default();
        let set = |app: &mut App, p: &str, v: Option<Value>| app.write(Some(n), p, v);
        let projects: Vec<(String, String)> = self
            .store
            .projects
            .iter()
            .map(|p| (p.name.clone(), p.path.clone()))
            .collect();
        let project = |name: &str| projects.iter().find(|p| p.0 == name).cloned();
        match path {
            "onError" => set(self, "onError", (value != "stop").then(|| json!(value))),
            "script.project" => match value {
                "" => {
                    for k in ["config.projectName", "config.projectPath", "config.cwd"] {
                        set(self, k, None);
                    }
                }
                "@context" => {
                    set(self, "config.projectName", Some(json!(CTX_NAME)));
                    set(self, "config.projectPath", Some(json!(CTX_PATH)));
                    set(self, "config.cwd", Some(json!(CTX_CWD)));
                }
                name => {
                    if let Some((pn, pp)) = project(name) {
                        set(self, "config.projectName", Some(json!(pn)));
                        set(self, "config.projectPath", Some(json!(pp.clone())));
                        set(self, "config.cwd", Some(json!(pp)));
                    }
                }
            },
            "agent.project" => match value {
                "@context" => {
                    set(self, "config.projectName", Some(json!(CTX_NAME)));
                    set(self, "config.projectPath", Some(json!(CTX_PATH)));
                }
                name => {
                    if let Some((pn, pp)) = project(name) {
                        set(self, "config.projectName", Some(json!(pn)));
                        set(self, "config.projectPath", Some(json!(pp)));
                    }
                }
            },
            "config.agentType" => {
                set(self, "config.agentType", Some(json!(value)));
                set(self, "config.model", None);
            }
            "promptSource" => match value {
                "inline" => {
                    set(self, "config.taskId", None);
                    set(self, "config.taskFromQueue", None);
                }
                "task" => {
                    let first = self
                        .store
                        .tasks
                        .iter()
                        .find(|t| {
                            Some(t.project_name.as_str()) == s(&cfg, "projectName")
                                && t.status.as_str() == "todo"
                        })
                        .map(|t| t.id.clone());
                    set(self, "config.prompt", None);
                    set(self, "config.taskFromQueue", None);
                    set(self, "config.taskId", first.map(|f| json!(f)));
                }
                _ => {
                    set(self, "config.prompt", None);
                    set(self, "config.taskId", None);
                    set(self, "config.taskFromQueue", Some(json!(true)));
                }
            },
            "worktreeMode" => {
                let has_branch = s(&cfg, "branch").is_some_and(|b| !b.trim().is_empty());
                if value == "new" && !has_branch {
                    return;
                }
                for k in ["config.worktreeFromStepSlug", "config.existingWorktreePath"] {
                    set(self, k, None);
                }
                if value == "fromContext" {
                    set(self, "config.useWorktree", Some(json!("fromContext")));
                    set(self, "config.worktreeMode", None);
                    return;
                }
                set(self, "config.worktreeMode", Some(json!(value)));
                set(
                    self,
                    "config.useWorktree",
                    (value == "new").then(|| json!(true)),
                );
                if value == "none" {
                    set(self, "config.branch", None);
                }
            }
            "branch.context" => {
                let from = s(&cfg, "branch") == Some(CTX_BRANCH);
                set(self, "config.branch", (!from).then(|| json!(CTX_BRANCH)));
            }
            "poll.connection" => {
                set(self, "config.connectionId", Some(json!(value)));
                set(self, "config.event", Some(json!("")));
            }
            "action.connection" => {
                set(self, "config.connectionId", Some(json!(value)));
                set(self, "config.action", Some(json!("")));
                set(self, "config.args", Some(json!({})));
            }
            "action.action" => {
                let label = s(&cfg, "connectionId")
                    .and_then(|c| self.st.wf.lib.actions.get(c))
                    .and_then(|l| l.iter().find(|a| s(a, "type") == Some(value)))
                    .and_then(|a| s(a, "label"))
                    .unwrap_or(value)
                    .to_owned();
                set(self, "config.action", Some(json!(value)));
                set(self, "config.actionLabel", Some(json!(label)));
                set(self, "config.args", Some(json!({})));
            }
            "feedback.from" => set(self, "config.feedback.from", Some(json!(value))),
            "config.method"
            | "config.scriptType"
            | "config.operator"
            | "config.mode"
            | "config.project"
            | "config.initialStatus"
            | "config.restore"
            | "config.concurrency" => {
                let v = match (path, value) {
                    ("config.restore", "cold") | ("config.concurrency", "perProject") => None,
                    _ => Some(json!(value)),
                };
                set(self, path, v);
            }
            "config.until.operator" => {
                let var = cfg.pointer("/until/variable").cloned().unwrap_or(json!(""));
                let val = cfg.pointer("/until/value").cloned().unwrap_or(json!(""));
                set(
                    self,
                    "config.until",
                    Some(json!({ "variable": var, "operator": value, "value": val })),
                );
            }
            "config.cron" => set(self, path, Some(json!(value))),
            _ if path.starts_with("args.") => {
                let key = &path["args.".len()..];
                set(self, &format!("config.args.{key}"), Some(json!(value)));
            }
            _ if path.starts_with("input.") => {
                let Some((i, prop)) = path["input.".len()..].split_once('.') else {
                    return;
                };
                let Ok(i) = i.parse::<usize>() else { return };
                let (prop, value) = (prop.to_owned(), value.to_owned());
                let n = n.to_owned();
                self.edit_graph(|d| {
                    let Some(item) = d
                        .node_mut(&n)
                        .and_then(|x| x.pointer_mut(&format!("/config/inputs/{i}")))
                        .and_then(Value::as_object_mut)
                    else {
                        return;
                    };
                    if prop == "type" {
                        let keeps = value != "boolean" && value != "select";
                        let opts = item.get("options").cloned().unwrap_or(json!([]));
                        item.insert("type".into(), json!(value));
                        if value == "select" {
                            item.insert("options".into(), opts);
                        } else {
                            item.remove("options");
                        }
                        if !keeps {
                            item.remove("defaultValue");
                        }
                    } else if prop == "required" {
                        let on = item.get("required").and_then(Value::as_bool) == Some(true);
                        if on {
                            item.remove("required");
                        } else {
                            item.insert("required".into(), json!(true));
                        }
                    }
                });
            }
            _ => set(self, path, (!value.is_empty()).then(|| json!(value))),
        }
    }

    /// The form controls that are not typing.
    pub(super) fn run_form(&mut self, ui: &mut Ui, act: WfAct) {
        match act {
            WfAct::Choose(node, path, value) => {
                self.choose_value(node.as_deref(), &path, &value);
                self.agent_facts();
            }
            WfAct::Advanced(node) => {
                if !self.st.wf.advanced.remove(&node) {
                    self.st.wf.advanced.insert(node);
                }
            }
            WfAct::AddHeader(node) => {
                let mut e = self.headers(&node);
                e.push((String::new(), String::new()));
                self.set_headers(&node, e);
            }
            WfAct::RemoveHeader(node, i) => {
                let mut e = self.headers(&node);
                if i < e.len() {
                    e.remove(i);
                }
                self.set_headers(&node, e);
            }
            WfAct::ArgFree(node, key, free) => {
                let k = (node.clone(), key.clone());
                if free {
                    self.st.wf.free_args.insert(k);
                } else {
                    self.st.wf.free_args.remove(&k);
                }
            }
            WfAct::Copy(text) => {
                ui.clipboard().set(&text);
                self.st.wf.copied = Some(crate::state::now_ms());
            }
            WfAct::Vars(bind) => {
                let id = field_of(&bind);
                if self.st.wf.ac.as_ref().is_some_and(|a| a.bind == bind) {
                    self.st.wf.ac = None;
                    return;
                }
                let at = ui.edit(id).map_or(ui.edit_text(id).len(), |e| e.caret());
                self.st.wf.ac = Some(Ac {
                    bind,
                    start: at,
                    filter: String::new(),
                    collapsed: Default::default(),
                    highlight: 0,
                });
                ui.set_focus(Some(id));
            }
            WfAct::VarGroup(g) => {
                if let Some(ac) = self.st.wf.ac.as_mut() {
                    if !ac.collapsed.remove(&g) {
                        ac.collapsed.insert(g);
                    }
                }
            }
            WfAct::Insert(pattern) => self.insert_var(ui, &pattern),
            _ => {}
        }
    }

    /// The dropdown's visible lines, as `VariableAutocomplete` lists them.
    pub fn ac_items(&self) -> Vec<crate::vars::Item> {
        let (Some(ac), Some(d)) = (&self.st.wf.ac, &self.st.wf.editor) else {
            return Vec::new();
        };
        let Some(node) = ac.bind.node.as_deref() else {
            return Vec::new();
        };
        let (groups, vars) = wf::var_sources(&self.st.wf, d, node, &ac.bind.path);
        crate::vars::filtered(crate::vars::items(&groups, &vars), &ac.filter)
            .into_iter()
            .filter(|i| !ac.collapsed.contains(&i.group_id))
            .collect()
    }

    fn insert_var(&mut self, ui: &mut Ui, pattern: &str) {
        let Some(ac) = self.st.wf.ac.take() else {
            return;
        };
        let id = field_of(&ac.bind);
        let text = ui.edit_text(id).to_owned();
        let end = ui
            .edit(id)
            .map_or(text.len(), |e| e.caret())
            .min(text.len());
        let start = ac.start.min(end);
        // An edit since the dropdown opened can leave its start inside a character.
        if !text.is_char_boundary(start) || !text.is_char_boundary(end) {
            return;
        }
        let next = format!("{}{pattern}{}", &text[..start], &text[end..]);
        ui.set_edit_text(id, &next);
        let caret = start + pattern.len();
        ui.edit_mut(id).set_selection(caret, caret);
        ui.set_focus(Some(id));
        self.apply_text(&ac.bind, &next);
    }

    /// After typing in a variable field: `{{` opens the dropdown, the text
    /// after it filters, `}}` closes it.
    fn track_vars(&mut self, ui: &Ui, id: Id, bind: &Bind) {
        let text = ui.edit_text(id);
        let caret = ui
            .edit(id)
            .map_or(text.len(), |e| e.caret())
            .min(text.len());
        if text[..caret].ends_with("{{") {
            self.st.wf.ac = Some(Ac {
                bind: bind.clone(),
                start: caret - 2,
                filter: String::new(),
                collapsed: Default::default(),
                highlight: 0,
            });
            return;
        }
        let Some(ac) = self.st.wf.ac.as_mut().filter(|a| &a.bind == bind) else {
            return;
        };
        if caret < ac.start + 2
            || !text.is_char_boundary(ac.start)
            || !text.is_char_boundary(ac.start + 2)
        {
            self.st.wf.ac = None;
            return;
        }
        let after = &text[ac.start + 2..caret];
        if after.contains("}}") {
            self.st.wf.ac = None;
        } else {
            ac.filter = after.to_owned();
            ac.highlight = 0;
        }
    }

    /// A bound field changed; true when it was one.
    pub(super) fn form_event(&mut self, ui: &mut Ui, event: &UiEvent) -> bool {
        let UiEvent::Changed(id) = *event else {
            return false;
        };
        let Some(Act::Workflow(WfAct::Text(bind))) = self.acts.text_of(id) else {
            return false;
        };
        let text = ui.edit_text(id).to_owned();
        self.apply_text(&bind, &text);
        if bind.vars {
            self.track_vars(ui, id, &bind);
        }
        true
    }

    /// The dropdown's keys while its field has focus.
    pub(super) fn form_key(&mut self, ui: &mut Ui, input: &Input) -> bool {
        let Input::Key { code, .. } = input else {
            return false;
        };
        let Some(ac) = &self.st.wf.ac else {
            return false;
        };
        if ui.focus() != Some(field_of(&ac.bind)) {
            return false;
        }
        let n = self.ac_items().len();
        match code.as_str() {
            "Escape" => self.st.wf.ac = None,
            "ArrowDown" => {
                if let Some(a) = self.st.wf.ac.as_mut() {
                    a.highlight = (a.highlight + 1).min(n.saturating_sub(1));
                }
            }
            "ArrowUp" => {
                if let Some(a) = self.st.wf.ac.as_mut() {
                    a.highlight = a.highlight.saturating_sub(1);
                }
            }
            "Enter" | "NumpadEnter" | "Tab" => {
                let i = self.st.wf.ac.as_ref().map_or(0, |a| a.highlight);
                match self.ac_items().get(i).map(|x| x.pattern.clone()) {
                    Some(p) => self.insert_var(ui, &p),
                    None => return false,
                }
            }
            _ => return false,
        }
        true
    }
}

/// The field a binding draws.
pub fn field_of(b: &Bind) -> Id {
    Id::new("wf-form").child((b.node.as_deref().unwrap_or(""), b.path.as_str()))
}
