//! GPU frame timing from arbitrary wgpu timestamp queries.
//!
//! Write labeled timestamps into any command encoder with
//! [`GpuProfiler::timestamp`]; [`end_gpu_frame`] resolves them and reports
//! the GPU time between consecutive timestamps.

use std::borrow::Cow;

use resident_gpu::GpuContext;
use shipyard::{AllStoragesViewMut, Unique, UniqueView, UniqueViewMut};
use w_gui::{AccentColor, Context};

use crate::{push, SECTION_ACCENTS};

/// Timestamp queries available per frame.
const CAPACITY: u32 = 64;

/// GPU timings of a single frame, from user-placed timestamp queries.
#[derive(Unique)]
pub struct GpuProfiler {
    /// Query set and readback buffer; `None` when timestamp queries are unsupported.
    queries: Option<Queries>,
    /// Labels of the timestamps written this frame, in query order.
    pending: Vec<Cow<'static, str>>,
    /// Segments of the last frame: (label, ms since the previous timestamp).
    results: Vec<(Cow<'static, str>, f32)>,
    /// GPU time between the first and last timestamp of the last frame, in ms.
    total_ms: f32,
    /// `total_ms` per frame, oldest first, at most [`HISTORY`](crate::HISTORY) long.
    total_history: Vec<f32>,
}

struct Queries {
    set: wgpu::QuerySet,
    buffer: wgpu::Buffer,
}

impl GpuProfiler {
    /// Creates the unique; disabled when the device lacks
    /// [`wgpu::Features::TIMESTAMP_QUERY`]. Runs after the renderer has added
    /// its [`GpuContext`].
    pub fn setup(storages: AllStoragesViewMut) {
        let gpu = storages
            .borrow::<UniqueView<GpuContext>>()
            .expect("GpuContext unique missing: run after the renderer setup");
        let device = gpu.device();

        let queries = if device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            Some(Queries {
                set: device.create_query_set(&wgpu::QuerySetDescriptor {
                    label: Some("profiler timestamps"),
                    ty: wgpu::QueryType::Timestamp,
                    count: CAPACITY,
                }),
                buffer: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("profiler timestamps"),
                    size: CAPACITY as u64 * wgpu::QUERY_SIZE as u64,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                }),
            })
        } else {
            log::warn!("TIMESTAMP_QUERY not supported by the device; GPU profiling disabled");
            None
        };
        drop(gpu);
        storages.add_unique(Self {
            queries,
            pending: Vec::new(),
            results: Vec::new(),
            total_ms: 0.0,
            total_history: Vec::new(),
        });
    }

    /// Whether timestamp queries are available on this device.
    pub fn enabled(&self) -> bool {
        self.queries.is_some()
    }

    /// Writes a GPU timestamp named `label` at the current point of `encoder`.
    ///
    /// Call before the encoder is submitted; the frame's timestamps are
    /// resolved by [`end_gpu_frame`]. At most [`CAPACITY`] timestamps per frame.
    pub fn timestamp(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        label: impl Into<Cow<'static, str>>,
    ) {
        let Some(queries) = &self.queries else {
            return;
        };
        let index = self.pending.len() as u32;
        if index == CAPACITY {
            log::warn!("GPU timestamp capacity ({CAPACITY}) reached; timestamp dropped");
            return;
        }
        encoder.write_timestamp(&queries.set, index);
        self.pending.push(label.into());
    }

    /// Segments of the last frame: (label, ms since the previous timestamp).
    pub fn results(&self) -> impl Iterator<Item = (&str, f32)> {
        self.results.iter().map(|(name, ms)| (name.as_ref(), *ms))
    }

    /// GPU time between the first and last timestamp of the last frame, in ms.
    pub fn total_ms(&self) -> f32 {
        self.total_ms
    }

    /// Draws the "GPU" window in the debug UI.
    pub fn ui(&self, ui: &mut Context) {
        // Nothing to show until a frame with at least two timestamps was resolved.
        if self.results.is_empty() {
            return;
        }

        let total = self.total_ms;

        let mut win = ui.window("GPU");
        win.set_accent(AccentColor::Teal);

        win.grid(2, |grid| {
            grid.stat(
                "GPU frame",
                &format!("{total:.2} ms"),
                Some(&format!("{} timestamps", self.results.len() + 1)),
                AccentColor::Teal,
            );
        });

        win.separator();
        win.plot(
            "History",
            &[("GPU ms", self.total_history.as_slice(), AccentColor::Teal)],
            Some("frame"),
            None,
        );

        win.separator();
        win.section("Frame breakdown");
        for (i, (name, ms)) in self.results.iter().enumerate() {
            let share = if total > 0.0 { *ms / total } else { 0.0 };
            win.progress_bar_with_subtitle(
                name.as_ref(),
                share as f64,
                SECTION_ACCENTS[i % SECTION_ACCENTS.len()],
                &format!("{ms:.2} ms"),
            );
        }
    }

    /// Resolves the pending timestamps and reads them back; blocks the CPU
    /// until the GPU catches up.
    fn resolve(&mut self, gpu: &GpuContext) {
        self.results.clear();
        self.total_ms = 0.0;
        let Some(queries) = &self.queries else {
            return;
        };
        // A single timestamp has no segment to report.
        let count = self.pending.len() as u32;
        if count < 2 {
            return;
        }

        let mut encoder =
            gpu.device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("profiler timestamps resolve"),
                });
        encoder.resolve_query_set(&queries.set, 0..count, &queries.buffer, 0);
        gpu.queue().submit([encoder.finish()]);

        let slice = queries
            .buffer
            .slice(..count as u64 * wgpu::QUERY_SIZE as u64);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        let polled = gpu.device().poll(wgpu::PollType::wait_indefinitely());
        let mapped = rx.recv().map(|result| result.is_ok()).unwrap_or(false);
        if polled.is_err() || !mapped {
            log::warn!("failed to read back GPU timestamps");
            return;
        }

        let mut ticks = [0u64; CAPACITY as usize];
        {
            let Ok(data) = slice.get_mapped_range() else {
                log::warn!("failed to read back GPU timestamps");
                return;
            };
            for (slot, bytes) in ticks.iter_mut().zip(data.chunks_exact(8)) {
                *slot = u64::from_le_bytes(bytes.try_into().unwrap());
            }
        }
        queries.buffer.unmap();

        // Nanoseconds per tick, per the queue's timestamp period.
        let period = gpu.queue().get_timestamp_period();
        let times = &ticks[..count as usize];
        for (i, pair) in times.windows(2).enumerate() {
            let ns = pair[1].saturating_sub(pair[0]) as f32 * period;
            self.results.push((self.pending[i + 1].clone(), ns / 1e6));
        }
        self.total_ms = times.last().unwrap().saturating_sub(times[0]) as f32 * period / 1e6;
        push(&mut self.total_history, self.total_ms);
    }
}

/// Opens a new GPU frame: clears the pending timestamp labels.
pub fn begin_gpu_frame(mut gpu: UniqueViewMut<GpuProfiler>) {
    gpu.pending.clear();
}

/// Closes the GPU frame: resolves and reads back its timestamps.
pub fn end_gpu_frame(mut profiler: UniqueViewMut<GpuProfiler>, gpu: UniqueView<GpuContext>) {
    profiler.resolve(&gpu);
}
