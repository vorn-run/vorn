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
pub use crate::host::{App, UserEvent, Waker, WindowOptions};
use crate::input::{from_winit, mods_of};
use crate::interact::WindowRequest;
use crate::interact::{WheelPhase, WHEEL_LINE};
use crate::pace::Pacer;
use crate::render::RenderMode;
use crate::ui::{Ui, UiConfig};

/// The system clipboard; a failed read or write is an empty clipboard,
/// as in a browser without the permission.
struct System(arboard::Clipboard);

impl Clipboard for System {
    fn get(&mut self) -> Option<String> {
        self.0.get_text().ok()
    }
    fn set(&mut self, text: &str) {
        let _ = self.0.set_text(text);
    }
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
        let mut cfg = self.cfg;
        cfg.scale = window.scale_factor() as f32;
        let instance = wgpu::Instance::new(instance_desc());
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| e.to_string())?;
        let mode = RenderMode::from_env();
        let adapter = Gpu::adapter(&instance, Some(&surface)).ok();
        let (present, mut ui) = match adapter {
            Some(a) if mode.wants_device(&a) => {
                let gpu = Gpu::open(instance, a)?;
                let (surface, config) = configure(&gpu, surface, &window, o.transparent)?;
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
        #[cfg(target_os = "macos")]
        if let Some(at) = o.inset_title {
            crate::macos::place_traffic_lights(&window, at);
        }
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
                    ElementState::Pressed => l.ui.pointer_down(),
                    ElementState::Released => l.ui.pointer_up(),
                }
                l.dispatch();
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Right,
                ..
            } => {
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
            WindowEvent::Focused(focused) => {
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
        let now = Instant::now();
        let ui_wake = l.ui.next_wake().map(|t| t.max(l.pacer.next_frame(now)));
        let wake = match (l.pending, ui_wake) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
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

    fn redraw(&mut self) -> Result<(), String> {
        let now = Instant::now();
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

/// Picks the surface's format and mode for `gpu` and configures it.
fn configure(
    gpu: &Gpu,
    surface: wgpu::Surface<'static>,
    window: &Window,
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
    let px = window.inner_size();
    // See-through only where the compositor blends the frame's alpha.
    let alpha_mode = [
        wgpu::CompositeAlphaMode::PreMultiplied,
        wgpu::CompositeAlphaMode::PostMultiplied,
    ]
    .into_iter()
    .find(|m| transparent && caps.alpha_modes.contains(m))
    .unwrap_or(caps.alpha_modes[0]);
    // Copying from the frame is what backdrop blur reads; without it blurs
    // are skipped.
    let usage =
        wgpu::TextureUsages::RENDER_ATTACHMENT | (caps.usages & wgpu::TextureUsages::COPY_SRC);
    let config = wgpu::SurfaceConfiguration {
        usage,
        format,
        width: px.width.max(1),
        height: px.height.max(1),
        present_mode,
        desired_maximum_frame_latency: 1,
        alpha_mode,
        view_formats: vec![],
    };
    surface.configure(&gpu.device, &config);
    Ok((surface, config))
}
