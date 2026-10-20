//! Device: an iOS simulator beside a session's terminal, chosen from its
//! card or claimed by its agent, which launches, installs and opens its
//! apps and looks at its screen.
//!
//! Vorn ships it as a Wasm component (`vorn-extension.toml` beside this
//! crate). It owns which session holds which simulator and each pane's
//! picker, zoom and notice, in its storage, and draws the pane as a tree
//! around a `video` part. The simulator itself is the client's: `video.call`
//! asks it to list, boot and drive one, and it draws the screen's frames
//! itself, so no frame ever passes through here.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use vorn_extension_sdk::tree::{self, EventKind, Surface, Tool};
use vorn_extension_sdk::{arg, json, serde_json, Ctx, Error, Extension, Host, Init, Op, Value};

pub mod devices;

use devices::Device;

/// The pane beside a session's terminal, drawn for the session as its scope.
pub const PANE: &str = "device";
/// One row per session whose pane is open, its id the session's.
pub const OPEN: &str = "open";
/// What a call that needs a simulator says without one.
pub const NO_CLAIM: &str =
    "No device is claimed for this session. Call device_list to see what is available, then device_claim.";
/// What a call that needs more than `simctl` says.
pub const NO_COMPANION: &str = "This needs the simulator companion, which the native app does not run yet. Use device_screenshot to see the screen.";
/// The screen's node.
const SCREEN: &str = "screen";
/// Every session's simulator, by session.
const CLAIMS: &str = "claims";

fn pane_key(session: &str) -> String {
    format!("pane/{session}")
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn failed(e: impl std::fmt::Display) -> Error {
    Error::new("failed", e.to_string())
}

/// A session's simulator.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Claim {
    pub udid: String,
    pub name: String,
    /// Vorn started it, so letting it go shuts it down.
    #[serde(default)]
    pub booted_here: bool,
    /// Picked in the pane and still starting: the client boots it and says
    /// how that went. The claim it replaces comes back if it fails.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<Pending>,
}

/// What a claim still starting replaced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pending {
    #[serde(default)]
    pub previous: Option<Box<Claim>>,
}

/// An open pane: what it shows besides the simulator.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Pane {
    /// The simulators listed under the title, while it is open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picker: Option<Listing>,
    /// Screen points per pixel; none fits the screen to the pane.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zoom: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notice: Option<String>,
}

/// The picker's simulators as listed when it opened, or why there are none.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Listing {
    #[serde(default)]
    pub devices: Vec<Device>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// How an agent's claim went: the answer for `device:claim`, and for
/// `device:openPane` an error.
enum Claimed {
    Held(Claim),
    Refused(Value),
}

/// The extension. It holds nothing between calls: claims and panes are in storage.
#[derive(Debug, Default)]
pub struct DeviceExt;

impl DeviceExt {
    fn claims(host: &mut dyn Host) -> Result<HashMap<String, Claim>, Error> {
        Ok(host
            .get(CLAIMS)?
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default())
    }

    fn set_claims(host: &mut dyn Host, claims: &HashMap<String, Claim>) -> Result<(), Error> {
        host.set(CLAIMS, &serde_json::to_value(claims).map_err(failed)?)
    }

    fn claim_of(host: &mut dyn Host, session: &str) -> Result<Option<Claim>, Error> {
        Ok(DeviceExt::claims(host)?.remove(session))
    }

    /// Sets or drops the session's claim, and draws its pane again.
    fn put_claim(host: &mut dyn Host, session: &str, claim: Option<Claim>) -> Result<(), Error> {
        let mut claims = DeviceExt::claims(host)?;
        match claim {
            Some(c) => claims.insert(session.to_owned(), c),
            None => claims.remove(session),
        };
        DeviceExt::set_claims(host, &claims)?;
        if let Some(p) = DeviceExt::pane(host, session)? {
            DeviceExt::save(host, session, &p)?;
        }
        Ok(())
    }

    /// Which session holds each simulator.
    fn holders(host: &mut dyn Host) -> Result<HashMap<String, String>, Error> {
        Ok(DeviceExt::claims(host)?
            .into_iter()
            .map(|(s, c)| (c.udid, s))
            .collect())
    }

    fn pane(host: &mut dyn Host, session: &str) -> Result<Option<Pane>, Error> {
        Ok(host
            .get(&pane_key(session))?
            .and_then(|v| serde_json::from_value(v).ok()))
    }

    fn save(host: &mut dyn Host, session: &str, p: &Pane) -> Result<(), Error> {
        host.set(
            &pane_key(session),
            &serde_json::to_value(p).map_err(failed)?,
        )?;
        // Every change announces the session's row again, so clients draw that pane again.
        host.publish(
            OPEN,
            vec![Op::Upsert(json!({ "id": session, "at": now_ms() }))],
        );
        Ok(())
    }

    /// The pane goes; the claim stays.
    fn close(host: &mut dyn Host, session: &str) -> Result<(), Error> {
        host.set(&pane_key(session), &Value::Null)?;
        host.publish(OPEN, vec![Op::Delete(session.to_owned())]);
        Ok(())
    }

    /// Asks the client for the simulators `op`.
    fn video(host: &mut dyn Host, session: &str, op: &str, params: Value) -> Result<Value, Error> {
        host.call(
            "video.call",
            json!({ "view": session, "op": op, "params": params }),
        )
    }

    /// Every simulator, each with the session holding it.
    fn list(host: &mut dyn Host, session: &str) -> Result<Vec<Device>, Error> {
        let listed = DeviceExt::video(host, session, "list", json!({}))?;
        let all: Vec<Device> = serde_json::from_value(listed).map_err(failed)?;
        Ok(devices::held(all, &DeviceExt::holders(host)?))
    }

    /// Shuts down a simulator Vorn started; one the person started stays up.
    fn let_go(host: &mut dyn Host, session: &str, claim: &Claim) {
        if claim.booted_here {
            // It may be down already, which is what was wanted.
            let _ = DeviceExt::video(host, session, "shutdown", json!({ "udid": claim.udid }));
        }
    }

    /// Lets the session's simulator go. Whether there was one.
    fn release(host: &mut dyn Host, session: &str) -> Result<bool, Error> {
        let Some(old) = DeviceExt::claim_of(host, session)? else {
            return Ok(false);
        };
        DeviceExt::put_claim(host, session, None)?;
        DeviceExt::let_go(host, session, &old);
        Ok(true)
    }

    /// Takes `udid` for `session` now, booting it if it is down; a
    /// simulator it held before goes.
    fn claim(host: &mut dyn Host, session: &str, udid: &str) -> Result<Claimed, Error> {
        let holders = DeviceExt::holders(host)?;
        if let Some(holder) = holders.get(udid).filter(|h| *h != session) {
            let message = format!("{udid} is in use by session {holder}.");
            return Ok(Claimed::Refused(json!({
                "ok": false, "reason": "held-by-session", "message": message, "holder": holder,
            })));
        }
        let booted = match DeviceExt::video(host, session, "boot", json!({ "udid": udid })) {
            Ok(b) => b,
            Err(e) => return Ok(Claimed::Refused(claim_failure(&e.message))),
        };
        let old = DeviceExt::claim_of(host, session)?;
        let claim = Claim {
            udid: udid.to_owned(),
            name: booted["name"].as_str().unwrap_or(udid).to_owned(),
            // Claimed again, it is still Vorn's to shut down if Vorn started it.
            booted_here: booted["bootedHere"].as_bool().unwrap_or(false)
                || old
                    .as_ref()
                    .is_some_and(|o| o.udid == udid && o.booted_here),
            pending: None,
        };
        DeviceExt::put_claim(host, session, Some(claim.clone()))?;
        if let Some(old) = old.filter(|o| o.udid != udid) {
            DeviceExt::let_go(host, session, &old);
        }
        Ok(Claimed::Held(claim))
    }

    /// Opens or closes the pane from the card. Open, it lists the
    /// simulators to choose from.
    fn button(host: &mut dyn Host, session: &str) -> Result<(), Error> {
        if DeviceExt::pane(host, session)?.is_some() {
            return DeviceExt::close(host, session);
        }
        let picker = DeviceExt::listing(host, session);
        DeviceExt::save(
            host,
            session,
            &Pane {
                picker: Some(picker),
                ..Pane::default()
            },
        )
    }

    fn listing(host: &mut dyn Host, session: &str) -> Listing {
        match DeviceExt::list(host, session) {
            Ok(devices) => Listing {
                devices,
                error: None,
            },
            Err(e) => Listing {
                devices: Vec::new(),
                error: Some(e.message),
            },
        }
    }

    /// The picker shuts; with no simulator to show, so does the pane.
    fn close_picker(host: &mut dyn Host, session: &str, mut p: Pane) -> Result<(), Error> {
        if DeviceExt::claim_of(host, session)?.is_none() {
            return DeviceExt::close(host, session);
        }
        p.picker = None;
        DeviceExt::save(host, session, &p)
    }

    /// A simulator chosen in the picker: the pane shows it as the client
    /// starts it, and says so if it does not.
    fn pick(host: &mut dyn Host, session: &str, mut p: Pane, udid: &str) -> Result<(), Error> {
        let old = DeviceExt::claim_of(host, session)?;
        p.picker = None;
        if old.as_ref().is_some_and(|o| o.udid == udid) {
            return DeviceExt::save(host, session, &p);
        }
        if let Some(holder) = DeviceExt::holders(host)?.get(udid) {
            p.notice = Some(format!("{udid} is in use by session {holder}."));
            return DeviceExt::save(host, session, &p);
        }
        let name = DeviceExt::list(host, session)
            .ok()
            .and_then(|all| all.into_iter().find(|d| d.udid == udid))
            .map_or_else(|| udid.to_owned(), |d| d.name);
        p.notice = None;
        DeviceExt::save(host, session, &p)?;
        DeviceExt::put_claim(
            host,
            session,
            Some(Claim {
                udid: udid.to_owned(),
                name,
                booted_here: false,
                // A claim still starting stands in for what it replaced.
                pending: Some(Pending {
                    previous: old.map(|o| Box::new(Claim { pending: None, ..o })),
                }),
            }),
        )
    }

    /// The client says how starting a picked simulator went.
    fn started(host: &mut dyn Host, session: &str, said: &Value) -> Result<(), Error> {
        let Some(mut claim) = DeviceExt::claim_of(host, session)? else {
            return Ok(());
        };
        let Some(pending) = claim.pending.take() else {
            return Ok(());
        };
        let previous = pending.previous.map(|b| *b);
        match said["type"].as_str() {
            Some("booted") => {
                claim.booted_here = said["bootedHere"].as_bool().unwrap_or(false);
                DeviceExt::put_claim(host, session, Some(claim))?;
                if let Some(old) = previous {
                    DeviceExt::let_go(host, session, &old);
                }
            }
            _ => {
                DeviceExt::put_claim(host, session, previous)?;
                if let Some(mut p) = DeviceExt::pane(host, session)? {
                    p.notice = said["message"].as_str().map(str::to_owned);
                    DeviceExt::save(host, session, &p)?;
                }
            }
        }
        Ok(())
    }

    /// An agent's `device:*` call, `method` without its group.
    fn agent(host: &mut dyn Host, method: &str, args: &Value) -> Result<Value, Error> {
        if method == "chrome" {
            return Ok(Value::Null);
        }
        let session = arg(args, "sessionId", method)?.to_owned();
        let session = session.as_str();
        // Each call names a session of this Vorn; its simulator answers for it only.
        host.call("sessions.get", json!({ "sessionId": session }))
            .ok()
            .filter(Value::is_object)
            .ok_or_else(|| failed(format!("No session {session} in this app.")))?;
        let text = |k: &str| args[k].as_str().unwrap_or_default().to_owned();
        match method {
            "list" => Ok(json!(DeviceExt::list(host, session)?)),
            "claim" => Ok(match DeviceExt::claim(host, session, &text("udid"))? {
                Claimed::Held(c) => {
                    json!({ "ok": true, "udid": c.udid, "name": c.name, "booted": true })
                }
                Claimed::Refused(answer) => answer,
            }),
            "release" => {
                let released = DeviceExt::release(host, session)?;
                DeviceExt::close(host, session)?;
                Ok(json!({ "released": released }))
            }
            "openPane" => {
                let udid = match args["udid"].as_str() {
                    Some(udid) => match DeviceExt::claim(host, session, udid)? {
                        Claimed::Held(c) => c.udid,
                        Claimed::Refused(answer) => {
                            return Err(failed(answer["message"].as_str().unwrap_or_default()))
                        }
                    },
                    None => DeviceExt::claim_of(host, session)?
                        .map(|c| c.udid)
                        .ok_or_else(|| {
                            failed("No device is claimed for this session. Call device_claim first, or pass a udid.")
                        })?,
                };
                let p = DeviceExt::pane(host, session)?.unwrap_or_default();
                DeviceExt::save(host, session, &Pane { picker: None, ..p })?;
                Ok(json!({ "udid": udid }))
            }
            "launch" | "terminate" | "openUrl" | "install" => {
                let udid = DeviceExt::claimed(host, session)?;
                let mut params = args.clone();
                params["udid"] = json!(udid);
                DeviceExt::video(host, session, method, params).map_err(|e| {
                    let bundle = text("bundleId");
                    match method {
                        "launch" if not_installed(&e.message) => failed(format!(
                            "{bundle} is not installed on this device. Install it first."
                        )),
                        _ => e,
                    }
                })?;
                Ok(json!({ "ok": true }))
            }
            "screenshot" => {
                let udid = DeviceExt::claimed(host, session)?;
                let shot = DeviceExt::video(host, session, "screenshot", json!({ "udid": udid }))?;
                Ok(json!({
                    "data": shot["data"],
                    "scale": 1,
                    "screen": { "width": shot["width"], "height": shot["height"] },
                    "orientation": "portrait",
                }))
            }
            "readScreen" | "find" | "interact" | "logs" => Err(failed(NO_COMPANION)),
            other => Err(Error::unknown(&format!("device:{other}"))),
        }
    }

    /// The session's simulator, or the agent told to claim one.
    fn claimed(host: &mut dyn Host, session: &str) -> Result<String, Error> {
        DeviceExt::claim_of(host, session)?
            .map(|c| c.udid)
            .ok_or_else(|| failed(NO_CLAIM))
    }

    /// The pane's tree: the simulator's screen, its header and its controls.
    fn render_pane(host: &mut dyn Host, session: &str) -> Result<tree::Node, Error> {
        let Some(p) = DeviceExt::pane(host, session)? else {
            return Ok(tree::sdk::col([]));
        };
        let claim = DeviceExt::claim_of(host, session)?;
        let pending = claim.as_ref().is_some_and(|c| c.pending.is_some());
        let picker = p.picker.map(|l| picker(&l, session));
        let video = tree::Video {
            view: session.to_owned(),
            device: claim.as_ref().map(|c| c.udid.clone()),
            title: claim
                .as_ref()
                .map_or_else(|| "Simulator".to_owned(), |c| c.name.clone()),
            busy: pending,
            boot: pending,
            zoom: p.zoom,
            notice: p.notice,
            tools: vec![
                tool("pick", "Point at an element for the agent", "mouse-pointer-click"),
                tool("draw", "Draw on the screen for the agent", "pencil"),
            ],
            controls: vec![
                tool("home", "Press Home", "house"),
                tool("lock", "Press Lock", "lock"),
                tool("save", "Save a screenshot", "image-down"),
                tool("keyboard", "Type on the device", "keyboard"),
                tool("rotate", "Rotate the device", "rotate-cw"),
            ],
            picker,
        };
        Ok(tree::sdk::video(SCREEN, video).grow())
    }

    fn pane_event(host: &mut dyn Host, session: &str, e: &tree::Event) -> Result<Value, Error> {
        let Some(mut p) = DeviceExt::pane(host, session)? else {
            return Ok(json!({ "same": true }));
        };
        if e.id != SCREEN {
            return Err(Error::new(
                "unknown-action",
                format!("{PANE} has no {}", e.id),
            ));
        }
        match &e.kind {
            EventKind::Select => DeviceExt::pick(host, session, p, &e.value)?,
            EventKind::Click => match e.value.as_str() {
                "save" => {
                    let Some(c) = DeviceExt::claim_of(host, session)? else {
                        p.notice = Some(NO_CLAIM.to_owned());
                        DeviceExt::save(host, session, &p)?;
                        return Ok(Value::Null);
                    };
                    let saved = DeviceExt::video(
                        host,
                        session,
                        "save",
                        json!({ "udid": c.udid, "name": c.name }),
                    );
                    return match saved {
                        Ok(_) => Ok(json!({ "same": true, "toast": "Screenshot saved" })),
                        Err(e) => {
                            p.notice = Some(e.message);
                            DeviceExt::save(host, session, &p).map(|()| Value::Null)
                        }
                    };
                }
                // Each of these needs the companion; the pane says so.
                _ => {
                    p.notice = Some(NO_COMPANION.to_owned());
                    DeviceExt::save(host, session, &p)?;
                }
            },
            EventKind::Page => {
                let said: Value = serde_json::from_str(&e.value)
                    .map_err(|_| Error::bad_args("a page event's value is a JSON object"))?;
                match said["type"].as_str().unwrap_or_default() {
                    "switch" if p.picker.is_some() => DeviceExt::close_picker(host, session, p)?,
                    "switch" => {
                        p.picker = Some(DeviceExt::listing(host, session));
                        DeviceExt::save(host, session, &p)?;
                    }
                    "dismiss" => {
                        p.notice = None;
                        DeviceExt::save(host, session, &p)?;
                    }
                    "zoom" => {
                        let zoom = said["zoom"].as_f64().filter(|z| z.is_finite() && *z > 0.0);
                        if zoom == p.zoom {
                            return Ok(json!({ "same": true }));
                        }
                        p.zoom = zoom;
                        DeviceExt::save(host, session, &p)?;
                    }
                    "booted" | "failed" => DeviceExt::started(host, session, &said)?,
                    other => return Err(Error::bad_args(format!("no page event {other}"))),
                }
            }
            _ => return Ok(json!({ "same": true })),
        }
        Ok(Value::Null)
    }

    fn act(host: &mut dyn Host, action: &str, args: &Value) -> Result<Value, Error> {
        if let Some(method) = action.strip_prefix("device:") {
            return DeviceExt::agent(host, method, args);
        }
        match action {
            "button" => {
                DeviceExt::button(host, arg(args, "session", action)?)?;
                Ok(json!({}))
            }
            // The pane's own ✕: it goes, and the simulator stays held.
            "close" => {
                DeviceExt::close(host, arg(args, "session", action)?)?;
                Ok(json!({}))
            }
            // Escape over the pane: the picker shuts, as a click outside it does.
            "closePicker" => {
                let session = arg(args, "session", action)?;
                if let Some(p) = DeviceExt::pane(host, session)?.filter(|p| p.picker.is_some()) {
                    DeviceExt::close_picker(host, session, p)?;
                }
                Ok(json!({}))
            }
            // What sessions are left: the panes and simulators of the rest go.
            "prune" => {
                let live: Vec<&str> = args
                    .get("live")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .collect();
                let mut gone: Vec<String> = host
                    .rows(OPEN)?
                    .iter()
                    .filter_map(|r| r.get("id").and_then(Value::as_str).map(str::to_owned))
                    .chain(DeviceExt::claims(host)?.into_keys())
                    .filter(|s| !live.contains(&s.as_str()))
                    .collect();
                gone.sort();
                gone.dedup();
                for session in &gone {
                    DeviceExt::release(host, session)?;
                    DeviceExt::close(host, session)?;
                }
                Ok(Value::Null)
            }
            other => Err(Error::unknown(other)),
        }
    }
}

/// A failed claim, by what went wrong.
fn claim_failure(message: &str) -> Value {
    let reason = if message.starts_with("No simulator") {
        "gone"
    } else {
        "boot-failed"
    };
    json!({ "ok": false, "reason": reason, "message": message })
}

/// Whether `simctl launch` failed for want of the app.
fn not_installed(message: &str) -> bool {
    let lower = message.to_lowercase();
    ["not installed", "no such", "cannot find", "not found"]
        .iter()
        .any(|s| lower.contains(s))
}

fn tool(id: &str, label: &str, icon: &str) -> Tool {
    Tool {
        id: id.to_owned(),
        label: label.to_owned(),
        icon: icon.to_owned(),
        on: false,
    }
}

/// The simulators under the title: this session's to take first, a dot
/// on the running ones, and the ones another session holds shown but shut.
fn picker(listing: &Listing, session: &str) -> tree::Picker {
    let choices = devices::order_devices(&listing.devices, session)
        .into_iter()
        .map(|l| {
            let held = !l.selectable;
            let badge = if held {
                Some(("in use", tree::Tone::Warn))
            } else if l.device.booted {
                Some(("booted", tree::Tone::Good))
            } else {
                None
            };
            tree::Choice {
                id: l.device.udid,
                title: l.device.name,
                detail: l.subtitle,
                status: l.device.booted.then_some(tree::Tone::Good),
                badge: badge.map(|(text, tone)| tree::Badge {
                    text: text.to_owned(),
                    tone,
                }),
                disabled: held,
            }
        })
        .collect();
    tree::Picker {
        heading: "Simulators".to_owned(),
        loading: false,
        error: listing.error.clone(),
        empty: Some("No simulators found".to_owned()),
        choices,
    }
}

impl Extension for DeviceExt {
    fn init(_: &Init, _: &mut dyn Host) -> Result<Self, Error> {
        Ok(DeviceExt)
    }

    fn handle(
        &mut self,
        host: &mut dyn Host,
        method: &str,
        args: Value,
        _: &Ctx,
    ) -> Result<Value, Error> {
        DeviceExt::act(host, method, &args)
    }

    fn render(
        &mut self,
        host: &mut dyn Host,
        point: &str,
        surface: Surface,
    ) -> Result<tree::Node, Error> {
        match point {
            PANE => DeviceExt::render_pane(host, &surface.scope.unwrap_or_default()),
            _ => Err(Error::new(
                "unknown-action",
                format!("nothing draws {point}"),
            )),
        }
    }

    fn event(&mut self, host: &mut dyn Host, point: &str, e: tree::Event) -> Result<Value, Error> {
        if point != PANE {
            return Err(Error::new(
                "unknown-action",
                format!("{point} has no {}", e.id),
            ));
        }
        let session = e.scope.clone().unwrap_or_default();
        // Null: the tree is drawn again; an object is the answer as it stands.
        Ok(match DeviceExt::pane_event(host, &session, &e)? {
            Value::Null => json!({}),
            other => other,
        })
    }
}

vorn_extension_sdk::export!(DeviceExt);

#[cfg(test)]
mod tests;
