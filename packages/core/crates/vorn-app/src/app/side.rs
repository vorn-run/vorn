//! The pane column's controls (`PaneColumn`, `FilesCard`, `TerminalsCard`,
//! `EditorCard`): opening and closing panes, their tabs, the file tree,
//! the cards a file pops out into, and each open file's load, edit, find
//! and save, with vornd reading and writing the files.

use std::collections::HashSet;
use std::time::Instant;

use serde_json::{json, Value};
use vorn_protocol::TerminalSession;

use super::git::cwd_of;
use super::{App, Call, Src};
use crate::act::Act;
use crate::client::rpc::{decode, RpcError};
use crate::editor::{self, Buffer, EditAct, Stamp, DRAFT_SETTLE_MS, HIGHLIGHT_SETTLE_MS};
use crate::screen::side::{editor_id, files_body_id, filter_id, find_id, PaneDivider};
use crate::side::{tab_key, Entry, SideAct, SideKind, Tree};
use crate::state::{Dialog, MenuKind};
use crate::ui::{Id, Ui};

/// A file or pane call whose answer changes the screen.
pub(super) enum SideCall {
    /// A shell for `owner`'s panel.
    Shell(String),
    /// A directory of `owner`'s tree.
    Listed {
        owner: String,
        dir: String,
    },
    Read(String),
    /// The stamp of a file that would not read: binary, or gone.
    Unread(String),
    /// The stamp the edit is based on.
    Base(String),
    /// The stamp now, against a restored draft's base.
    DraftCheck(String),
    /// The stamp now, before a save writes.
    PreSave(String),
    Wrote {
        key: String,
        text: String,
    },
    /// What is on disk, to replace the edit with.
    Disk(String),
}

impl App {
    /// The key of every buffer on screen: each open tab and card.
    fn open_keys(&self) -> Vec<(String, String, String)> {
        let p = &self.st.side.saved;
        let tabs = p.files.iter().flat_map(|(owner, f)| {
            f.tabs
                .iter()
                .map(move |t| (tab_key(owner, t), t.clone(), owner.clone()))
        });
        let cards = p
            .cards
            .iter()
            .map(|(id, c)| (id.clone(), c.file_path.clone(), c.session_id.clone()));
        tabs.chain(cards).collect()
    }

    /// The working directory of the session that owns `owner`'s panes.
    pub(super) fn owner_cwd(&self, owner: &str) -> Option<String> {
        self.store.session(owner).map(|s| cwd_of(&s.info))
    }

    /// Loads what newly opened panes and files need, drops what closed,
    /// colours and drafts what settled, and keeps the Files panes' layout.
    pub(super) fn panes_tick(&mut self, ui: &mut Ui) {
        let live: HashSet<&str> = self
            .store
            .sessions
            .iter()
            .map(|s| s.info.id.as_str())
            .collect();
        if self.st.side.prune(&live) {
            self.st.side.save();
        }
        for owner in self.st.side.saved.files.keys().cloned().collect::<Vec<_>>() {
            let Some(cwd) = self.owner_cwd(&owner) else {
                continue;
            };
            let stale = self.st.side.trees.get(&owner).is_none_or(|t| t.cwd != cwd);
            if stale {
                self.st.side.trees.insert(
                    owner.clone(),
                    Tree {
                        cwd: cwd.clone(),
                        loading: true,
                        ..Tree::default()
                    },
                );
                self.call(
                    Call::Side(SideCall::Listed {
                        owner: owner.clone(),
                        dir: cwd.clone(),
                    }),
                    "file:listDir",
                    json!({ "dirPath": cwd }),
                );
            }
            if let Some(r) = ui.rect_of(files_body_id(&owner)) {
                let narrow = r.w > 0.0 && r.w < crate::side::SIDE_BY_SIDE_MIN;
                if narrow {
                    self.st.side.narrow.insert(owner.clone());
                } else {
                    self.st.side.narrow.remove(&owner);
                }
            }
        }
        let keys = self.open_keys();
        let open: HashSet<&str> = keys.iter().map(|(k, _, _)| k.as_str()).collect();
        self.st
            .editors
            .buffers
            .retain(|k, _| open.contains(k.as_str()));
        for (key, path, _) in &keys {
            let fresh = self
                .st
                .editors
                .buffers
                .get(key)
                .is_none_or(|b| b.path != *path);
            if fresh {
                self.st
                    .editors
                    .buffers
                    .insert(key.clone(), Buffer::new(path));
                self.call(
                    Call::Side(SideCall::Read(key.clone())),
                    "file:readContent",
                    json!({ "filePath": path }),
                );
            }
        }
        let now = Instant::now();
        let ed = &mut self.st.editors;
        for (key, b) in &mut ed.buffers {
            let id = editor_id(key);
            if b.ready && !ui.focused(id) && ui.edit_text(id) != b.draft {
                ui.set_edit_text(id, &b.draft);
            }
            let settled = |ms: u128| {
                b.typed
                    .is_none_or(|t| now.duration_since(t).as_millis() >= ms)
            };
            let text = b.text();
            let stale = b.coloured.as_ref().is_none_or(|(t, _)| t != text);
            if stale && settled(HIGHLIGHT_SETTLE_MS) && b.content.is_some() {
                let lang = crate::highlight::lang_for(crate::state::basename(&b.path));
                let spans = match lang {
                    Some(l) if text.split('\n').count() <= editor::MAX_HIGHLIGHT_LINES => {
                        crate::highlight::highlight(text, l)
                    }
                    _ => Vec::new(),
                };
                b.coloured = Some((text.to_owned(), spans));
            }
            if b.ready && b.content.is_some() && settled(DRAFT_SETTLE_MS) {
                let unsaved = b.content.as_deref() != Some(b.draft.as_str());
                let written = b.drafted.as_deref() == Some(b.draft.as_str());
                if unsaved && !written {
                    ed.drafts.write(key, &b.path, &b.draft, b.base);
                    b.drafted = Some(b.draft.clone());
                } else if !unsaved && b.drafted.is_some() {
                    ed.drafts.forget(key);
                    b.drafted = None;
                }
            }
        }
        // An edited preview tab is kept, or the next click would replace it.
        let pins: Vec<(String, String)> = self
            .st
            .side
            .saved
            .files
            .iter()
            .filter_map(|(o, f)| f.preview.clone().map(|p| (o.clone(), p)))
            .filter(|(o, p)| self.st.editors.dirty(&tab_key(o, p)))
            .collect();
        for (o, p) in pins {
            self.st.side.pin_tab(&o, &p);
            self.st.side.save();
        }
    }

    pub(super) fn side_answered(&mut self, call: SideCall, answer: Result<Value, RpcError>) {
        match call {
            SideCall::Shell(owner) => match answer.and_then(decode::<TerminalSession>) {
                Ok(info) => {
                    let id = info.id.clone();
                    self.store.upsert(info);
                    self.st.side.open_panel(&owner, &id);
                    self.st.side.focus = Some(id);
                    self.st.side.save();
                }
                Err(e) => self
                    .st
                    .toast(format!("Failed to start terminal: {e}"), true),
            },
            SideCall::Listed { owner, dir } => {
                let Some(t) = self.st.side.trees.get_mut(&owner) else {
                    return;
                };
                let entries: Vec<Entry> = super::list(answer)
                    .iter()
                    .filter_map(Entry::from_value)
                    .collect();
                if dir == t.cwd {
                    t.loading = false;
                    t.root = Some(entries);
                } else {
                    t.pending.remove(&dir);
                    t.dirs.entry(dir).or_insert(entries);
                }
            }
            SideCall::Read(key) => {
                let Some(b) = self.st.editors.buffers.get_mut(&key) else {
                    return;
                };
                match answer.ok().and_then(|v| v.as_str().map(str::to_owned)) {
                    Some(text) => {
                        b.loaded(Some(text), true);
                        self.take_into(&key);
                    }
                    None => {
                        let path = b.path.clone();
                        self.call(
                            Call::Side(SideCall::Unread(key)),
                            "file:stamp",
                            json!({ "filePath": path }),
                        );
                    }
                }
            }
            SideCall::Unread(key) => {
                let known = answer.ok().is_some_and(|v| !v.is_null());
                if let Some(b) = self.st.editors.buffers.get_mut(&key) {
                    b.loaded(None, known);
                }
            }
            SideCall::Base(key) => {
                if let Some(b) = self.st.editors.buffers.get_mut(&key) {
                    b.base = answer.ok().as_ref().and_then(Stamp::from_value);
                }
            }
            SideCall::DraftCheck(key) => {
                if let Some(b) = self.st.editors.buffers.get_mut(&key) {
                    let now = answer.ok().as_ref().and_then(Stamp::from_value);
                    b.conflict = editor::has_moved(b.base, now);
                }
            }
            SideCall::PreSave(key) => {
                let Some(b) = self.st.editors.buffers.get_mut(&key) else {
                    return;
                };
                let now = answer.ok().as_ref().and_then(Stamp::from_value);
                if editor::has_moved(b.base, now) {
                    b.conflict = true;
                    b.saving = false;
                    return;
                }
                self.write_file(&key);
            }
            SideCall::Wrote { key, text } => {
                let ok = match &answer {
                    Ok(v) => v.get("success").and_then(Value::as_bool) == Some(true),
                    Err(_) => false,
                };
                let Some(b) = self.st.editors.buffers.get_mut(&key) else {
                    return;
                };
                if !ok {
                    b.saving = false;
                    b.error = Some(match answer {
                        Ok(v) => v
                            .get("error")
                            .and_then(Value::as_str)
                            .unwrap_or("Failed to save")
                            .to_owned(),
                        Err(e) => e.to_string(),
                    });
                    return;
                }
                let path = b.path.clone();
                b.saved(text);
                self.st.editors.drafts.forget(&key);
                self.stamp_base(&key, &path);
            }
            SideCall::Disk(key) => {
                let Some(b) = self.st.editors.buffers.get_mut(&key) else {
                    return;
                };
                match answer.ok().and_then(|v| v.as_str().map(str::to_owned)) {
                    Some(text) => {
                        let path = b.path.clone();
                        b.draft = text.clone();
                        b.saved(text);
                        self.st.editors.drafts.forget(&key);
                        self.stamp_base(&key, &path);
                    }
                    None => b.error = Some("The file could not be read.".to_owned()),
                }
            }
        }
    }

    fn stamp_base(&mut self, key: &str, path: &str) {
        self.call(
            Call::Side(SideCall::Base(key.to_owned())),
            "file:stamp",
            json!({ "filePath": path }),
        );
    }

    /// The file, or the draft left over it, into the buffer.
    fn take_into(&mut self, key: &str) {
        let ed = &mut self.st.editors;
        let Some(b) = ed.buffers.get_mut(key) else {
            return;
        };
        let path = b.path.clone();
        let kept = ed.drafts.read(key, &path).cloned();
        if b.take(kept.as_ref()) {
            self.call(
                Call::Side(SideCall::DraftCheck(key.to_owned())),
                "file:stamp",
                json!({ "filePath": path }),
            );
        } else {
            if kept.is_some() {
                ed.drafts.forget(key);
            }
            if b.ready {
                self.stamp_base(key, &path);
            }
        }
    }

    fn write_file(&mut self, key: &str) {
        let Some(b) = self.st.editors.buffers.get_mut(key) else {
            return;
        };
        let (path, text) = (b.path.clone(), b.draft.clone());
        self.call(
            Call::Side(SideCall::Wrote {
                key: key.to_owned(),
                text: text.clone(),
            }),
            "file:writeContent",
            json!({ "filePath": path, "content": text }),
        );
    }

    /// `handleSave`: checked against the base unless `force`.
    fn save(&mut self, key: &str, force: bool) {
        let Some(b) = self.st.editors.buffers.get_mut(key) else {
            return;
        };
        if !b.dirty() || b.saving {
            return;
        }
        b.saving = true;
        b.error = None;
        if force {
            self.write_file(key);
            return;
        }
        let path = b.path.clone();
        self.call(
            Call::Side(SideCall::PreSave(key.to_owned())),
            "file:stamp",
            json!({ "filePath": path }),
        );
    }

    /// `confirmDiscardAll`: runs `act` now when none of `keys` has edits,
    /// else asks first. True when it may go ahead now.
    fn may_discard(&mut self, keys: &[String], act: Act) -> bool {
        if self.discarding || !keys.iter().any(|k| self.st.editors.dirty(k)) {
            return true;
        }
        self.st.editors.confirm = Some(act);
        self.st.dialog = Some(Dialog::Discard);
        false
    }

    /// Drops buffers and drafts that are gone with their panes.
    fn forget(&mut self, keys: &[String]) {
        for k in keys {
            self.st.editors.buffers.remove(k);
            self.st.editors.drafts.forget(k);
        }
    }

    fn panel_shell(&mut self, owner: &str) {
        let Some(cwd) = self.owner_cwd(owner) else {
            return;
        };
        self.call(
            Call::Side(SideCall::Shell(owner.to_owned())),
            "shell:create",
            json!(cwd),
        );
    }

    /// Stops `ids` and lets go of their cards, as `closeTerminalsPanel` does.
    fn kill_shells(&mut self, ids: Vec<String>) {
        for id in ids {
            self.call(Call::Ignore, "terminal:kill", json!(id));
            self.store.remove(&id);
            if let Some(g) = &self.grid {
                g.detach(&id);
            }
        }
    }

    pub(super) fn run_side(&mut self, ui: &mut Ui, act: SideAct, src: Src) {
        match act {
            SideAct::ToggleFiles(owner) => {
                match self.st.side.saved.files.get(&owner) {
                    None => self.st.side.open_files(&owner),
                    Some(f) => {
                        let keys: Vec<String> = f.tabs.iter().map(|t| tab_key(&owner, t)).collect();
                        if !self.may_discard(&keys, Act::Side(SideAct::ToggleFiles(owner.clone())))
                        {
                            return;
                        }
                        let gone = self.st.side.close_files(&owner);
                        self.forget(&gone);
                    }
                }
                self.st.side.save();
            }
            SideAct::ToggleTerminals(owner) => {
                if self.st.side.saved.panels.contains_key(&owner) {
                    let held = self.st.side.close_panel(&owner);
                    self.kill_shells(held);
                    self.st.side.save();
                } else {
                    self.panel_shell(&owner);
                }
            }
            SideAct::AddTerminal(owner) => self.panel_shell(&owner),
            SideAct::Maximize(pane) => {
                self.st.side.saved.maximized =
                    if self.st.side.saved.maximized.as_deref() == Some(pane.as_str()) {
                        None
                    } else {
                        Some(pane)
                    };
                self.st.side.save();
            }
            SideAct::Close(kind, owner) => match kind {
                SideKind::Files => {
                    let keys: Vec<String> = self
                        .st
                        .side
                        .saved
                        .files
                        .get(&owner)
                        .map(|f| f.tabs.iter().map(|t| tab_key(&owner, t)).collect())
                        .unwrap_or_default();
                    if !self.may_discard(&keys, Act::Side(SideAct::Close(kind, owner.clone()))) {
                        return;
                    }
                    let gone = self.st.side.close_files(&owner);
                    self.forget(&gone);
                    self.st.side.save();
                }
                SideKind::Terminals => {
                    let held = self.st.side.close_panel(&owner);
                    self.kill_shells(held);
                    self.st.side.save();
                }
            },
            SideAct::Tab(owner, path) => {
                self.st.side.set_active_tab(&owner, &path);
                if self.st.side.narrow.contains(&owner) {
                    self.st.side.set_tree_visible(&owner, false);
                }
                self.st.side.save();
            }
            SideAct::PinTab(owner, path) => {
                self.st.side.pin_tab(&owner, &path);
                self.st.side.save();
            }
            SideAct::CloseTab(owner, path) => {
                let key = tab_key(&owner, &path);
                if !self.may_discard(
                    std::slice::from_ref(&key),
                    Act::Side(SideAct::CloseTab(owner.clone(), path.clone())),
                ) {
                    return;
                }
                self.st.side.close_tab(&owner, &path);
                self.forget(&[key]);
                self.st.side.save();
            }
            SideAct::CloseSavedTabs(owner) => {
                let ed = &self.st.editors;
                let gone: Vec<String> = self
                    .st
                    .side
                    .saved
                    .files
                    .get(&owner)
                    .map(|f| {
                        f.tabs
                            .iter()
                            .map(|t| tab_key(&owner, t))
                            .filter(|k| !ed.dirty(k))
                            .collect()
                    })
                    .unwrap_or_default();
                self.st
                    .side
                    .close_saved_tabs(&owner, |t| ed.dirty(&tab_key(&owner, t)));
                self.st.side.save();
                self.forget(&gone);
            }
            SideAct::TabsMenu(owner) => self.open_menu(ui, MenuKind::FileTabs(owner), src),
            SideAct::ToggleTree(owner) => {
                self.st.side.toggle_tree(&owner);
                self.st.side.save();
            }
            SideAct::Dir(owner, dir) => {
                let Some(t) = self.st.side.trees.get_mut(&owner) else {
                    return;
                };
                if t.expanded.remove(&dir) {
                    return;
                }
                t.expanded.insert(dir.clone());
                if !t.dirs.contains_key(&dir) && t.pending.insert(dir.clone()) {
                    self.call(
                        Call::Side(SideCall::Listed {
                            owner,
                            dir: dir.clone(),
                        }),
                        "file:listDir",
                        json!({ "dirPath": dir }),
                    );
                }
            }
            SideAct::File(owner, path) => self.open_file(&owner, &path, false),
            SideAct::PinFile(owner, path) => self.open_file(&owner, &path, true),
            SideAct::PopOut(owner, path) => {
                let card = self.st.side.promote(&owner, &path);
                self.st.side.save();
                self.st.selected = Some(card);
            }
            SideAct::PopOutActive(owner) => {
                let Some(path) = self
                    .st
                    .side
                    .saved
                    .files
                    .get(&owner)
                    .and_then(|f| f.active.clone())
                else {
                    return;
                };
                let key = tab_key(&owner, &path);
                if !self.may_discard(
                    std::slice::from_ref(&key),
                    Act::Side(SideAct::PopOutActive(owner.clone())),
                ) {
                    return;
                }
                let p = &mut self.st.side;
                let card = p.promote(&owner, &path);
                p.close_tab(&owner, &path);
                p.save();
                self.forget(&[key]);
                self.st.selected = Some(card);
            }
            SideAct::PanelTab(owner, i) => {
                self.st.side.set_active_panel(&owner, i);
                self.st.side.focus = self
                    .st
                    .side
                    .saved
                    .panels
                    .get(&owner)
                    .and_then(|x| x.terminals.get(i))
                    .cloned();
                self.st.selected = Some(owner);
                self.st.side.save();
            }
            SideAct::Extract(_, id) => {
                self.st.side.release(&id);
                self.st.side.save();
                self.st.selected = Some(id);
            }
            SideAct::CloseTerminal(id) => {
                self.st.side.release(&id);
                self.st.side.save();
                self.kill_shells(vec![id]);
            }
            SideAct::FocusTerminal(id) => {
                if let Some(owner) = self.st.side.holder(&id).map(str::to_owned) {
                    self.st.side.focus = Some(id);
                    self.st.selected = Some(owner);
                    ui.set_focus(None);
                }
            }
            SideAct::ReturnCard(card) => {
                if !self.may_discard(
                    std::slice::from_ref(&card),
                    Act::Side(SideAct::ReturnCard(card.clone())),
                ) {
                    return;
                }
                let p = &mut self.st.side;
                let Some(c) = p.saved.cards.remove(&card) else {
                    return;
                };
                p.open_tab(&c.session_id, &c.file_path, true, false);
                p.save();
                self.forget(&[card]);
                self.st.selected = Some(c.session_id);
            }
            SideAct::CloseCard(card) => {
                if !self.may_discard(
                    std::slice::from_ref(&card),
                    Act::Side(SideAct::CloseCard(card.clone())),
                ) {
                    return;
                }
                self.st.side.saved.cards.remove(&card);
                self.st.side.save();
                self.forget(&[card]);
            }
            SideAct::SelectCard(card) => {
                self.st.selected = Some(card);
                self.st.side.focus = None;
            }
            SideAct::ClearFilter(owner) => {
                if let Some(t) = self.st.side.trees.get_mut(&owner) {
                    t.filter.clear();
                }
                ui.set_edit_text(filter_id(&owner), "");
            }
        }
    }

    /// `handleOpen`: a click previews, a double click pins; a narrow pane
    /// hides the tree to show the file.
    fn open_file(&mut self, owner: &str, path: &str, pin: bool) {
        let preview_dirty = self
            .st
            .side
            .saved
            .files
            .get(owner)
            .and_then(|f| f.preview.as_deref())
            .is_some_and(|pv| self.st.editors.dirty(&tab_key(owner, pv)));
        let p = &mut self.st.side;
        p.open_tab(owner, path, pin, preview_dirty);
        if p.narrow.contains(owner) {
            p.set_tree_visible(owner, false);
        }
        p.save();
    }

    pub(super) fn run_edit(&mut self, ui: &mut Ui, act: EditAct) {
        match act {
            EditAct::ToggleFind(key) => {
                let Some(b) = self.st.editors.buffers.get_mut(&key) else {
                    return;
                };
                if b.content.is_none() || b.loading {
                    return;
                }
                if b.find.take().is_none() {
                    b.find = Some(editor::Find::default());
                    ui.set_edit_text(find_id(&key), "");
                    ui.set_focus(Some(find_id(&key)));
                }
            }
            EditAct::FindStep(key, d) => {
                let Some(b) = self.st.editors.buffers.get_mut(&key) else {
                    return;
                };
                let n = b.matches().len();
                if let Some(f) = b.find.as_mut().filter(|_| n > 0) {
                    f.index = (f.index as i64 + i64::from(d)).rem_euclid(n as i64) as usize;
                }
                self.reveal_match(ui, &key);
            }
            EditAct::CloseFind(key) => {
                if let Some(b) = self.st.editors.buffers.get_mut(&key) {
                    b.find = None;
                }
                ui.set_focus(None);
            }
            EditAct::Save(key) => self.save(&key, false),
            EditAct::SaveAnyway(key) => self.save(&key, true),
            EditAct::Discard(key) => {
                if !self.may_discard(
                    std::slice::from_ref(&key),
                    Act::Edit(EditAct::Discard(key.clone())),
                ) {
                    return;
                }
                let Some(b) = self.st.editors.buffers.get_mut(&key) else {
                    return;
                };
                let Some(content) = b.content.clone() else {
                    return;
                };
                b.draft = content.clone();
                b.error = None;
                b.conflict = false;
                b.drafted = None;
                ui.set_edit_text(editor_id(&key), &content);
                self.st.editors.drafts.forget(&key);
            }
            EditAct::TakeDisk(key) => {
                let Some(path) = self.st.editors.buffers.get(&key).map(|b| b.path.clone()) else {
                    return;
                };
                self.call(
                    Call::Side(SideCall::Disk(key)),
                    "file:readContent",
                    json!({ "filePath": path }),
                );
            }
            EditAct::KeepEditing(key) => {
                if let Some(b) = self.st.editors.buffers.get_mut(&key) {
                    b.conflict = false;
                }
            }
            EditAct::ConfirmDiscard => {
                self.st.dialog = None;
                if let Some(act) = self.st.editors.confirm.take() {
                    self.discarding = true;
                    self.run(ui, act, Src::default());
                    self.discarding = false;
                }
            }
            EditAct::CancelDiscard => {
                self.st.dialog = None;
                self.st.editors.confirm = None;
            }
        }
    }

    /// Scrolls the current find match to the middle of the editor.
    fn reveal_match(&mut self, ui: &mut Ui, key: &str) {
        let Some(b) = self.st.editors.buffers.get(key) else {
            return;
        };
        let m = b.matches();
        let Some(f) = &b.find else { return };
        let Some(hit) = m.get(f.index % m.len().max(1)) else {
            return;
        };
        let scroller = crate::screen::side::scroller_id(key);
        let h = ui.rect_of(scroller).map_or(0.0, |r| r.h);
        let y = hit.line as f32 * editor::LINE_HEIGHT - h / 2.0;
        ui.set_scroll_offset(scroller, y.max(0.0));
    }

    /// Keeps the caret's line inside the editor's view as it moves.
    pub(super) fn follow_caret(&mut self, ui: &mut Ui, key: &str) {
        let id = editor_id(key);
        let Some(caret) = ui.edit(id).map(|e| e.caret()) else {
            return;
        };
        let text = ui.edit_text(id);
        let line = text[..caret.min(text.len())].matches('\n').count() as f32;
        let scroller = crate::screen::side::scroller_id(key);
        let Some(r) = ui.rect_of(scroller) else {
            return;
        };
        let top = line * editor::LINE_HEIGHT;
        let off = ui.scroll_offset(scroller);
        if top < off {
            ui.set_scroll_offset(scroller, top);
        } else if top + editor::LINE_HEIGHT + 8.0 > off + r.h {
            ui.set_scroll_offset(scroller, top + editor::LINE_HEIGHT + 8.0 - r.h);
        }
    }

    /// A buffer's key from its editor or find field, when `id` is one.
    pub(super) fn buffer_of(&self, id: Id) -> Option<(String, bool)> {
        self.st.editors.buffers.keys().find_map(|k| {
            if editor_id(k) == id {
                Some((k.clone(), false))
            } else if find_id(k) == id {
                Some((k.clone(), true))
            } else {
                None
            }
        })
    }

    /// Typing in an editor, a find bar or a tree filter.
    pub(super) fn pane_changed(&mut self, ui: &mut Ui, id: Id) -> bool {
        if let Some((key, find)) = self.buffer_of(id) {
            let text = ui.edit_text(id).to_owned();
            let Some(b) = self.st.editors.buffers.get_mut(&key) else {
                return true;
            };
            if find {
                if let Some(f) = b.find.as_mut() {
                    f.query = text;
                    f.index = 0;
                }
                self.reveal_match(ui, &key);
            } else if b.ready {
                b.draft = text;
                b.typed = Some(Instant::now());
                self.follow_caret(ui, &key);
            }
            return true;
        }
        let owner = self
            .st
            .side
            .trees
            .keys()
            .find(|o| filter_id(o) == id)
            .cloned();
        if let Some(owner) = owner {
            let text = ui.edit_text(id).to_owned();
            if let Some(t) = self.st.side.trees.get_mut(&owner) {
                t.filter = text;
            }
            return true;
        }
        false
    }

    /// The editor's keys before the UI's: Mod+S saves, the find bar's Enter
    /// steps and Escape closes it, a tree filter's Escape clears it.
    pub(super) fn pane_key(
        &mut self,
        ui: &mut Ui,
        code: &str,
        with_mod: bool,
        shift: bool,
    ) -> bool {
        let Some(focus) = ui.focus() else {
            return false;
        };
        if let Some((key, find)) = self.buffer_of(focus) {
            if code == "KeyS" && with_mod {
                self.save(&key, false);
                return true;
            }
            if find && code == "Enter" {
                self.run_edit(ui, EditAct::FindStep(key, if shift { -1 } else { 1 }));
                return true;
            }
            if find && code == "Escape" {
                self.run_edit(ui, EditAct::CloseFind(key));
                return true;
            }
            return false;
        }
        let owner = self
            .st
            .side
            .trees
            .keys()
            .find(|o| filter_id(o) == focus)
            .cloned();
        if let (Some(owner), "Escape") = (owner, code) {
            self.run_side(ui, SideAct::ClearFilter(owner), Src::default());
            return true;
        }
        false
    }

    /// A divider moved: the card's split, a column's weights, a Files
    /// pane's tree, or the Changes panel's width. Saved when it ends.
    pub(super) fn pane_resized(&mut self, ui: &Ui, id: Id, ratio: f32, end: bool) -> bool {
        if id == crate::screen::sidebar::SIDEBAR_HANDLE {
            // The handle sits on the sidebar's right edge, so the sidebar's
            // left is that edge less its width, taken as the drag starts.
            let now = self.st.sidebar_width.unwrap_or(crate::look::SIDEBAR_W);
            let left = self
                .sidebar_left
                .or_else(|| ui.rect_of(id).map(|r| r.x + r.w - now));
            if let (Some(left), Some((x, _))) = (left, ui.pointer()) {
                let w = (x - left).clamp(crate::look::SIDEBAR_MIN, crate::look::SIDEBAR_MAX);
                self.st.sidebar_width = Some(w.round());
            }
            self.sidebar_left = if end { None } else { left };
            return true;
        }
        let Some(which) = crate::screen::side::divider_of(&self.st, id) else {
            return false;
        };
        let p = &mut self.st.side;
        match which {
            PaneDivider::Card(owner) => {
                if end {
                    p.drag_card = None;
                    p.set_terminal_ratio(&owner, ratio);
                    p.save();
                } else {
                    p.drag_card = Some((owner, ratio));
                }
            }
            PaneDivider::Column(owner, i) => {
                let w = crate::side::resize_weights(&p.weights(&owner), i, ratio);
                if end {
                    p.drag_panes = None;
                    p.set_weights(&owner, w);
                    p.save();
                } else {
                    p.drag_panes = Some((owner, w));
                }
            }
            PaneDivider::Tree => {
                if end {
                    p.drag_tree = None;
                    p.saved.tree_ratio = Some(crate::side::clamp_split(ratio));
                    p.save();
                } else {
                    p.drag_tree = Some(ratio);
                }
            }
            PaneDivider::Changes => {
                // The handle sits inside the panel; the width is the row's right
                // edge less the pointer, as today's drag measures it.
                let (Some(row), Some((x, _))) = (
                    ui.rect_of(crate::screen::git::changes_row_id()),
                    ui.pointer(),
                ) else {
                    return true;
                };
                self.st.git.maximized = false;
                self.st.git.width =
                    Some((row.x + row.w - x).clamp(crate::git::PANEL_MIN, crate::git::PANEL_MAX));
            }
        }
        true
    }
}
