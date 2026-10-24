//! What the Remote Access page's controls do, and the answers to its calls:
//! `tailscale:status`, `server:reachableUrls`, `token:*` and `pairing:*`.

use web_time::{Duration, Instant};

use serde_json::{json, Value};

use super::{App, Call};
use crate::client::rpc::decode;
use crate::client::RpcError;
use crate::network::{
    qr_svg, AddressKind, Code, DeviceToken, Minted, NetAct, Network, PairingRequest, Reachable,
    Tailscale, DEVICE_NAME, REBIND_SETTLE, TAILSCALE_DOWNLOAD,
};
use crate::settings::Category;
use crate::state::Dialog;
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
            NetAct::Route(AddressKind::Tailnet)
                if !n.status.as_ref().is_some_and(|s| s.installed) =>
            {
                super::tasks::open_url(TAILSCALE_DOWNLOAD);
            }
            NetAct::Route(kind) => n.route = Some(kind),
            NetAct::ToggleAdvanced => n.advanced = !n.advanced,
            NetAct::Copy(text) => {
                ui.clipboard().set(&text);
                n.copied = Some((text, Instant::now()));
            }
            NetAct::AddDevice => {
                n.adding = Some(None);
                self.st.dialog = Some(Dialog::AddDevice);
                if self.st.network.can_scan() {
                    self.net_start_scan();
                }
            }
            NetAct::ShowAnother => {
                n.adding = Some(None);
                self.net_start_scan();
            }
            NetAct::CloseAdding => {
                if self.st.dialog == Some(Dialog::AddDevice) {
                    self.st.dialog = None;
                }
                self.st.network.close_adding();
                // Retired, not merely hidden: a photographed code stops working.
                self.net_cancel_pairing();
            }
            NetAct::NameToken => {
                n.naming = true;
                ui.set_edit_text(DEVICE_NAME, "");
                ui.set_focus(Some(DEVICE_NAME));
            }
            NetAct::CancelToken => {
                n.naming = false;
                ui.set_edit_text(DEVICE_NAME, "");
            }
            NetAct::CreateToken => {
                n.error = None;
                let typed = ui.edit_text(DEVICE_NAME);
                let name = match typed.trim() {
                    "" => "Computer",
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
                Ok(code) if n.adding.is_some() => n.adding = Some(Some(code)),
                Ok(_) => self.net_cancel_pairing(),
                Err(_) => n.error = Some("Could not start pairing.".to_owned()),
            },
            NetCall::Created => match answer {
                Ok(v) => {
                    let name = v["token"]["name"].as_str().unwrap_or_default().to_owned();
                    let plaintext = v["plaintext"].as_str().unwrap_or_default().to_owned();
                    n.minted = Some(Minted { name, plaintext });
                    n.naming = false;
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
                if self.st.dialog == Some(Dialog::AddDevice) {
                    self.st.dialog = None;
                }
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
                if self.st.dialog == Some(Dialog::AddDevice) {
                    self.st.dialog = None;
                }
                self.st.network.closed();
                // Closing the page puts any code away: it is only safe while watched.
                self.net_cancel_pairing();
                return;
            }
            // Just opened: load it.
            (true, false) => self.run_net(ui, NetAct::Refresh),
            (true, true) => {}
        }
        // The sheet closed some other way (Escape, the backdrop): retire its code.
        if self.st.network.adding.is_some() && self.st.dialog != Some(Dialog::AddDevice) {
            self.run_net(ui, NetAct::CloseAdding);
        }
        let link = self.st.network.code_link(now_ms());
        let n = &mut self.st.network;
        match link {
            Some(link) if n.qr.as_ref().is_none_or(|(l, _)| *l != link) => {
                n.qr = qr_svg(&link).map(|svg| (link, svg));
            }
            Some(_) => {}
            None => n.qr = None,
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
        if let Some(Some(code)) = &n.adding {
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
