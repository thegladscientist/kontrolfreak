//! Milestone 2 NDI dynamic runtime loader, synthetic Test Pattern source,
//! flexible-resolution SIMD CPU (`fast_image_resize`) + optional GPU (`wgpu`) scaler,
//! debounced resolution reallocation, control-signal modulation bridge, and UI thumbnail encoder.

pub mod ndi_ffi;
pub mod pipeline;
pub mod scaler;
pub mod test_pattern;

pub use kf_core::{
    AspectScaleMode, FrameRef, PixelFormat, VideoPreviewFrame, VideoResizeConfig, VideoSourceInfo,
};
pub use ndi_ffi::{candidate_ndi_library_paths, DynamicNdiRuntime, NdiReceiver, NdiSender, NDI_INSTALL_URL};
pub use pipeline::{
    DebouncedResolution, FpsThrottler, NdiNodeConfigPatch, NdiNodePipeline, NdiNodeState,
    NdiPreviewPayload, VideoPipelineManager, DEFAULT_RESIZE_DEBOUNCE_MS,
};
pub use scaler::{
    compute_transform_rects, frame_to_rgba8, AdaptiveVideoScaler, ComputedRects, CpuVideoScaler,
    NdiTransformConfig, VideoFitMode,
};
#[cfg(feature = "gpu")]
pub use scaler::WgpuVideoScaler;
pub use test_pattern::{
    generate_test_pattern_frame, parse_synthetic_source, synthetic_test_sources, TestPatternKind,
    SYNTHETIC_GRADIENT_720P, SYNTHETIC_MOTION_GRID_1080P, SYNTHETIC_SMPTE_1080P,
};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum VideoError {
    #[error("NDI Runtime is not installed on this system (expected NDI v6 shared library)")]
    NdiRuntimeMissing,
    #[error("Video pipeline not implemented yet (scheduled for Milestone 2)")]
    NotImplementedYet,
    #[error("Video backend error: {0}")]
    Backend(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NdiRuntimeStatus {
    pub available: bool,
    pub library_path: Option<String>,
    pub version: Option<String>,
    pub install_url: String,
}

impl Default for NdiRuntimeStatus {
    fn default() -> Self {
        Self {
            available: false,
            library_path: None,
            version: None,
            install_url: NDI_INSTALL_URL.to_string(),
        }
    }
}

/// Raw frame buffer exchanged between NDI receivers, GPU/CPU scalers, and NDI senders.
#[derive(Debug, Clone)]
pub struct VideoFrame {
    pub meta: FrameRef,
    pub stride: usize,
    pub data: Vec<u8>,
}

/// Dynamic NDI runtime loader trait.
pub trait NdiRuntimeLoader: Send + Sync {
    fn probe_runtime(&self) -> NdiRuntimeStatus;
    fn discover_sources(&self) -> Result<Vec<VideoSourceInfo>, VideoError>;
}

/// Frame scaler trait (`wgpu` optional GPU primary + `fast_image_resize` SIMD CPU fallback).
pub trait VideoScaler: Send + Sync {
    fn scale_frame(
        &mut self,
        input: &VideoFrame,
        config: &VideoResizeConfig,
    ) -> Result<VideoFrame, VideoError>;

    fn encode_preview_jpeg(
        &mut self,
        input: &VideoFrame,
        max_width: u32,
        max_height: u32,
    ) -> Result<VideoPreviewFrame, VideoError>;
}

/// Legacy M1 stub loader preserved for compatibility.
#[derive(Debug, Default)]
pub struct NdiRuntimeStub;

impl NdiRuntimeLoader for NdiRuntimeStub {
    fn probe_runtime(&self) -> NdiRuntimeStatus {
        NdiRuntimeStatus::default()
    }

    fn discover_sources(&self) -> Result<Vec<VideoSourceInfo>, VideoError> {
        Err(VideoError::NdiRuntimeMissing)
    }
}

/// Per-node runtime metadata struct preserved from M1 seam.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NdiNodeRuntimeStub {
    pub device_id: Uuid,
    pub config: VideoResizeConfig,
    pub runtime: NdiRuntimeStatus,
}

#[cfg(test)]
mod tests {
    use super::*;
    use kf_core::{
        ControlPlane, Device, DeviceKind, Port, PortDirection, SignalEvent, SignalKind,
        SignalRouter, SignalValue, TelemetryAggregator, WorkspaceGraph,
    };
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    #[test]
    fn ndi_stub_and_dynamic_loader_report_cleanly() {
        let stub = NdiRuntimeStub;
        assert!(!stub.probe_runtime().available);
        assert!(matches!(
            stub.discover_sources(),
            Err(VideoError::NdiRuntimeMissing)
        ));

        // Dynamic loader must probe cleanly without panicking regardless of host NDI install
        let dyn_rt = DynamicNdiRuntime::probe();
        let status = dyn_rt.probe_runtime();
        assert_eq!(status.install_url, NDI_INSTALL_URL);
        assert!(!candidate_ndi_library_paths().is_empty());

        // VideoPipelineManager always provides synthetic Test Pattern sources even without NDI
        let mgr = VideoPipelineManager::new();
        let sources = mgr.discover_all_sources();
        assert!(sources.len() >= 3);
        assert!(sources.iter().any(|s| s.name.contains("SMPTE")));
    }

    #[test]
    fn ndi_resize_crop_letterbox_to_arbitrary_resolutions() {
        let dev_id = Uuid::new_v4();
        let src_1080p =
            generate_test_pattern_frame(dev_id, TestPatternKind::SmpteBars, 1920, 1080, 1, 0.25);
        assert_eq!(src_1080p.meta.width, 1920);
        assert_eq!(src_1080p.meta.height, 1080);

        let mut scaler = AdaptiveVideoScaler::new();

        // 1. Contain (pillarbox) 1920x1080 -> 1280x360 ultra-wide banner
        // 16:9 inside 32:9 -> scaled content is 640x360 centered at dst_x=320 with black pillarbars!
        let contain_cfg = NdiTransformConfig {
            fit_mode: VideoFitMode::Contain,
            out_width: 1280,
            out_height: 360,
            ..Default::default()
        };
        let rects = compute_transform_rects(1920, 1080, &contain_cfg);
        assert_eq!(rects.dst_w, 640);
        assert_eq!(rects.dst_h, 360);
        assert_eq!(rects.dst_x, 320);
        assert_eq!(rects.dst_y, 0);

        let out_banner = scaler.transform_frame(&src_1080p, &contain_cfg).unwrap();
        assert_eq!(out_banner.meta.width, 1280);
        assert_eq!(out_banner.meta.height, 360);
        assert_eq!(out_banner.data.len(), 1280 * 360 * 4);
        // Left pillarbox pixel (x=10, y=180) must be black (0, 0, 0, 255)
        let left_px = (180 * 1280 + 10) * 4;
        assert_eq!(&out_banner.data[left_px..left_px + 4], &[0, 0, 0, 255]);
        // Center pixel (x=640, y=50) must contain non-black SMPTE bar content
        let center_px = (50 * 1280 + 640) * 4;
        assert!(
            out_banner.data[center_px] > 0
                || out_banner.data[center_px + 1] > 0
                || out_banner.data[center_px + 2] > 0
        );

        // 2. Cover 1920x1080 -> 512x512 square (aspect-preserving center crop)
        let cover_cfg = NdiTransformConfig {
            fit_mode: VideoFitMode::Cover,
            out_width: 512,
            out_height: 512,
            ..Default::default()
        };
        let cover_rects = compute_transform_rects(1920, 1080, &cover_cfg);
        assert_eq!(cover_rects.src_w, 1080);
        assert_eq!(cover_rects.src_h, 1080);
        assert_eq!(cover_rects.src_x, 420);
        let out_sq = scaler.transform_frame(&src_1080p, &cover_cfg).unwrap();
        assert_eq!(out_sq.meta.width, 512);
        assert_eq!(out_sq.meta.height, 512);

        // 3. Explicit normalized Crop + Zoom 1920x1080 -> 640x360
        let crop_cfg = NdiTransformConfig {
            fit_mode: VideoFitMode::Crop,
            crop_x: 0.25,
            crop_y: 0.25,
            crop_w: 0.5,
            crop_h: 0.5,
            zoom: 2.0,
            out_width: 640,
            out_height: 360,
            opacity: 0.5,
            ..Default::default()
        };
        let crop_rects = compute_transform_rects(1920, 1080, &crop_cfg);
        // 0.5 / 2.0 = 0.25 of 1920x1080 -> 480x270 centered in [0.25..0.75] => starts at 0.375
        assert_eq!(crop_rects.src_w, 480);
        assert_eq!(crop_rects.src_h, 270);
        assert_eq!(crop_rects.src_x, 720);
        assert_eq!(crop_rects.src_y, 405);
        let out_crop = scaler.transform_frame(&src_1080p, &crop_cfg).unwrap();
        assert_eq!(out_crop.meta.width, 640);
        assert_eq!(out_crop.meta.height, 360);

        // 4. Low-res JPEG thumbnail encoding works cleanly (valid JPEG SOI/EOI markers)
        let thumb = scaler.encode_preview_jpeg(&out_banner, 320, 180).unwrap();
        assert!(thumb.width <= 320);
        assert!(thumb.height <= 180);
        assert!(thumb.jpeg_bytes.len() > 200);
        assert_eq!(&thumb.jpeg_bytes[0..2], &[0xFF, 0xD8]); // JPEG SOI
        assert_eq!(
            &thumb.jpeg_bytes[thumb.jpeg_bytes.len() - 2..],
            &[0xFF, 0xD9]
        ); // JPEG EOI
    }

    #[test]
    fn debounced_resolution_reallocation_and_instant_crop_updates() {
        let dev_id = Uuid::new_v4();
        let mut pipe = NdiNodePipeline::new(dev_id)
            .with_debounce(Duration::from_millis(DEFAULT_RESIZE_DEBOUNCE_MS));
        let t0 = Instant::now();
        let input = generate_test_pattern_frame(dev_id, TestPatternKind::MotionGrid, 640, 360, 1, 0.0);

        assert_eq!(pipe.active_resolution(), (1280, 720));
        assert_eq!(pipe.reallocation_count(), 0);

        // Rapid resolution scrubbing every 40ms (below 150ms debounce) must NOT reallocate yet
        pipe.apply_patch(
            NdiNodeConfigPatch {
                out_width: Some(1024),
                out_height: Some(512),
                ..Default::default()
            },
            t0,
        );
        pipe.apply_patch(
            NdiNodeConfigPatch {
                out_width: Some(1280),
                out_height: Some(360),
                crop_x: Some(0.2),
                zoom: Some(1.5),
                ..Default::default()
            },
            t0 + Duration::from_millis(40),
        );

        // At t0 + 100ms (only 60ms after last resize), resolution is still 1280x720,
        // while crop_x & zoom are ALREADY active instantaneously!
        let f_early = pipe
            .process_input_frame(&input, t0 + Duration::from_millis(100), false)
            .unwrap()
            .unwrap();
        assert_eq!(f_early.meta.width, 1280);
        assert_eq!(f_early.meta.height, 720);
        assert_eq!(pipe.reallocation_count(), 0);
        assert!((pipe.transform.crop_x - 0.2).abs() < 1e-5);
        assert!((pipe.transform.zoom - 1.5).abs() < 1e-5);

        // At t0 + 200ms (160ms after last resize >= 150ms debounce), single reallocation commits!
        let f_settled = pipe
            .process_input_frame(&input, t0 + Duration::from_millis(200), false)
            .unwrap()
            .unwrap();
        assert_eq!(f_settled.meta.width, 1280);
        assert_eq!(f_settled.meta.height, 360);
        assert_eq!(pipe.reallocation_count(), 1);
    }

    #[test]
    fn fps_throttler_paces_output_frames() {
        let mut throttler = FpsThrottler::new(10.0); // 100 ms per frame
        let t0 = Instant::now();
        assert!(throttler.should_emit(t0));
        assert!(!throttler.should_emit(t0 + Duration::from_millis(30)));
        assert!(!throttler.should_emit(t0 + Duration::from_millis(90)));
        assert!(throttler.should_emit(t0 + Duration::from_millis(105)));
        assert_eq!(throttler.emitted_frames(), 2);
    }

    #[test]
    fn live_control_signal_modulation_of_ndi_crop_zoom_opacity_and_resolution() {
        let cp = Arc::new(ControlPlane::new(WorkspaceGraph::new("NDI Mod")).unwrap());
        let telemetry = Arc::new(TelemetryAggregator::default());
        let router = SignalRouter::new(cp.compiled_handle(), telemetry);
        let video_mgr = VideoPipelineManager::new();

        let p_knob_x = Port::new("knob.x", "Pan Knob", PortDirection::Out, SignalKind::Control);
        let p_knob_zoom = Port::new("knob.zoom", "Zoom Knob", PortDirection::Out, SignalKind::Control);
        let p_knob_opacity = Port::new("knob.op", "Opacity", PortDirection::Out, SignalKind::Control);
        let p_knob_x_id = p_knob_x.id;
        let p_knob_zoom_id = p_knob_zoom.id;
        let p_knob_opacity_id = p_knob_opacity.id;

        let p_ndi_x = Port::new("ndi.crop_x", "Crop X", PortDirection::In, SignalKind::Control);
        let p_ndi_zoom = Port::new("ndi.zoom", "Zoom", PortDirection::In, SignalKind::Control);
        let p_ndi_opacity = Port::new("ndi.opacity", "Opacity", PortDirection::In, SignalKind::Control);
        let p_ndi_x_id = p_ndi_x.id;
        let p_ndi_zoom_id = p_ndi_zoom.id;
        let p_ndi_opacity_id = p_ndi_opacity.id;

        let _ctrl = cp
            .add_device(
                Device::new("ctrl.1", "MIDI/OSC Controller", DeviceKind::VirtualControl)
                    .with_port(p_knob_x)
                    .with_port(p_knob_zoom)
                    .with_port(p_knob_opacity),
            )
            .unwrap();
        let ndi = cp
            .add_device(
                Device::new(
                    "ndi.scaler",
                    "Main NDI Scaler",
                    DeviceKind::NdiVideoNode {
                        source_name: Some(SYNTHETIC_SMPTE_1080P.into()),
                        output_name: Some("Stage Out".into()),
                        out_width: 1280,
                        out_height: 360,
                        out_fps: 30.0,
                    },
                )
                .with_port(p_ndi_x)
                .with_port(p_ndi_zoom)
                .with_port(p_ndi_opacity),
            )
            .unwrap();

        cp.add_wire(p_knob_x_id, p_ndi_x_id).unwrap();
        cp.add_wire(p_knob_zoom_id, p_ndi_zoom_id).unwrap();
        cp.add_wire(p_knob_opacity_id, p_ndi_opacity_id).unwrap();

        // Configure crop_w=0.5 so crop_x can pan across 0.0..=0.5
        video_mgr.configure_node(
            ndi.id,
            NdiNodeConfigPatch {
                fit_mode: Some(VideoFitMode::Crop),
                crop_w: Some(0.5),
                crop_h: Some(0.5),
                ..Default::default()
            },
        );

        // Route live control signals through SignalRouter -> VideoPipelineManager
        let outs_x = router.route(SignalEvent::new(p_knob_x_id, SignalValue::Float(0.35), 100));
        let outs_z = router.route(SignalEvent::new(p_knob_zoom_id, SignalValue::Float(0.5), 200));
        let outs_o = router.route(SignalEvent::new(p_knob_opacity_id, SignalValue::Float(0.75), 300));

        for o in outs_x.iter().chain(outs_z.iter()).chain(outs_o.iter()) {
            assert!(video_mgr.apply_routed_output(o));
        }

        let st = video_mgr.get_node_state(ndi.id);
        assert!((st.transform.crop_x - 0.35).abs() < 1e-4);
        // Normalized zoom 0.5 maps to 1.0 + 0.5 * 3.0 = 2.5x
        assert!((st.transform.zoom - 2.5).abs() < 1e-4);
        assert!((st.transform.opacity - 0.75).abs() < 1e-4);

        // Preview frame renders cleanly with modulated parameters
        let preview = video_mgr.render_preview(ndi.id, 320, 180).unwrap();
        assert!(!preview.jpeg_base64.is_empty());
        assert!(preview.thumb_width <= 320);
    }
}
