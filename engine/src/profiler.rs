//! Frame timing: what the last frame cost, measured with puffin.

use std::borrow::Cow;
use std::time::Instant;

use puffin::{GlobalFrameView, GlobalProfiler};
use shipyard::{AllStoragesViewMut, Unique, UniqueView, UniqueViewMut};
use w_gui::{AccentColor, Context};

use crate::gui::Panel;

/// Frames kept for the history plot.
const HISTORY: usize = 240;

/// Weight of the newest sample in the smoothed frame time.
const SMOOTHING: f32 = 0.1;

/// Frames puffin retains
const RECENT_FRAMES: usize = 2;

const SECTION_ACCENTS: [AccentColor; 4] = [
    AccentColor::Coral,
    AccentColor::Teal,
    AccentColor::Blue,
    AccentColor::Purple,
];

/// Puffin's window onto the frames it has closed.
#[derive(Unique)]
pub struct CPUProfiling(GlobalFrameView);

impl CPUProfiling {
    pub(crate) fn setup(storages: AllStoragesViewMut) {
        puffin::set_scopes_on(true);
        let view = GlobalFrameView::default();
        {
            let mut frames = view.lock();
            frames.set_max_recent(RECENT_FRAMES);
            frames.set_max_slow(0);
        }
        storages.add_unique(Self(view));
    }
}

// Timings of a single frame
#[derive(Unique, Default)]
pub struct FrameTimings {
    /// When the previous frame was sampled, for the wall-clock delta.
    last: Option<Instant>,
    /// Exponentially smoothed wall-clock frame time in ms.
    smoothed_ms: f32,
    /// Duration of the last frame in ms, as measured by puffin.
    frame_ms: f32,
    /// FPS per frame, oldest first, at most [`HISTORY`] long.
    fps_history: Vec<f32>,
    /// Puffin frame duration per frame, aligned with `fps_history`.
    frame_ms_history: Vec<f32>,
    /// Top-level puffin scopes of the last frame in ms, summed per name.
    sections: Vec<(Cow<'static, str>, f32)>,
}

impl FrameTimings {
    pub(crate) fn setup(storages: AllStoragesViewMut) {
        storages.add_unique(Self::default());
    }

    /// Frames per second, smoothed over recent frames.
    pub fn fps(&self) -> f32 {
        if self.smoothed_ms > 0.0 {
            1000.0 / self.smoothed_ms
        } else {
            0.0
        }
    }

    /// Duration of the last frame in milliseconds, as measured by puffin.
    pub fn frame_time_ms(&self) -> f32 {
        self.frame_ms
    }

    /// What each section of the frame loop cost, in milliseconds.
    pub fn sections(&self) -> impl Iterator<Item = (&str, f32)> {
        self.sections.iter().map(|(name, ms)| (name.as_ref(), *ms))
    }
}

/// Closes the puffin frame and records what it cost.
pub(crate) fn sample(
    mut timings: UniqueViewMut<FrameTimings>,
    profiling: UniqueView<CPUProfiling>,
) {
    GlobalProfiler::lock().new_frame();

    let now = Instant::now();
    let Some(last) = timings.last.replace(now) else {
        return;
    };

    let wall_ms = (now - last).as_secs_f32() * 1000.0;
    timings.smoothed_ms = if timings.smoothed_ms > 0.0 {
        timings.smoothed_ms + (wall_ms - timings.smoothed_ms) * SMOOTHING
    } else {
        wall_ms
    };
    let fps = if wall_ms > 0.0 { 1000.0 / wall_ms } else { 0.0 };
    push(&mut timings.fps_history, fps);

    let frames = profiling.0.lock();
    let Some(frame) = frames.latest_frame() else {
        return;
    };

    let unpacked = match frame.unpacked() {
        Ok(unpacked) => unpacked,
        Err(_) => return,
    };

    timings.frame_ms = unpacked.duration_ns() as f32 / 1e6;
    timings.sections.clear();
    for stream in unpacked.thread_streams.values() {
        let Ok(scopes) = puffin::Reader::from_start(&stream.stream).read_top_scopes() else {
            continue;
        };
        for scope in scopes {
            let name = frames
                .scope_collection()
                .fetch_by_id(&scope.id)
                .map_or(Cow::Borrowed("unknown"), |details| details.name().clone());
            let ms = scope.record.duration_ns as f32 / 1e6;
            match timings.sections.iter_mut().find(|(n, _)| *n == name) {
                Some((_, total)) => *total += ms,
                None => timings.sections.push((name, ms)),
            }
        }
    }

    let frame_ms = timings.frame_ms;
    push(&mut timings.frame_ms_history, frame_ms);
}

impl Panel for FrameTimings {
    fn create_ui(&self, ui: &mut Context) {
        // Nothing to show until the first frame has been sampled.
        if self.fps_history.is_empty() {
            return;
        }

        let fps = self.fps();
        let low = self
            .fps_history
            .iter()
            .copied()
            .fold(f32::INFINITY, f32::min);
        let avg = self.fps_history.iter().sum::<f32>() / self.fps_history.len() as f32;
        let (smoothed_ms, frame_ms, frames) =
            (self.smoothed_ms, self.frame_ms, self.fps_history.len());

        let mut win = ui.window("Performance");
        win.set_accent(AccentColor::Purple);

        win.grid(4, |grid| {
            grid.stat(
                "FPS",
                &format!("{fps:.0}"),
                Some(&format!("{smoothed_ms:.2} ms")),
                accent_for(fps),
            );
            grid.stat(
                "Frame",
                &format!("{frame_ms:.2} ms"),
                Some("puffin"),
                AccentColor::Blue,
            );
            grid.stat(
                "Low",
                &format!("{low:.0}"),
                Some("worst frame"),
                accent_for(low),
            );
            grid.stat(
                "Avg",
                &format!("{avg:.0}"),
                Some(&format!("{frames} frames")),
                AccentColor::Teal,
            );
        });

        win.separator();
        win.plot(
            "History",
            &[
                ("FPS", self.fps_history.as_slice(), AccentColor::Green),
                (
                    "Frame ms",
                    self.frame_ms_history.as_slice(),
                    AccentColor::Blue,
                ),
            ],
            Some("frame"),
            None,
        );

        win.separator();
        win.section("Frame breakdown");
        for (i, (name, ms)) in self.sections.iter().enumerate() {
            let share = if frame_ms > 0.0 { *ms / frame_ms } else { 0.0 };
            win.progress_bar_with_subtitle(
                name.as_ref(),
                share as f64,
                SECTION_ACCENTS[i % SECTION_ACCENTS.len()],
                &format!("{ms:.2} ms"),
            );
        }
    }
}

fn push(history: &mut Vec<f32>, value: f32) {
    if history.len() == HISTORY {
        history.remove(0);
    }
    history.push(value);
}

fn accent_for(fps: f32) -> AccentColor {
    if fps >= 60.0 {
        AccentColor::Green
    } else if fps >= 30.0 {
        AccentColor::Yellow
    } else {
        AccentColor::Red
    }
}
