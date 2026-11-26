//! Lightweight pure-Rust CPU Computer Vision extractors for RGB camera frames.
//!
//! Emits normalized hackable `kf-core` parameters:
//! - Global & ROI luma/brightness
//! - Frame-difference motion energy & motion centroid
//! - Dominant RGB & HSV color inside ROI
//! - Brightest-point tracking & target-HSV color blob centroid (`x`, `y`, `area`)
//! - Multi-zone motion trigger grid with per-zone energy & thresholded triggers

use crate::RgbCameraExtractor;
use kf_core::{
    CompiledModifierChain, Modifier, ModifierChainState, SignalEvent, SignalValue,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Normalized Region of Interest rectangle (`0.0..=1.0`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct NormalizedRoi {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Default for NormalizedRoi {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
        }
    }
}

impl NormalizedRoi {
    pub fn pixel_bounds(&self, width: u32, height: u32) -> (u32, u32, u32, u32) {
        let x0 = ((self.x.clamp(0.0, 1.0)) * width as f32).floor() as u32;
        let y0 = ((self.y.clamp(0.0, 1.0)) * height as f32).floor() as u32;
        let x1 = (((self.x + self.w).clamp(0.0, 1.0)) * width as f32)
            .ceil()
            .max((x0 + 1) as f32) as u32;
        let y1 = (((self.y + self.h).clamp(0.0, 1.0)) * height as f32)
            .ceil()
            .max((y0 + 1) as f32) as u32;
        (x0.min(width.saturating_sub(1)), y0.min(height.saturating_sub(1)), x1.min(width), y1.min(height))
    }
}

/// Configuration for the unified RGB CV extraction pipeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RgbCvConfig {
    pub roi: NormalizedRoi,
    /// Target color hue in degrees `0.0..=360.0` (e.g. `158.0` for emerald-cyan).
    pub target_hue_deg: f32,
    /// Half-width hue window in degrees (e.g. `32.0`).
    pub hue_tolerance_deg: f32,
    /// Minimum saturation `0.0..=1.0` for target-color blob mask.
    pub min_saturation: f32,
    /// Minimum value/brightness `0.0..=1.0` for target-color blob mask.
    pub min_value: f32,
    /// Per-pixel luma delta threshold `0.0..=1.0` to count as motion.
    pub motion_pixel_threshold: f32,
    /// Motion zone grid dimensions (columns x rows).
    pub zone_cols: u32,
    pub zone_rows: u32,
    /// Normalized zone motion energy threshold to trigger a zone.
    pub zone_trigger_threshold: f32,
    /// Optional per-parameter modifier stacks keyed by parameter logical key.
    #[serde(default)]
    pub parameter_modifiers: HashMap<String, Vec<Modifier>>,
}

impl Default for RgbCvConfig {
    fn default() -> Self {
        Self {
            roi: NormalizedRoi::default(),
            target_hue_deg: 158.0,
            hue_tolerance_deg: 35.0,
            min_saturation: 0.35,
            min_value: 0.25,
            motion_pixel_threshold: 0.08,
            zone_cols: 2,
            zone_rows: 2,
            zone_trigger_threshold: 0.05,
            parameter_modifiers: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MotionZoneReading {
    pub index: usize,
    pub col: u32,
    pub row: u32,
    pub energy: f64,
    pub triggered: bool,
}

/// Structured telemetry summary produced by `RgbCvPipeline::process_frame`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RgbCvResult {
    pub global_luma: f64,
    pub roi_luma: f64,
    pub motion_energy: f64,
    pub motion_centroid: [f64; 2],
    pub dominant_rgb: [f64; 3],
    pub dominant_hsv: [f64; 3],
    pub brightest_point: [f64; 2],
    pub brightest_luma: f64,
    pub blob_centroid: [f64; 2],
    pub blob_area: f64,
    pub zones: Vec<MotionZoneReading>,
}

/// Converts normalized RGB (`0.0..=1.0`) to HSV (`hue_deg: 0..360`, `sat: 0..1`, `val: 0..1`).
pub fn rgb_to_hsv(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;

    let h = if delta < 1e-6 {
        0.0
    } else if (max - r).abs() < 1e-6 {
        60.0 * (((g - b) / delta).rem_euclid(6.0))
    } else if (max - g).abs() < 1e-6 {
        60.0 * (((b - r) / delta) + 2.0)
    } else {
        60.0 * (((r - g) / delta) + 4.0)
    };
    let s = if max < 1e-6 { 0.0 } else { delta / max };
    (h, s, max)
}

#[inline]
fn rec709_luma(r: f32, g: f32, b: f32) -> f32 {
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

#[inline]
fn circular_hue_dist_deg(a: f32, b: f32) -> f32 {
    let diff = (a - b).abs() % 360.0;
    diff.min(360.0 - diff)
}

/// Stateful CPU CV pipeline maintaining previous-frame luma buffer and per-parameter modifier chains.
pub struct RgbCvPipeline {
    config: RgbCvConfig,
    prev_luma: Vec<f32>,
    prev_dims: (u32, u32),
    compiled_mods: HashMap<String, (CompiledModifierChain, ModifierChainState)>,
}

impl RgbCvPipeline {
    pub fn new(config: RgbCvConfig) -> Self {
        let mut pipe = Self {
            config: RgbCvConfig::default(),
            prev_luma: Vec::new(),
            prev_dims: (0, 0),
            compiled_mods: HashMap::new(),
        };
        pipe.set_config(config);
        pipe
    }

    pub fn config(&self) -> &RgbCvConfig {
        &self.config
    }

    pub fn set_config(&mut self, config: RgbCvConfig) {
        self.compiled_mods.clear();
        for (key, mods) in &config.parameter_modifiers {
            if let Ok(chain) = CompiledModifierChain::compile(mods) {
                self.compiled_mods
                    .insert(key.clone(), (chain, ModifierChainState::default()));
            }
        }
        self.config = config;
    }

    pub fn reset_temporal(&mut self) {
        self.prev_luma.clear();
        self.prev_dims = (0, 0);
    }

    /// Processes one RGBA8 frame and returns both the structured `RgbCvResult` and the flat list
    /// of normalized `(logical_key, SignalValue)` signals (with modifier chains applied).
    pub fn process_frame(
        &mut self,
        rgba: &[u8],
        width: u32,
        height: u32,
        timestamp_ns: u64,
    ) -> (RgbCvResult, Vec<(String, SignalValue)>) {
        let total_px = (width as usize).saturating_mul(height as usize);
        if total_px == 0 || rgba.len() < total_px * 4 {
            let empty = RgbCvResult {
                global_luma: 0.0,
                roi_luma: 0.0,
                motion_energy: 0.0,
                motion_centroid: [0.5, 0.5],
                dominant_rgb: [0.0, 0.0, 0.0],
                dominant_hsv: [0.0, 0.0, 0.0],
                brightest_point: [0.5, 0.5],
                brightest_luma: 0.0,
                blob_centroid: [0.5, 0.5],
                blob_area: 0.0,
                zones: Vec::new(),
            };
            return (empty, Vec::new());
        }

        let (rx0, ry0, rx1, ry1) = self.config.roi.pixel_bounds(width, height);
        let has_prev = self.prev_dims == (width, height) && self.prev_luma.len() == total_px;

        let zone_cols = self.config.zone_cols.clamp(1, 8);
        let zone_rows = self.config.zone_rows.clamp(1, 8);
        let zone_count = (zone_cols * zone_rows) as usize;
        let mut zone_motion_sum = vec![0.0f64; zone_count];
        let mut zone_pixel_count = vec![0usize; zone_count];

        let mut curr_luma = vec![0.0f32; total_px];

        let mut global_luma_sum = 0.0f64;
        let mut roi_luma_sum = 0.0f64;
        let mut roi_r_sum = 0.0f64;
        let mut roi_g_sum = 0.0f64;
        let mut roi_b_sum = 0.0f64;
        let mut roi_count = 0usize;

        let mut motion_sum = 0.0f64;
        let mut motion_wx = 0.0f64;
        let mut motion_wy = 0.0f64;
        let mut motion_weight = 0.0f64;

        let mut best_luma = -1.0f32;
        let mut best_x_sum = 0.0f64;
        let mut best_y_sum = 0.0f64;
        let mut best_count = 0usize;

        let mut blob_x_sum = 0.0f64;
        let mut blob_y_sum = 0.0f64;
        let mut blob_count = 0usize;

        let inv_255 = 1.0f32 / 255.0;
        let denom_w = (width.saturating_sub(1)).max(1) as f64;
        let denom_h = (height.saturating_sub(1)).max(1) as f64;

        for y in 0..height {
            let z_row = ((y * zone_rows) / height).min(zone_rows - 1);
            let in_roi_y = y >= ry0 && y < ry1;
            let row_offset = (y * width) as usize;

            for x in 0..width {
                let p_idx = row_offset + x as usize;
                let byte_idx = p_idx * 4;
                let r = rgba[byte_idx] as f32 * inv_255;
                let g = rgba[byte_idx + 1] as f32 * inv_255;
                let b = rgba[byte_idx + 2] as f32 * inv_255;
                let luma = rec709_luma(r, g, b);
                curr_luma[p_idx] = luma;
                global_luma_sum += luma as f64;

                // Brightest-point tracking (handles flat peak plateau cleanly via centroid of peak)
                if luma > best_luma + 1e-4 {
                    best_luma = luma;
                    best_x_sum = x as f64;
                    best_y_sum = y as f64;
                    best_count = 1;
                } else if (luma - best_luma).abs() <= 1e-4 {
                    best_x_sum += x as f64;
                    best_y_sum += y as f64;
                    best_count += 1;
                }

                // Frame-diff motion energy + motion zone grid
                let z_col = ((x * zone_cols) / width).min(zone_cols - 1);
                let z_idx = (z_row * zone_cols + z_col) as usize;
                zone_pixel_count[z_idx] += 1;

                if has_prev {
                    let diff = (luma - self.prev_luma[p_idx]).abs();
                    if diff >= self.config.motion_pixel_threshold {
                        let d64 = diff as f64;
                        motion_sum += d64;
                        motion_wx += (x as f64 / denom_w) * d64;
                        motion_wy += (y as f64 / denom_h) * d64;
                        motion_weight += d64;
                        zone_motion_sum[z_idx] += d64;
                    }
                }

                // ROI luma, dominant color, and target-color blob mask
                if in_roi_y && x >= rx0 && x < rx1 {
                    roi_count += 1;
                    roi_luma_sum += luma as f64;
                    roi_r_sum += r as f64;
                    roi_g_sum += g as f64;
                    roi_b_sum += b as f64;

                    let (h_deg, sat, val) = rgb_to_hsv(r, g, b);
                    if sat >= self.config.min_saturation
                        && val >= self.config.min_value
                        && circular_hue_dist_deg(h_deg, self.config.target_hue_deg)
                            <= self.config.hue_tolerance_deg
                    {
                        blob_x_sum += x as f64 / denom_w;
                        blob_y_sum += y as f64 / denom_h;
                        blob_count += 1;
                    }
                }
            }
        }

        self.prev_luma = curr_luma;
        self.prev_dims = (width, height);

        let global_luma = (global_luma_sum / total_px as f64).clamp(0.0, 1.0);
        let roi_denom = roi_count.max(1) as f64;
        let roi_luma = (roi_luma_sum / roi_denom).clamp(0.0, 1.0);
        let dom_r = (roi_r_sum / roi_denom).clamp(0.0, 1.0);
        let dom_g = (roi_g_sum / roi_denom).clamp(0.0, 1.0);
        let dom_b = (roi_b_sum / roi_denom).clamp(0.0, 1.0);
        let (dom_h, dom_s, dom_v) = rgb_to_hsv(dom_r as f32, dom_g as f32, dom_b as f32);

        // Motion energy scaled so moderate scene movement maps cleanly into 0.0..=1.0
        let motion_energy = if has_prev {
            ((motion_sum / total_px as f64) * 4.0).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let motion_centroid = if motion_weight > 1e-6 {
            [
                (motion_wx / motion_weight).clamp(0.0, 1.0),
                (motion_wy / motion_weight).clamp(0.0, 1.0),
            ]
        } else {
            [0.5, 0.5]
        };

        let brightest_point = if best_count > 0 {
            [
                ((best_x_sum / best_count as f64) / denom_w).clamp(0.0, 1.0),
                ((best_y_sum / best_count as f64) / denom_h).clamp(0.0, 1.0),
            ]
        } else {
            [0.5, 0.5]
        };
        let brightest_luma = best_luma.clamp(0.0, 1.0) as f64;

        let blob_area = (blob_count as f64 / roi_denom).clamp(0.0, 1.0);
        let blob_centroid = if blob_count > 0 {
            [
                (blob_x_sum / blob_count as f64).clamp(0.0, 1.0),
                (blob_y_sum / blob_count as f64).clamp(0.0, 1.0),
            ]
        } else {
            [0.5, 0.5]
        };

        let mut zones = Vec::with_capacity(zone_count);
        for row in 0..zone_rows {
            for col in 0..zone_cols {
                let idx = (row * zone_cols + col) as usize;
                let px = zone_pixel_count[idx].max(1) as f64;
                let energy = if has_prev {
                    ((zone_motion_sum[idx] / px) * 4.0).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let triggered = energy >= self.config.zone_trigger_threshold as f64;
                zones.push(MotionZoneReading {
                    index: idx,
                    col,
                    row,
                    energy,
                    triggered,
                });
            }
        }

        let result = RgbCvResult {
            global_luma,
            roi_luma,
            motion_energy,
            motion_centroid,
            dominant_rgb: [dom_r, dom_g, dom_b],
            dominant_hsv: [(dom_h / 360.0) as f64, dom_s as f64, dom_v as f64],
            brightest_point,
            brightest_luma,
            blob_centroid,
            blob_area,
            zones,
        };

        let mut raw_signals: Vec<(String, SignalValue)> = vec![
            ("cam.luma.global".into(), SignalValue::Float(result.global_luma)),
            ("cam.luma.roi".into(), SignalValue::Float(result.roi_luma)),
            ("cam.motion.energy".into(), SignalValue::Float(result.motion_energy)),
            ("cam.motion.x".into(), SignalValue::Float(result.motion_centroid[0])),
            ("cam.motion.y".into(), SignalValue::Float(result.motion_centroid[1])),
            ("cam.motion.centroid".into(), SignalValue::Vec2(result.motion_centroid)),
            ("cam.color.r".into(), SignalValue::Float(result.dominant_rgb[0])),
            ("cam.color.g".into(), SignalValue::Float(result.dominant_rgb[1])),
            ("cam.color.b".into(), SignalValue::Float(result.dominant_rgb[2])),
            ("cam.color.hue".into(), SignalValue::Float(result.dominant_hsv[0])),
            ("cam.color.sat".into(), SignalValue::Float(result.dominant_hsv[1])),
            ("cam.color.val".into(), SignalValue::Float(result.dominant_hsv[2])),
            ("cam.brightest.x".into(), SignalValue::Float(result.brightest_point[0])),
            ("cam.brightest.y".into(), SignalValue::Float(result.brightest_point[1])),
            ("cam.brightest.val".into(), SignalValue::Float(result.brightest_luma)),
            ("cam.blob.x".into(), SignalValue::Float(result.blob_centroid[0])),
            ("cam.blob.y".into(), SignalValue::Float(result.blob_centroid[1])),
            ("cam.blob.area".into(), SignalValue::Float(result.blob_area)),
            ("cam.blob.centroid".into(), SignalValue::Vec2(result.blob_centroid)),
        ];

        for z in &result.zones {
            raw_signals.push((
                format!("cam.zone.{}.energy", z.index),
                SignalValue::Float(z.energy),
            ));
            raw_signals.push((
                format!("cam.zone.{}.trigger", z.index),
                SignalValue::Bool(z.triggered),
            ));
        }

        // Apply optional per-parameter hackable modifier chains
        let mut shaped_signals = Vec::with_capacity(raw_signals.len());
        for (key, val) in raw_signals {
            if let Some((chain, state)) = self.compiled_mods.get_mut(&key) {
                if let Some(shaped) = chain.apply(val, timestamp_ns, state) {
                    shaped_signals.push((key, shaped));
                }
            } else {
                shaped_signals.push((key, val));
            }
        }

        (result, shaped_signals)
    }
}

impl RgbCameraExtractor for RgbCvPipeline {
    fn extractor_name(&self) -> &'static str {
        "kf-rgb-cv-pipeline"
    }

    fn extract_signals(&mut self, rgba: &[u8], width: u32, height: u32) -> Vec<(String, SignalValue)> {
        let (_, signals) = self.process_frame(rgba, width, height, SignalEvent::now_ns());
        signals
    }
}
