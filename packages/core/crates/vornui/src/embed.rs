//! A UI drawn into a layer a platform view owns, such as a CAMetalLayer, framed by its display link.

use std::ffi::c_void;

use web_time::Instant;

use crate::gpu::{instance_desc, Gpu};
use crate::host::{draw, App};
use crate::ui::{Laid, Ui, UiConfig};
use crate::RenderMode;

/// A surface on the `CAMetalLayer` `layer`.
///
/// # Safety
/// `layer` outlives the surface.
#[cfg(any(target_os = "macos", target_os = "ios"))]
unsafe fn layer_surface(
    instance: &wgpu::Instance,
    layer: *mut c_void,
) -> Result<wgpu::Surface<'static>, String> {
    // SAFETY: the caller's guarantee.
    unsafe { instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(layer)) }
        .map_err(|e| e.to_string())
}

/// Core Animation exists only on Apple's platforms.
#[cfg(not(any(target_os = "macos", target_os = "ios")))]
unsafe fn layer_surface(
    _: &wgpu::Instance,
    _: *mut c_void,
) -> Result<wgpu::Surface<'static>, String> {
    Err("there are no Core Animation layers on this platform".to_owned())
}

/// A UI and the surface of the layer it draws into.
pub struct Layer {
    pub ui: Ui,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
}

impl Layer {
    /// Draws into the Core Animation layer `layer`, `px` physical pixels at `cfg.scale`.
    ///
    /// # Safety
    /// `layer` is a `CAMetalLayer` that outlives the returned `Layer`.
    pub unsafe fn core_animation(
        layer: *mut c_void,
        px: (u32, u32),
        cfg: &UiConfig,
    ) -> Result<Layer, String> {
        let instance = wgpu::Instance::new(instance_desc());
        // SAFETY: the caller keeps the layer alive for the surface's life.
        let surface = unsafe { layer_surface(&instance, layer) }?;
        let gpu = Gpu::new(instance, Some(&surface))?;
        let (surface, config) = crate::window::configure(&gpu, surface, px, false)?;
        let ui = Ui::new(Some(gpu), RenderMode::Gpu, config.format, cfg);
        Ok(Layer {
            ui,
            surface,
            config,
        })
    }

    /// The layer's size in physical pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    /// Follows the view to `px` physical pixels at `scale`.
    pub fn resize(&mut self, px: (u32, u32), scale: f32) {
        if (scale - self.ui.scale()).abs() > f32::EPSILON {
            self.ui.set_scale(scale);
        }
        let px = (px.0.max(1), px.1.max(1));
        if px != self.size() {
            self.config.width = px.0;
            self.config.height = px.1;
            self.reconfigure();
        }
    }

    fn reconfigure(&self) {
        if let Some(gpu) = &self.ui.gpu {
            self.surface.configure(&gpu.device, &self.config);
        }
    }

    /// Builds, lays out and draws a frame of `app`; `None` when the layer had no drawable.
    pub fn frame<A: App>(&mut self, app: &mut A) -> Result<Option<Laid>, String> {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.reconfigure();
                return Ok(None);
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(None)
            }
            other => return Err(format!("surface: {other:?}")),
        };
        let view = frame.texture.create_view(&Default::default());
        let px = self.size();
        let laid = draw(&mut self.ui, app, Instant::now(), px, Some(&view));
        frame.present();
        Ok(Some(laid))
    }
}
