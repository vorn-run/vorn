//! What the Remote Access page's controls do, and the answers to its calls:
//! `tailscale:status`, `server:reachableUrls`, `token:*` and `pairing:*`.

use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::{App, Call};
use crate::client::rpc::decode;
use crate::client::RpcError;
use crate::network::{
    connect_error, Adding, Code, DeviceToken, Minted, NetAct, Network, PairingRequest, Reachable,
    Tailscale, DEVICE_NAME, HOST_TOKEN, HOST_URL, REBIND_SETTLE,
};
use crate::settings::Category;
use crate::ui::{Event as UiEvent, Ui};

/// A Remote Access call whose answer changes the screen.
pub(super) enum NetCall {
    Status,
    Reachable,
    Tokens,
    /// `pairing:start`: a code to show.
    Started,
    /// `token:create`: the plaintext, shown once.
    Created,
    Revoked,
    /// `pairing:approve` or `pairing:deny`.
    Decided,
    /// Answers nobody reads: retiring a code, saving the share switch.
    Ignored,
}

fn now_ms() -> u64 {
    // Wall-clock ms since the epoch, as vornd stamps `expiresAt`.
    crate::state::now_ms().max(0.0) as u64
}

impl App {
    pub(super) fn run_net(&mut self, ui: &mut Ui, act: NetAct) {
        let n = &mut self.st.network;
        match act {
            NetAct::Refresh => {
                self.st.network = Network {
                    open: true,
                    ..Network::default()
                };
                self.net_fetch_status();
                self.net_load_tokens();
            }
            NetAct::View(mode) => {
                n.viewing = mode;
                if mode == crate::network::Mode::Host {
                    ui.set_focus(Some(HOST_URL));
                }
            }
            NetAct::Share(on) => {
                let tailnet = n.status.as_ref().is_some_and(|s| s.running);
                if !on || tailnet {
                    self.net_share(on);
                } else {
                    n.confirming = true;
                }
            }
            NetAct::EnableAnyway => self.net_share(true),
            NetAct::CancelShare => n.confirming = false,
            NetAct::ToggleAddresses => n.show_all = !n.show_all,
            NetAct::Copy(text) => {
                ui.clipboard().set(&text);
                n.copied = Some((text, Instant::now()));
            }
            NetAct::AddDevice => {
                if n.can_scan() {
                    self.net_start_scan();
                } else {
                    n.adding = Some(Adding::Token);
                    net_name_form(ui);
                }
            }
            NetAct::ShowAnother => self.net_start_scan(),
            NetAct::PairManually => {
                n.adding = Some(Adding::Token);
                net_name_form(ui);
                self.net_cancel_pairing();
            }
            NetAct::CloseAdding => {
                n.close_adding();
                ui.set_edit_text(DEVICE_NAME, "");
                // Retired, not merely hidden: a photographed code stops working.
                self.net_cancel_pairing();
            }
            NetAct::ChooseUrl(url) => n.chosen_url = Some(url),
            NetAct::CreateToken => {
                n.error = None;
                let typed = ui.edit_text(DEVICE_NAME);
                let name = match typed.trim() {
                    "" => "Device",
                    t => t,
                };
                let params = json!({ "name": name });
                self.call(Call::Net(NetCall::Created), "token:create", params);
            }
            NetAct::DoneMinted => n.minted = None,
            NetAct::AskRevoke(id) => n.confirm_revoke = Some(id),
            NetAct::CancelRevoke => n.confirm_revoke = None,
            NetAct::Revoke(id) => {
                n.confirm_revoke = None;
                self.call(Call::Net(NetCall::Revoked), "token:revoke", json!(id));
            }
            NetAct::Decide(approve) => {
                let Some(request) = n.asking.take() else {
                    return;
                };
                let method = if approve {
                    "pairing:approve"
                } else {
                    "pairing:deny"
                };
                let params = json!({ "requestId": request.request_id });
                self.call(Call::Net(NetCall::Decided), method, params);
            }
            NetAct::Connect => {
                let message = connect_error(ui.edit_text(HOST_URL), ui.edit_text(HOST_TOKEN));
                n.connect_error = Some(message.to_owned());
            }
        }
    }

    pub(super) fn net_answered(&mut self, call: NetCall, answer: Result<Value, RpcError>) {
        let n = &mut self.st.network;
        if !n.open {
            return;
        }
        match call {
            NetCall::Status => {
                n.loading = false;
                match answer.and_then(decode::<Tailscale>) {
                    Ok(status) => n.status = Some(status),
                    Err(_) => {
                        // Addresses go with the status: none is confirmed reachable.
                        n.status = Some(Tailscale::failed());
                        n.reachable = None;
                    }
                }
            }
            NetCall::Reachable => n.reachable = answer.and_then(decode::<Reachable>).ok(),
            NetCall::Tokens => match answer.and_then(decode::<Vec<DeviceToken>>) {
                Ok(tokens) => n.tokens = Some(tokens),
                Err(_) => n.error = Some("Could not read the device list.".to_owned()),
            },
            NetCall::Started => match answer.and_then(decode::<Code>) {
                Ok(code) => n.adding = Some(Adding::Scan(code)),
                Err(_) => n.error = Some("Could not start pairing.".to_owned()),
            },
            NetCall::Created => match answer {
                Ok(v) => {
                    let name = v["token"]["name"].as_str().unwrap_or_default().to_owned();
                    let plaintext = v["plaintext"].as_str().unwrap_or_default().to_owned();
                    n.minted = Some(Minted { name, plaintext });
                    n.adding = None;
                    self.net_load_tokens();
                }
                Err(_) => n.error = Some("Could not create a token.".to_owned()),
            },
            NetCall::Revoked => match answer {
                Ok(_) => self.net_load_tokens(),
                Err(_) => n.error = Some("Could not revoke that token.".to_owned()),
            },
            NetCall::Decided => {
                if answer.is_err() {
                    n.error = Some("Could not answer that request.".to_owned());
                }
                n.close_adding();
                self.net_cancel_pairing();
                self.net_load_tokens();
            }
            NetCall::Ignored => {}
        }
    }

    /// A vornd notification the page follows.
    pub(super) fn net_notified(&mut self, method: &str, params: &Value) {
        if !self.st.network.open {
            return;
        }
        match method {
            "pairing:requested" => {
                if let Ok(request) = decode::<PairingRequest>(params.clone()) {
                    self.st.network.asking = Some(request);
                }
            }
            // Approving mints nothing; the phone collecting does, a moment later.
            "pairing:collected" => self.net_load_tokens(),
            _ => {}
        }
    }

    pub(super) fn net_event(&mut self, ui: &mut Ui, event: &UiEvent) -> bool {
        match event {
            UiEvent::Submit(id) if *id == DEVICE_NAME => {
                self.run_net(ui, NetAct::CreateToken);
                true
            }
            _ => false,
        }
    }

    /// Once a frame: retires the page's code when it closes, asks for the
    /// addresses again once vornd has rebound, and wakes for the countdown.
    pub(super) fn net_tick(&mut self, ui: &mut Ui) {
        let shown = self.st.shown(Category::Network);
        match (shown, self.st.network.open) {
            (false, false) => return,
            (false, true) => {
                self.st.network.closed();
                // Closing the page puts any code away: it is only safe while watched.
                self.net_cancel_pairing();
                return;
            }
            // Just opened: load it.
            (true, false) => self.run_net(ui, NetAct::Refresh),
            (true, true) => {}
        }
        let now = Instant::now();
        if self.st.network.recheck_at.is_some_and(|at| at <= now) {
            self.st.network.recheck_at = None;
            self.net_fetch_status();
        }
        let n = &self.st.network;
        if let Some(at) = n.recheck_at {
            ui.wake_at(at);
        }
        if let Some((_, at)) = &n.copied {
            let until = *at + crate::network::COPIED_FOR;
            if until > now {
                ui.wake_at(until);
            }
        }
        if let Some(Adding::Scan(code)) = &n.adding {
            let left = code.remaining(now_ms());
            if left > 0 {
                // The next whole-second boundary, where the countdown changes.
                let step = match left % 1000 {
                    0 => 1000,
                    r => r,
                };
                ui.wake_at(now + Duration::from_millis(step));
            }
        }
    }

    fn net_fetch_status(&mut self) {
        self.st.network.loading = true;
        self.call(Call::Net(NetCall::Status), "tailscale:status", Value::Null);
        self.call(
            Call::Net(NetCall::Reachable),
            "server:reachableUrls",
            Value::Null,
        );
    }

    fn net_load_tokens(&mut self) {
        self.call(Call::Net(NetCall::Tokens), "token:list", Value::Null);
    }

    fn net_start_scan(&mut self) {
        self.st.network.error = None;
        self.call(Call::Net(NetCall::Started), "pairing:start", Value::Null);
    }

    fn net_cancel_pairing(&mut self) {
        self.call(Call::Net(NetCall::Ignored), "pairing:cancel", Value::Null);
    }

    /// Saves `networkAccessEnabled`; vornd rebinds on the change.
    fn net_share(&mut self, on: bool) {
        self.st.network.confirming = false;
        self.st.network.recheck_at = Some(Instant::now() + REBIND_SETTLE);
        let mut config = self.store.config.clone();
        if !config.get("defaults").is_some_and(Value::is_object) {
            config["defaults"] = json!({});
        }
        config["defaults"]["networkAccessEnabled"] = json!(on);
        self.save_config(config, Call::Net(NetCall::Ignored));
    }
}

/// Opens the token form empty with its field focused (`autoFocus`).
fn net_name_form(ui: &mut Ui) {
    ui.set_edit_text(DEVICE_NAME, "");
    ui.set_focus(Some(DEVICE_NAME));
}
