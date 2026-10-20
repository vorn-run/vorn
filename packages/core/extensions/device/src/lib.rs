//! Device: an iOS simulator beside a session's terminal, chosen from its
//! card or claimed by its agent, which launches, installs and opens its
//! apps and looks at its screen.
//!
//! Vorn ships it as a Wasm component (`vorn-extension.toml` beside this
//! crate). It owns which session holds which simulator and each pane's
//! picker, zoom and notice, in its storage, and draws the pane as a tree
//! around a `video` part. The simulator itself is the client's: `video.call`
//! asks it to list and drive one, and it draws the screen's frames itself,
//! so no frame ever passes through here. A claim outlives vornd, so a
//! session that ends lets its simulator go, and vornd's start ([`START`])
//! drops the claims of sessions that ended while it was down.
//!
//! Nothing here waits on `simctl` for long. The component answers one call
//! at a time, so a simulator starting (tens of seconds) is never awaited
//! here: a pick is started by the client, which reports `booted`, and an
//! agent's claim by vornd, which calls [`BOOTED`] once it is up. The
//! picker's list and a saved screenshot come back as page events too.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use vorn_extension_sdk::tree::{self, EventKind, Surface, Tool};
use vorn_extension_sdk::{
    arg, json, serde_json, Ctx, Error, Event, Extension, Host, Init, Op, Value,
};

pub mod devices;

use devices::Device;

/// The pane beside a session's terminal, drawn for the session as its scope.
pub const PANE: &str = "device";
/// One row per session whose pane is open, its id the session's; `card`
/// when it shows on the card's button instead (its picker, or the
/// simulator picked there while it starts).
pub const OPEN: &str = "open";
/// The action vornd calls once a simulator an agent claimed has started,
/// or could not: `{session, udid, method, bootedHere}` or `{..., error}`.
/// It answers what the agent's `device:{method}` answers.
pub const BOOTED: &str = "booted";
/// What a call that needs a simulator says without one.
pub const NO_CLAIM: &str =
    "No device is claimed for this session. Call device_list to see what is available, then device_claim.";
/// What a call that needs more than `simctl` says.
pub const NO_COMPANION: &str = "This needs the simulator companion, which the native app does not run yet. Use device_screenshot to see the screen.";
/// What vornd calls as it starts it: claims kept from before are checked.
pub const START: &str = "start";
/// What vornd calls as it is switched off: every claim goes, and the
/// simulators Vorn started shut down.
pub const STOP: &str = "stop";
/// The event that a session ended, which lets its simulator go.
pub const SESSION_EXITED: &str = "session.exited";
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

#[allow(clippy::trivially_copy_pass_by_ref)] // serde's skip_serializing_if takes a reference
fn is_false(b: &bool) -> bool {
    !*b
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
    /// Still starting. The claim it replaces comes back if it fails.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<Pending>,
}

/// What a claim still starting replaced, and who starts it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pending {
    #[serde(default)]
    pub previous: Option<Box<Claim>>,
    /// An agent's claim, which vornd starts and reports with [`BOOTED`];
    /// otherwise it was picked in the pane, and the client starts it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub agent: bool,
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
    /// Shown on the card's button, not as a pane: the picker opened from
    /// the card, then the simulator picked there while it starts. The pane
    /// opens once it is up; if it fails, nothing opens.
    #[serde(default, skip_serializing_if = "is_false")]
    pub card: bool,
    /// A screenshot is being saved; the client says where it went.
    #[serde(default, skip_serializing_if = "is_false")]
    pub saving: bool,
}

/// The picker's simulators as the client listed them, or why there are
/// none; `loading` until it has.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Listing {
    #[serde(default)]
    pub devices: Vec<Device>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub loading: bool,
}

impl Listing {
    fn loading() -> Listing {
        Listing {
            loading: true,
            ..Listing::default()
        }
    }

    /// What the client's `listed` event says.
    fn from_event(said: &Value) -> Listing {
        if let Some(e) = said["error"].as_str() {
            return Listing {
                error: Some(e.to_owned()),
                ..Listing::default()
            };
        }
        match serde_json::from_value(said["devices"].clone()) {
            Ok(devices) => Listing {
                devices,
                ..Listing::default()
            },
            Err(e) => Listing {
                error: Some(format!("The simulators could not be read: {e}")),
                ..Listing::default()
            },
        }
    }
}

/// How an agent's claim went: the answer for `device:claim`, and for
/// `device:openPane` an error.
enum Claimed {
    Held(Claim),
    /// Down, so held while vornd starts it.
    Booting(Claim),
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

    /// Which session holds each simulator, counting one a claim still
    /// starting replaces: it is let go only once the new one is up.
    fn holders(host: &mut dyn Host) -> Result<HashMap<String, String>, Error> {
        let mut out = HashMap::new();
        for (s, c) in DeviceExt::claims(host)? {
            if let Some(old) = c.pending.and_then(|p| p.previous) {
                out.insert(old.udid, s.clone());
            }
            out.insert(c.udid, s);
        }
        Ok(out)
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
            vec![Op::Upsert(
                json!({ "id": session, "at": now_ms(), "card": p.card }),
            )],
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

    /// Every simulator, as the client lists them.
    fn listed(host: &mut dyn Host, session: &str) -> Result<Vec<Device>, Error> {
        let listed = DeviceExt::video(host, session, "list", json!({}))?;
        serde_json::from_value(listed).map_err(failed)
    }

    /// Every simulator, each with the session holding it.
    fn list(host: &mut dyn Host, session: &str) -> Result<Vec<Device>, Error> {
        let all = DeviceExt::listed(host, session)?;
        Ok(devices::held(all, &DeviceExt::holders(host)?))
    }

    /// Shuts down a simulator Vorn started; one the person started stays up.
    fn let_go(host: &mut dyn Host, session: &str, claim: &Claim) {
        if claim.booted_here {
            // It may be down already, which is what was wanted.
            let _ = DeviceExt::video(host, session, "shutdown", json!({ "udid": claim.udid }));
        }
    }

    /// Lets the session's simulator go, and one a claim still starting
    /// replaced. Whether there was one.
    fn release(host: &mut dyn Host, session: &str) -> Result<bool, Error> {
        let Some(mut old) = DeviceExt::claim_of(host, session)? else {
            return Ok(false);
        };
        DeviceExt::put_claim(host, session, None)?;
        if let Some(previous) = old.pending.take().and_then(|p| p.previous) {
            DeviceExt::let_go(host, session, &previous);
        }
        DeviceExt::let_go(host, session, &old);
        Ok(true)
    }

    /// Takes `udid` for `session`: now if it is running, else as a claim
    /// still starting, which vornd starts and reports with [`BOOTED`]. A
    /// simulator it held before goes once the new one is held.
    fn claim(host: &mut dyn Host, session: &str, udid: &str) -> Result<Claimed, Error> {
        let holders = DeviceExt::holders(host)?;
        if let Some(holder) = holders.get(udid).filter(|h| *h != session) {
            let message = format!("{udid} is in use by session {holder}.");
            return Ok(Claimed::Refused(json!({
                "ok": false, "reason": "held-by-session", "message": message, "holder": holder,
            })));
        }
        let old = DeviceExt::claim_of(host, session)?;
        if let Some(c) = old.as_ref().filter(|c| c.pending.is_some()) {
            return Ok(Claimed::Refused(json!({
                "ok": false, "reason": "booting", "message": still_starting(&c.name),
            })));
        }
        let device = match DeviceExt::listed(host, session) {
            Ok(all) => all.into_iter().find(|d| d.udid == udid),
            Err(e) => return Ok(Claimed::Refused(claim_failure(&e.message))),
        };
        let Some(device) = device else {
            let message = format!(
                "No simulator with udid {udid}. Call device_list to see what is available."
            );
            return Ok(Claimed::Refused(claim_failure(&message)));
        };
        if !device.booted {
            let claim = Claim {
                udid: udid.to_owned(),
                name: device.name,
                booted_here: false,
                pending: Some(Pending {
                    previous: old.map(Box::new),
                    agent: true,
                }),
            };
            DeviceExt::put_claim(host, session, Some(claim.clone()))?;
            return Ok(Claimed::Booting(claim));
        }
        let claim = Claim {
            udid: udid.to_owned(),
            name: device.name,
            // Claimed again, it is still Vorn's to shut down if Vorn started it.
            booted_here: old
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

    /// vornd says how starting an agent's claim went: it is held, or the
    /// claim it replaced comes back. What the agent's `method` answers.
    fn agent_booted(host: &mut dyn Host, args: &Value) -> Result<Value, Error> {
        let session = arg(args, "session", BOOTED)?;
        let udid = arg(args, "udid", BOOTED)?;
        let method = args["method"].as_str().unwrap_or("claim");
        let error = args["error"].as_str();
        let here = args["bootedHere"].as_bool().unwrap_or(false);
        let waiting = DeviceExt::claim_of(host, session)?
            .filter(|c| c.udid == udid && c.pending.as_ref().is_some_and(|p| p.agent));
        let Some(mut claim) = waiting else {
            // Let go or replaced while it started: what Vorn started goes down again.
            if error.is_none() && here {
                let _ = DeviceExt::video(host, session, "shutdown", json!({ "udid": udid }));
            }
            let message = format!("{udid} was let go while it started.");
            return answer_failure(
                method,
                json!({ "ok": false, "reason": "boot-failed", "message": message }),
            );
        };
        let previous = claim.pending.take().and_then(|p| p.previous).map(|b| *b);
        if let Some(message) = error {
            let pane = DeviceExt::pane(host, session)?;
            match (&previous, pane) {
                // A pane opened for it has nothing to show.
                (None, Some(_)) => DeviceExt::close(host, session)?,
                (Some(_), Some(mut p)) => {
                    p.notice = Some(message.to_owned());
                    DeviceExt::save(host, session, &p)?;
                }
                (_, None) => {}
            }
            DeviceExt::put_claim(host, session, previous)?;
            return answer_failure(method, claim_failure(message));
        }
        claim.booted_here = here;
        DeviceExt::put_claim(host, session, Some(claim.clone()))?;
        if let Some(old) = previous.filter(|o| o.udid != udid) {
            DeviceExt::let_go(host, session, &old);
        }
        Ok(match method {
            "openPane" => json!({ "udid": claim.udid }),
            _ => held(&claim),
        })
    }

    /// The card's button: hides an open pane, else opens the picker over
    /// the card, listing afresh if it is open already. While what was
    /// picked there starts, the button waits.
    fn button(host: &mut dyn Host, session: &str) -> Result<(), Error> {
        match DeviceExt::pane(host, session)? {
            Some(p) if !p.card => DeviceExt::close(host, session),
            Some(p) if p.picker.is_none() => Ok(()),
            _ => DeviceExt::save(
                host,
                session,
                &Pane {
                    picker: Some(Listing::loading()),
                    card: true,
                    ..Pane::default()
                },
            ),
        }
    }

    /// The picker shuts; with no simulator to show, so does the pane, and
    /// one opened from the card was never more than its picker.
    fn close_picker(host: &mut dyn Host, session: &str, mut p: Pane) -> Result<(), Error> {
        if p.card || DeviceExt::claim_of(host, session)?.is_none() {
            return DeviceExt::close(host, session);
        }
        p.picker = None;
        DeviceExt::save(host, session, &p)
    }

    /// A simulator chosen in the picker: the pane shows it as the client
    /// starts it, and says so if it does not. Chosen over the card, the
    /// card waits for it, and a refusal is a toast with nothing opened.
    fn pick(host: &mut dyn Host, session: &str, mut p: Pane, udid: &str) -> Result<Value, Error> {
        let old = DeviceExt::claim_of(host, session)?;
        let listing = p.picker.take();
        let refuse = |host: &mut dyn Host, mut p: Pane, message: String| -> Result<Value, Error> {
            if p.card {
                DeviceExt::close(host, session)?;
                return Ok(json!({ "error": message }));
            }
            p.notice = Some(message);
            DeviceExt::save(host, session, &p).map(|()| Value::Null)
        };
        if let Some(c) = old.as_ref().filter(|c| c.pending.is_some()) {
            return refuse(host, p, still_starting(&c.name));
        }
        if old.as_ref().is_some_and(|o| o.udid == udid) {
            p.card = false;
            return DeviceExt::save(host, session, &p).map(|()| Value::Null);
        }
        if let Some(holder) = DeviceExt::holders(host)?.get(udid) {
            return refuse(host, p, format!("{udid} is in use by session {holder}."));
        }
        let name = listing
            .into_iter()
            .flat_map(|l| l.devices)
            .find(|d| d.udid == udid)
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
                    previous: old.map(Box::new),
                    agent: false,
                }),
            }),
        )?;
        Ok(Value::Null)
    }

    /// The client says how starting a picked simulator went. Picked over
    /// the card, the pane opens now, or the toast says why it did not.
    fn started(host: &mut dyn Host, session: &str, said: &Value) -> Result<Value, Error> {
        let Some(mut claim) = DeviceExt::claim_of(host, session)? else {
            return Ok(Value::Null);
        };
        let Some(pending) = claim.pending.take_if(|p| !p.agent) else {
            return Ok(Value::Null);
        };
        let previous = pending.previous.map(|b| *b);
        let pane = DeviceExt::pane(host, session)?;
        match said["type"].as_str() {
            Some("booted") => {
                claim.booted_here = said["bootedHere"].as_bool().unwrap_or(false);
                if let Some(p) = pane.filter(|p| p.card) {
                    DeviceExt::save(host, session, &Pane { card: false, ..p })?;
                }
                DeviceExt::put_claim(host, session, Some(claim))?;
                if let Some(old) = previous {
                    DeviceExt::let_go(host, session, &old);
                }
                Ok(Value::Null)
            }
            _ => {
                let message = said["message"]
                    .as_str()
                    .unwrap_or("The simulator did not start.")
                    .to_owned();
                DeviceExt::put_claim(host, session, previous)?;
                match pane {
                    Some(p) if p.card => {
                        DeviceExt::close(host, session)?;
                        Ok(json!({ "error": message }))
                    }
                    Some(mut p) => {
                        p.notice = Some(message);
                        DeviceExt::save(host, session, &p).map(|()| Value::Null)
                    }
                    None => Ok(Value::Null),
                }
            }
        }
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
                Claimed::Held(c) => held(&c),
                Claimed::Booting(c) => json!({ "booting": c.udid }),
                Claimed::Refused(answer) => answer,
            }),
            "release" => {
                let released = DeviceExt::release(host, session)?;
                DeviceExt::close(host, session)?;
                Ok(json!({ "released": released }))
            }
            "openPane" => {
                let (udid, booting) = match args["udid"].as_str() {
                    Some(udid) => match DeviceExt::claim(host, session, udid)? {
                        Claimed::Held(c) => (c.udid, false),
                        // The pane opens now, its title saying the simulator starts.
                        Claimed::Booting(c) => (c.udid, true),
                        Claimed::Refused(answer) => {
                            return Err(failed(answer["message"].as_str().unwrap_or_default()))
                        }
                    },
                    None => DeviceExt::claim_of(host, session)?
                        .map(|c| (c.udid, false))
                        .ok_or_else(|| {
                            failed("No device is claimed for this session. Call device_claim first, or pass a udid.")
                        })?,
                };
                let p = DeviceExt::pane(host, session)?.unwrap_or_default();
                let p = Pane {
                    picker: None,
                    card: false,
                    ..p
                };
                DeviceExt::save(host, session, &p)?;
                Ok(if booting {
                    json!({ "booting": udid })
                } else {
                    json!({ "udid": udid })
                })
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

    /// The session's running simulator, or the agent told to claim one or
    /// to wait for it.
    fn claimed(host: &mut dyn Host, session: &str) -> Result<String, Error> {
        match DeviceExt::claim_of(host, session)? {
            Some(c) if c.pending.is_some() => Err(failed(still_starting(&c.name))),
            Some(c) => Ok(c.udid),
            None => Err(failed(NO_CLAIM)),
        }
    }

    /// The pane's tree: the simulator's screen, its header and its controls.
    fn render_pane(host: &mut dyn Host, session: &str) -> Result<tree::Node, Error> {
        let Some(p) = DeviceExt::pane(host, session)? else {
            return Ok(tree::sdk::col([]));
        };
        let claim = DeviceExt::claim_of(host, session)?;
        let pending = claim.as_ref().and_then(|c| c.pending.as_ref());
        let holders = if p.picker.is_some() {
            DeviceExt::holders(host)?
        } else {
            HashMap::new()
        };
        let picker = p.picker.map(|l| picker(l, session, &holders));
        let saving = p.saving && claim.as_ref().is_some_and(|c| c.pending.is_none());
        let video = tree::Video {
            view: session.to_owned(),
            device: claim.as_ref().map(|c| c.udid.clone()),
            title: claim
                .as_ref()
                .map_or_else(|| "Simulator".to_owned(), |c| c.name.clone()),
            busy: pending.is_some(),
            // An agent's claim is vornd's to start.
            boot: pending.is_some_and(|p| !p.agent),
            save: saving,
            zoom: p.zoom,
            notice: p.notice,
            // The words and dividers are 0.8's pane's, kept as they were.
            tools: vec![
                Tool {
                    tip: Some(
                        "Point at an element to describe it to this session's agent".to_owned(),
                    ),
                    ..tool(
                        "pick",
                        "Point at an element for the agent",
                        "mouse-pointer-click",
                    )
                },
                Tool {
                    tip: Some(
                        "Draw over the screen, then click again to send it to this session's agent"
                            .to_owned(),
                    ),
                    ..tool("draw", "Draw on the screen for the agent", "pencil")
                },
            ],
            controls: vec![
                Tool {
                    tip: Some("Home".to_owned()),
                    ..tool("home", "Press Home", "house")
                },
                Tool {
                    tip: Some("Lock".to_owned()),
                    ..tool("lock", "Press Lock", "lock")
                },
                Tool {
                    busy: saving,
                    divider: true,
                    ..tool("save", "Save a screenshot", "image-down")
                },
                tool("keyboard", "Type on the device", "keyboard"),
                Tool {
                    tip: Some("Rotate".to_owned()),
                    ..tool("rotate", "Rotate the device", "rotate-cw")
                },
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
            EventKind::Select => return DeviceExt::pick(host, session, p, &e.value),
            EventKind::Click => match e.value.as_str() {
                "save" if p.saving => return Ok(json!({ "same": true })),
                "save" => {
                    match DeviceExt::claimed(host, session) {
                        Ok(_) => p.saving = true,
                        Err(e) => p.notice = Some(e.message),
                    }
                    DeviceExt::save(host, session, &p)?;
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
                        p.picker = Some(Listing::loading());
                        DeviceExt::save(host, session, &p)?;
                    }
                    "listed" => {
                        let Some(l) = p.picker.as_mut().filter(|l| l.loading) else {
                            return Ok(json!({ "same": true }));
                        };
                        *l = Listing::from_event(&said);
                        DeviceExt::save(host, session, &p)?;
                    }
                    "saved" => {
                        p.saving = false;
                        let answer = match said["path"].as_str() {
                            Some(_) => json!({ "toast": "Screenshot saved" }),
                            None => {
                                let e = said["error"].as_str().unwrap_or("Saving failed.");
                                p.notice = Some(e.to_owned());
                                Value::Null
                            }
                        };
                        DeviceExt::save(host, session, &p)?;
                        return Ok(answer);
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
                    "booted" | "failed" => return DeviceExt::started(host, session, &said),
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
            BOOTED => DeviceExt::agent_booted(host, args),
            // The pane's own ✕: it goes, and the simulator stays held.
            "close" => {
                DeviceExt::close(host, arg(args, "session", action)?)?;
                Ok(json!({}))
            }
            // Escape: the picker shuts, and over the card nothing is left.
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
                let gone: Vec<String> = host
                    .rows(OPEN)?
                    .iter()
                    .filter_map(|r| r.get("id").and_then(Value::as_str).map(str::to_owned))
                    .chain(DeviceExt::claims(host)?.into_keys())
                    .filter(|s| !live.contains(&s.as_str()))
                    .collect();
                DeviceExt::forget(host, gone)?;
                Ok(Value::Null)
            }
            START => DeviceExt::start(host).map(|()| Value::Null),
            STOP => {
                let held: Vec<String> = DeviceExt::claims(host)?.into_keys().collect();
                DeviceExt::forget(host, held)?;
                Ok(Value::Null)
            }
            other => Err(Error::unknown(other)),
        }
    }

    /// Lets go of what each of `sessions` holds and closes its pane.
    fn forget(host: &mut dyn Host, sessions: Vec<String>) -> Result<(), Error> {
        let mut sessions = sessions;
        sessions.sort();
        sessions.dedup();
        for session in &sessions {
            // The pane first, so letting go does not draw it again.
            DeviceExt::close(host, session)?;
            DeviceExt::release(host, session)?;
        }
        Ok(())
    }

    /// Claims kept from before vornd started: those of sessions that ended
    /// meanwhile go, and one still starting is held if its simulator came
    /// up, else the claim it replaced comes back. Nothing reports on a
    /// start vornd stopped waiting for.
    fn start(host: &mut dyn Host) -> Result<(), Error> {
        let claims = DeviceExt::claims(host)?;
        if claims.is_empty() {
            return Ok(());
        }
        // Without the sessions, no claim is taken for gone.
        let live: Option<Vec<String>> = host.call("sessions.list", json!({})).ok().and_then(|v| {
            v.as_array().map(|all| {
                all.iter()
                    .filter_map(|s| s["id"].as_str().map(str::to_owned))
                    .collect()
            })
        });
        let (gone, kept): (Vec<_>, Vec<_>) = claims
            .into_iter()
            .partition(|(s, _)| live.as_ref().is_some_and(|l| !l.contains(s)));
        DeviceExt::forget(host, gone.into_iter().map(|(s, _)| s).collect())?;
        let mut running: Option<Vec<Device>> = None;
        for (session, mut claim) in kept {
            let Some(pending) = claim.pending.take() else {
                continue;
            };
            if running.is_none() {
                running = Some(DeviceExt::listed(host, &session).unwrap_or_default());
            }
            let up = running
                .iter()
                .flatten()
                .any(|d| d.udid == claim.udid && d.booted);
            let previous = pending.previous.map(|b| *b);
            let pane = DeviceExt::pane(host, &session)?;
            if up {
                // It was down when claimed, so Vorn started it.
                claim.booted_here = true;
                if let Some(p) = pane.filter(|p| p.card) {
                    DeviceExt::save(host, &session, &Pane { card: false, ..p })?;
                }
                let udid = claim.udid.clone();
                DeviceExt::put_claim(host, &session, Some(claim))?;
                if let Some(old) = previous.filter(|o| o.udid != udid) {
                    DeviceExt::let_go(host, &session, &old);
                }
                continue;
            }
            match pane {
                Some(p) if p.card || previous.is_none() => DeviceExt::close(host, &session)?,
                Some(mut p) => {
                    p.notice = Some(format!("{} did not start.", claim.name));
                    DeviceExt::save(host, &session, &p)?;
                }
                None => {}
            }
            DeviceExt::put_claim(host, &session, previous)?;
        }
        Ok(())
    }

    /// The sessions in `batch` that ended.
    fn ended(batch: &[Event]) -> Vec<String> {
        batch
            .iter()
            .filter(|e| e.name == SESSION_EXITED)
            .filter_map(|e| e.data["sessionId"].as_str().map(str::to_owned))
            .collect()
    }
}

/// `device:claim`'s answer for a simulator held.
fn held(c: &Claim) -> Value {
    json!({ "ok": true, "udid": c.udid, "name": c.name, "booted": true })
}

/// What a call on a simulator still starting is told.
fn still_starting(name: &str) -> String {
    format!("{name} is still starting; it is this session's once it is up.")
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

/// A failed claim as `method` answers it: `device:claim` with the failure,
/// `device:openPane` with an error.
fn answer_failure(method: &str, failure: Value) -> Result<Value, Error> {
    match method {
        "openPane" => Err(failed(failure["message"].as_str().unwrap_or_default())),
        _ => Ok(failure),
    }
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
        ..Tool::default()
    }
}

/// The simulators under the title: this session's to take first, a dot
/// on the running ones, and the ones another session holds shown but shut.
fn picker(listing: Listing, session: &str, holders: &HashMap<String, String>) -> tree::Picker {
    let all = devices::held(listing.devices, holders);
    let choices = devices::order_devices(&all, session)
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
        loading: listing.loading,
        error: listing.error,
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

    /// A session that ended lets its simulator go, as its agent's `device_release` would.
    fn on_events(&mut self, host: &mut dyn Host, batch: Vec<Event>) {
        if let Err(e) = DeviceExt::forget(host, DeviceExt::ended(&batch)) {
            host.log(
                vorn_extension_sdk::Level::Warn,
                &format!("an ended session's simulator was not let go: {}", e.message),
            );
        }
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
