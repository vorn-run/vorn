//! The browser host: what [`crate::window`] is on a desktop. It picks a
//! renderer (WebGPU, then WebGL2, then the CPU into a 2D canvas), lets
//! winit drive the canvas's pointer, size and animation frames, takes keys
//! and composition from a hidden textarea (see [`keys`]), keeps a clipboard
//! the page's copy and paste events reach, and mirrors the accessibility
//! tree into the DOM ([`dom`]).
//!
//! A tab has one thread, so the state is one `Rc<RefCell<Live>>` that the
//! winit handler and the DOM callbacks borrow in turn; none of them calls
//! into another while holding it.

mod dom;
pub mod keys;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use web_time::{Duration, Instant};

use wasm_bindgen::prelude::*;
use wasm_bindgen::{Clamped, JsCast};
use web_sys::{
    CanvasRenderingContext2d, ClipboardEvent, CompositionEvent, DragEvent, HtmlCanvasElement,
    HtmlElement, HtmlTextAreaElement, ImageData, KeyboardEvent, MouseEvent,
};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::platform::web::{EventLoopExtWebSys, WindowAttributesExtWebSys};
use winit::window::{Window, WindowId};

use crate::edit::Clipboard;
use crate::gpu::Gpu;
use crate::host::{cursor_icon, draw, App, Dropped, UserEvent, Waker, WindowOptions};
use crate::input::{mods, Input};
use crate::interact::{WheelPhase, WindowRequest, WHEEL_LINE};
use crate::pace::Pacer;
use crate::render::RenderMode;
use crate::ui::{Ui, UiConfig};
use keys::{Inserted, KeyAction, KeyDown};

enum Present {
    Gpu {
        surface: wgpu::Surface<'static>,
        config: wgpu::SurfaceConfiguration,
    },
    /// The CPU renderer's RGBA rows, put into a 2D canvas.
    Cpu {
        ctx: CanvasRenderingContext2d,
        whole: bool,
    },
}

/// The renderer the page drew with, for the page's own checks.
static RENDERER: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// The renderer in use: `webgpu: …`, `webgl: …` or `cpu (why)`.
#[wasm_bindgen(js_name = vornRenderer)]
pub fn renderer_label() -> String {
    RENDERER.lock().map(|r| r.clone()).unwrap_or_default()
}

fn log_error(e: &str) {
    web_sys::console::error_1(&e.into());
}

struct Live<A> {
    ui: Ui,
    app: A,
    present: Present,
    canvas: HtmlCanvasElement,
    input: HtmlTextAreaElement,
    window: Option<Arc<Window>>,
    a11y: dom::A11yMirror,
    pacer: Pacer,
    pending: Option<Instant>,
    /// Canvas size in physical pixels.
    px: (u32, u32),
    mac: bool,
    clip: Rc<RefCell<Option<String>>>,
}

impl<A: App> Live<A> {
    fn redraw_later(&self) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    fn redraw(&mut self) -> Result<(), String> {
        let now = Instant::now();
        let px = self.px;
        if px.0 == 0 || px.1 == 0 {
            return Ok(());
        }
        let laid = match &mut self.present {
            Present::Gpu { surface, config } => {
                let gpu = self
                    .ui
                    .gpu
                    .as_ref()
                    .ok_or("gpu renderer without a device")?;
                if (config.width, config.height) != px {
                    (config.width, config.height) = px;
                    surface.configure(&gpu.device, config);
                }
                let frame = match surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(t)
                    | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
                    wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                        surface.configure(&gpu.device, config);
                        self.redraw_later();
                        return Ok(());
                    }
                    wgpu::CurrentSurfaceTexture::Timeout
                    | wgpu::CurrentSurfaceTexture::Occluded => return Ok(()),
                    other => return Err(format!("surface: {other:?}")),
                };
                let view = frame.texture.create_view(&Default::default());
                let laid = draw(&mut self.ui, &mut self.app, now, px, Some(&view));
                frame.present();
                laid
            }
            Present::Cpu { ctx, whole } => {
                let laid = draw(&mut self.ui, &mut self.app, now, px, None);
                if let Some((bytes, (w, h), damage)) = self.ui.cpu_pixels() {
                    let all = [(0, h)];
                    let rows = if *whole { &all[..] } else { damage };
                    let row = w as usize * 4;
                    for &(y0, y1) in rows {
                        let part = &bytes[y0 as usize * row..y1 as usize * row];
                        let img =
                            ImageData::new_with_u8_clamped_array_and_sh(Clamped(part), w, y1 - y0)
                                .map_err(|e| format!("image data: {e:?}"))?;
                        ctx.put_image_data(&img, 0.0, f64::from(y0))
                            .map_err(|e| format!("put image: {e:?}"))?;
                    }
                    *whole = false;
                }
                laid
            }
        };
        let echoed = self.app.echoed();
        self.pacer.frame(now, echoed);
        let k = self.ui.scale();
        self.a11y.update(&laid.tree, k);
        if let Some(r) = self.ui.ime_area().or_else(|| self.app.ime_area()) {
            let s = self.input.style();
            let _ = s.set_property("left", &format!("{}px", r.x));
            let _ = s.set_property("top", &format!("{}px", r.y));
            let _ = s.set_property("height", &format!("{}px", r.h.max(1.0)));
        }
        Ok(())
    }

    /// Hands the UI's events to the app, does what it asked of the page and
    /// asks for the frame that shows their effect.
    fn dispatch(&mut self) {
        for e in self.ui.take_events() {
            self.app.event(&mut self.ui, e);
        }
        for r in self.ui.take_requests() {
            match r {
                WindowRequest::ToggleFullscreen => toggle_fullscreen(),
                WindowRequest::Focus => {
                    if let Some(w) = web_sys::window() {
                        let _ = w.focus();
                    }
                }
                // A tab has no window of its own to move, size or close.
                WindowRequest::Minimize
                | WindowRequest::ToggleMaximize
                | WindowRequest::Close
                | WindowRequest::Drag
                | WindowRequest::Resize(..) => {}
            }
        }
        if let Some(w) = &self.window {
            w.set_cursor(cursor_icon(self.ui.cursor()));
        }
        self.redraw_later();
    }

    /// A key, a commit or a preedit: shortcuts first, then the UI, then the app.
    fn send(&mut self, input: Input) {
        self.ui.set_now(Instant::now());
        self.pacer.input(Instant::now());
        if !self.app.intercept(&mut self.ui, &input) && !self.ui.key(&input) {
            self.app.input(&mut self.ui, input);
        }
        self.dispatch();
    }

    fn key(&mut self, e: &KeyboardEvent) {
        let mut m = 0;
        for (on, bit) in [
            (e.shift_key(), mods::SHIFT),
            (e.alt_key(), mods::ALT),
            (e.ctrl_key(), mods::CTRL),
            (e.meta_key(), mods::SUPER),
        ] {
            if on {
                m |= bit;
            }
        }
        let k = KeyDown {
            key: &e.key(),
            code: &e.code(),
            mods: m,
            composing: e.is_composing(),
            mac: self.mac,
        };
        match keys::on_keydown(k) {
            KeyAction::Pass => {}
            KeyAction::Share { code, mods } => self.send(Input::Key {
                code,
                mods,
                text: None,
            }),
            KeyAction::Send { code, mods, text } => {
                e.prevent_default();
                self.send(Input::Key { code, mods, text });
            }
        }
    }

    fn inserted(&mut self, what: Inserted) {
        match what {
            Inserted::Nothing => return,
            Inserted::Text(t) => self.send(Input::Commit(t)),
            Inserted::Key(k) => self.send(Input::named(k)),
        }
        self.input.set_value("");
    }

    fn paste(&mut self, text: String) {
        *self.clip.borrow_mut() = Some(text.clone());
        self.ui.set_now(Instant::now());
        self.pacer.input(Instant::now());
        self.app.paste(&mut self.ui, text);
        self.dispatch();
    }

    fn dropped(&mut self, what: Dropped, at: Option<(f32, f32)>) {
        self.ui.set_now(Instant::now());
        self.app.dropped(&mut self.ui, what, at);
        self.dispatch();
    }

    /// Focuses the hidden input, so a phone raises its keyboard.
    fn focus_input(&self) {
        let _ = self.input.focus();
    }
}

fn toggle_fullscreen() {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    if doc.fullscreen_element().is_some() {
        doc.exit_fullscreen();
    } else if let Some(el) = doc.document_element() {
        let _ = el.request_fullscreen();
    }
}

/// What the app's copies land in: kept for the app's own pastes, written to
/// the system clipboard where the page may (a secure context), and handed
/// to the page's `copy` event where it may not.
struct WebClipboard(Rc<RefCell<Option<String>>>);

impl Clipboard for WebClipboard {
    fn get(&mut self) -> Option<String> {
        self.0.borrow().clone()
    }

    fn set(&mut self, text: &str) {
        *self.0.borrow_mut() = Some(text.to_owned());
        if let Some(w) = web_sys::window().filter(|w| w.is_secure_context()) {
            // A refused write keeps the copy for the app's own paste.
            let _ = w.navigator().clipboard().write_text(text);
        }
    }
}

struct Runner<A>(Rc<RefCell<Live<A>>>);

impl<A: App> ApplicationHandler<UserEvent> for Runner<A> {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        let Ok(mut s) = self.0.try_borrow_mut() else {
            return;
        };
        if s.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_canvas(Some(s.canvas.clone()))
            // Default actions are what synthesize the click that focuses the
            // input on a tap; `touch-action: none` already stops panning.
            .with_prevent_default(false)
            .with_focusable(false);
        match el.create_window(attrs) {
            Ok(w) => {
                let w = Arc::new(w);
                w.request_redraw();
                let s = &mut *s;
                s.app.opened(&w);
                s.window = Some(w);
            }
            Err(e) => log_error(&format!("window: {e}")),
        }
    }

    fn user_event(&mut self, _: &ActiveEventLoop, ev: UserEvent) {
        let Ok(mut s) = self.0.try_borrow_mut() else {
            return;
        };
        let UserEvent::Wake = ev;
        let now = Instant::now();
        let at = s.pacer.next_frame(now);
        if at <= now {
            s.redraw_later();
        } else {
            s.pending = Some(s.pending.map_or(at, |p| p.min(at)));
        }
    }

    fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, ev: WindowEvent) {
        let Ok(mut s) = self.0.try_borrow_mut() else {
            return;
        };
        // winit's own scale, which its pointer positions are in.
        let k = s.window.as_ref().map_or(1.0, |w| w.scale_factor() as f32);
        s.ui.set_now(Instant::now());
        match ev {
            // winit's size and scale disagree with devicePixelRatio under
            // mobile emulation, so both are read from the page.
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                let scale = web_sys::window().map_or(1.0, |w| w.device_pixel_ratio());
                let px = (
                    (f64::from(s.canvas.client_width()) * scale).round() as u32,
                    (f64::from(s.canvas.client_height()) * scale).round() as u32,
                );
                s.ui.set_scale(scale as f32);
                s.px = px;
                s.canvas.set_width(px.0);
                s.canvas.set_height(px.1);
                if let Present::Cpu { whole, .. } = &mut s.present {
                    *whole = true;
                }
                s.redraw_later();
            }
            WindowEvent::RedrawRequested => {
                s.pending = None;
                if let Err(e) = s.redraw() {
                    log_error(&e);
                }
            }
            WindowEvent::Focused(focused) => {
                s.ui.set_window_focused(focused);
                if !focused {
                    s.ui.pointer_leave();
                }
                s.dispatch();
            }
            WindowEvent::CursorMoved { position, .. } => {
                s.ui.pointer_move(position.x as f32 / k, position.y as f32 / k);
                s.dispatch();
            }
            WindowEvent::CursorLeft { .. } => {
                s.ui.pointer_leave();
                s.dispatch();
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                match state {
                    ElementState::Pressed => s.ui.pointer_down(),
                    ElementState::Released => s.ui.pointer_up(),
                }
                s.dispatch();
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Right,
                ..
            } => {
                s.ui.pointer_down_secondary();
                s.dispatch();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                // A page's pixel deltas carry the browser's own momentum, phaseless.
                let (dy, phase) = match delta {
                    MouseScrollDelta::LineDelta(_, y) => (-y * WHEEL_LINE, WheelPhase::Line),
                    MouseScrollDelta::PixelDelta(p) => (-(p.y as f32) / k, WheelPhase::Moved),
                };
                s.ui.wheel_phase(dy, phase);
                s.dispatch();
            }
            // A finger is a pointer that is down while it touches.
            WindowEvent::Touch(t) => {
                s.ui.pointer_move(t.location.x as f32 / k, t.location.y as f32 / k);
                match t.phase {
                    TouchPhase::Started => s.ui.pointer_down(),
                    TouchPhase::Moved => {}
                    TouchPhase::Ended => s.ui.pointer_up(),
                    TouchPhase::Cancelled => s.ui.pointer_leave(),
                }
                s.dispatch();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        let Ok(mut s) = self.0.try_borrow_mut() else {
            return;
        };
        let now = Instant::now();
        let ui_wake = s.ui.next_wake().map(|t| t.max(s.pacer.next_frame(now)));
        let wake = match (s.pending, ui_wake) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        let tick = s.app.tick();
        match wake {
            Some(t) if t <= now => {
                s.pending = None;
                s.redraw_later();
                el.set_control_flow(tick.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
            }
            Some(t) => el.set_control_flow(ControlFlow::WaitUntil(tick.map_or(t, |k| k.min(t)))),
            None => el.set_control_flow(tick.map_or(ControlFlow::Wait, ControlFlow::WaitUntil)),
        }
    }
}

/// A fresh canvas in `old`'s place: a canvas keeps the first context it
/// gave out, so WebGL cannot follow a WebGPU attempt on the same one.
fn replace_canvas(old: &HtmlCanvasElement) -> Result<HtmlCanvasElement, JsValue> {
    let doc = old.owner_document().ok_or("canvas outside a document")?;
    let new: HtmlCanvasElement = doc.create_element("canvas")?.dyn_into()?;
    for name in ["id", "class", "style", "tabindex", "aria-hidden"] {
        if let Some(v) = old.get_attribute(name) {
            new.set_attribute(name, &v)?;
        }
    }
    old.replace_with_with_node_1(&new)?;
    Ok(new)
}

/// The page as wgpu's GL backend wants its display named; WebGPU needs none.
#[derive(Debug)]
struct WebDisplay;

impl raw_window_handle::HasDisplayHandle for WebDisplay {
    fn display_handle(
        &self,
    ) -> Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
        Ok(raw_window_handle::DisplayHandle::web())
    }
}

async fn open_gpu(
    canvas: &HtmlCanvasElement,
    backends: wgpu::Backends,
) -> Result<(Gpu, wgpu::Surface<'static>), String> {
    let mut desc = wgpu::InstanceDescriptor::new_with_display_handle(Box::new(WebDisplay));
    desc.backends = backends;
    let instance = wgpu::Instance::new(desc);
    let surface = instance
        .create_surface(wgpu::SurfaceTarget::Canvas(canvas.clone()))
        .map_err(|e| e.to_string())?;
    let adapter = Gpu::adapter_async(&instance, Some(&surface)).await?;
    let gpu = Gpu::open_async(instance, adapter).await?;
    Ok((gpu, surface))
}

fn gpu_present(
    gpu: &Gpu,
    surface: wgpu::Surface<'static>,
    px: (u32, u32),
) -> Result<Present, String> {
    let caps = surface.get_capabilities(&gpu.adapter);
    // A non-sRGB target blends in sRGB space, as CSS colors do.
    let format = caps
        .formats
        .iter()
        .copied()
        .find(|f| !f.is_srgb())
        .or(caps.formats.first().copied())
        .ok_or("surface has no formats")?;
    // Copying from the frame is what backdrop blur reads; without it blurs are skipped.
    let usage =
        wgpu::TextureUsages::RENDER_ATTACHMENT | (caps.usages & wgpu::TextureUsages::COPY_SRC);
    let config = wgpu::SurfaceConfiguration {
        usage,
        format,
        width: px.0.max(1),
        height: px.1.max(1),
        present_mode: wgpu::PresentMode::Fifo,
        desired_maximum_frame_latency: 1,
        alpha_mode: caps.alpha_modes[0],
        view_formats: vec![],
    };
    surface.configure(&gpu.device, &config);
    Ok(Present::Gpu { surface, config })
}

/// Picks the renderer `want` names ("webgpu", "webgl", "cpu" or "auto",
/// which tries them in that order) and answers it with the canvas it
/// draws on, which a failed attempt replaced.
async fn renderer(
    want: &str,
    mut canvas: HtmlCanvasElement,
    px: (u32, u32),
) -> Result<(HtmlCanvasElement, Option<Gpu>, Present, String), JsValue> {
    let mut tried = Vec::new();
    for (name, backends) in [
        ("webgpu", wgpu::Backends::BROWSER_WEBGPU),
        ("webgl", wgpu::Backends::GL),
    ] {
        if want != "auto" && want != name {
            continue;
        }
        if name == "webgpu" && !wgpu::util::is_browser_webgpu_supported().await {
            tried.push("webgpu: not offered".to_owned());
            continue;
        }
        match open_gpu(&canvas, backends).await {
            Ok((gpu, surface)) => {
                let present = gpu_present(&gpu, surface, px)?;
                let label = format!("{name}: {}", gpu.adapter_name());
                return Ok((canvas, Some(gpu), present, label));
            }
            Err(e) => {
                tried.push(format!("{name}: {e}"));
                canvas = replace_canvas(&canvas)?;
            }
        }
    }
    let ctx: CanvasRenderingContext2d = canvas
        .get_context("2d")?
        .ok_or("no 2d context")?
        .dyn_into()?;
    let why = if tried.is_empty() {
        String::new()
    } else {
        format!(" ({})", tried.join("; "))
    };
    Ok((
        canvas,
        None,
        Present::Cpu { ctx, whole: true },
        format!("cpu{why}"),
    ))
}

/// The page's own elements: the canvas filling it, the hidden textarea keys
/// and composition arrive in, and the accessibility mirror's root.
fn page(
    doc: &web_sys::Document,
) -> Result<(HtmlCanvasElement, HtmlTextAreaElement, HtmlElement), JsValue> {
    let body = doc.body().ok_or("no body")?;
    let canvas: HtmlCanvasElement = doc.create_element("canvas")?.dyn_into()?;
    canvas.set_id("vorn-canvas");
    canvas.set_attribute("aria-hidden", "true")?;
    canvas.set_attribute(
        "style",
        "position:fixed;inset:0;width:100vw;height:100dvh;display:block;touch-action:none;outline:none",
    )?;
    let input: HtmlTextAreaElement = doc.create_element("textarea")?.dyn_into()?;
    input.set_id("vorn-input");
    for (k, v) in [
        ("autocapitalize", "off"),
        ("autocomplete", "off"),
        ("autocorrect", "off"),
        ("spellcheck", "false"),
        ("aria-label", "Terminal input"),
    ] {
        input.set_attribute(k, v)?;
    }
    // Kept on screen and tiny, so the IME's window follows the caret.
    input.set_attribute(
        "style",
        "position:fixed;left:0;top:0;width:1px;height:1px;opacity:0;padding:0;border:0;resize:none;overflow:hidden;white-space:pre;z-index:-1;caret-color:transparent;font-size:16px",
    )?;
    let a11y: HtmlElement = doc.create_element("div")?.dyn_into()?;
    a11y.set_id("vorn-a11y");
    a11y.set_attribute(
        "style",
        "position:fixed;inset:0;pointer-events:none;color:transparent;overflow:hidden",
    )?;
    body.append_child(&canvas)?;
    body.append_child(&a11y)?;
    body.append_child(&input)?;
    Ok((canvas, input, a11y))
}

/// Opens the app in the page and returns at once; the renderer is picked
/// and the app made on the page's own time. `?renderer=webgpu|webgl|cpu`
/// in the page's address overrides the choice.
pub fn run_with<A, F>(
    title: &str,
    _opts: WindowOptions,
    cfg: UiConfig,
    make: F,
) -> Result<(), String>
where
    A: App,
    F: FnOnce(&mut Ui, Waker) -> A + 'static,
{
    let title = title.to_owned();
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(e) = start(&title, cfg, make).await {
            log_error(&format!("{e:?}"));
        }
    });
    Ok(())
}

/// [`run_with`] with the default options.
pub fn run<A, F>(title: &str, size: (f32, f32), cfg: UiConfig, make: F) -> Result<(), String>
where
    A: App,
    F: FnOnce(&mut Ui, Waker) -> A + 'static,
{
    run_with(title, WindowOptions::sized(size), cfg, make)
}

async fn start<A, F>(title: &str, mut cfg: UiConfig, make: F) -> Result<(), JsValue>
where
    A: App,
    F: FnOnce(&mut Ui, Waker) -> A,
{
    let win = web_sys::window().ok_or("no window")?;
    let doc = win.document().ok_or("no document")?;
    doc.set_title(title);
    let (canvas, input, a11y_root) = page(&doc)?;
    let want = web_sys::UrlSearchParams::new_with_str(&win.location().search()?)?
        .get("renderer")
        .unwrap_or_else(|| "auto".into());
    let scale = win.device_pixel_ratio();
    let px = (
        (f64::from(canvas.client_width()) * scale).round() as u32,
        (f64::from(canvas.client_height()) * scale).round() as u32,
    );
    canvas.set_width(px.0);
    canvas.set_height(px.1);

    let (canvas, gpu, present, label) = renderer(&want, canvas, px).await?;
    let format = match &present {
        Present::Gpu { config, .. } => config.format,
        Present::Cpu { .. } => wgpu::TextureFormat::Rgba8Unorm,
    };
    let mode = if gpu.is_some() {
        RenderMode::Gpu
    } else {
        RenderMode::Cpu
    };
    cfg.scale = scale as f32;
    let adapter = gpu.as_ref().map(Gpu::adapter_name);
    let mut ui = Ui::build(gpu, adapter, mode, format, &cfg);
    let _ = canvas.set_attribute("data-renderer", &label);
    if let Ok(mut r) = RENDERER.lock() {
        *r = label;
    }
    let clip = Rc::new(RefCell::new(None));
    ui.set_clipboard(Box::new(WebClipboard(clip.clone())));

    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .map_err(|e| e.to_string())?;
    let app = make(&mut ui, Waker::new(event_loop.create_proxy()));

    let nav = win.navigator();
    let mac = nav.platform().unwrap_or_default().starts_with("Mac")
        || nav.user_agent().unwrap_or_default().contains("iPhone")
        || nav.user_agent().unwrap_or_default().contains("iPad");
    mods::set_mac(mac);
    let shared = Rc::new(RefCell::new(Live {
        ui,
        app,
        present,
        canvas: canvas.clone(),
        input: input.clone(),
        window: None,
        a11y: dom::A11yMirror::new(doc.clone(), a11y_root.clone()),
        pacer: Pacer::new(Duration::from_micros(16_667)),
        pending: None,
        px,
        mac,
        clip,
    }));
    listen(&shared, &canvas, &input, &a11y_root, &doc)?;
    follow_system_look(&shared, &win)?;
    let _ = input.focus();
    event_loop.spawn_app(Runner(shared));
    Ok(())
}

/// Keeps the UI's idea of the system's dark appearance and Reduce
/// transparency in step with the browser's media queries, live.
fn follow_system_look<A: App>(
    shared: &Rc<RefCell<Live<A>>>,
    win: &web_sys::Window,
) -> Result<(), JsValue> {
    type Set = fn(&mut Ui, bool);
    let queries: [(&str, Set); 2] = [
        ("(prefers-color-scheme: dark)", Ui::set_system_dark),
        (
            "(prefers-reduced-transparency: reduce)",
            Ui::set_reduced_transparency,
        ),
    ];
    for (query, set) in queries {
        let Some(list) = win.match_media(query)? else {
            continue;
        };
        set(&mut shared.borrow_mut().ui, list.matches());
        on(
            &list,
            "change",
            shared,
            move |s, e: web_sys::MediaQueryListEvent| {
                set(&mut s.ui, e.matches());
                s.redraw_later();
            },
        )?;
    }
    Ok(())
}

/// Adds a DOM listener that borrows the shared state; one that finds it
/// busy drops the event, which only a nested dispatch can cause.
fn on<A: App, E: JsCast + 'static>(
    target: &web_sys::EventTarget,
    name: &str,
    shared: &Rc<RefCell<Live<A>>>,
    f: impl Fn(&mut Live<A>, E) + 'static,
) -> Result<(), JsValue> {
    let s = shared.clone();
    let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |e: web_sys::Event| {
        let Ok(e) = e.dyn_into::<E>() else {
            return;
        };
        match s.try_borrow_mut() {
            Ok(mut s) => f(&mut s, e),
            Err(_) => log_error("dom event while busy"),
        }
    });
    target.add_event_listener_with_callback(name, cb.as_ref().unchecked_ref())?;
    cb.forget();
    Ok(())
}

/// Reads the images among `files` and hands each to the app as a drop,
/// once its bytes arrive.
fn read_images<A: App>(
    shared: &Rc<RefCell<Live<A>>>,
    files: web_sys::FileList,
    at: Option<(f32, f32)>,
) {
    let files: Vec<web_sys::File> = (0..files.length())
        .filter_map(|i| files.get(i))
        .filter(|f| f.type_().starts_with("image/"))
        .collect();
    for file in files {
        let s = shared.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let Ok(buf) = wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await else {
                return;
            };
            let bytes = js_sys::Uint8Array::new(&buf).to_vec();
            let what = Dropped::Image {
                name: file.name(),
                bytes,
            };
            match s.try_borrow_mut() {
                Ok(mut s) => s.dropped(what, at),
                Err(_) => log_error("dropped image while busy"),
            }
        });
    }
}

fn listen<A: App>(
    shared: &Rc<RefCell<Live<A>>>,
    canvas: &HtmlCanvasElement,
    input: &HtmlTextAreaElement,
    a11y: &HtmlElement,
    doc: &web_sys::Document,
) -> Result<(), JsValue> {
    on(input, "keydown", shared, |s, e: KeyboardEvent| s.key(&e))?;
    on(input, "input", shared, |s, e: web_sys::InputEvent| {
        let what = keys::on_input(&e.input_type(), e.data().as_deref(), e.is_composing());
        s.inserted(what);
    })?;
    on(
        input,
        "compositionupdate",
        shared,
        |s, e: CompositionEvent| {
            s.send(Input::Preedit(e.data().unwrap_or_default()));
        },
    )?;
    on(input, "compositionend", shared, |s, e: CompositionEvent| {
        s.send(Input::Preedit(String::new()));
        let text = e.data().unwrap_or_default();
        if !text.is_empty() {
            s.send(Input::Commit(text));
        }
        s.input.set_value("");
    })?;
    let images = shared.clone();
    on(doc, "paste", shared, move |s, e: ClipboardEvent| {
        e.prevent_default();
        let data = e.clipboard_data();
        let text = data
            .as_ref()
            .and_then(|d| d.get_data("text/plain").ok())
            .unwrap_or_default();
        if !text.is_empty() {
            s.paste(text);
        } else if let Some(files) = data.and_then(|d| d.files()) {
            read_images(&images, files, None);
        }
    })?;
    // A page may not see where a dropped file lives, only its bytes: images
    // reach the app as bytes, and other files are refused.
    on(canvas, "dragover", shared, |_, e: DragEvent| {
        let files = e
            .data_transfer()
            .is_some_and(|d| d.types().includes(&"Files".into(), 0));
        if files {
            e.prevent_default();
        }
    })?;
    let images = shared.clone();
    on(canvas, "drop", shared, move |_, e: DragEvent| {
        e.prevent_default();
        let at = (e.offset_x() as f32, e.offset_y() as f32);
        if let Some(files) = e.data_transfer().and_then(|d| d.files()) {
            read_images(&images, files, Some(at));
        }
    })?;
    // The app took the copy on the key; the page's event carries it out
    // where the async clipboard is not offered (plain http).
    for name in ["copy", "cut"] {
        on(doc, name, shared, |s, e: ClipboardEvent| {
            let text = s.clip.borrow().clone();
            if let (Some(text), Some(data)) = (text, e.clipboard_data()) {
                if data.set_data("text/plain", &text).is_ok() {
                    e.prevent_default();
                }
            }
        })?;
    }
    // Click, not pointerdown: a phone raises its keyboard only for a focus
    // made inside a click's handler.
    on(canvas, "click", shared, |s, _: MouseEvent| s.focus_input())?;
    on(canvas, "contextmenu", shared, |_, e: MouseEvent| {
        e.prevent_default()
    })?;
    // A screen reader's activation of a mirrored node.
    on(a11y, "click", shared, |s, e: MouseEvent| {
        let id = e
            .target()
            .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
            .and_then(|el| el.closest("[data-node]").ok().flatten())
            .and_then(|el| el.get_attribute("data-node"))
            .and_then(|id| id.parse().ok());
        if let Some(id) = id {
            let req = dom::click(id);
            s.ui.set_now(Instant::now());
            s.ui.access(&req);
            let s = &mut *s;
            s.app.access(&mut s.ui, &req);
            s.dispatch();
        }
    })?;
    Ok(())
}
