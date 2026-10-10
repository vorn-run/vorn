//! A real window: winit's event loop, a wgpu surface, IME, the system
//! clipboard and the AccessKit adapter, around one [`Ui`] and an [`App`].
//! Tests and benches never open one (they draw offscreen through the same
//! [`Ui`]); this is what an app built on vornui runs.
//!
//! Frames are paced by a [`Pacer`]: input is handled as it arrives and
//! drawn at once, and so is the first change after a keystroke (its echo
//! from a terminal); everything else waits for the display's next beat.

use std::sync::Arc;
use std::time::{Duration, Instant};

use accesskit::{ActionRequest, TreeUpdate};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::event::{
    ElementState, Modifiers, MouseButton, MouseScrollDelta, StartCause, WindowEvent,
};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::{CursorIcon, Window, WindowId};

use crate::edit::Clipboard;
use crate::element::{Cursor, El};
use crate::gpu::{instance_desc, Gpu};
use crate::input::{from_winit, mods_of, Input};
use crate::interact::Event;
use crate::pace::Pacer;
use crate::render::RenderMode;
use crate::scene::{Rect, Rgba};
use crate::ui::{Laid, Ui, UiConfig};

/// Lines a wheel notch scrolls, in logical pixels (a browser's 40).
const LINE: f32 = 40.0;

/// An app drawn by vornui.
pub trait App: 'static {
    /// The frame's element tree, built from the app's state and `ui`'s.
    fn view(&mut self, ui: &mut Ui) -> El;
    /// Paints the app's custom boxes after layout (into the window's layer,
    /// under menus and tooltips).
    fn paint(&mut self, _ui: &mut Ui, _laid: &Laid) {}
    /// Something the UI reported: a click, a choice, an edit.
    fn event(&mut self, _ui: &mut Ui, _event: Event) {}
    /// Input the UI did not use (a terminal takes what a field did not).
    fn input(&mut self, _ui: &mut Ui, _input: Input) {}
    /// A screen-reader request for a node the UI does not own.
    fn access(&mut self, _ui: &mut Ui, _req: &ActionRequest) {}
    /// Whether the last frame showed what the last keystroke was waiting
    /// for. Apps whose input echoes asynchronously answer it; others are
    /// done once the key itself is drawn.
    fn echoed(&mut self) -> bool {
        true
    }
    /// Whether the app wants IME input (a terminal does).
    fn wants_ime(&self) -> bool {
        false
    }
    /// Where the app's text cursor is (logical pixels), for the IME.
    fn ime_area(&self) -> Option<Rect> {
        None
    }
    fn background(&self) -> Rgba {
        crate::theme::color::SURFACE_BASE
    }
}

pub enum UserEvent {
    Access(accesskit_winit::Event),
    Wake,
}

impl From<accesskit_winit::Event> for UserEvent {
    fn from(e: accesskit_winit::Event) -> UserEvent {
        UserEvent::Access(e)
    }
}

/// Asks the window for a frame from any thread (a terminal's reader).
#[derive(Clone)]
pub struct Waker(EventLoopProxy<UserEvent>);

impl Waker {
    pub fn wake(&self) {
        // Only fails once the loop has exited, when no frame is wanted.
        let _ = self.0.send_event(UserEvent::Wake);
    }
}

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

/// Where the CPU renderer's frame lives between presents: surface
/// textures do not keep their contents, so damage is uploaded here and
/// the whole frame copied over on the GPU.
struct Back {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    size: (u32, u32),
}

struct Live<A> {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    ui: Ui,
    app: A,
    access: accesskit_winit::Adapter,
    tree: Option<TreeUpdate>,
    back: Option<Back>,
    pacer: Pacer,
    /// A frame the app asked for that waits for the beat.
    pending: Option<Instant>,
}

struct Runner<A, F> {
    title: String,
    size: (f32, f32),
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
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .map_err(|e| e.to_string())?;
    let mut r = Runner {
        title: title.to_owned(),
        size,
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
        let attrs = Window::default_attributes()
            .with_title(&self.title)
            .with_inner_size(LogicalSize::new(self.size.0, self.size.1))
            // AccessKit's adapter must exist before the window is first shown.
            .with_visible(false);
        let window = Arc::new(el.create_window(attrs).map_err(|e| e.to_string())?);
        let access =
            accesskit_winit::Adapter::with_event_loop_proxy(el, &window, self.proxy.clone());
        let instance = wgpu::Instance::new(instance_desc());
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| e.to_string())?;
        let gpu = Gpu::new(instance, Some(&surface))?;
        let caps = surface.get_capabilities(&gpu.adapter);
        // A non-sRGB target blends in sRGB space, as CSS colors do.
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .or(caps.formats.first().copied())
            .ok_or("surface has no formats")?;
        let mut mode = RenderMode::from_env();
        let copyable = caps.usages.contains(wgpu::TextureUsages::COPY_DST);
        if mode.uses_cpu(Some(&gpu)) && !copyable {
            mode = RenderMode::Gpu;
        }
        // Mailbox replaces a queued frame instead of waiting a beat for it.
        let present_mode = if caps.present_modes.contains(&wgpu::PresentMode::Mailbox) {
            wgpu::PresentMode::Mailbox
        } else {
            wgpu::PresentMode::Fifo
        };
        let px = window.inner_size();
        let mut config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: px.width.max(1),
            height: px.height.max(1),
            present_mode,
            desired_maximum_frame_latency: 1,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
        };
        let mut cfg = self.cfg;
        cfg.scale = window.scale_factor() as f32;
        let mut ui = Ui::new(Some(gpu), mode, format, &cfg);
        if ui.is_cpu() {
            config.usage |= wgpu::TextureUsages::COPY_DST;
        }
        if let Some(g) = &ui.gpu {
            surface.configure(&g.device, &config);
        }
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
        let app = make(&mut ui, Waker(self.proxy.clone()));
        window.set_visible(true);
        Ok(Live {
            window,
            surface,
            config,
            ui,
            app,
            access,
            tree: None,
            back: None,
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

    fn user_event(&mut self, _: &ActiveEventLoop, ev: UserEvent) {
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
                l.config.width = px.width.max(1);
                l.config.height = px.height.max(1);
                if let Some(g) = &l.ui.gpu {
                    l.surface.configure(&g.device, &l.config);
                }
                l.window.request_redraw();
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                l.ui.set_scale(*scale_factor as f32);
                l.window.request_redraw();
            }
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
            WindowEvent::MouseWheel { delta, .. } => {
                let dy = match delta {
                    MouseScrollDelta::LineDelta(_, y) => -y * LINE,
                    MouseScrollDelta::PixelDelta(p) => -(p.y as f32) / k,
                };
                l.ui.wheel(dy);
                l.dispatch();
            }
            WindowEvent::Focused(false) => {
                l.ui.pointer_leave();
                l.dispatch();
            }
            _ => {
                if let Some(input) = from_winit(&ev, mods_of(&self.mods)) {
                    l.pacer.input(now);
                    if !l.ui.key(&input) {
                        l.app.input(&mut l.ui, input);
                    }
                    l.dispatch();
                }
            }
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
        match wake {
            Some(t) if t <= now => {
                l.pending = None;
                l.window.request_redraw();
                el.set_control_flow(ControlFlow::Wait);
            }
            Some(t) => el.set_control_flow(ControlFlow::WaitUntil(t)),
            None => el.set_control_flow(ControlFlow::Wait),
        }
    }
}

impl<A: App> Live<A> {
    /// Hands the UI's events to the app and asks for the frame that shows
    /// their effect.
    fn dispatch(&mut self) {
        for e in self.ui.take_events() {
            self.app.event(&mut self.ui, e);
        }
        self.window.set_cursor(match self.ui.cursor() {
            Cursor::Default => CursorIcon::Default,
            Cursor::Pointer => CursorIcon::Pointer,
            Cursor::Text => CursorIcon::Text,
            Cursor::ColResize => CursorIcon::ColResize,
            Cursor::RowResize => CursorIcon::RowResize,
        });
        self.window.request_redraw();
    }

    fn redraw(&mut self) -> Result<(), String> {
        let Some(gpu) = &self.ui.gpu else {
            return Err("window without a device".into());
        };
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&gpu.device, &self.config);
                self.window.request_redraw();
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(())
            }
            other => return Err(format!("surface: {other:?}")),
        };
        let px = (self.config.width, self.config.height);
        if self.ui.is_cpu() && self.back.as_ref().is_none_or(|b| b.size != px) {
            let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("cpu frame"),
                size: wgpu::Extent3d {
                    width: px.0,
                    height: px.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.config.format,
                usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            self.back = Some(Back {
                texture,
                view,
                size: px,
            });
            self.ui.renderer.invalidate();
        }
        let now = Instant::now();
        let k = self.ui.scale();
        let size = (px.0 as f32 / k, px.1 as f32 / k);
        let view = frame.texture.create_view(&Default::default());
        // A full atlas empties itself and asks for the frame again; twice
        // means one frame's glyphs do not fit, and it is drawn as is.
        let mut laid;
        let mut tries = 0;
        loop {
            self.ui.begin_at(self.app.background(), now);
            let root = self.app.view(&mut self.ui);
            laid = self.ui.layout(root, size);
            self.app.paint(&mut self.ui, &laid);
            tries += 1;
            let target = match &self.back {
                Some(b) if self.ui.is_cpu() => (&b.texture, &b.view),
                _ => (&frame.texture, &view),
            };
            if self.ui.render_target(Some(target), px, true) || tries == 2 {
                break;
            }
        }
        if let (Some(b), Some(g)) = (&self.back, &self.ui.gpu) {
            if self.ui.is_cpu() {
                let mut enc = g.device.create_command_encoder(&Default::default());
                enc.copy_texture_to_texture(
                    b.texture.as_image_copy(),
                    frame.texture.as_image_copy(),
                    wgpu::Extent3d {
                        width: px.0,
                        height: px.1,
                        depth_or_array_layers: 1,
                    },
                );
                g.queue.submit([enc.finish()]);
            }
        }
        self.window.pre_present_notify();
        frame.present();
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
