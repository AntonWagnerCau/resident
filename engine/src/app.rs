//! Window, event loop and frame loop.

use std::sync::Arc;

use anyhow::{Context, Result};
use log::error;
use shipyard::{Workload, World};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use crate::args::Args;
use crate::gui::{self, Gui};
use crate::profiler::{self, CPUProfiling, FrameTimings, GpuProfiler};
use crate::renderer::{RenderError, Renderer};
use resident_profiler::puffin;

struct App {
    args: Args,
    world: World,
    on_frame: Box<dyn FnMut(&World)>,
    window: Option<Arc<Window>>,
}

impl App {
    fn new(args: Args, world: World, on_frame: Box<dyn FnMut(&World)>) -> Self {
        Self {
            args,
            world,
            on_frame,
            window: None,
        }
    }

    fn init(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        let attrs = Window::default_attributes()
            .with_title(&self.args.title)
            .with_inner_size(winit::dpi::LogicalSize::new(
                self.args.width,
                self.args.height,
            ));
        let window = Arc::new(
            event_loop
                .create_window(attrs)
                .context("failed to create window")?,
        );
        self.world
            .run_with_data(Renderer::setup, (window.clone(), !self.args.no_vsync))?;
        self.world
            .run_with_data(Gui::setup, self.args.title.clone());
        self.world.run(CPUProfiling::setup);
        self.world.run(FrameTimings::setup);
        self.world.run(GpuProfiler::setup);
        Workload::new(gui::UI)
            .with_system(gui::draw::<FrameTimings>)
            .with_system(gui::draw::<GpuProfiler>)
            .with_barrier()
            .with_system(gui::end_frame)
            .add_to_world(&self.world)?;

        self.window = Some(window);
        Ok(())
    }

    /// One iteration of the frame loop: acquire, user hook, present.
    fn frame(&mut self) -> Result<()> {
        self.world.run(profiler::begin_gpu_frame);
        {
            puffin::profile_scope!("acquire");
            match self.world.run(Renderer::acquire_frame) {
                Ok(()) => {}
                Err(RenderError::Reconfigured | RenderError::Unavailable) => return Ok(()),
                Err(e) => return Err(e.into()),
            }
        }

        {
            puffin::profile_scope!("on_frame");
            (self.on_frame)(&self.world);
        }

        self.world.run(profiler::end_gpu_frame);

        {
            puffin::profile_scope!("present");
            self.world.run(Renderer::present)?;
        }

        self.world.run(profiler::sample);

        {
            puffin::profile_scope!("ui");
            self.world.run_workload(gui::UI)?;
        }
        Ok(())
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            if let Err(e) = self.init(event_loop) {
                error!("init failed: {e:#}");
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Err(e) = self
                    .world
                    .run_with_data(Renderer::resize, (size.width, size.height))
                {
                    error!("resize failed: {e}");
                }
            }
            WindowEvent::RedrawRequested => {
                if let Err(e) = self.frame() {
                    error!("frame failed: {e:#}");
                    event_loop.exit();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }
}

/// Runs the engine: opens the window and starts the frame loop.
pub fn run(args: Args, world: World, on_frame: impl FnMut(&World) + 'static) -> Result<()> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    event_loop.run_app(&mut App::new(args, world, Box::new(on_frame)))?;
    Ok(())
}
