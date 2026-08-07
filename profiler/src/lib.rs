//! Frame profiling: puffin CPU scopes and arbitrary GPU timestamps.

mod cpu;
mod gpu;

pub use cpu::{sample, CPUProfiling, FrameTimings};
pub use gpu::{begin_gpu_frame, end_gpu_frame, GpuProfiler};

pub use resident_gpu::GpuContext;

pub use puffin;

/// Frames kept for the history plots.
pub(crate) const HISTORY: usize = 240;

/// Accent per breakdown row, cycling.
pub(crate) const SECTION_ACCENTS: [w_gui::AccentColor; 4] = [
    w_gui::AccentColor::Coral,
    w_gui::AccentColor::Teal,
    w_gui::AccentColor::Blue,
    w_gui::AccentColor::Purple,
];

pub(crate) fn push(history: &mut Vec<f32>, value: f32) {
    if history.len() == HISTORY {
        history.remove(0);
    }
    history.push(value);
}
