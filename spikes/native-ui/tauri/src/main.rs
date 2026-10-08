//! The Tauri prototype: the shared grid client runs in Rust; the panes are
//! drawn on canvases in the system webview from the same views the other
//! prototypes draw, shipped as bytes over an IPC channel. The webview does
//! no terminal parsing.

use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use serde::Deserialize;
use spike_core::bench::{Bench, Config, Mode, Step};
use spike_core::Grid;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{Emitter, WebviewUrl, WebviewWindowBuilder};

/// When the pusher may send: the grid changed and the page drew the last
/// batch (so at most one batch is in flight, as with a frame callback).
#[derive(Default)]
struct Gate {
    dirty: bool,
    acked: bool,
}

struct App {
    grid: Arc<Grid>,
    bench: Mutex<Bench>,
    gate: Arc<(Mutex<Gate>, Condvar)>,
}

#[derive(serde::Serialize)]
struct Hello {
    panes: usize,
    bench: bool,
}

#[tauri::command]
fn attach(state: tauri::State<'_, Arc<App>>, channel: Channel<InvokeResponseBody>) -> Hello {
    let app = Arc::clone(&state);
    let gate = Arc::clone(&app.gate);
    {
        let gate = Arc::clone(&gate);
        app.grid.set_waker(move || {
            let (m, cv) = &*gate;
            m.lock().unwrap().dirty = true;
            cv.notify_one();
        });
    }
    gate.0.lock().unwrap().acked = true;
    let grid = Arc::clone(&app.grid);
    std::thread::spawn(move || {
        let (m, cv) = &*gate;
        let mut buf = Vec::new();
        loop {
            {
                let mut g = cv.wait_while(m.lock().unwrap(), |g| !(g.dirty && g.acked)).unwrap();
                g.dirty = false;
                g.acked = false;
            }
            buf.clear();
            for i in grid.take_dirty() {
                if let Some(v) = grid.view(i) {
                    v.to_bytes(i as u32, &mut buf);
                }
            }
            if buf.is_empty() {
                m.lock().unwrap().acked = true;
                continue;
            }
            if channel.send(InvokeResponseBody::Raw(buf.clone())).is_err() {
                break;
            }
        }
    });
    let bench = app.bench.lock().unwrap().cfg.mode != Mode::Interactive;
    Hello {
        panes: app.grid.panes(),
        bench,
    }
}

/// The page drew the last batch.
#[tauri::command]
fn ack(state: tauri::State<'_, Arc<App>>) {
    let (m, cv) = &*state.gate;
    m.lock().unwrap().acked = true;
    cv.notify_one();
}

#[tauri::command]
fn key(state: tauri::State<'_, Arc<App>>, pane: usize, code: String, mods: u16, text: Option<String>) {
    state.grid.key(pane, &code, mods, text.as_deref());
}

#[tauri::command]
fn text(state: tauri::State<'_, Arc<App>>, pane: usize, text: String) {
    state.grid.text(pane, &text);
}

#[tauri::command]
fn resize(state: tauri::State<'_, Arc<App>>, pane: usize, cols: u16, rows: u16) {
    state.grid.resize(pane, cols, rows);
}

#[tauri::command]
fn first_frame(state: tauri::State<'_, Arc<App>>) {
    state.bench.lock().unwrap().first_frame();
}

/// Frame and latency samples, batched by the page (in its own clock).
#[derive(Deserialize)]
struct Samples {
    at: Vec<f64>,
    work: Vec<f64>,
    latency: Vec<f64>,
    done: bool,
}

#[tauri::command]
fn samples(app: tauri::AppHandle, state: tauri::State<'_, Arc<App>>, s: Samples) {
    let mut b = state.bench.lock().unwrap();
    for (at, work) in s.at.iter().zip(&s.work) {
        b.frame(*at, *work);
    }
    for l in s.latency {
        b.latency(l);
    }
    if s.done {
        let errs: Vec<String> = state.grid.errors().iter().map(|e| format!("{e:?}")).collect();
        let name = if std::env::var_os("VORN_SPIKE_WEBVIEW_120").is_some() { "tauri-120" } else { "tauri" };
        b.write(name, state.grid.panes(), &[("errors", format!("[{}]", errs.join(",")))]);
        app.exit(0);
    }
}

/// Drives the latency bench: the page injects each key itself, so the
/// clock starts and stops in the page.
fn bench_loop(app: tauri::AppHandle, state: Arc<App>) {
    loop {
        std::thread::sleep(Duration::from_millis(4));
        let step = state.bench.lock().unwrap().tick(&state.grid);
        let _ = match step {
            Step::Idle => continue,
            Step::Type(c) => app.emit("inject", c.to_string()),
            Step::Enter => app.emit("inject", "Enter"),
            Step::Done => {
                let _ = app.emit("done", ());
                return;
            }
        };
    }
}

/// WebKit holds `requestAnimationFrame` near 60 fps by default, even on a
/// 120 Hz display; this private preference lifts that (a variant only).
#[cfg(target_os = "macos")]
fn unlock_120(webview: *mut std::ffi::c_void) {
    use objc2::runtime::{AnyObject, Bool};
    use objc2::{msg_send, sel};
    // SAFETY: `webview` is the live WKWebView; each call is checked first.
    unsafe {
        let v = webview.cast::<AnyObject>();
        let cfg: *mut AnyObject = msg_send![v, configuration];
        let prefs: *mut AnyObject = msg_send![cfg, preferences];
        // Feature flags are listed by `+[WKPreferences _features]`.
        let Some(cls) = objc2::runtime::AnyClass::get(c"WKPreferences") else {
            return;
        };
        if !msg_send![cls, respondsToSelector: sel!(_features)] {
            eprintln!("webview: no feature flags");
            return;
        }
        let features: *mut AnyObject = msg_send![cls, _features];
        let n: usize = msg_send![features, count];
        for i in 0..n {
            let f: *mut AnyObject = msg_send![features, objectAtIndex: i];
            let key: *mut AnyObject = msg_send![f, key];
            let utf8: *const std::ffi::c_char = msg_send![key, UTF8String];
            if std::ffi::CStr::from_ptr(utf8).to_bytes() == b"PreferPageRenderingUpdatesNear60FPSEnabled" {
                let _: () = msg_send![prefs, _setEnabled: Bool::NO, forFeature: f];
                eprintln!("webview: 120 Hz unlocked");
                return;
            }
        }
        eprintln!("webview: no 60 fps flag");
    }
}

#[cfg(not(target_os = "macos"))]
fn unlock_120(_: *mut std::ffi::c_void) {}

fn main() {
    let Some(socket) = spike_core::env::grid() else {
        eprintln!("no grid endpoint (VORN_SPIKE_GRID/VORN_SPIKE_SESSIONS)");
        std::process::exit(1);
    };
    let sessions = spike_core::env::sessions();
    let cfg = Config::from_env();
    let interactive = cfg.mode == Mode::Interactive;
    let mut bench = Bench::new(cfg);
    bench.period_ms = 1000.0 / 120.0;
    let grid = match Grid::connect(&socket, sessions, 80, 24) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("grid: {e}");
            std::process::exit(1);
        }
    };
    let state = Arc::new(App {
        grid,
        bench: Mutex::new(bench),
        gate: Arc::new((Mutex::new(Gate::default()), Condvar::new())),
    });
    tauri::Builder::default()
        .manage(Arc::clone(&state))
        .invoke_handler(tauri::generate_handler![attach, ack, key, text, resize, first_frame, samples])
        .setup(move |app| {
            let w = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .title("Vorn spike: Tauri")
                .inner_size(1440.0, 900.0)
                .position(40.0, 40.0)
                .focused(interactive)
                .always_on_top(!interactive)
                .background_throttling(tauri::utils::config::BackgroundThrottlingPolicy::Disabled)
                .build()?;
            if std::env::var_os("VORN_SPIKE_WEBVIEW_120").is_some() {
                let _ = w.with_webview(|wv| unlock_120(wv.inner()));
            }
            if !interactive {
                let handle = app.handle().clone();
                let st = Arc::clone(&state);
                std::thread::spawn(move || bench_loop(handle, st));
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("tauri");
}
