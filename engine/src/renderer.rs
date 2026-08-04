//! wgpu device, surface and frame presentation.
//!
use std::sync::Arc;

use anyhow::{Context, Result};
use shipyard::{AllStoragesViewMut, Unique, UniqueView, UniqueViewMut};
use winit::window::Window;

/// Errors from the frame-loop operations on [`Renderer`].
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("storage borrow failed: {0}")]
    Storage(#[from] shipyard::error::GetStorage),

    #[error("no frame to present: Frame unique missing from the world")]
    MissingFrame,

    #[error("surface lost or outdated; reconfigured")]
    Reconfigured,

    #[error("no surface texture available (timeout or occluded)")]
    Unavailable,

    #[error("surface validation error")]
    Validation,
}

/// GPU context: device, queue, surface and its configuration.
#[derive(Unique)]
pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
}

impl Renderer {
    pub fn surface(&self) -> &wgpu::Surface<'static> {
        &self.surface
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    pub fn config(&self) -> &wgpu::SurfaceConfiguration {
        &self.config
    }

    pub(crate) fn setup(
        (window, vsync): (Arc<Window>, bool),
        storages: AllStoragesViewMut,
    ) -> Result<()> {
        let size = window.inner_size();

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let surface = instance.create_surface(window)?;

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .context("no suitable GPU adapter found")?;

        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .context("failed to create device")?;

        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .context("surface is not supported by the selected adapter")?;
        if !vsync {
            config.present_mode = wgpu::PresentMode::AutoNoVsync;
        }
        surface.configure(&device, &config);

        storages.add_unique(Self {
            surface,
            device,
            queue,
            config,
        });

        Ok(())
    }

    pub(crate) fn resize(
        (width, height): (u32, u32),
        storages: AllStoragesViewMut,
    ) -> std::result::Result<(), RenderError> {
        if width > 0 && height > 0 {
            let mut renderer = storages.borrow::<UniqueViewMut<Renderer>>()?;
            renderer.config.width = width;
            renderer.config.height = height;
            renderer
                .surface
                .configure(&renderer.device, &renderer.config);
        }
        Ok(())
    }

    /// Acquires the next frame and inserts it into the storages as a [`Frame`] unique.
    pub(crate) fn acquire_frame(
        storages: AllStoragesViewMut,
    ) -> std::result::Result<(), RenderError> {
        use wgpu::CurrentSurfaceTexture as Cst;
        let renderer = storages.borrow::<UniqueView<Renderer>>()?;
        let texture = match renderer.surface.get_current_texture() {
            Cst::Success(frame) => frame,
            // Acquired, but no longer matches the surface: reconfigure, still usable.
            Cst::Suboptimal(frame) => {
                renderer
                    .surface
                    .configure(&renderer.device, &renderer.config);
                frame
            }
            Cst::Lost | Cst::Outdated => {
                renderer
                    .surface
                    .configure(&renderer.device, &renderer.config);
                return Err(RenderError::Reconfigured);
            }
            Cst::Timeout | Cst::Occluded => return Err(RenderError::Unavailable),
            Cst::Validation => return Err(RenderError::Validation),
        };

        let view = texture.texture.create_view(&Default::default());
        storages.add_unique(Frame { texture, view });
        Ok(())
    }

    /// Takes the [`Frame`] unique out of the storages and presents it.
    pub(crate) fn present(storages: AllStoragesViewMut) -> std::result::Result<(), RenderError> {
        let frame = storages
            .remove_unique::<Frame>()
            .map_err(|_| RenderError::MissingFrame)?;
        let renderer = storages.borrow::<UniqueView<Renderer>>()?;
        renderer.queue.present(frame.texture);
        Ok(())
    }
}

/// A frame acquired from the surface, the render target for one frame.
#[derive(Unique)]
pub struct Frame {
    texture: wgpu::SurfaceTexture,
    view: wgpu::TextureView,
}

impl Frame {
    /// Render target view for this frame.
    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// The frame's underlying texture (format, dimensions).
    pub fn texture(&self) -> &wgpu::Texture {
        &self.texture.texture
    }
}
