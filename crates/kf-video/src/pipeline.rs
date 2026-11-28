//! NDI node pipeline runtime:
//! - ~150 ms debounced resolution reallocation (prevents thrashing downstream receivers)
//! - Instantaneous per-frame crop / pan / zoom / opacity updates
//! - Target FPS throttling
//! - Live `kf-core` control-signal modulation bridge (OSC/MIDI/WS/UDP/Macros -> NDI params)
//! - Thread-safe `VideoPipelineManager` for Tauri commands & UI preview polling.

use crate::ndi_ffi::{DynamicNdiRuntime, NdiReceiver};
use crate::scaler::{AdaptiveVideoScaler, CpuVideoScaler, NdiTransformConfig, VideoFitMode};
use crate::test_pattern::{
    generate_test_pattern_frame, parse_synthetic_source, synthetic_test_sources, TestPatternKind,
    SYNTHETIC_SMPTE_1080P,
};
use crate::{NdiRuntimeLoader, NdiRuntimeStatus, VideoError, VideoFrame, VideoScaler};
use base64::Engine;
use kf_core::{RoutedOutput, SignalValue, VideoSourceInfo};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

pub const DEFAULT_RESIZE_DEBOUNCE_MS: u64 = 150;

/// Debounces output resolution reallocations (`~150 ms`) so dragging a slider or scrubbing a MIDI
/// knob does not thrash NDI output buffers, while crop/pan/zoom stay instantaneous per frame.
#[derive(Debug, Clone)]
pub struct DebouncedResolution {
    active_width: u32,
    active_height: u32,
    pending: Option<(u32, u32, Instant)>,
    debounce_window: Duration,
    reallocation_count: u64,
}

impl DebouncedResolution {
    pub fn new(width: u32, height: u32, debounce_window: Duration) -> Self {
        Self {
            active_width: width.clamp(16, 7680),
            active_height: height.clamp(16, 4320),
            pending: None,
            debounce_window,
            reallocation_count: 0,
        }
    }

    pub fn active(&self) -> (u32, u32) {
        (self.active_width, self.active_height)
    }

    pub fn pending(&self) -> Option<(u32, u32)> {
        self.pending.map(|(w, h, _)| (w, h))
    }

    pub fn reallocation_count(&self) -> u64 {
        self.reallocation_count
    }

    /// Requests a new output resolution. If different from the currently active resolution,
    /// schedules reallocation after `debounce_window` elapses without further changes.
    pub fn request(&mut self, width: u32, height: u32, now: Instant) {
        let w = width.clamp(16, 7680);
        let h = height.clamp(16, 4320);
        if w == self.active_width && h == self.active_height {
            self.pending = None;
            return;
        }
        self.pending = Some((w, h, now));
    }

    /// Commits any pending resolution whose debounce window has elapsed. Returns `true` if a
    /// reallocation occurred on this poll.
    pub fn poll(&mut self, now: Instant) -> bool {
        if let Some((w, h, requested_at)) = self.pending {
            if now.duration_since(requested_at) >= self.debounce_window {
                self.active_width = w;
                self.active_height = h;
                self.pending = None;
                self.reallocation_count += 1;
                return true;
            }
        }
        false
    }
}

/// Target FPS frame-pacer preventing output over-emission.
#[derive(Debug, Clone)]
pub struct FpsThrottler {
    target_fps: f32,
    last_emit: Option<Instant>,
    emitted_frames: u64,
    dropped_frames: u64,
}

impl FpsThrottler {
    pub fn new(target_fps: f32) -> Self {
        Self {
            target_fps: target_fps.clamp(1.0, 120.0),
            last_emit: None,
            emitted_frames: 0,
            dropped_frames: 0,
        }
    }

    pub fn set_target_fps(&mut self, fps: f32) {
        self.target_fps = fps.clamp(1.0, 120.0);
    }

    pub fn target_fps(&self) -> f32 {
        self.target_fps
    }

    pub fn min_frame_interval(&self) -> Duration {
        Duration::from_secs_f64(1.0 / (self.target_fps.max(1.0) as f64))
    }

    /// Returns `true` if a frame should be emitted at `now`, advancing pacing state.
    pub fn should_emit(&mut self, now: Instant) -> bool {
        let interval = self.min_frame_interval();
        match self.last_emit {
            None => {
                self.last_emit = Some(now);
                self.emitted_frames += 1;
                true
            }
            Some(prev) if now.duration_since(prev) >= interval => {
                self.last_emit = Some(now);
                self.emitted_frames += 1;
                true
            }
            Some(_) => {
                self.dropped_frames += 1;
                false
            }
        }
    }

    pub fn emitted_frames(&self) -> u64 {
        self.emitted_frames
    }
}

/// Patch DTO accepted by Tauri commands / UI to configure an NDI video node.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NdiNodeConfigPatch {
    pub source_name: Option<String>,
    pub output_name: Option<String>,
    pub test_pattern: Option<TestPatternKind>,
    pub fit_mode: Option<VideoFitMode>,
    pub crop_x: Option<f32>,
    pub crop_y: Option<f32>,
    pub crop_w: Option<f32>,
    pub crop_h: Option<f32>,
    pub zoom: Option<f32>,
    pub out_width: Option<u32>,
    pub out_height: Option<u32>,
    pub out_fps: Option<f32>,
    pub opacity: Option<f32>,
    pub freeze: Option<bool>,
    pub bypass: Option<bool>,
}

/// Snapshot of an NDI video node's live pipeline state for UI display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NdiNodeState {
    pub device_id: Uuid,
    pub source_name: String,
    pub output_name: String,
    pub using_synthetic_source: bool,
    pub test_pattern: TestPatternKind,
    pub transform: NdiTransformConfig,
    pub active_width: u32,
    pub active_height: u32,
    pub pending_width: Option<u32>,
    pub pending_height: Option<u32>,
    pub reallocation_count: u64,
    pub frames_rendered: u64,
    pub scaler_backend: String,
}

/// Low-res preview thumbnail payload returned to the React `<canvas>` tile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NdiPreviewPayload {
    pub device_id: Uuid,
    pub thumb_width: u32,
    pub thumb_height: u32,
    pub output_width: u32,
    pub output_height: u32,
    pub out_fps: f32,
    pub jpeg_base64: String,
    pub timestamp_ms: u64,
    pub scaler_backend: String,
    pub using_synthetic_source: bool,
    pub pending_resize: bool,
}

/// Per-node NDI video processing pipeline.
pub struct NdiNodePipeline {
    pub device_id: Uuid,
    pub source_name: String,
    pub output_name: String,
    pub test_pattern: TestPatternKind,
    pub transform: NdiTransformConfig,
    resolution: DebouncedResolution,
    throttler: FpsThrottler,
    scaler: AdaptiveVideoScaler,
    started_at: Instant,
    frame_counter: u64,
    frozen_frame: Option<VideoFrame>,
    last_output: Option<VideoFrame>,
}

impl NdiNodePipeline {
    pub fn new(device_id: Uuid) -> Self {
        let transform = NdiTransformConfig::default();
        let resolution = DebouncedResolution::new(
            transform.out_width,
            transform.out_height,
            Duration::from_millis(DEFAULT_RESIZE_DEBOUNCE_MS),
        );
        let throttler = FpsThrottler::new(transform.out_fps);
        Self {
            device_id,
            source_name: SYNTHETIC_SMPTE_1080P.to_string(),
            output_name: "Kontrolfreak Out".to_string(),
            test_pattern: TestPatternKind::SmpteBars,
            transform,
            resolution,
            throttler,
            scaler: AdaptiveVideoScaler::new(),
            started_at: Instant::now(),
            frame_counter: 0,
            frozen_frame: None,
            last_output: None,
        }
    }

    pub fn with_debounce(mut self, debounce: Duration) -> Self {
        let (w, h) = self.resolution.active();
        self.resolution = DebouncedResolution::new(w, h, debounce);
        self
    }

    pub fn apply_patch(&mut self, patch: NdiNodeConfigPatch, now: Instant) {
        if let Some(src) = patch.source_name {
            if let Some((pattern, _, _)) = parse_synthetic_source(Some(&src)) {
                self.test_pattern = pattern;
            }
            self.source_name = src;
        }
        if let Some(out) = patch.output_name {
            self.output_name = out;
        }
        if let Some(tp) = patch.test_pattern {
            self.test_pattern = tp;
        }
        if let Some(fm) = patch.fit_mode {
            self.transform.fit_mode = fm;
        }
        if let Some(v) = patch.crop_x {
            self.transform.crop_x = v;
        }
        if let Some(v) = patch.crop_y {
            self.transform.crop_y = v;
        }
        if let Some(v) = patch.crop_w {
            self.transform.crop_w = v;
        }
        if let Some(v) = patch.crop_h {
            self.transform.crop_h = v;
        }
        if let Some(v) = patch.zoom {
            self.transform.zoom = v;
        }
        if let Some(v) = patch.opacity {
            self.transform.opacity = v;
        }
        if let Some(v) = patch.freeze {
            self.transform.freeze = v;
            if !v {
                self.frozen_frame = None;
            }
        }
        if let Some(v) = patch.bypass {
            self.transform.bypass = v;
        }
        if let Some(fps) = patch.out_fps {
            self.transform.out_fps = fps.clamp(1.0, 120.0);
            self.throttler.set_target_fps(self.transform.out_fps);
        }

        let target_w = patch.out_width.unwrap_or(self.transform.out_width);
        let target_h = patch.out_height.unwrap_or(self.transform.out_height);
        if patch.out_width.is_some() || patch.out_height.is_some() {
            self.transform.out_width = target_w.clamp(16, 7680);
            self.transform.out_height = target_h.clamp(16, 4320);
            self.resolution
                .request(self.transform.out_width, self.transform.out_height, now);
        }

        self.transform = self.transform.sanitized();
    }

    /// Applies an incoming `kf-core` control signal (from OSC, MIDI, RTPMIDI, WS, UDP, or Macro)
    /// directly to this NDI node's live video parameters.
    ///
    /// - Crop / pan / zoom / opacity / freeze / bypass update **instantaneously** per frame.
    /// - `output_width` / `output_height` go through the ~150 ms debounced resolution allocator.
    pub fn apply_control_signal(
        &mut self,
        logical_key: &str,
        value: &SignalValue,
        now: Instant,
    ) -> bool {
        let key = logical_key
            .trim()
            .to_ascii_lowercase();
        let key = key.strip_prefix("ndi.").unwrap_or(&key);

        match key {
            "crop_x" => {
                if let Some(v) = value.as_f64() {
                    self.transform.crop_x = v as f32;
                    self.transform = self.transform.sanitized();
                    return true;
                }
            }
            "crop_y" => {
                if let Some(v) = value.as_f64() {
                    self.transform.crop_y = v as f32;
                    self.transform = self.transform.sanitized();
                    return true;
                }
            }
            "crop_w" => {
                if let Some(v) = value.as_f64() {
                    self.transform.crop_w = v as f32;
                    self.transform = self.transform.sanitized();
                    return true;
                }
            }
            "crop_h" => {
                if let Some(v) = value.as_f64() {
                    self.transform.crop_h = v as f32;
                    self.transform = self.transform.sanitized();
                    return true;
                }
            }
            "zoom" => {
                if let Some(v) = value.as_f64() {
                    // Accept normalized 0..=1 mapped to 1x..4x, or explicit > 1.0 zoom factor up to 8x
                    let z = if v <= 1.0 {
                        1.0 + v.clamp(0.0, 1.0) * 3.0
                    } else {
                        v.clamp(1.0, 8.0)
                    };
                    self.transform.zoom = z as f32;
                    self.transform = self.transform.sanitized();
                    return true;
                }
            }
            "output_width" | "out_width" => {
                if let Some(v) = value.as_f64() {
                    let px = if v <= 1.0 {
                        (320.0 + v.clamp(0.0, 1.0) * (1920.0 - 320.0)).round() as u32
                    } else {
                        v.round().clamp(16.0, 7680.0) as u32
                    };
                    self.transform.out_width = px;
                    self.resolution.request(px, self.transform.out_height, now);
                    return true;
                }
            }
            "output_height" | "out_height" => {
                if let Some(v) = value.as_f64() {
                    let px = if v <= 1.0 {
                        (180.0 + v.clamp(0.0, 1.0) * (1080.0 - 180.0)).round() as u32
                    } else {
                        v.round().clamp(16.0, 4320.0) as u32
                    };
                    self.transform.out_height = px;
                    self.resolution.request(self.transform.out_width, px, now);
                    return true;
                }
            }
            "opacity" => {
                if let Some(v) = value.as_f64() {
                    self.transform.opacity = (v as f32).clamp(0.0, 1.0);
                    return true;
                }
            }
            "out_fps" | "fps" => {
                if let Some(v) = value.as_f64() {
                    let fps = if v <= 1.0 {
                        5.0 + v.clamp(0.0, 1.0) * 55.0
                    } else {
                        v.clamp(1.0, 120.0)
                    } as f32;
                    self.transform.out_fps = fps;
                    self.throttler.set_target_fps(fps);
                    return true;
                }
            }
            "freeze" => {
                self.transform.freeze = value.as_bool();
                if !self.transform.freeze {
                    self.frozen_frame = None;
                }
                return true;
            }
            "bypass" => {
                self.transform.bypass = value.as_bool();
                return true;
            }
            "fit_mode" | "scale_mode" => {
                if let Some(v) = value.as_f64() {
                    self.transform.fit_mode = match (v.round() as i64).clamp(0, 3) {
                        0 => VideoFitMode::Contain,
                        1 => VideoFitMode::Cover,
                        2 => VideoFitMode::Stretch,
                        _ => VideoFitMode::Crop,
                    };
                    return true;
                }
            }
            _ => {}
        }
        false
    }

    pub fn active_resolution(&self) -> (u32, u32) {
        self.resolution.active()
    }

    pub fn reallocation_count(&self) -> u64 {
        self.resolution.reallocation_count()
    }

    /// Polls debounced resolution and processes an incoming frame (respecting FPS throttle & freeze).
    /// Returns `Ok(None)` when throttled by target FPS.
    pub fn process_input_frame(
        &mut self,
        input: &VideoFrame,
        now: Instant,
        enforce_fps_throttle: bool,
    ) -> Result<Option<VideoFrame>, VideoError> {
        self.resolution.poll(now);
        if enforce_fps_throttle && !self.throttler.should_emit(now) {
            return Ok(None);
        }

        if self.transform.freeze {
            if let Some(ref cached) = self.frozen_frame {
                return Ok(Some(cached.clone()));
            }
        }

        let (active_w, active_h) = self.resolution.active();
        let mut effective = self.transform.sanitized();
        effective.out_width = active_w;
        effective.out_height = active_h;

        let out = self.scaler.transform_frame(input, &effective)?;
        self.frame_counter += 1;
        if self.transform.freeze {
            self.frozen_frame = Some(out.clone());
        }
        self.last_output = Some(out.clone());
        Ok(Some(out))
    }

    /// Generates and scales a synthetic frame (used when a Test Pattern source is selected or when
    /// NDI Runtime is unavailable) and encodes a compact JPEG preview thumbnail.
    pub fn render_preview_jpeg(
        &mut self,
        max_width: u32,
        max_height: u32,
        now: Instant,
    ) -> Result<NdiPreviewPayload, VideoError> {
        self.resolution.poll(now);
        let (pattern, src_w, src_h) = parse_synthetic_source(Some(&self.source_name))
            .unwrap_or((self.test_pattern, 640, 360));

        // For live preview thumbnails, generate source at a snappy resolution that matches aspect
        // ratio cleanly (e.g. 640x360 for 16:9) so 15fps UI polling uses minimal CPU.
        let preview_src_w = src_w.min(640);
        let preview_src_h = (src_h as f64 * (preview_src_w as f64 / src_w.max(1) as f64))
            .round()
            .max(16.0) as u32;

        let phase = now.duration_since(self.started_at).as_secs_f32();
        let synthetic = generate_test_pattern_frame(
            self.device_id,
            pattern,
            preview_src_w,
            preview_src_h,
            self.frame_counter + 1,
            phase,
        );

        let (active_w, active_h) = self.resolution.active();
        // Scale to preview bounds preserving exact output aspect ratio (so 1280x360 renders ultra-wide!)
        let out_aspect = active_w as f64 / active_h.max(1) as f64;
        let max_wf = max_width.max(32) as f64;
        let max_hf = max_height.max(32) as f64;
        let (thumb_w, thumb_h) = if max_wf / max_hf > out_aspect {
            let h = max_hf.round() as u32;
            let w = (max_hf * out_aspect).round().clamp(32.0, max_wf) as u32;
            (w, h)
        } else {
            let w = max_wf.round() as u32;
            let h = (max_wf / out_aspect).round().clamp(32.0, max_hf) as u32;
            (w, h)
        };

        let mut preview_cfg = self.transform.sanitized();
        preview_cfg.out_width = thumb_w;
        preview_cfg.out_height = thumb_h;

        let frame = if preview_cfg.freeze {
            if let Some(ref cached) = self.frozen_frame {
                cached.clone()
            } else {
                let f = self.scaler.transform_frame(&synthetic, &preview_cfg)?;
                self.frozen_frame = Some(f.clone());
                f
            }
        } else {
            self.scaler.transform_frame(&synthetic, &preview_cfg)?
        };
        self.frame_counter += 1;

        let encoded = self
            .scaler
            .encode_preview_jpeg(&frame, thumb_w, thumb_h)?;
        let jpeg_base64 = base64::engine::general_purpose::STANDARD.encode(&encoded.jpeg_bytes);

        Ok(NdiPreviewPayload {
            device_id: self.device_id,
            thumb_width: encoded.width,
            thumb_height: encoded.height,
            output_width: active_w,
            output_height: active_h,
            out_fps: self.transform.out_fps,
            jpeg_base64,
            timestamp_ms: encoded.timestamp_ms,
            scaler_backend: self.scaler.backend_name(),
            using_synthetic_source: true,
            pending_resize: self.resolution.pending().is_some(),
        })
    }

    pub fn state(&mut self, now: Instant) -> NdiNodeState {
        self.resolution.poll(now);
        let (active_w, active_h) = self.resolution.active();
        let pending = self.resolution.pending();
        let using_synthetic = parse_synthetic_source(Some(&self.source_name)).is_some();
        NdiNodeState {
            device_id: self.device_id,
            source_name: self.source_name.clone(),
            output_name: self.output_name.clone(),
            using_synthetic_source: using_synthetic,
            test_pattern: self.test_pattern,
            transform: self.transform.clone(),
            active_width: active_w,
            active_height: active_h,
            pending_width: pending.map(|p| p.0),
            pending_height: pending.map(|p| p.1),
            reallocation_count: self.resolution.reallocation_count(),
            frames_rendered: self.frame_counter,
            scaler_backend: self.scaler.backend_name(),
        }
    }
}

/// Manages all live `NdiVideoNode` pipelines, NDI runtime probing, source discovery, and
/// live control-signal modulation from `kf-core`'s `SignalRouter`.
pub struct VideoPipelineManager {
    runtime: DynamicNdiRuntime,
    nodes: Mutex<HashMap<Uuid, NdiNodePipeline>>,
    /// One live NDI receiver for the simple output window. Dropped when the user picks something else.
    simple_recv: Mutex<Option<(String, NdiReceiver)>>,
    simple_scaler: Mutex<CpuVideoScaler>,
    simple_last: Mutex<Option<(String, NdiPreviewPayload)>>,
}

impl Default for VideoPipelineManager {
    fn default() -> Self {
        Self::new()
    }
}

impl VideoPipelineManager {
    pub fn new() -> Self {
        Self {
            runtime: DynamicNdiRuntime::probe(),
            nodes: Mutex::new(HashMap::new()),
            simple_recv: Mutex::new(None),
            simple_scaler: Mutex::new(CpuVideoScaler::new()),
            simple_last: Mutex::new(None),
        }
    }

    pub fn new_arc() -> Arc<Self> {
        Arc::new(Self::new())
    }

    pub fn runtime_status(&self) -> NdiRuntimeStatus {
        self.runtime.probe_runtime()
    }

    /// Discovers real NDI network sources (if NDI Runtime is installed) and appends built-in
    /// synthetic Test Pattern sources so users can always test resizing/cropping/thumbnails.
    pub fn discover_all_sources(&self) -> Vec<VideoSourceInfo> {
        let mut sources = self.runtime.discover_sources().unwrap_or_default();
        sources.extend(synthetic_test_sources());
        sources
    }

    pub fn ensure_node(&self, device_id: Uuid) -> NdiNodeState {
        let mut guard = self.nodes.lock();
        let node = guard
            .entry(device_id)
            .or_insert_with(|| NdiNodePipeline::new(device_id));
        node.state(Instant::now())
    }

    pub fn remove_node(&self, device_id: Uuid) {
        self.nodes.lock().remove(&device_id);
    }

    pub fn configure_node(
        &self,
        device_id: Uuid,
        patch: NdiNodeConfigPatch,
    ) -> NdiNodeState {
        let now = Instant::now();
        let mut guard = self.nodes.lock();
        let node = guard
            .entry(device_id)
            .or_insert_with(|| NdiNodePipeline::new(device_id));
        node.apply_patch(patch, now);
        node.state(now)
    }

    pub fn get_node_state(&self, device_id: Uuid) -> NdiNodeState {
        let now = Instant::now();
        let mut guard = self.nodes.lock();
        let node = guard
            .entry(device_id)
            .or_insert_with(|| NdiNodePipeline::new(device_id));
        node.state(now)
    }

    /// Applies any routed `kf-core` signal output targeting an NDI video node.
    pub fn apply_routed_output(&self, output: &RoutedOutput) -> bool {
        let now = Instant::now();
        let mut guard = self.nodes.lock();
        let Some(node) = guard.get_mut(&output.target_device_id) else {
            // If key explicitly carries the `ndi.` prefix, auto-initialize pipeline state on first hit.
            if output.target_port_logical_key.starts_with("ndi.") {
                let node = guard
                    .entry(output.target_device_id)
                    .or_insert_with(|| NdiNodePipeline::new(output.target_device_id));
                return node.apply_control_signal(&output.target_port_logical_key, &output.value, now);
            }
            return false;
        };
        node.apply_control_signal(&output.target_port_logical_key, &output.value, now)
    }

    pub fn render_preview(
        &self,
        device_id: Uuid,
        max_width: u32,
        max_height: u32,
    ) -> Result<NdiPreviewPayload, VideoError> {
        let now = Instant::now();
        let mut guard = self.nodes.lock();
        let node = guard
            .entry(device_id)
            .or_insert_with(|| NdiNodePipeline::new(device_id));
        node.render_preview_jpeg(max_width, max_height, now)
    }

    /// Captures one frame from a real NDI source for the simple output window.
    /// Returns the previous frame when the receiver has nothing new yet.
    /// Missing runtime is an error so the caller can fall back to a calm waiting state.
    pub fn render_external_ndi_jpeg(
        &self,
        source_name: &str,
        max_width: u32,
        max_height: u32,
    ) -> Result<Option<NdiPreviewPayload>, VideoError> {
        if !self.runtime.is_available() {
            return Err(VideoError::NdiRuntimeMissing);
        }
        let captured = {
            let mut slot = self.simple_recv.lock();
            let need_new = slot
                .as_ref()
                .map(|(name, _)| name != source_name)
                .unwrap_or(true);
            if need_new {
                let rx = self
                    .runtime
                    .create_receiver(source_name, "Kontrolfreak Simple Output")?;
                *slot = Some((source_name.to_string(), rx));
                *self.simple_last.lock() = None;
            }
            let Some((_, rx)) = slot.as_mut() else {
                return Ok(None);
            };
            rx.capture_video(Uuid::nil(), 0)?
        };
        let Some(frame) = captured else {
            let cached = self.simple_last.lock();
            return Ok(cached.as_ref().and_then(|(name, payload)| {
                (name == source_name).then(|| payload.clone())
            }));
        };
        let encoded = self.simple_scaler.lock().encode_preview_jpeg_frame(
            &frame,
            max_width.max(16),
            max_height.max(16),
        )?;
        let jpeg_base64 = base64::engine::general_purpose::STANDARD.encode(&encoded.jpeg_bytes);
        let payload = NdiPreviewPayload {
            device_id: Uuid::nil(),
            thumb_width: encoded.width,
            thumb_height: encoded.height,
            output_width: encoded.width,
            output_height: encoded.height,
            out_fps: 0.0,
            jpeg_base64,
            timestamp_ms: encoded.timestamp_ms,
            scaler_backend: "cpu".into(),
            using_synthetic_source: false,
            pending_resize: false,
        };
        *self.simple_last.lock() = Some((source_name.to_string(), payload.clone()));
        Ok(Some(payload))
    }

    /// Releases the simple-output NDI receiver so other apps can use the source.
    pub fn release_external_ndi(&self) {
        *self.simple_recv.lock() = None;
        *self.simple_last.lock() = None;
    }
}

#[cfg(test)]
mod simple_ndi_tests {
    use super::*;

    #[test]
    fn external_ndi_preview_reports_missing_runtime() {
        let mgr = VideoPipelineManager::new();
        if mgr.runtime_status().available {
            return;
        }
        let err = mgr
            .render_external_ndi_jpeg("Camera", 320, 180)
            .unwrap_err();
        assert!(matches!(err, VideoError::NdiRuntimeMissing));
    }
}
