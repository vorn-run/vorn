//! The artifact review the app draws around a page: comments pinned to
//! what they are about, the marks over the page, a design's versions and
//! controls, and its manifest. It sits beside the web primitive's host,
//! which shows the page; nothing here knows of the Browser's tabs.

use serde_json::{json, Map, Value};
use web_time::{Duration, Instant};

use super::web::{js, PaneCall, WaitKind, Watch};
use super::{App, Call};
use crate::artifact::{self, Anchor};
use crate::panes::{
    artifact_address, board_key, page_key, session_of, PageArtifact, PendingComment,
    SELECTION_SETTLE,
};
use crate::web::address;

/// How often a design file is checked for the agent's edits.
const WATCH_EVERY: Duration = Duration::from_millis(500);

impl App {
    pub(super) fn save_tweaks(&self) {
        let Some(file) = &self.web.tweaks_file else {
            return;
        };
        let saved = serde_json::to_string(&self.web.tweaks)
            .map_err(std::io::Error::other)
            .and_then(|body| crate::keep::write(file, &body));
        if let Err(e) = saved {
            eprintln!("design controls not saved: {e}");
        }
    }

    /// The artifact the session's page shows.
    pub(super) fn art(&self, session: &str) -> Option<PageArtifact> {
        self.st.bench.artifact(session).map(|(a, _)| a)
    }

    pub(super) fn refresh_artifact(&mut self, id: &str) {
        self.call(
            Call::Pane(PaneCall::Artifact(id.to_owned())),
            "artifact:get",
            json!({ "artifactId": id }),
        );
    }

    /// The session's page goes to `version` once its address is known.
    pub(super) fn show_version(&mut self, session: &str, id: &str, version: u32) {
        self.call(
            Call::Pane(PaneCall::Version {
                session: session.to_owned(),
                version,
            }),
            "artifact:versionUrl",
            json!({ "artifactId": id, "version": version }),
        );
    }

    /// What the page declares itself to be: a design's controls are put
    /// back as they were set for its file, and the page is told any that
    /// differ from what it holds.
    pub(super) fn manifest_read(&mut self, session: &str, key: &str, read: Option<Value>) {
        let declared = read
            .as_ref()
            .and_then(|v| v["declared"].as_str())
            .unwrap_or_default();
        let held = read.as_ref().and_then(|v| v["live"].as_object()).cloned();
        let manifest = artifact::parse_manifest(declared);
        let path = address::design_path(&self.page_url(session));
        let Some(m) = manifest else {
            let live = self.st.bench.live_mut(session);
            live.manifest = None;
            live.values.clear();
            self.web.watching.remove(session);
            return self.report_page(session);
        };
        let stored = path
            .as_ref()
            .and_then(|p| self.web.tweaks.get(p.to_string_lossy().as_ref()))
            .cloned()
            .unwrap_or_default();
        let merged = artifact::merge_tweaks(&m.tweaks, &stored, held.as_ref());
        let push: Map<String, Value> = merged
            .iter()
            .filter(|(k, v)| held.as_ref().and_then(|h| h.get(*k)) != Some(*v))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        if !push.is_empty() {
            self.web
                .host
                .run(key, &format!("window.__vorn.tweak({})", js(&push)));
        }
        match (&path, m.kind.as_str()) {
            (Some(p), "design") => {
                let modified = std::fs::metadata(p).and_then(|m| m.modified()).ok();
                self.web.watching.insert(
                    session.to_owned(),
                    Watch {
                        path: p.clone(),
                        modified,
                        checked: Instant::now(),
                    },
                );
            }
            _ => {
                self.web.watching.remove(session);
            }
        }
        let live = self.st.bench.live_mut(session);
        live.values = merged;
        live.manifest = Some(m);
        self.report_page(session);
    }

    /// A click on an artboard while commenting: the popover asks what to say.
    pub(super) fn pin_point(&mut self, session: &str, key: &str, msg: &Value) {
        let Some(board) = key.split_once("#board:").map(|(_, b)| b.to_owned()) else {
            return;
        };
        let live = self.st.bench.live_mut(session);
        if !live.commenting || live.pending.is_some() {
            return;
        }
        let (x, y) = (
            msg["x"].as_f64().unwrap_or(0.0),
            msg["y"].as_f64().unwrap_or(0.0),
        );
        let element = msg["element"].as_str().unwrap_or("element").to_owned();
        let found = self
            .st
            .bench
            .boards(session)
            .and_then(|b| b.iter().find(|a| a.id == board));
        let label = found.map_or_else(|| board.clone(), |a| a.label.clone());
        let area = found.map_or((f32::MAX, f32::MAX), |a| (a.width as f32, a.height as f32));
        let label = format!("{label} \u{b7} {element}");
        let at = artifact::place_popover((x as f32, y as f32, 0.0, 0.0), area);
        self.web.host.run(
            key,
            &format!(
                "window.__vorn.popover({}, {}, {}, {})",
                js(&label),
                at.0,
                at.1,
                artifact::POPOVER_WIDTH
            ),
        );
        self.st.bench.live_mut(session).pending = Some(PendingComment {
            anchor: Anchor::Point {
                artboard: board,
                x,
                y,
                element,
            },
            label,
            at,
        });
    }

    /// A selection that held still opens the popover over it.
    pub(super) fn settle_selections(&mut self, now: Instant) {
        let settled: Vec<String> = self
            .st
            .bench
            .live
            .iter()
            .filter(|(_, l)| {
                l.pending.is_none()
                    && l.selection
                        .as_ref()
                        .is_some_and(|(_, _, at)| now.duration_since(*at) >= SELECTION_SETTLE)
            })
            .map(|(s, _)| s.clone())
            .collect();
        for session in settled {
            let key = page_key(&session);
            let area = self.web.host_area(&key);
            let live = self.st.bench.live_mut(&session);
            let Some((anchor, rect, _)) = live.selection.take() else {
                continue;
            };
            let Anchor::Quote { quote, .. } = &anchor else {
                continue;
            };
            let label = quote.clone();
            let at = artifact::place_popover(rect, area);
            live.pending = Some(PendingComment {
                anchor,
                label: label.clone(),
                at,
            });
            self.web.host.run(
                &key,
                &format!(
                    "window.__vorn.popover({}, {}, {}, {})",
                    js(&label),
                    at.0,
                    at.1,
                    artifact::POPOVER_WIDTH
                ),
            );
        }
    }

    /// The popover goes, and the page's own selection.
    pub(super) fn drop_selection(&mut self, session: &str) {
        let live = self.st.bench.live_mut(session);
        live.pending = None;
        live.selection = None;
        let keys = self.session_keys(session);
        for key in keys {
            self.web.host.run(&key, "window.__vorn.clear()");
        }
        self.web.painted.retain(|k, _| session_of(k) != session);
    }

    pub(super) fn reveal(&mut self, session: &str, id: &str) {
        let point = self
            .quote_comments(session)
            .0
            .into_iter()
            .chain(self.quote_comments(session).1)
            .find(|c| c.id == id)
            .and_then(|c| match &c.anchor {
                Some(Anchor::Point { artboard, .. }) => Some(artboard.clone()),
                _ => None,
            });
        let live = self.st.bench.live_mut(session);
        live.focus = Some(id.to_owned());
        match point {
            Some(board) => live.board = Some(board),
            None => self.web.host.run(
                &page_key(session),
                &format!("window.__vorn.reveal({})", js(id)),
            ),
        }
    }

    /// The comments' marks on an artifact page, and pins on artboards, sent
    /// whenever they change.
    pub(super) fn paint_marks(&mut self) {
        let sessions: Vec<String> = self.st.bench.pages.keys().cloned().collect();
        for session in sessions {
            if self.art(&session).is_none() {
                continue;
            }
            let live = self.st.bench.live(&session);
            let commenting = live.is_some_and(|l| l.commenting);
            let pinning = commenting && live.is_some_and(|l| l.pending.is_none());
            let focus = live.and_then(|l| l.focus.clone());
            let (drafts, sent) = self.quote_comments(&session);
            match self.st.bench.boards(&session) {
                None => {
                    let marks = if commenting {
                        artifact::marks_for(&drafts, &sent, focus.as_deref())
                    } else {
                        Vec::new()
                    };
                    let script = format!("JSON.stringify(window.__vorn.marks({}))", js(&marks));
                    let key = page_key(&session);
                    if self.web.host.has(&key) && self.web.painted.get(&key) != Some(&script) {
                        self.web.painted.insert(key.clone(), script.clone());
                        self.ask_quietly(&key, &script, WaitKind::Marks { session });
                    }
                }
                Some(boards) => {
                    let pins: Vec<(String, Value)> = drafts
                        .iter()
                        .chain(&sent)
                        .filter_map(|c| match &c.anchor {
                            Some(Anchor::Point { artboard, x, y, .. }) => {
                                Some((c, artboard, *x, *y))
                            }
                            _ => None,
                        })
                        .enumerate()
                        .map(|(i, (c, board, x, y))| {
                            let state = if focus.as_deref() == Some(c.id.as_str()) {
                                "focus"
                            } else if c.is_draft() {
                                "draft"
                            } else {
                                "sent"
                            };
                            let pin =
                                json!({ "id": c.id, "n": i + 1, "x": x, "y": y, "state": state });
                            (board.clone(), pin)
                        })
                        .collect();
                    let keys: Vec<(String, String)> = boards
                        .iter()
                        .map(|b| {
                            let list: Vec<&Value> = pins
                                .iter()
                                .filter(|(on, _)| *on == b.id && commenting)
                                .map(|(_, p)| p)
                                .collect();
                            (
                                board_key(&session, &b.id),
                                format!("window.__vorn.pins({pinning}, {})", js(&list)),
                            )
                        })
                        .collect();
                    for (key, script) in keys {
                        if self.web.host.has(&key) && self.web.painted.get(&key) != Some(&script) {
                            self.web.host.run(&key, &script);
                            self.web.painted.insert(key, script);
                        }
                    }
                }
            }
        }
    }

    /// A design that changed on disk is shown again.
    pub(super) fn watch_files(&mut self, now: Instant) {
        let mut changed = Vec::new();
        for (session, w) in &mut self.web.watching {
            if now.duration_since(w.checked) < WATCH_EVERY {
                continue;
            }
            w.checked = now;
            let modified = std::fs::metadata(&w.path).and_then(|m| m.modified()).ok();
            if modified != w.modified {
                w.modified = modified;
                changed.push(session.clone());
            }
        }
        for session in changed {
            for key in self.session_keys(&session) {
                self.web.host.reload(&key);
            }
        }
    }

    /// An artifact's address carries the server's port, which a restart
    /// changes: each page's version is asked for again once, and each
    /// artifact shown is fetched, which is what makes its page a review.
    pub(super) fn refresh_artifact_pages(&mut self) {
        let mut versions = Vec::new();
        let mut fetch = Vec::new();
        for (session, url) in &self.st.bench.pages {
            let Some((id, version)) = artifact_address(url) else {
                continue;
            };
            let tag = format!("{id}:{version}");
            let seen = self
                .st
                .bench
                .live(session)
                .is_some_and(|l| l.refreshed.contains(&tag));
            if !seen {
                versions.push((session.clone(), id.clone(), version, tag));
            }
            if !self.web.fetched.contains(&id) {
                fetch.push(id);
            }
        }
        for (session, id, version, tag) in versions {
            self.st.bench.live_mut(&session).refreshed.insert(tag);
            self.show_version(&session, &id, version);
        }
        for id in fetch {
            if self.web.fetched.insert(id.clone()) {
                self.refresh_artifact(&id);
            }
        }
    }

    pub(super) fn save_comment(
        &mut self,
        session: &str,
        art: &PageArtifact,
        anchor: Option<Anchor>,
        body: String,
    ) {
        let error = if anchor.is_some() {
            "Could not save the comment"
        } else {
            "Could not save the note"
        };
        self.call(
            Call::Pane(PaneCall::Comment {
                session: session.to_owned(),
                artifact: art.id.clone(),
                error: Some(error),
            }),
            "artifact:saveComment",
            json!({ "artifactId": art.id, "version": art.version, "anchor": anchor, "body": body }),
        );
    }

    pub(super) fn save_doc(&mut self, session: &str, send: bool) {
        let Some(art) = self.art(session) else {
            return;
        };
        let Some(draft) = self.st.bench.live(session).and_then(|l| l.editing.clone()) else {
            return;
        };
        let (body, edits) = artifact::merge_doc_edit(&draft.original, &draft.edited);
        if edits.is_empty() {
            return;
        }
        self.st.bench.live_mut(session).saving = true;
        self.call(
            Call::Pane(PaneCall::SavedDoc {
                session: session.to_owned(),
                artifact: art.id.clone(),
            }),
            "artifact:saveUserVersion",
            json!({ "artifactId": art.id, "body": body, "edits": edits, "send": send }),
        );
    }

    /// Shown at once, kept for the design's file, and
    /// written into every view of the page.
    pub(super) fn apply_tweak(&mut self, session: &str, key: &str, value: Value) {
        if let Some(path) = address::design_path(&self.page_url(session)) {
            self.web
                .tweaks
                .entry(path.to_string_lossy().into_owned())
                .or_default()
                .insert(key.to_owned(), value.clone());
            self.save_tweaks();
        }
        self.st
            .bench
            .live_mut(session)
            .values
            .insert(key.to_owned(), value.clone());
        let script = format!("window.__vorn.tweak({})", js(json!({ key: value })));
        for k in self.session_keys(session) {
            self.web.host.run(&k, &script);
        }
    }
}
