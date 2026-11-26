//! Built-in synthetic Test Pattern NDI sources (SMPTE bars, animated gradient, motion grid).
//! Enables full verification of resizing, cropping, FPS throttling, and live UI thumbnails
//! even when the external NDI Runtime is not installed.

use crate::{VideoFrame, VideoSourceInfo};
use kf_core::{FrameRef, PixelFormat, SignalEvent};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const SYNTHETIC_SMPTE_1080P: &str = "Test Pattern: SMPTE Bars (1920x1080)";
pub const SYNTHETIC_MOTION_GRID_1080P: &str = "Test Pattern: Motion Grid (1920x1080)";
pub const SYNTHETIC_GRADIENT_720P: &str = "Test Pattern: Animated Gradient (1280x720)";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestPatternKind {
    SmpteBars,
    AnimatedGradient,
    MotionGrid,
}

impl Default for TestPatternKind {
    fn default() -> Self {
        Self::SmpteBars
    }
}

/// Returns built-in synthetic NDI sources always available for testing & live previews.
pub fn synthetic_test_sources() -> Vec<VideoSourceInfo> {
    vec![
        VideoSourceInfo {
            name: SYNTHETIC_SMPTE_1080P.into(),
            url_address: Some("synthetic://smpte-bars".into()),
            width: Some(1920),
            height: Some(1080),
            fps: Some(60.0),
        },
        VideoSourceInfo {
            name: SYNTHETIC_MOTION_GRID_1080P.into(),
            url_address: Some("synthetic://motion-grid".into()),
            width: Some(1920),
            height: Some(1080),
            fps: Some(60.0),
        },
        VideoSourceInfo {
            name: SYNTHETIC_GRADIENT_720P.into(),
            url_address: Some("synthetic://animated-gradient".into()),
            width: Some(1280),
            height: Some(720),
            fps: Some(30.0),
        },
    ]
}

/// Resolves a source name to a built-in synthetic `(TestPatternKind, width, height)` if matched.
pub fn parse_synthetic_source(source_name: Option<&str>) -> Option<(TestPatternKind, u32, u32)> {
    let name = source_name?.to_ascii_lowercase();
    if name.contains("smpte") {
        Some((TestPatternKind::SmpteBars, 1920, 1080))
    } else if name.contains("motion") || name.contains("grid") {
        Some((TestPatternKind::MotionGrid, 1920, 1080))
    } else if name.contains("gradient") {
        Some((TestPatternKind::AnimatedGradient, 1280, 720))
    } else if name.contains("test pattern") || name.starts_with("synthetic://") {
        Some((TestPatternKind::SmpteBars, 1920, 1080))
    } else {
        None
    }
}

/// Generates an animated RGBA8 synthetic video frame for `kind` at `(width, height)`.
pub fn generate_test_pattern_frame(
    source_id: Uuid,
    kind: TestPatternKind,
    width: u32,
    height: u32,
    frame_id: u64,
    phase_secs: f32,
) -> VideoFrame {
    let w = width.max(16);
    let h = height.max(16);
    let stride = (w as usize) * 4;
    let mut data = vec![0u8; stride * (h as usize)];

    match kind {
        TestPatternKind::SmpteBars => render_smpte_bars(&mut data, w, h, phase_secs),
        TestPatternKind::AnimatedGradient => render_animated_gradient(&mut data, w, h, phase_secs),
        TestPatternKind::MotionGrid => render_motion_grid(&mut data, w, h, phase_secs),
    }

    VideoFrame {
        meta: FrameRef {
            frame_id,
            width: w,
            height: h,
            format: PixelFormat::Rgba8,
            source_id,
            timestamp_ns: SignalEvent::now_ns(),
        },
        stride,
        data,
    }
}

fn render_smpte_bars(buf: &mut [u8], w: u32, h: u32, phase: f32) {
    // Classic 7-bar SMPTE top section (75% amplitude)
    const TOP_BARS: [[u8; 3]; 7] = [
        [192, 192, 192], // Gray/White
        [192, 192, 0],   // Yellow
        [0, 192, 192],   // Cyan
        [0, 192, 0],     // Green
        [192, 0, 192],   // Magenta
        [192, 0, 0],     // Red
        [0, 0, 192],     // Blue
    ];
    const MID_BARS: [[u8; 3]; 7] = [
        [0, 0, 192],
        [19, 19, 19],
        [192, 0, 192],
        [19, 19, 19],
        [0, 192, 192],
        [19, 19, 19],
        [192, 192, 192],
    ];

    let top_end = (h * 2) / 3;
    let mid_end = (h * 3) / 4;
    let sweep_x = (((phase * 0.35).fract()) * w as f32) as u32;
    let sweep_half = (w / 48).max(3);
    let border = (w.min(h) / 60).max(2);

    for y in 0..h {
        let row_offset = (y as usize) * (w as usize) * 4;
        for x in 0..w {
            let mut rgb = if y < top_end {
                let idx = ((x as usize * 7) / (w as usize)).min(6);
                TOP_BARS[idx]
            } else if y < mid_end {
                let idx = ((x as usize * 7) / (w as usize)).min(6);
                MID_BARS[idx]
            } else {
                // Bottom luma ramp + PLUGE bars
                let t = ((x as f32 / w as f32) * 255.0) as u8;
                if x < w / 6 {
                    [0, 33, 76] // -I
                } else if x < w / 3 {
                    [245, 245, 245] // 100% White
                } else if x < w / 2 {
                    [50, 0, 106] // +Q
                } else {
                    [t, t, t]
                }
            };

            // Moving vertical motion verification bar in bottom section
            if y >= mid_end && x.abs_diff(sweep_x) <= sweep_half {
                rgb = [245, 158, 11]; // Kontrolfreak amber indicator
            }

            // Crisp frame border & center crosshair so crop/letterbox/cover is unmistakable
            let is_border = x < border || x >= w - border || y < border || y >= h - border;
            let is_cross = (x.abs_diff(w / 2) <= 1 && y.abs_diff(h / 2) <= h / 10)
                || (y.abs_diff(h / 2) <= 1 && x.abs_diff(w / 2) <= w / 10);
            if is_border {
                rgb = [248, 250, 252];
            } else if is_cross {
                rgb = [16, 185, 129];
            }

            let i = row_offset + (x as usize) * 4;
            buf[i] = rgb[0];
            buf[i + 1] = rgb[1];
            buf[i + 2] = rgb[2];
            buf[i + 3] = 255;
        }
    }
}

fn render_animated_gradient(buf: &mut [u8], w: u32, h: u32, phase: f32) {
    let wf = w as f32;
    let hf = h as f32;
    let cx = wf * (0.5 + 0.28 * (phase * 1.1).cos());
    let cy = hf * (0.5 + 0.25 * (phase * 0.85).sin());
    let rad = (w.min(h) as f32 * 0.08).max(6.0);
    let rad2 = rad * rad;

    for y in 0..h {
        let ny = y as f32 / hf;
        let dy = y as f32 - cy;
        let row_offset = (y as usize) * (w as usize) * 4;
        for x in 0..w {
            let nx = x as f32 / wf;
            let r = (127.5 + 127.5 * (nx * 4.0 + phase * 1.4).sin()) as u8;
            let g = (127.5 + 127.5 * (ny * 4.0 - phase * 1.1).cos()) as u8;
            let b = (127.5 + 127.5 * ((nx + ny) * 3.0 + phase * 0.7).sin()) as u8;

            let dx = x as f32 - cx;
            let i = row_offset + (x as usize) * 4;
            if dx * dx + dy * dy <= rad2 {
                buf[i] = 255;
                buf[i + 1] = 255;
                buf[i + 2] = 255;
                buf[i + 3] = 255;
            } else {
                buf[i] = r;
                buf[i + 1] = g;
                buf[i + 2] = b;
                buf[i + 3] = 255;
            }
        }
    }
}

fn render_motion_grid(buf: &mut [u8], w: u32, h: u32, phase: f32) {
    let cell = (w.min(h) / 10).max(16);
    let bx = (w as f32 * (0.5 + 0.38 * (phase * 1.6).sin())) as i32;
    let by = (h as f32 * (0.5 + 0.35 * (phase * 1.2).cos())) as i32;
    let radius = (w.min(h) / 14).max(8) as i32;
    let r2 = radius * radius;
    let shift = ((phase * 48.0) as u32) % cell;

    for y in 0..h {
        let row_offset = (y as usize) * (w as usize) * 4;
        let dy = y as i32 - by;
        let y_line = ((y + shift) % cell) < 2;
        for x in 0..w {
            let x_line = ((x + shift) % cell) < 2;
            let dx = x as i32 - bx;
            let in_ball = dx * dx + dy * dy <= r2;
            let rgb = if in_ball {
                [139, 92, 246] // Violet target puck
            } else if x_line || y_line {
                [56, 189, 248] // Cyan grid lines
            } else if x < 3 || y < 3 || x >= w - 3 || y >= h - 3 {
                [245, 158, 11] // Amber framing border
            } else {
                [15, 23, 42] // Dark studio slate
            };
            let i = row_offset + (x as usize) * 4;
            buf[i] = rgb[0];
            buf[i + 1] = rgb[1];
            buf[i + 2] = rgb[2];
            buf[i + 3] = 255;
        }
    }
}
