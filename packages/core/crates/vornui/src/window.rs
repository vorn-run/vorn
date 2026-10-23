//! A real window: winit's event loop, a wgpu surface (or, when the CPU
//! draws, a plain pixel buffer the system presents), IME, the system
//! clipboard and the AccessKit adapter, around one [`Ui`] and an [`App`].
//! Tests and benches never open one (they draw offscreen through the same
//! [`Ui`]); this is what an app built on vornui runs.
//!
//! Frames are paced by a [`Pacer`]: input is handled as it arrives and
//! drawn at once, and so is the first change after a keystroke (its echo
//! from a terminal); everything else waits for the display's next beat.

use std::num::NonZeroU32;
use std::sync::Arc;
use web_time::{Duration, Instant};

use accesskit::TreeUpdate;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::event::{
    ElementState, Modifiers, MouseButton, MouseScrollDelta, StartCause, TouchPhase, WindowEvent,
};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::{Fullscreen, Window, WindowId};

use crate::edit::Clipboard;
use crate::gpu::{adapter_name, instance_desc, Gpu};
use crate::host::{cursor_icon, draw};
pub use crate::host::{App, Dropped, UserEvent, Waker, WindowOptions};
use crate::input::{from_winit, mods_of};
use crate::interact::WindowRequest;
use crate::interact::{WheelPhase, WHEEL_LINE};
use crate::pace::Pacer;
use crate::render::RenderMode;
use crate::ui::{Ui, UiConfig};

/// How often a live resize is checked for its end, which no event reports.
#[cfg(target_os = "macos")]
const LIVE_RESIZE_POLL: Duration = Duration::from_millis(50);

/// The system clipboard; a failed read or write is an empty clipboard,
/// as in a browser without the permission.
struct System(arboard::Clipboard);

impl Clipboard for System {
    fn get(&mut self) -> Option<String> {
        self.0.get_text().ok().map(crate::edit::from_system)
    }
    fn set(&mut self, text: &str) {
        let _ = self.0.set_text(crate::edit::to_system(text, cfg!(windows)));
    }
    fn has_image(&mut self) -> bool {
        clipboard_has_image()
    }
    #[cfg(target_os = "macos")]
    fn files(&mut self) -> Vec<std::path::PathBuf> {
        crate::macos::clipboard_files()
    }
}

#[cfg(target_os = "macos")]
fn clipboard_has_image() -> bool {
    crate::macos::clipboard_has_image()
}

#[cfg(windows)]
fn clipboard_has_image() -> bool {
    use windows_sys::Win32::System::DataExchange::{
        IsClipboardFormatAvailable, RegisterClipboardFormatW,
    };
    const BITMAP: u32 = 2;
    const DIB: u32 = 8;
    const DIBV5: u32 = 17;
    let png: Vec<u16> = "PNG\0".encode_utf16().collect();
    // SAFETY: reads which formats the clipboard holds; `png` is NUL-terminated.
    unsafe {
        [BITMAP, DIB, DIBV5, RegisterClipboardFormatW(png.as_ptr())]
            .into_iter()
            .any(|f| f != 0 && IsClipboardFormatAvailable(f) != 0)
    }
}

/// Asks the window system's clipboard tool which types it holds; without
/// one, no image.
#[cfg(not(any(target_os = "macos", windows)))]
fn clipboard_has_image() -> bool {
    let mut cmd = if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        let mut c = vorn_spawn::command("wl-paste");
        c.arg("--list-types");
        c
    } else {
        let mut c = vorn_spawn::command("xclip");
        c.args(["-selection", "clipboard", "-t", "TARGETS", "-o"]);
        c
    };
    cmd.stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .is_ok_and(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .any(|t| t.trim().starts_with("image/"))
        })
}

/// How frames reach the window.
enum Present {
    Gpu {
        surface: wgpu::Surface<'static>,
        config: wgpu::SurfaceConfiguration,
    },
    /// The CPU renderer's changed rows, copied into a buffer the system
    /// presents: no device, so no software adapter's threads to wait on.
    Soft {
        surface: softbuffer::Surface<Arc<Window>, Arc<Window>>,
        size: (u32, u32),
    },
}

struct Live<A> {
    window: Arc<Window>,
    present: Present,
    ui: Ui,
    app: A,
    access: accesskit_winit::Adapter,
    tree: Option<TreeUpdate>,
    pacer: Pacer,
    /// A frame the app asked for that waits for the beat.
    pending: Option<Instant>,
    /// The OS material behind the window, as last set: `Some(dark)` while on.
    material: Option<bool>,
    /// The swapchain can show the OS material: on Windows only a composited one.
    see_through: bool,
}

struct Runner<A, F> {
    title: String,
    opts: WindowOptions,
    cfg: UiConfig,
    make: Option<F>,
    proxy: EventLoopProxy<UserEvent>,
    live: Option<Live<A>>,
    mods: Modifiers,
    error: Option<String>,
    /// Files dropped since the loop last went idle: winit sends one event
    /// per file, and the app takes them as one drop.
    dropped: Vec<std::path::PathBuf>,
}

/// Opens a window and runs `make`'s app in it until the window closes.
/// `VORNUI_RENDERER=cpu|gpu` overrides the renderer choice.
pub fn run<A, F>(title: &str, size: (f32, f32), cfg: UiConfig, make: F) -> Result<(), String>
where
    A: App,
    F: FnOnce(&mut Ui, Waker) -> A,
{
    run_with(title, WindowOptions::sized(size), cfg, make)
}

/// [`run`] with a window opened as `opts` says.
pub fn run_with<A, F>(
    title: &str,
    opts: WindowOptions,
    cfg: UiConfig,
    make: F,
) -> Result<(), String>
where
    A: App,
    F: FnOnce(&mut Ui, Waker) -> A,
{
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .map_err(|e| e.to_string())?;
    let mut r = Runner {
        title: title.to_owned(),
        opts,
        cfg,
        make: Some(make),
        proxy: event_loop.create_proxy(),
        live: None,
        mods: Modifiers::default(),
        error: None,
        dropped: Vec::new(),
    };
    event_loop.run_app(&mut r).map_err(|e| e.to_string())?;
    r.error.map_or(Ok(()), Err)
}

impl<A: App, F: FnOnce(&mut Ui, Waker) -> A> Runner<A, F> {
    fn open(&mut self, el: &ActiveEventLoop) -> Result<Live<A>, String> {
        let o = self.opts;
        let mut attrs = Window::default_attributes()
            .with_title(&self.title)
            .with_inner_size(LogicalSize::new(o.size.0, o.size.1))
            .with_decorations(o.decorations)
            .with_maximized(o.maximized)
            // AccessKit's adapter must exist before the window is first shown.
            .with_visible(false);
        if let Some((w, h)) = o.min_size {
            attrs = attrs.with_min_inner_size(LogicalSize::new(w, h));
        }
        if o.always_on_top {
            attrs = attrs.with_window_level(winit::window::WindowLevel::AlwaysOnTop);
        }
        if o.transparent {
            attrs = attrs.with_transparent(true);
        }
        #[cfg(windows)]
        if o.skip_taskbar {
            use winit::platform::windows::WindowAttributesExtWindows;
            attrs = attrs.with_skip_taskbar(true);
        }
        if let (Some((mx, my)), Some(m)) = (
            o.corner,
            el.primary_monitor()
                .or_else(|| el.available_monitors().next()),
        ) {
            let k = m.scale_factor();
            let size = m.size().to_logical::<f64>(k);
            let at = m.position().to_logical::<f64>(k);
            let x = at.x + size.width - f64::from(o.size.0 + mx);
            let y = at.y + size.height - f64::from(o.size.1 + my);
            attrs = attrs.with_position(winit::dpi::LogicalPosition::new(x, y));
        }
        #[cfg(target_os = "macos")]
        if o.inset_title.is_some() {
            use winit::platform::macos::WindowAttributesExtMacOS;
            attrs = attrs
                .with_titlebar_transparent(true)
                .with_title_hidden(true)
                .with_fullsize_content_view(true);
        }
        let window = Arc::new(el.create_window(attrs).map_err(|e| e.to_string())?);
        let access =
            accesskit_winit::Adapter::with_event_loop_proxy(el, &window, self.proxy.clone());
        // On macOS the window shows now, in the base surface colour, while the GPU and the fonts
        // load; elsewhere a window shown before its first frame flashes the system's white.
        #[cfg(target_os = "macos")]
        if !o.transparent {
            let [r, g, b, _] = crate::theme::color::SURFACE_BASE.rgba().0;
            crate::macos::set_background(&window, (f64::from(r), f64::from(g), f64::from(b)));
            window.set_visible(true);
            crate::macos::flush();
        }
        let mut cfg = self.cfg;
        cfg.scale = window.scale_factor() as f32;
        let (instance, surface, adapter, composited) = renderer(&window, o.os_material)?;
        // A composited window launching solid still needs its frame extended to show.
        #[cfg(windows)]
        if composited {
            os_material(&window, None);
        }
        let mode = RenderMode::from_env();
        let (present, mut ui) = match adapter {
            Some(a) if mode.wants_device(&a) => {
                let gpu = Gpu::open(instance, a)?;
                let px = window.inner_size();
                let (surface, config) =
                    configure(&gpu, surface, (px.width, px.height), o.transparent)?;
                let ui = Ui::new(Some(gpu), RenderMode::Gpu, config.format, &cfg);
                (Present::Gpu { surface, config }, ui)
            }
            a => {
                drop(surface);
                let ctx = softbuffer::Context::new(window.clone()).map_err(|e| e.to_string())?;
                let surface =
                    softbuffer::Surface::new(&ctx, window.clone()).map_err(|e| e.to_string())?;
                let name = a.as_ref().map(adapter_name);
                let format = wgpu::TextureFormat::Bgra8Unorm;
                let ui = Ui::build(None, name, RenderMode::Cpu, format, &cfg);
                (
                    Present::Soft {
                        surface,
                        size: (0, 0),
                    },
                    ui,
                )
            }
        };
        if let Ok(c) = arboard::Clipboard::new() {
            ui.set_clipboard(Box::new(System(c)));
        }
        let period = window
            .current_monitor()
            .and_then(|m| m.refresh_rate_millihertz())
            .filter(|&mhz| mhz > 0)
            .map_or(Duration::from_micros(16_667), |mhz| {
                Duration::from_secs_f64(1000.0 / f64::from(mhz))
            });
        let make = self.make.take().ok_or("app already made")?;
        ui.set_maximized(window.is_maximized());
        ui.set_system_dark(window.theme() != Some(winit::window::Theme::Light));
        #[cfg(target_os = "macos")]
        if let Some(at) = o.inset_title {
            crate::macos::place_traffic_lights(&window, at);
        }
        #[cfg(target_os = "macos")]
        crate::macos::take_drops(&window, Waker::new(self.proxy.clone()));
        let mut app = make(&mut ui, Waker::new(self.proxy.clone()));
        app.opened(&window);
        window.set_visible(true);
        Ok(Live {
            window,
            present,
            ui,
            app,
            access,
            tree: None,
            pacer: Pacer::new(period),
            pending: None,
            material: None,
            see_through: composited || !cfg!(windows),
        })
    }
}

impl<A: App, F: FnOnce(&mut Ui, Waker) -> A> ApplicationHandler<UserEvent> for Runner<A, F> {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.live.is_some() {
            return;
        }
        match self.open(el) {
            Ok(l) => {
                l.window.request_redraw();
                self.live = Some(l);
            }
            Err(e) => {
                self.error = Some(e);
                el.exit();
            }
        }
    }

    fn new_events(&mut self, _: &ActiveEventLoop, cause: StartCause) {
        if let (StartCause::ResumeTimeReached { .. }, Some(l)) = (cause, &mut self.live) {
            l.window.request_redraw();
        }
    }

    fn user_event(&mut self, el: &ActiveEventLoop, ev: UserEvent) {
        let Some(l) = &mut self.live else {
            return;
        };
        match ev {
            UserEvent::Wake => {
                let now = Instant::now();
                let at = l.pacer.next_frame(now);
                if at <= now {
                    l.window.request_redraw();
                } else {
                    l.pending = Some(l.pending.map_or(at, |p| p.min(at)));
                }
            }
            UserEvent::Access(e) => match e.window_event {
                accesskit_winit::WindowEvent::InitialTreeRequested => {
                    if let Some(t) = l.tree.clone() {
                        l.access.update_if_active(|| t);
                    }
                    l.window.request_redraw();
                }
                accesskit_winit::WindowEvent::ActionRequested(req) => {
                    l.ui.set_now(Instant::now());
                    l.ui.access(&req);
                    l.app.access(&mut l.ui, &req);
                    l.dispatch();
                    if l.apply_requests() {
                        el.exit();
                    }
                }
                accesskit_winit::WindowEvent::AccessibilityDeactivated => {}
            },
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, ev: WindowEvent) {
        let Some(l) = &mut self.live else {
            return;
        };
        l.access.process_event(&l.window, &ev);
        let now = Instant::now();
        l.ui.set_now(now);
        let k = l.ui.scale();
        match &ev {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::ModifiersChanged(m) => self.mods = *m,
            WindowEvent::Resized(px) => {
                l.ui.set_maximized(l.window.is_maximized());
                // AppKit lays the buttons out again on every resize.
                #[cfg(target_os = "macos")]
                if let Some(at) = self.opts.inset_title {
                    crate::macos::place_traffic_lights(&l.window, at);
                }
                if let (Present::Gpu { surface, config }, Some(g)) = (&mut l.present, &l.ui.gpu) {
                    config.width = px.width.max(1);
                    config.height = px.height.max(1);
                    surface.configure(&g.device, config);
                }
                l.window.request_redraw();
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                l.ui.set_scale(*scale_factor as f32);
                l.window.request_redraw();
            }
            // A new window is hidden until first composited; its skipped frame is drawn now.
            WindowEvent::Occluded(false) => l.window.request_redraw(),
            WindowEvent::RedrawRequested => {
                if let Err(e) = l.redraw() {
                    self.error = Some(e);
                    el.exit();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                l.ui.pointer_move(position.x as f32 / k, position.y as f32 / k);
                l.dispatch();
            }
            WindowEvent::DroppedFile(path) => self.dropped.push(path.clone()),
            WindowEvent::CursorLeft { .. } => {
                l.ui.pointer_leave();
                l.dispatch();
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                match state {
                    ElementState::Pressed => {
                        l.app.pressed();
                        l.ui.pointer_down();
                    }
                    ElementState::Released => l.ui.pointer_up(),
                }
                l.dispatch();
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Right,
                ..
            } => {
                l.app.pressed();
                l.ui.pointer_down_secondary();
                l.dispatch();
            }
            WindowEvent::MouseWheel { delta, phase, .. } => {
                let (dy, phase) = match delta {
                    MouseScrollDelta::LineDelta(_, y) => (-y * WHEEL_LINE, WheelPhase::Line),
                    MouseScrollDelta::PixelDelta(p) => (
                        -(p.y as f32) / k,
                        match phase {
                            TouchPhase::Started => WheelPhase::Began,
                            TouchPhase::Moved => WheelPhase::Moved,
                            TouchPhase::Ended | TouchPhase::Cancelled => WheelPhase::Ended,
                        },
                    ),
                };
                l.ui.wheel_phase(dy, phase);
                l.dispatch();
            }
            WindowEvent::ThemeChanged(theme) => {
                l.ui.set_system_dark(*theme != winit::window::Theme::Light);
                l.window.request_redraw();
            }
            WindowEvent::Focused(focused) => {
                // Reduce transparency sends no event; a window coming forward looks again.
                if *focused {
                    l.ui.set_reduced_transparency(crate::palette::system_reduced_transparency());
                }
                l.ui.set_window_focused(*focused);
                if !*focused {
                    l.ui.pointer_leave();
                }
                l.dispatch();
            }
            _ => {
                if let Some(input) = from_winit(&ev, mods_of(&self.mods)) {
                    l.pacer.input(now);
                    if !l.app.intercept(&mut l.ui, &input) && !l.ui.key(&input) {
                        l.app.input(&mut l.ui, input);
                    }
                    l.dispatch();
                }
            }
        }
        if l.apply_requests() {
            el.exit();
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        let Some(l) = &mut self.live else {
            return;
        };
        let mut drops = Vec::new();
        if !self.dropped.is_empty() {
            let at = cursor_in(&l.window);
            drops.push((Dropped::Files(std::mem::take(&mut self.dropped)), at));
        }
        #[cfg(target_os = "macos")]
        drops.extend(crate::macos::drops());
        if !drops.is_empty() {
            l.ui.set_now(Instant::now());
            for (what, at) in drops {
                l.app.dropped(&mut l.ui, what, at);
            }
            l.dispatch();
            if l.apply_requests() {
                el.exit();
            }
        }
        let now = Instant::now();
        let ui_wake = l.ui.next_wake().map(|t| t.max(l.pacer.next_frame(now)));
        let wake = match (l.pending, ui_wake) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        // A live resize ends with no event of its own: look again shortly,
        // and draw once more when it has ended, for what waited on it.
        #[cfg(target_os = "macos")]
        let wake = match l.ui.live_resize() {
            true if crate::macos::in_live_resize(&l.window) => {
                let again = now + LIVE_RESIZE_POLL;
                Some(wake.map_or(again, |w| w.min(again)))
            }
            true => {
                l.ui.set_live_resize(false);
                Some(now)
            }
            false => wake,
        };
        let tick = l.app.tick();
        match wake {
            Some(t) if t <= now => {
                l.pending = None;
                l.window.request_redraw();
                el.set_control_flow(tick.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
            }
            Some(t) => el.set_control_flow(ControlFlow::WaitUntil(tick.map_or(t, |k| k.min(t)))),
            None => el.set_control_flow(tick.map_or(ControlFlow::Wait, ControlFlow::WaitUntil)),
        }
    }
}

/// Where the pointer is in the window, in logical pixels, when a drop does
/// not say: Windows asks the system; elsewhere it is unknown (the pointer's
/// last move may predate the drag), and the app picks its own target.
#[cfg(windows)]
fn cursor_in(window: &Window) -> Option<(f32, f32)> {
    use windows_sys::Win32::Foundation::{HWND, POINT};
    use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let RawWindowHandle::Win32(h) = window.window_handle().ok()?.as_raw() else {
        return None;
    };
    let mut p = POINT { x: 0, y: 0 };
    // SAFETY: the handle is winit's live window; `p` is ours to write.
    let ok =
        unsafe { GetCursorPos(&mut p) != 0 && ScreenToClient(h.hwnd.get() as HWND, &mut p) != 0 };
    let k = window.scale_factor() as f32;
    ok.then(|| (p.x as f32 / k, p.y as f32 / k))
}

#[cfg(not(windows))]
fn cursor_in(_: &Window) -> Option<(f32, f32)> {
    None
}

type Renderer = (
    wgpu::Instance,
    wgpu::Surface<'static>,
    Option<wgpu::Adapter>,
    bool,
);

/// The instance, surface and adapter `window` draws with, and whether they
/// are composited: on Windows with `material`, DirectComposition when that
/// finds a hardware adapter; otherwise the default.
fn renderer(window: &Arc<Window>, material: bool) -> Result<Renderer, String> {
    let open = |desc| -> Result<Renderer, String> {
        let instance = wgpu::Instance::new(desc);
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| e.to_string())?;
        let adapter = Gpu::adapter(&instance, Some(&surface)).ok();
        Ok((instance, surface, adapter, false))
    };
    #[cfg(windows)]
    if material {
        use crate::gpu::{composited_desc, swapchain, Swapchain};
        // A failed DX12 instance or surface falls through to the default too.
        let tried = open(composited_desc()).ok();
        let kind = tried
            .as_ref()
            .and_then(|r| r.2.as_ref())
            .map(|a| a.get_info().device_type);
        if let (Swapchain::Composited, Some((instance, surface, adapter, _))) =
            (swapchain(material, kind), tried)
        {
            return Ok((instance, surface, adapter, true));
        }
    }
    #[cfg(not(windows))]
    let _ = material;
    open(instance_desc())
}

/// Shows the system's material behind the window in the dark or light
/// appearance, or takes it away (`None`); `false` when the system has none.
#[cfg(target_os = "macos")]
fn os_material(window: &Window, dark: Option<bool>) -> bool {
    let [r, g, b, _] = crate::theme::color::SURFACE_BASE.rgba().0;
    let solid = (f64::from(r), f64::from(g), f64::from(b));
    crate::macos::set_material(window, dark, solid)
}

/// Mica, which Windows 11 has and Windows 10 refuses; only for a composited window.
#[cfg(windows)]
fn os_material(window: &Window, dark: Option<bool>) -> bool {
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::Graphics::Dwm::{
        DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMSBT_MAINWINDOW, DWMSBT_NONE,
        DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE,
    };
    use windows_sys::Win32::UI::Controls::MARGINS;
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Some(RawWindowHandle::Win32(h)) = window.window_handle().ok().map(|h| h.as_raw()) else {
        return false;
    };
    let hwnd = h.hwnd.get() as HWND;
    let kind = if dark.is_some() {
        DWMSBT_MAINWINDOW
    } else {
        DWMSBT_NONE
    };
    // Always extended: otherwise the window's GDI surface covers the composited frame.
    let edge = -1;
    let margins = MARGINS {
        cxLeftWidth: edge,
        cxRightWidth: edge,
        cyTopHeight: edge,
        cyBottomHeight: edge,
    };
    let dark_mode = i32::from(dark.unwrap_or(true));
    let size = |n: usize| n as u32;
    // SAFETY: winit's live window; each value outlives its call.
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
            (&raw const dark_mode).cast(),
            size(std::mem::size_of::<i32>()),
        );
        let ok = DwmSetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE as u32,
            (&raw const kind).cast(),
            size(std::mem::size_of_val(&kind)),
        ) >= 0;
        DwmExtendFrameIntoClientArea(hwnd, &margins);
        ok
    }
}

/// Linux and Windows 10 stay solid.
#[cfg(not(any(target_os = "macos", windows)))]
fn os_material(_: &Window, _: Option<bool>) -> bool {
    false
}

impl<A: App> Live<A> {
    /// Does what the app asked of the window; true when it asked to close.
    fn apply_requests(&mut self) -> bool {
        let mut close = false;
        for r in self.ui.take_requests() {
            match r {
                WindowRequest::Minimize => self.window.set_minimized(true),
                WindowRequest::ToggleMaximize => {
                    self.window.set_maximized(!self.window.is_maximized());
                }
                WindowRequest::Close => close = true,
                // Fails only when the button is no longer down.
                WindowRequest::Drag => {
                    let _ = self.window.drag_window();
                }
                WindowRequest::ToggleFullscreen => {
                    let full = self.window.fullscreen().is_some();
                    self.window
                        .set_fullscreen((!full).then_some(Fullscreen::Borderless(None)));
                }
                WindowRequest::Focus => {
                    self.window.set_visible(true);
                    self.window.set_minimized(false);
                    self.window.focus_window();
                }
                WindowRequest::Resize(w, h) => {
                    let k = self.window.scale_factor();
                    let old = self.window.outer_size().to_logical::<f64>(k);
                    let at = self
                        .window
                        .outer_position()
                        .ok()
                        .map(|p| p.to_logical::<f64>(k));
                    let _ = self
                        .window
                        .request_inner_size(LogicalSize::new(f64::from(w), f64::from(h)));
                    if let Some(at) = at {
                        let x = at.x + old.width - f64::from(w);
                        let y = at.y + old.height - f64::from(h);
                        self.window
                            .set_outer_position(winit::dpi::LogicalPosition::new(
                                x.max(0.0),
                                y.max(0.0),
                            ));
                    }
                }
            }
        }
        close
    }

    /// Hands the UI's events to the app and asks for the frame that shows
    /// their effect.
    fn dispatch(&mut self) {
        for e in self.ui.take_events() {
            self.app.event(&mut self.ui, e);
        }
        self.window.set_cursor(cursor_icon(self.ui.cursor()));
        self.window.request_redraw();
    }

    /// Puts the OS material behind the window when the palette asks for one
    /// and the compositor blends the frame's alpha, or takes it away.
    fn sync_material(&mut self) {
        let Present::Gpu { surface, config } = &mut self.present else {
            return;
        };
        let Some(gpu) = &self.ui.gpu else {
            return;
        };
        let want = (self.see_through
            && crate::palette::materials() != vorn_theme::Materials::Solid)
            .then(crate::palette::dark);
        if want == self.material {
            return;
        }
        let caps = surface.get_capabilities(&gpu.adapter);
        let blend = [
            wgpu::CompositeAlphaMode::PreMultiplied,
            wgpu::CompositeAlphaMode::PostMultiplied,
        ]
        .into_iter()
        .find(|m| caps.alpha_modes.contains(m));
        let on = match (want, blend) {
            (Some(dark), Some(mode)) if os_material(&self.window, Some(dark)) => {
                config.alpha_mode = mode;
                true
            }
            _ => {
                if self.material.is_some() {
                    os_material(&self.window, None);
                }
                config.alpha_mode = opaque(&caps);
                false
            }
        };
        surface.configure(&gpu.device, config);
        // Asked for and unavailable is remembered too, so it is tried once.
        self.material = want;
        self.ui.set_see_through(on);
    }

    fn redraw(&mut self) -> Result<(), String> {
        let now = Instant::now();
        self.sync_material();
        #[cfg(target_os = "macos")]
        self.ui
            .set_live_resize(crate::macos::in_live_resize(&self.window));
        let laid = match &mut self.present {
            Present::Gpu { surface, config } => {
                let Some(gpu) = &self.ui.gpu else {
                    return Err("window without a device".into());
                };
                let frame = match surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(t)
                    | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
                    wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                        surface.configure(&gpu.device, config);
                        self.window.request_redraw();
                        return Ok(());
                    }
                    wgpu::CurrentSurfaceTexture::Timeout
                    | wgpu::CurrentSurfaceTexture::Occluded => return Ok(()),
                    other => return Err(format!("surface: {other:?}")),
                };
                let px = (config.width, config.height);
                let view = frame.texture.create_view(&Default::default());
                let laid = draw(&mut self.ui, &mut self.app, now, px, Some(&view));
                #[cfg(target_os = "macos")]
                crate::macos::sync_glass(
                    &self.window,
                    if self.ui.see_through() {
                        &self.ui.glass
                    } else {
                        &[]
                    },
                    self.window.scale_factor(),
                );
                self.window.pre_present_notify();
                frame.present();
                laid
            }
            Present::Soft { surface, size } => {
                let px = self.window.inner_size();
                let (Some(w), Some(h)) = (NonZeroU32::new(px.width), NonZeroU32::new(px.height))
                else {
                    return Ok(());
                };
                if *size != (w.get(), h.get()) {
                    surface.resize(w, h).map_err(|e| e.to_string())?;
                    *size = (w.get(), h.get());
                }
                let laid = draw(&mut self.ui, &mut self.app, now, *size, None);
                let mut buf = surface.buffer_mut().map_err(|e| e.to_string())?;
                // Age 1: the buffer still holds the last frame shown.
                let whole = buf.age() != 1;
                let damage = self
                    .ui
                    .copy_cpu_frame(&mut buf, whole)
                    .ok_or("cpu frame does not fit the window")?;
                let mut rects: Vec<softbuffer::Rect> = damage
                    .iter()
                    .filter_map(|&(y0, y1)| {
                        Some(softbuffer::Rect {
                            x: 0,
                            y: y0,
                            width: w,
                            height: NonZeroU32::new(y1 - y0)?,
                        })
                    })
                    .collect();
                // No change means a system repaint, which wants every pixel.
                if whole || rects.is_empty() {
                    rects = vec![softbuffer::Rect {
                        x: 0,
                        y: 0,
                        width: w,
                        height: h,
                    }];
                }
                self.window.pre_present_notify();
                buf.present_with_damage(&rects).map_err(|e| e.to_string())?;
                laid
            }
        };
        let echoed = self.app.echoed();
        self.pacer.frame(now, echoed);
        let ime = self.ui.wants_ime() || self.app.wants_ime();
        self.window.set_ime_allowed(ime);
        if let Some(r) = self.ui.ime_area().or_else(|| self.app.ime_area()) {
            self.window
                .set_ime_cursor_area(LogicalPosition::new(r.x, r.y), LogicalSize::new(r.w, r.h));
        }
        let tree = laid.tree;
        self.access.update_if_active(|| tree.clone());
        self.tree = Some(tree);
        Ok(())
    }
}

/// Opaque where offered: a DirectComposition swapchain lists `Auto` first,
/// which leaves its alpha unspecified.
fn opaque(caps: &wgpu::SurfaceCapabilities) -> wgpu::CompositeAlphaMode {
    let o = wgpu::CompositeAlphaMode::Opaque;
    if caps.alpha_modes.contains(&o) {
        o
    } else {
        caps.alpha_modes[0]
    }
}

/// Picks the surface's format and mode for `gpu` and configures it at `px`.
pub(crate) fn configure(
    gpu: &Gpu,
    surface: wgpu::Surface<'static>,
    px: (u32, u32),
    transparent: bool,
) -> Result<(wgpu::Surface<'static>, wgpu::SurfaceConfiguration), String> {
    let caps = surface.get_capabilities(&gpu.adapter);
    // A non-sRGB target blends in sRGB space, as CSS colors do.
    let format = caps
        .formats
        .iter()
        .copied()
        .find(|f| !f.is_srgb())
        .or(caps.formats.first().copied())
        .ok_or("surface has no formats")?;
    // Mailbox replaces a queued frame instead of waiting a beat for it.
    let present_mode = if caps.present_modes.contains(&wgpu::PresentMode::Mailbox) {
        wgpu::PresentMode::Mailbox
    } else {
        wgpu::PresentMode::Fifo
    };
    // See-through only where the compositor blends the frame's alpha.
    let alpha_mode = [
        wgpu::CompositeAlphaMode::PreMultiplied,
        wgpu::CompositeAlphaMode::PostMultiplied,
    ]
    .into_iter()
    .find(|m| transparent && caps.alpha_modes.contains(m))
    .unwrap_or_else(|| opaque(&caps));
    // Copying from the frame is what backdrop blur reads; without it blurs
    // are skipped.
    let usage =
        wgpu::TextureUsages::RENDER_ATTACHMENT | (caps.usages & wgpu::TextureUsages::COPY_SRC);
    let config = wgpu::SurfaceConfiguration {
        usage,
        format,
        width: px.0.max(1),
        height: px.1.max(1),
        present_mode,
        desired_maximum_frame_latency: 1,
        alpha_mode,
        view_formats: vec![],
    };
    surface.configure(&gpu.device, &config);
    Ok((surface, config))
}
