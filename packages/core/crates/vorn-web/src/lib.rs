//! Vorn in a browser: the desktop app's screens drawn by vornui in a canvas,
//! talking to the vornd that served the page over its `/ws` and `/grid`
//! WebSockets. The build's loader fetches the wasm and calls [`start`].

#![cfg(target_arch = "wasm32")]

mod pair;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use vorn_app::client::rpc::{RpcError, Wake};
use vorn_app::client::Endpoint;
use vorn_app::ui::{
    div, run_with, text, use_bundled_fonts, El, Event, Input, Laid, NativeWindow, Rect, Rgba, Ui,
    Waker, WindowApp, WindowOptions,
};
use vorn_app::{look, page, ui_config, App};
use wasm_bindgen::prelude::*;
use web_time::{Duration, Instant};

use pair::{Pair, Paired};

/// Where this browser keeps its device token; the pairing flow writes it.
const TOKEN_KEY: &str = "vorn.deviceToken";
/// The fonts the build ships beside the wasm: the UI's and the terminal's.
const FONTS: [&str; 4] = [
    "fonts/Inter-Regular.ttf",
    "fonts/Inter-Medium.ttf",
    "fonts/JetBrainsMono-Regular.ttf",
    "fonts/JetBrainsMono-Bold.ttf",
];
/// The app's own files (open panes, drafts) live in the page's storage
/// under names from this directory.
const DATA_DIR: &str = "/vorn";
const RETRY: Duration = Duration::from_secs(2);

/// Opens the app in the page.
#[wasm_bindgen(start)]
pub fn start() {
    std::panic::set_hook(Box::new(|info| {
        web_sys::console::error_1(&JsValue::from_str(&info.to_string()));
    }));
    wasm_bindgen_futures::spawn_local(async {
        if let Err(e) = boot().await {
            web_sys::console::error_1(&JsValue::from_str(&e));
        }
    });
}

async fn boot() -> Result<(), String> {
    let fonts = futures_util::future::join_all(FONTS.map(page::get_bytes)).await;
    use_bundled_fonts(fonts.into_iter().flatten().collect());
    take_token_from_address();
    // The page's waker exists only once the app is made; the app's wake
    // reaches it through this slot.
    let waker: Arc<Mutex<Option<Waker>>> = Arc::default();
    let w = Arc::clone(&waker);
    let wake: Wake = Arc::new(move || {
        if let Some(w) = w.lock().ok().as_ref().and_then(|w| w.as_ref()) {
            w.wake();
        }
    });
    run_with(
        "Vorn",
        WindowOptions::sized(look::WINDOW),
        ui_config(1.0),
        move |_, page_waker| {
            if let Ok(mut slot) = waker.lock() {
                *slot = Some(page_waker);
            }
            Web::new(wake)
        },
    )
}

/// A token handed over in the address (`?token=` or `#token=`, as a phone
/// opening this page passes it) is kept, then taken out of the address.
fn take_token_from_address() {
    let Some(win) = web_sys::window() else { return };
    let loc = win.location();
    let (search, hash) = (
        loc.search().unwrap_or_default(),
        loc.hash().unwrap_or_default(),
    );
    let found = [search.trim_start_matches('?'), hash.trim_start_matches('#')]
        .into_iter()
        .flat_map(|q| q.split('&'))
        .find_map(|kv| kv.strip_prefix("token="))
        .map(str::to_owned);
    let Some(token) = found.filter(|t| !t.is_empty()) else {
        return;
    };
    page::save(TOKEN_KEY, &token);
    // Everything but the token stays, `?renderer=` among it.
    let without = |q: &str| {
        q.split('&')
            .filter(|kv| !kv.is_empty() && !kv.starts_with("token="))
            .collect::<Vec<_>>()
            .join("&")
    };
    let mut path = loc.pathname().unwrap_or_else(|_| "/app/".into());
    for (mark, q) in [
        ("?", without(search.trim_start_matches('?'))),
        ("#", without(hash.trim_start_matches('#'))),
    ] {
        if !q.is_empty() {
            path = format!("{path}{mark}{q}");
        }
    }
    if let Ok(history) = win.history() {
        let _ = history.replace_state_with_url(&JsValue::NULL, "", Some(&path));
    }
}

/// What the page shows.
enum Phase {
    /// No accepted key yet.
    Pairing(Pair),
    /// Asking vornd for the app's data.
    Connecting,
    Running(Box<App>),
    /// vornd went away; tries again at the instant.
    Lost(String, Instant),
}

struct Web {
    phase: Phase,
    wake: Wake,
    /// Where a connection attempt lands.
    landed: Rc<RefCell<Option<Result<App, RpcError>>>>,
    window: Option<Arc<NativeWindow>>,
}

impl Web {
    fn new(wake: Wake) -> Web {
        let mut web = Web {
            phase: Phase::Connecting,
            wake,
            landed: Rc::default(),
            window: None,
        };
        match page::load(TOKEN_KEY).filter(|t| !t.is_empty()) {
            Some(token) => web.connect(token),
            None => web.phase = Phase::Pairing(Pair::new(None)),
        }
        web
    }

    fn connect(&mut self, token: String) {
        self.phase = Phase::Connecting;
        let (landed, wake) = (Rc::clone(&self.landed), Arc::clone(&self.wake));
        wasm_bindgen_futures::spawn_local(async move {
            let got = App::connect_async(&Endpoint::page(token), Arc::clone(&wake)).await;
            *landed.borrow_mut() = Some(got);
            wake();
        });
    }

    /// Asks for a new key, saying why the last one is gone.
    fn unpaired(&mut self) {
        page::forget(TOKEN_KEY);
        self.phase = Phase::Pairing(Pair::new(Some(
            "Vorn on your computer no longer accepts this browser. Pair it again.",
        )));
    }

    fn landed(&mut self, got: Result<App, RpcError>) {
        match got {
            Ok(mut app) => {
                let dir = PathBuf::from(DATA_DIR);
                app.set_data_dir(dir.clone());
                app.load_panes(&dir);
                if let Some(w) = &self.window {
                    app.opened(w);
                }
                self.phase = Phase::Running(Box::new(app));
            }
            Err(RpcError::Rejected) => self.unpaired(),
            Err(e) => {
                let why = format!("Could not reach vornd ({e}). Trying again.");
                self.phase = Phase::Lost(why, Instant::now() + RETRY);
            }
        }
    }

    fn redraw(&self) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }
}

impl WindowApp for Web {
    fn view(&mut self, ui: &mut Ui) -> El {
        let size = ui.viewport();
        match &mut self.phase {
            Phase::Running(app) => app.view(ui, size),
            Phase::Pairing(p) => p.view(ui),
            Phase::Connecting => status(ui, "Connecting to vornd.", None),
            Phase::Lost(why, _) => status(ui, "Lost the connection to vornd.", Some(why)),
        }
    }

    fn paint(&mut self, ui: &mut Ui, laid: &Laid) {
        if let Phase::Running(app) = &mut self.phase {
            app.paint(ui, laid);
        }
    }

    fn event(&mut self, ui: &mut Ui, event: Event) {
        match &mut self.phase {
            Phase::Running(app) => app.event(ui, event),
            Phase::Pairing(p) => {
                if let Some(Paired(token)) = p.event(ui, &event, Arc::clone(&self.wake)) {
                    page::save(TOKEN_KEY, &token);
                    self.connect(token);
                }
            }
            _ => {}
        }
    }

    fn input(&mut self, _ui: &mut Ui, input: Input) {
        if let Phase::Running(app) = &mut self.phase {
            app.input(input);
        }
    }

    fn intercept(&mut self, ui: &mut Ui, input: &Input) -> bool {
        match &mut self.phase {
            Phase::Running(app) => app.intercept(ui, input),
            _ => false,
        }
    }

    fn paste(&mut self, ui: &mut Ui, text: String) {
        if let Phase::Running(app) = &mut self.phase {
            return app.paste(ui, text);
        }
        let chord = Input::Key {
            code: "KeyV".into(),
            mods: vorn_app::app::MOD(),
            text: None,
        };
        ui.key(&chord);
    }

    fn background(&self) -> Rgba {
        Rgba::hex(look::SURFACE_BASE)
    }

    fn wants_ime(&self) -> bool {
        matches!(self.phase, Phase::Running(_))
    }

    fn ime_area(&self) -> Option<Rect> {
        None
    }

    fn opened(&mut self, window: &Arc<NativeWindow>) {
        self.window = Some(Arc::clone(window));
        if let Phase::Running(app) = &mut self.phase {
            app.opened(window);
        }
    }

    fn tick(&mut self) -> Option<Instant> {
        let got = self.landed.borrow_mut().take();
        if let Some(got) = got {
            self.landed(got);
            self.redraw();
        }
        match &mut self.phase {
            Phase::Running(app) if app.rejected() => {
                self.unpaired();
                self.redraw();
                None
            }
            Phase::Running(app) if app.closed => {
                let why = "vornd stopped answering. Trying again.".to_owned();
                self.phase = Phase::Lost(why, Instant::now() + RETRY);
                self.redraw();
                Some(Instant::now() + RETRY)
            }
            Phase::Running(app) => app.tick(),
            Phase::Pairing(p) => {
                let next = p.tick();
                if let Some(Paired(token)) = p.take_paired() {
                    page::save(TOKEN_KEY, &token);
                    self.connect(token);
                    self.redraw();
                }
                next
            }
            Phase::Lost(_, at) if Instant::now() >= *at => {
                match page::load(TOKEN_KEY).filter(|t| !t.is_empty()) {
                    Some(token) => self.connect(token),
                    None => self.phase = Phase::Pairing(Pair::new(None)),
                }
                self.redraw();
                None
            }
            Phase::Lost(_, at) => Some(*at),
            Phase::Connecting => None,
        }
    }
}

/// A quiet screen with one line and an optional detail, while there is no app.
fn status(ui: &Ui, line: &str, detail: Option<&str>) -> El {
    let (w, h) = ui.viewport();
    let ink = |a| Rgba::hexa(look::INK, a);
    div()
        .size(w, h)
        .col()
        .center()
        .gap(8.0)
        .bg(Rgba::hex(look::SURFACE_BASE))
        .child(text(line, 13.0, ink(look::INK_SECONDARY)))
        .child_if(detail.map(|d| text(d, 12.0, ink(look::INK_FAINT))))
}
