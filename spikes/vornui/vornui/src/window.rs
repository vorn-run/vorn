//! A real window: winit's event loop, a wgpu surface, IME and the AccessKit
//! adapter. The benches never open one (they draw offscreen through the
//! same [`Ui`]); this is what an app built on vornui runs.

use std::sync::Arc;

use accesskit::TreeUpdate;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::event::{Modifiers, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::window::{Window, WindowId};

use crate::gpu::{instance_desc, Gpu, Rect};
use crate::input::{from_winit, mods_of, Input};
use crate::{Ui, UiConfig};

/// An app drawn by vornui.
pub trait App: 'static {
    /// Builds, lays out and paints the frame into `ui.scene`; answers the
    /// accessibility tree.
    fn frame(&mut self, ui: &mut Ui, size: (f32, f32)) -> TreeUpdate;
    fn input(&mut self, input: Input);
    /// Where the text cursor is (logical pixels), for the IME's candidates.
    fn ime_area(&self) -> Option<Rect> {
        None
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

/// Asks the window for a frame from any thread (the grid's reader).
#[derive(Clone)]
pub struct Waker(EventLoopProxy<UserEvent>);

impl Waker {
    pub fn wake(&self) {
        let _ = self.0.send_event(UserEvent::Wake);
    }
}

struct Live<A> {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    ui: Ui,
    app: A,
    access: accesskit_winit::Adapter,
    tree: Option<TreeUpdate>,
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
        window.set_ime_allowed(true);
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
        let px = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: px.width.max(1),
            height: px.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 1,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
        };
        surface.configure(&gpu.device, &config);
        let mut cfg = self.cfg;
        cfg.scale = window.scale_factor() as f32;
        let mut ui = Ui::new(gpu, format, &cfg);
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

    fn user_event(&mut self, _: &ActiveEventLoop, ev: UserEvent) {
        let Some(l) = &mut self.live else {
            return;
        };
        match ev {
            UserEvent::Wake => l.window.request_redraw(),
            UserEvent::Access(e) => match e.window_event {
                accesskit_winit::WindowEvent::InitialTreeRequested => {
                    if let Some(t) = l.tree.clone() {
                        l.access.update_if_active(|| t);
                    }
                    l.window.request_redraw();
                }
                accesskit_winit::WindowEvent::ActionRequested(_)
                | accesskit_winit::WindowEvent::AccessibilityDeactivated => {}
            },
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, ev: WindowEvent) {
        let Some(l) = &mut self.live else {
            return;
        };
        l.access.process_event(&l.window, &ev);
        match &ev {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::ModifiersChanged(m) => self.mods = *m,
            WindowEvent::Resized(px) => {
                l.config.width = px.width.max(1);
                l.config.height = px.height.max(1);
                l.surface.configure(&l.ui.gpu.device, &l.config);
                l.window.request_redraw();
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                l.ui.text.set_scale(*scale_factor as f32);
                l.window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                if let Err(e) = l.redraw() {
                    self.error = Some(e);
                    el.exit();
                }
            }
            _ => {
                if let Some(input) = from_winit(&ev, mods_of(&self.mods)) {
                    l.app.input(input);
                    if let Some(r) = l.app.ime_area() {
                        l.window.set_ime_cursor_area(
                            LogicalPosition::new(r.x, r.y),
                            LogicalSize::new(r.w, r.h),
                        );
                    }
                    l.window.request_redraw();
                }
            }
        }
    }
}

impl<A: App> Live<A> {
    fn redraw(&mut self) -> Result<(), String> {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.ui.gpu.device, &self.config);
                self.window.request_redraw();
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(())
            }
            other => return Err(format!("surface: {other:?}")),
        };
        let view = frame.texture.create_view(&Default::default());
        let k = self.ui.scale();
        let size = (self.config.width as f32 / k, self.config.height as f32 / k);
        let tree = self.app.frame(&mut self.ui, size);
        self.ui
            .render(&view, (self.config.width, self.config.height));
        self.window.pre_present_notify();
        frame.present();
        self.access.update_if_active(|| tree.clone());
        self.tree = Some(tree);
        Ok(())
    }
}
