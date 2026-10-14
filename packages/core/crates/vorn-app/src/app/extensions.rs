//! Extension panes against vornd (`openExtensionPane`, `closeExtensionPane`,
//! `hydrateExtensions`): the packs, each session's activation, and the pane
//! a card has open.

use serde_json::{json, Value};

use super::{App, Call};
use crate::client::RpcError;
use crate::extensions::{ExtAct, OpenPane};
use crate::panes::PaneKind;

/// An extension call on its way.
#[derive(Debug)]
pub(super) enum ExtCall {
    List,
    Activation(String),
    Open(String),
    /// `ext:list`: the sections of extensions in two tiers.
    Sections,
}

impl App {
    pub(super) fn run_ext(&mut self, a: ExtAct) {
        match a {
            ExtAct::Toggle {
                session,
                extension,
                pane,
            } => {
                let showing = self
                    .st
                    .ext
                    .panes
                    .get(&session)
                    .is_some_and(|p| p.extension_id == extension && p.pane_id == pane);
                if showing {
                    return self.ext_close(&session);
                }
                // Asked for before anything is committed: the grant is the pane.
                self.call(
                    Call::Ext(ExtCall::Open(session.clone())),
                    "extension:openPane",
                    json!({ "extensionId": extension, "paneId": pane, "sessionId": session }),
                );
            }
            ExtAct::Close(session) => self.ext_close(&session),
            ExtAct::Focus(session) => {
                self.st.selected = Some(session.clone());
                self.st.side.focus = None;
                self.st.ext.focus = Some(session);
            }
        }
    }

    /// Hands the pane's grant back and drops it from the card.
    pub(super) fn ext_close(&mut self, session: &str) {
        let Some(p) = self.st.ext.panes.remove(session) else {
            return;
        };
        self.call(
            Call::Quiet,
            "extension:closePane",
            json!({ "nonce": p.nonce }),
        );
        if self.st.bench.maximized.get(session) == Some(&PaneKind::Extension) {
            self.st.bench.maximized.remove(session);
        }
        if self.st.ext.focus.as_deref() == Some(session) {
            self.st.ext.focus = None;
        }
    }

    pub(super) fn ext_answered(&mut self, call: ExtCall, answer: Result<Value, RpcError>) {
        match call {
            // Asked once; an install or removal asks again.
            ExtCall::List => {
                self.st.ext.loaded = true;
                if let Ok(Value::Array(packs)) = answer {
                    self.st.ext.packs = packs;
                }
            }
            // A session the host does not know yet is told about by its activation push.
            ExtCall::Activation(session) => {
                if let Ok(Value::Array(states)) = answer {
                    self.ext_activation(&session, states);
                }
            }
            ExtCall::Sections => {
                if let Ok(v) = answer {
                    self.st.tiers.take_all(&v);
                } else {
                    // A vornd without the tiers host: nothing to show, nothing to ask again.
                    self.st.tiers.loaded = true;
                }
            }
            ExtCall::Open(session) => match answer {
                Ok(grant) => {
                    let Some(open) = OpenPane::from_grant(&grant, &self.st.ext.packs) else {
                        return;
                    };
                    // A card holds one, so whatever was showing hands its grant back.
                    self.ext_close(&session);
                    if self.store.session(&session).is_some() {
                        self.st.ext.panes.insert(session, open);
                    } else {
                        self.call(
                            Call::Quiet,
                            "extension:closePane",
                            json!({ "nonce": open.nonce }),
                        );
                    }
                }
                Err(e) => self.st.toast(e.to_string(), true),
            },
        }
    }

    fn ext_activation(&mut self, session: &str, states: Vec<Value>) {
        if !self.st.ext.still_shown(session, &states) {
            self.ext_close(session);
        }
        if states.is_empty() {
            self.st.ext.activation.remove(session);
        } else {
            self.st.ext.activation.insert(session.to_owned(), states);
        }
    }

    pub(super) fn ext_notified(&mut self, method: &str, params: &Value) {
        if method == "ext:section" {
            self.st.tiers.take(params);
        }
        if method == "extension:activation" {
            let session = params["sessionId"].as_str().unwrap_or_default().to_owned();
            let states = params["states"].as_array().cloned().unwrap_or_default();
            self.ext_activation(&session, states);
        }
    }

    /// Once a frame: the pack list, each new session asked about once, and
    /// panes whose session has gone.
    pub(super) fn ext_tick(&mut self) {
        if !self.st.ext.loaded && !self.ext_loading() {
            self.call(Call::Ext(ExtCall::List), "extension:list", Value::Null);
        }
        let asking = self
            .calls
            .iter()
            .any(|(c, _)| matches!(c, Call::Ext(ExtCall::Sections)));
        if !self.st.tiers.loaded && !asking {
            self.call(Call::Ext(ExtCall::Sections), "ext:list", Value::Null);
        }
        let fresh: Vec<String> = self
            .store
            .sessions
            .iter()
            .filter(|s| !s.ended())
            .map(|s| s.info.id.clone())
            .filter(|id| !self.st.ext.hydrated.contains(id))
            .collect();
        for id in fresh {
            self.st.ext.hydrated.insert(id.clone());
            self.call(
                Call::Ext(ExtCall::Activation(id.clone())),
                "extension:activation",
                json!({ "sessionId": id }),
            );
        }
        let gone: Vec<String> = self
            .st
            .ext
            .panes
            .keys()
            .filter(|s| self.store.session(s).is_none())
            .cloned()
            .collect();
        for s in gone {
            self.ext_close(&s);
        }
        let store = &self.store;
        self.st
            .ext
            .activation
            .retain(|s, _| store.session(s).is_some());
        self.st.ext.hydrated.retain(|s| store.session(s).is_some());
        self.st.ext.slots = self.st.ext.terminals();
    }

    fn ext_loading(&self) -> bool {
        self.calls
            .iter()
            .any(|(c, _)| matches!(c, Call::Ext(ExtCall::List)))
    }
}
