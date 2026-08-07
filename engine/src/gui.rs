//! The debug UI: one w-gui context, drawn from the [`Panel`]s of the [`UI`] workload.

use shipyard::{AllStoragesViewMut, Unique, UniqueView, UniqueViewMut};
use w_gui::{Context, ContextOptions};

pub const UI: &str = "ui";

#[derive(Unique)]
pub struct Gui(Context);

impl Gui {
    pub(crate) fn setup(title: String, storages: AllStoragesViewMut) {
        storages.add_unique(Self(Context::with_options(ContextOptions {
            title,
            ..Default::default()
        })));
    }
}

/// Data that can show itself in the debug UI.
pub trait Panel {
    fn create_ui(&self, ui: &mut Context);
}

impl Panel for resident_profiler::FrameTimings {
    fn create_ui(&self, ui: &mut Context) {
        self.ui(ui);
    }
}

impl Panel for resident_profiler::GpuProfiler {
    fn create_ui(&self, ui: &mut Context) {
        self.ui(ui);
    }
}

/// Draws the panel held by the unique `P`.
pub fn draw<P: Panel + Unique + Send + Sync>(mut gui: UniqueViewMut<Gui>, panel: UniqueView<P>) {
    panel.create_ui(&mut gui.0);
}

/// Sends the frame's declarations to the browser.
pub(crate) fn end_frame(mut gui: UniqueViewMut<Gui>) {
    gui.0.end_frame();
}
