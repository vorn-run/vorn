//! The screen a browser shows before vornd accepts it: a pairing code from
//! the desktop's Settings › Network (approved there), or a device token.

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::{json, Value};
use vorn_app::client::rpc::Wake;
use vorn_app::look;
use vorn_app::page;
use vorn_app::ui::{div, field, text, Action, El, Event, Id, Role, TextStyle, Ui};
use web_time::{Duration, Instant};

const FIELD: Id = Id(0x7765_625f_7061_6972);
const CONNECT: Id = Id(0x7765_625f_676f_0001);
const POLL_EVERY: Duration = Duration::from_secs(1);
const TOKEN_PREFIX: &str = "vorn_";

/// A token vornd will accept.
pub struct Paired(pub String);

/// What a request to vornd came back with.
enum Heard {
    /// The code was taken; waiting on the desktop's approval.
    Asked(String),
    Approved(String),
    /// Still waiting.
    Pending,
    Failed(String),
}

pub struct Pair {
    /// Why the browser is asked again, when it had a key.
    note: Option<&'static str>,
    error: Option<String>,
    /// The request the desktop is asked to approve.
    waiting: Option<String>,
    busy: bool,
    next_poll: Option<Instant>,
    heard: Rc<RefCell<Option<Heard>>>,
    paired: Option<Paired>,
    wake: Option<Wake>,
}

impl Pair {
    pub fn new(note: Option<&'static str>) -> Pair {
        Pair {
            note,
            error: None,
            waiting: None,
            busy: false,
            next_poll: None,
            heard: Rc::default(),
            paired: None,
            wake: None,
        }
    }

    pub fn take_paired(&mut self) -> Option<Paired> {
        self.paired.take()
    }

    pub fn event(&mut self, ui: &mut Ui, event: &Event, wake: Wake) -> Option<Paired> {
        let go =
            matches!(event, Event::Click(id) | Event::Submit(id) if *id == CONNECT || *id == FIELD);
        if !go || self.busy {
            return None;
        }
        self.wake = Some(wake);
        let typed = ui.edit_text(FIELD).trim().to_owned();
        if typed.is_empty() {
            self.error = Some("Enter the pairing code or a device token.".into());
            return None;
        }
        if typed.starts_with(TOKEN_PREFIX) {
            return Some(Paired(typed));
        }
        self.error = None;
        self.busy = true;
        let body = json!({ "code": typed, "deviceName": device_name() });
        self.ask("/api/pair/redeem", body);
        None
    }

    fn ask(&self, path: &'static str, body: Value) {
        let (heard, wake) = (Rc::clone(&self.heard), self.wake.clone());
        wasm_bindgen_futures::spawn_local(async move {
            let got = match page::post_json(path, &body).await {
                Ok(v) => read(&v),
                Err(e) => Heard::Failed(refusal(&e)),
            };
            *heard.borrow_mut() = Some(got);
            if let Some(wake) = wake {
                wake();
            }
        });
    }

    /// Takes what vornd answered; when to look again.
    pub fn tick(&mut self) -> Option<Instant> {
        let heard = self.heard.borrow_mut().take();
        match heard {
            Some(Heard::Asked(id)) => {
                self.waiting = Some(id);
                self.next_poll = Some(Instant::now() + POLL_EVERY);
            }
            Some(Heard::Pending) => self.next_poll = Some(Instant::now() + POLL_EVERY),
            Some(Heard::Approved(token)) => {
                self.busy = false;
                self.waiting = None;
                self.next_poll = None;
                self.paired = Some(Paired(token));
            }
            Some(Heard::Failed(why)) => {
                self.busy = false;
                self.waiting = None;
                self.next_poll = None;
                self.error = Some(why);
            }
            None => {}
        }
        let at = self.next_poll?;
        if Instant::now() < at {
            return Some(at);
        }
        self.next_poll = None;
        if let Some(id) = &self.waiting {
            self.ask("/api/pair/poll", json!({ "requestId": id }));
        }
        None
    }

    pub fn view(&mut self, ui: &mut Ui) -> El {
        let (w, h) = ui.viewport();
        let ink = |a| look::INK.alpha(a);
        let white = |a| look::WHITE.alpha(a);
        let focus = if ui.focused(FIELD) { 1.0 } else { 0.0 };
        let style = TextStyle {
            line_height: 20.0,
            mono: true,
            ..TextStyle::ui(14.0)
        };
        let input = field(
            FIELD,
            "Pairing code or device token",
            "Code or vorn_ token",
            style,
            ink(0.9),
        )
        .px(12.0)
        .py(8.0)
        .rounded(4.0)
        .bg(white(0.04))
        .border(1.0, white(0.08).mix(white(0.2), focus))
        .w_full();
        let hot = ui.hover(CONNECT);
        let label = if self.waiting.is_some() {
            "Waiting"
        } else if self.busy {
            "Connecting"
        } else {
            "Connect"
        };
        let button = div()
            .id(CONNECT)
            .on(Action::Click)
            .ring(4.0)
            .px(16.0)
            .py(8.0)
            .rounded(4.0)
            .shrink0()
            .border(1.0, white(0.08))
            .bg(white(0.0).mix(white(0.06), hot))
            .role(Role::Button, label)
            .child(
                text(label, 13.0, ink(0.9))
                    .weight(500)
                    .line_height(16.0)
                    .silent(),
            );
        let line = |t: String, a: f32| text(t, 12.0, ink(a)).line_height(16.0).wrapping().silent();
        let status = match (&self.error, &self.waiting) {
            (Some(e), _) => Some(
                text(e.clone(), 12.0, look::RED_400.rgba())
                    .line_height(16.0)
                    .wrapping(),
            ),
            (None, Some(_)) => Some(line(
                "Approve this browser in Vorn on your computer.".into(),
                look::INK_SECONDARY,
            )),
            _ => None,
        };
        let card = div()
            .col()
            .w(400.0)
            .gap(8.0)
            .child_if(
                self.note
                    .map(|n| line(n.into(), look::INK_SECONDARY).mb(8.0)),
            )
            .child(
                text("Pair this browser", 16.0, ink(0.9))
                    .weight(500)
                    .line_height(24.0),
            )
            .child(line(
                "Vorn on your computer shows a pairing code in Settings › Network › Add device. \
                 Enter it here, then approve the request there. A device token from \
                 `vorn server token create` works too."
                    .into(),
                look::INK_SECONDARY,
            ))
            .child(
                text(
                    "Pairing code or device token",
                    12.0,
                    ink(look::INK_SECONDARY),
                )
                .line_height(16.0)
                .mt(8.0)
                .silent(),
            )
            .child(
                div()
                    .row()
                    .gap(8.0)
                    .items_center()
                    .child(input)
                    .child(button),
            )
            .child_if(status);
        div()
            .size(w, h)
            .col()
            .center()
            .bg(look::SURFACE_BASE.rgba())
            .child(card)
    }
}

fn read(v: &Value) -> Heard {
    if let Some(id) = v.get("requestId").and_then(Value::as_str) {
        return Heard::Asked(id.to_owned());
    }
    match v.get("status").and_then(Value::as_str) {
        Some("pending") => Heard::Pending,
        Some("approved") => match v.get("token").and_then(Value::as_str) {
            Some(t) => Heard::Approved(t.to_owned()),
            None => Heard::Failed("vornd approved this browser but sent no token.".into()),
        },
        Some("denied") => Heard::Failed("The request was declined on your computer.".into()),
        Some("expired") => Heard::Failed("The request expired. Enter a new code.".into()),
        _ => Heard::Failed("vornd answered something this page does not understand.".into()),
    }
}

/// vornd's reason for refusing a code, in words.
fn refusal(name: &str) -> String {
    match name {
        "unknown" => "That code is not one Vorn is showing. Check it and try again.".into(),
        "expired" => "That code expired. Vorn shows a new one in Settings › Network.".into(),
        "spent" => "That code was already used. Vorn shows a new one in Settings › Network.".into(),
        "throttled" => "Too many tries. Wait a minute, then try again.".into(),
        other => other.to_owned(),
    }
}

/// How the desktop lists this browser: its platform, from the user agent.
fn device_name() -> String {
    let ua = web_sys::window()
        .and_then(|w| w.navigator().user_agent().ok())
        .unwrap_or_default();
    let os = [
        ("iPhone", "iPhone"),
        ("iPad", "iPad"),
        ("Android", "Android"),
        ("Mac OS X", "Mac"),
        ("Windows", "Windows"),
        ("Linux", "Linux"),
    ]
    .into_iter()
    .find(|(k, _)| ua.contains(k))
    .map_or("Device", |(_, n)| n);
    format!("Browser on {os}")
}
