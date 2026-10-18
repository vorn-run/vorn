//! Files: a session's file tree and open files in a pane beside its
//! terminal, each file edited, found in, saved with a guard against writing
//! over a file that changed on disk, its unsaved edit kept as a draft, and
//! files popped out into cards of their own.
//!
//! Vorn ships it as a Wasm component (`vorn-extension.toml` beside this
//! crate). It reaches the file system only through Vorn's `files.*` calls.
//! The panes, tabs, cards and drafts live in its storage and rows, so a
//! restart of its instance loses nothing a person sees; what was listed and
//! read is read again.

use std::collections::HashMap;

use vorn_extension_sdk::tree::{self, Event, EventKind, Surface};
use vorn_extension_sdk::{arg, json, serde_json, Ctx, Error, Extension, Host, Init, Op, Value};

mod buffer;
mod listing;
mod pane;
mod view;

pub use buffer::{has_moved, is_truncated, Buffer, Draft, Stamp};
pub use listing::{Entry, Tree};
pub use pane::{Card, Cards, Pane};

/// The pane beside a session's terminal, drawn for the session as its scope.
pub const PANE: &str = "files";
/// A popped-out file's card, drawn for the card's id as its scope.
pub const CARD: &str = "file";
/// One row per session whose Files pane is open, its id the session's.
pub const OPEN: &str = "open";
/// One row per card: `{id, session, path, title}`.
pub const CARDS: &str = "cards";
/// Below this width the tree and the file take turns.
pub const SIDE_BY_SIDE_MIN: f32 = 480.0;
/// The tree's share of the pane until the divider moves.
pub const DEFAULT_TREE: f32 = 0.38;
/// What the question before throwing edits away asks.
pub const DISCARD: &str = "Discard unsaved changes?";
/// The event a question's OK sends.
const CONFIRMED: &str = "discard-ok";

/// An open tab's buffer and draft key.
pub fn tab_key(owner: &str, path: &str) -> String {
    format!("files:{owner}:{path}")
}

fn pane_key(owner: &str) -> String {
    format!("pane/{owner}")
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn failed(e: impl std::fmt::Display) -> Error {
    Error::new("failed", e.to_string())
}

/// What waits for the answer to "Discard unsaved changes?".
#[derive(Debug, Clone, PartialEq, Eq)]
enum Pending {
    Toggle(String),
    CloseTab(String, String),
    PopActive(String),
    ReturnCard(String),
    CloseCard(String),
    Discard(String),
}

/// The extension: what it listed and read this instance, and the question asked.
#[derive(Debug, Default)]
pub struct Files {
    trees: HashMap<String, Tree>,
    buffers: HashMap<String, Buffer>,
    pending: Option<Pending>,
}

/// What an event answers: nothing to change, a part to draw again, the whole tree, or a question.
fn answer_ask(event: &Event) -> Value {
    json!({ "ask": { "title": DISCARD, "event": Event::new(CONFIRMED, EventKind::Click, "")
        .scoped(event.scope.clone()) } })
}

trait Scoped {
    fn scoped(self, scope: Option<String>) -> Self;
}

impl Scoped for Event {
    fn scoped(mut self, scope: Option<String>) -> Event {
        self.scope = scope;
        self
    }
}

impl Files {
    fn pane(&self, host: &mut dyn Host, owner: &str) -> Result<Option<Pane>, Error> {
        Ok(host
            .get(&pane_key(owner))?
            .and_then(|v| serde_json::from_value(v).ok()))
    }

    fn save_pane(&self, host: &mut dyn Host, owner: &str, p: &Pane) -> Result<(), Error> {
        host.set(&pane_key(owner), &serde_json::to_value(p).map_err(failed)?)?;
        // Every change announces the session's row again, so clients draw that pane again.
        host.publish(OPEN, vec![Op::Upsert(json!({ "id": owner, "at": now_ms() }))]);
        Ok(())
    }

    fn cards(&self, host: &mut dyn Host) -> Result<Cards, Error> {
        Ok(host
            .get(CARDS)?
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default())
    }

    fn save_cards(&self, host: &mut dyn Host, c: &Cards) -> Result<(), Error> {
        host.set(CARDS, &serde_json::to_value(c).map_err(failed)?)?;
        let rows: Vec<Op> = c
            .all
            .iter()
            .map(|c| {
                Op::Upsert(json!({
                    "id": c.id, "session": c.session, "path": c.path,
                    "title": view::base(&c.path), "at": now_ms(),
                }))
            })
            .collect();
        host.publish(CARDS, std::iter::once(Op::Clear).chain(rows).collect());
        Ok(())
    }

    /// The session's working folder: its worktree, else its project.
    fn cwd(host: &mut dyn Host, owner: &str) -> Option<String> {
        let s = host
            .call("sessions.get", json!({ "sessionId": owner }))
            .ok()?;
        let text = |k: &str| s.get(k).and_then(Value::as_str).filter(|p| !p.is_empty());
        text("worktreePath").or(text("projectPath")).map(str::to_owned)
    }

    /// `owner`'s tree, its top listed again when the session's folder moved.
    fn tree(&mut self, host: &mut dyn Host, owner: &str) -> &mut Tree {
        let cwd = Files::cwd(host, owner).unwrap_or_default();
        let stale = self.trees.get(owner).is_none_or(|t| t.cwd != cwd);
        if stale {
            let root = host
                .call("files.list", json!({ "path": cwd }))
                .map(|v| listing::entries(&v))
                .unwrap_or_default();
            self.trees.insert(
                owner.to_owned(),
                Tree {
                    cwd: cwd.clone(),
                    root,
                    ..Tree::default()
                },
            );
        }
        self.trees.entry(owner.to_owned()).or_default()
    }

    /// Buffer `key` for `path`, read the first time it is asked for.
    fn buffer(&mut self, host: &mut dyn Host, key: &str, path: &str) -> Result<&mut Buffer, Error> {
        if self.buffers.get(key).is_none_or(|b| b.path != path) {
            let b = Buffer::load(host, key, path, now_ms())?;
            self.buffers.insert(key.to_owned(), b);
        }
        Ok(self.buffers.entry(key.to_owned()).or_insert_with(|| Buffer {
            path: path.to_owned(),
            content: None,
            state: tree::CodeState::Loading,
            draft: None,
            base: None,
            conflict: false,
            error: None,
            rev: 0,
        }))
    }

    /// Whether buffer `key` holds edits, as this instance or its kept draft knows.
    fn dirty(&self, host: &mut dyn Host, key: &str) -> bool {
        match self.buffers.get(key) {
            Some(b) => b.dirty(),
            None => buffer::read_draft(host, key).ok().flatten().is_some(),
        }
    }

    fn forget(&mut self, host: &mut dyn Host, keys: &[String]) -> Result<(), Error> {
        for k in keys {
            self.buffers.remove(k);
            buffer::forget_draft(host, k)?;
        }
        Ok(())
    }

    /// Opens `owner`'s pane, or closes it unless it holds edits and `force` is not given.
    fn toggle(&mut self, host: &mut dyn Host, owner: &str, force: bool) -> Result<bool, Error> {
        let Some(p) = self.pane(host, owner)? else {
            self.save_pane(host, owner, &Pane::default())?;
            return Ok(true);
        };
        let keys: Vec<String> = p.tabs.iter().map(|t| tab_key(owner, t)).collect();
        if !force && keys.iter().any(|k| self.dirty(host, k)) {
            self.pending = Some(Pending::Toggle(owner.to_owned()));
            return Ok(false);
        }
        host.set(&pane_key(owner), &Value::Null)?;
        host.publish(OPEN, vec![Op::Delete(owner.to_owned())]);
        self.trees.remove(owner);
        self.forget(host, &keys)?;
        Ok(true)
    }

    /// A file clicked (previewed) or double-clicked (kept); a narrow pane hides the tree to show it.
    fn open_file(&mut self, host: &mut dyn Host, owner: &str, path: &str, pin: bool, narrow: bool) -> Result<(), Error> {
        let mut p = self.pane(host, owner)?.unwrap_or_default();
        let preview_dirty = p
            .preview
            .as_deref()
            .is_some_and(|pv| self.dirty(host, &tab_key(owner, pv)));
        p.open(path, pin, preview_dirty);
        if narrow {
            p.tree_visible = false;
        }
        self.save_pane(host, owner, &p)
    }

    fn close_tab(&mut self, host: &mut dyn Host, owner: &str, path: &str, force: bool) -> Result<bool, Error> {
        let key = tab_key(owner, path);
        if !force && self.dirty(host, &key) {
            self.pending = Some(Pending::CloseTab(owner.to_owned(), path.to_owned()));
            return Ok(false);
        }
        let mut p = self.pane(host, owner)?.unwrap_or_default();
        p.close(path);
        self.save_pane(host, owner, &p)?;
        self.forget(host, &[key])?;
        Ok(true)
    }

    /// A file into a card of its own; `from_tab` takes it out of the pane, asking first over edits.
    fn pop(&mut self, host: &mut dyn Host, owner: &str, path: &str, from_tab: bool, force: bool) -> Result<bool, Error> {
        let key = tab_key(owner, path);
        if from_tab && !force && self.dirty(host, &key) {
            self.pending = Some(Pending::PopActive(owner.to_owned()));
            return Ok(false);
        }
        let mut cards = self.cards(host)?;
        cards.promote(owner, path);
        self.save_cards(host, &cards)?;
        if from_tab {
            let mut p = self.pane(host, owner)?.unwrap_or_default();
            p.close(path);
            self.save_pane(host, owner, &p)?;
            self.forget(host, &[key])?;
        }
        Ok(true)
    }

    fn card_out(&mut self, host: &mut dyn Host, id: &str, back: bool, force: bool) -> Result<bool, Error> {
        if !force && self.dirty(host, id) {
            self.pending = Some(if back {
                Pending::ReturnCard(id.to_owned())
            } else {
                Pending::CloseCard(id.to_owned())
            });
            return Ok(false);
        }
        let mut cards = self.cards(host)?;
        let Some(c) = cards.remove(id) else {
            return Ok(true);
        };
        self.save_cards(host, &cards)?;
        self.forget(host, &[id.to_owned()])?;
        if back {
            let mut p = self.pane(host, &c.session)?.unwrap_or_default();
            p.open(&c.path, true, false);
            self.save_pane(host, &c.session, &p)?;
        }
        Ok(true)
    }

    /// What the question's OK was asked for, done without asking again.
    fn confirmed(&mut self, host: &mut dyn Host) -> Result<(), Error> {
        match self.pending.take() {
            Some(Pending::Toggle(o)) => self.toggle(host, &o, true).map(drop),
            Some(Pending::CloseTab(o, p)) => self.close_tab(host, &o, &p, true).map(drop),
            Some(Pending::PopActive(o)) => {
                let active = self.pane(host, &o)?.and_then(|p| p.active);
                match active {
                    Some(path) => self.pop(host, &o, &path, true, true).map(drop),
                    None => Ok(()),
                }
            }
            Some(Pending::ReturnCard(id)) => self.card_out(host, &id, true, true).map(drop),
            Some(Pending::CloseCard(id)) => self.card_out(host, &id, false, true).map(drop),
            Some(Pending::Discard(key)) => match self.buffers.get_mut(&key) {
                Some(b) => b.discard(host, &key, now_ms()),
                None => Ok(()),
            },
            None => Ok(()),
        }
    }

    /// Something done to an open file's code: an edit, a save, a discard or the moved-file banner's answers.
    fn code_event(&mut self, host: &mut dyn Host, key: &str, path: &str, e: &Event) -> Result<Value, Error> {
        let b = self.buffer(host, key, path)?;
        match (e.kind, e.value.as_str()) {
            (EventKind::Change, _) => {
                let was = b.dirty();
                b.edited(host, key, e.value.clone(), now_ms())?;
                if was == b.dirty() {
                    return Ok(json!({ "same": true }));
                }
                // Draws the tabs again with their dots; an edited preview is kept.
                Ok(Value::Null)
            }
            (EventKind::Submit, _) => {
                b.save(host, key, e.value.clone(), false)?;
                Ok(Value::Null)
            }
            (EventKind::Click, "force") => {
                let text = b.draft.clone().unwrap_or_default();
                b.save(host, key, text, true)?;
                Ok(Value::Null)
            }
            (EventKind::Click, "disk") => {
                b.take_disk(host, key, now_ms())?;
                Ok(Value::Null)
            }
            (EventKind::Click, "keep") => {
                b.conflict = false;
                Ok(Value::Null)
            }
            (EventKind::Click, "discard") => {
                if b.dirty() {
                    self.pending = Some(Pending::Discard(key.to_owned()));
                    return Ok(answer_ask(e));
                }
                Ok(Value::Null)
            }
            _ => Ok(json!({ "same": true })),
        }
    }

    fn pane_event(&mut self, host: &mut dyn Host, owner: &str, e: &Event) -> Result<Value, Error> {
        let narrow = e.width.is_some_and(|w| (w as f32) < SIDE_BY_SIDE_MIN);
        let ask = |done: bool| if done { Value::Null } else { answer_ask(e) };
        let v = e.value.as_str();
        match (e.id.as_str(), e.kind) {
            (CONFIRMED, _) => self.confirmed(host).map(|_| Value::Null),
            ("pane", EventKind::Close) => self.toggle(host, owner, false).map(ask),
            ("tabs", EventKind::Select) => {
                let mut p = self.pane(host, owner)?.unwrap_or_default();
                p.show(v);
                if narrow {
                    p.tree_visible = false;
                }
                self.save_pane(host, owner, &p).map(|_| Value::Null)
            }
            ("tabs", EventKind::Open) => {
                let mut p = self.pane(host, owner)?.unwrap_or_default();
                p.pin(v);
                self.save_pane(host, owner, &p).map(|_| Value::Null)
            }
            ("tabs", EventKind::Close) => self.close_tab(host, owner, v, false).map(ask),
            ("toggle-tree", EventKind::Click) => {
                let mut p = self.pane(host, owner)?.unwrap_or_default();
                p.tree_visible = !p.tree_visible;
                self.save_pane(host, owner, &p).map(|_| Value::Null)
            }
            ("pop", EventKind::Click) => {
                let Some(path) = self.pane(host, owner)?.and_then(|p| p.active) else {
                    return Ok(Value::Null);
                };
                self.pop(host, owner, &path, true, false).map(ask)
            }
            ("close-saved", EventKind::Click) => {
                let mut p = self.pane(host, owner)?.unwrap_or_default();
                let dirty: Vec<String> = p
                    .tabs
                    .iter()
                    .filter(|t| self.dirty(host, &tab_key(owner, t)))
                    .cloned()
                    .collect();
                let gone: Vec<String> = p
                    .tabs
                    .iter()
                    .filter(|t| !dirty.contains(t))
                    .map(|t| tab_key(owner, t))
                    .collect();
                p.close_saved(|t| dirty.iter().any(|d| d == t));
                self.save_pane(host, owner, &p)?;
                self.forget(host, &gone).map(|_| Value::Null)
            }
            ("split", EventKind::Resize) => {
                let r: f32 = v.parse().map_err(|_| Error::bad_args("a ratio"))?;
                let r = if r.is_finite() {
                    r.clamp(tree::MIN_SPLIT, tree::MAX_SPLIT)
                } else {
                    DEFAULT_TREE
                };
                host.set("ratio", &json!(r))?;
                Ok(Value::Null)
            }
            ("tree", EventKind::Select) => {
                let t = self.tree(host, owner);
                if t.is_dir(v) {
                    if !t.expanded.remove(v) {
                        t.expanded.insert(v.to_owned());
                        if !t.dirs.contains_key(v) {
                            let listed = host
                                .call("files.list", json!({ "path": v }))
                                .map(|l| listing::entries(&l))
                                .unwrap_or_default();
                            self.tree(host, owner).dirs.insert(v.to_owned(), listed);
                        }
                    }
                    return Ok(Value::Null);
                }
                self.open_file(host, owner, v, false, narrow).map(|_| Value::Null)
            }
            ("tree", EventKind::Open) => self.open_file(host, owner, v, true, narrow).map(|_| Value::Null),
            ("tree", EventKind::Click) => self.pop(host, owner, v, false, false).map(|_| Value::Null),
            ("tree", EventKind::Change) => {
                self.tree(host, owner).filter = e.value.clone();
                Ok(Value::Null)
            }
            ("code", _) => {
                let Some(path) = self.pane(host, owner)?.and_then(|p| p.active) else {
                    return Ok(json!({ "same": true }));
                };
                let key = tab_key(owner, &path);
                let done = self.code_event(host, &key, &path, e)?;
                if e.kind == EventKind::Change && self.dirty(host, &key) {
                    let mut p = self.pane(host, owner)?.unwrap_or_default();
                    if p.preview.as_deref() == Some(path.as_str()) {
                        p.pin(&path);
                        self.save_pane(host, owner, &p)?;
                    }
                }
                Ok(done)
            }
            _ => Ok(json!({ "same": true })),
        }
    }

    fn card_event(&mut self, host: &mut dyn Host, id: &str, e: &Event) -> Result<Value, Error> {
        let ask = |done: bool| if done { Value::Null } else { answer_ask(e) };
        match (e.id.as_str(), e.kind) {
            (CONFIRMED, _) => self.confirmed(host).map(|_| Value::Null),
            ("return", EventKind::Click) => self.card_out(host, id, true, false).map(ask),
            ("close", EventKind::Click) => self.card_out(host, id, false, false).map(ask),
            ("code", _) => {
                let Some(card) = self.cards(host)?.get(id).cloned() else {
                    return Ok(json!({ "same": true }));
                };
                self.code_event(host, id, &card.path, e)
            }
            _ => Ok(json!({ "same": true })),
        }
    }

    fn act(&mut self, host: &mut dyn Host, action: &str, args: &Value) -> Result<Value, Error> {
        match action {
            "toggle" => {
                let owner = arg(args, "session", action)?;
                Ok(if self.toggle(host, owner, false)? {
                    json!({})
                } else {
                    answer_ask(&Event::new(CONFIRMED, EventKind::Click, "").scoped(Some(owner.to_owned())))
                })
            }
            // What sessions are left: panes and cards of the rest go.
            "prune" => {
                let live: Vec<&str> = args
                    .get("live")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .collect();
                let open: Vec<String> = host
                    .rows(OPEN)?
                    .iter()
                    .filter_map(|r| r.get("id").and_then(Value::as_str).map(str::to_owned))
                    .collect();
                for owner in open.iter().filter(|o| !live.contains(&o.as_str())) {
                    if let Some(p) = self.pane(host, owner)? {
                        let keys: Vec<String> = p.tabs.iter().map(|t| tab_key(owner, t)).collect();
                        self.forget(host, &keys)?;
                    }
                    host.set(&pane_key(owner), &Value::Null)?;
                    host.publish(OPEN, vec![Op::Delete(owner.clone())]);
                    self.trees.remove(owner);
                }
                let mut cards = self.cards(host)?;
                let before = cards.all.len();
                cards.all.retain(|c| live.contains(&c.session.as_str()));
                if cards.all.len() != before {
                    self.save_cards(host, &cards)?;
                }
                Ok(Value::Null)
            }
            // What the desktop kept on its own before: its panes, cards and drafts.
            "import" => self.import(host, args),
            other => Err(Error::unknown(other)),
        }
    }

    fn import(&mut self, host: &mut dyn Host, args: &Value) -> Result<Value, Error> {
        let mut n = 0;
        if let Some(panes) = args.pointer("/view/files").and_then(Value::as_object) {
            for (owner, p) in panes {
                if self.pane(host, owner)?.is_none() {
                    if let Ok(p) = serde_json::from_value::<Pane>(p.clone()) {
                        self.save_pane(host, owner, &p)?;
                        n += 1;
                    }
                }
            }
        }
        if let Some(old) = args.pointer("/view/cards").and_then(Value::as_object) {
            let mut cards = self.cards(host)?;
            for c in old.values() {
                let text = |k: &str| c.get(k).and_then(Value::as_str);
                if let (Some(s), Some(p)) = (text("sessionId"), text("filePath")) {
                    cards.promote(s, p);
                    n += 1;
                }
            }
            self.save_cards(host, &cards)?;
        }
        if let Some(drafts) = args.get("drafts").and_then(Value::as_object) {
            let cards = self.cards(host)?;
            for (key, d) in drafts {
                let Ok(d) = serde_json::from_value::<Draft>(d.clone()) else {
                    continue;
                };
                // A card's draft follows the card it became.
                let key = match key.strip_prefix("card:") {
                    Some(_) => match cards.all.iter().find(|c| c.path == d.file_path) {
                        Some(c) => c.id.clone(),
                        None => continue,
                    },
                    None => key.clone(),
                };
                if buffer::read_draft(host, &key)?.is_none() {
                    host.set(&format!("draft/{key}"), &serde_json::to_value(&d).map_err(failed)?)?;
                    n += 1;
                }
            }
        }
        Ok(json!({ "imported": n }))
    }
}

impl Extension for Files {
    fn init(_: &Init, _: &mut dyn Host) -> Result<Self, Error> {
        Ok(Files::default())
    }

    fn handle(&mut self, host: &mut dyn Host, method: &str, args: Value, _: &Ctx) -> Result<Value, Error> {
        self.act(host, method, &args)
    }

    fn render(&mut self, host: &mut dyn Host, point: &str, surface: Surface) -> Result<tree::Node, Error> {
        let scope = surface.scope.unwrap_or_default();
        match point {
            PANE => view::pane(self, host, &scope),
            CARD => view::card(self, host, &scope),
            _ => Err(Error::new("unknown-action", format!("nothing draws {point}"))),
        }
    }

    fn event(&mut self, host: &mut dyn Host, point: &str, e: tree::Event) -> Result<Value, Error> {
        let scope = e.scope.clone().unwrap_or_default();
        let done = match point {
            PANE => self.pane_event(host, &scope, &e)?,
            CARD => self.card_event(host, &scope, &e)?,
            _ => return Err(Error::new("unknown-action", format!("{point} has no {}", e.id))),
        };
        // Null: the tree is drawn again; an object is the answer as it stands.
        Ok(match done {
            Value::Null => json!({}),
            other => other,
        })
    }
}

vorn_extension_sdk::export!(Files);

#[cfg(test)]
mod tests;
