//! Milestone 2 seam crate for NDI dynamic runtime loading, GPU (`wgpu`) / CPU video scaling,
//! and low-res preview thumbnail streaming.

pub use kf_core::{
    AspectScaleMode, FrameRef, PixelFormat, VideoPreviewFrame, VideoResizeConfig, VideoSourceInfo,
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
            install_url: "https://ndi.video/tools/".to_string(),
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

/// Dynamic NDI runtime loader seam (M2 implements via `libloading`).
pub trait NdiRuntimeLoader: Send + Sync {
    fn probe_runtime(&self) -> NdiRuntimeStatus;
    fn discover_sources(&self) -> Result<Vec<VideoSourceInfo>, VideoError>;
}

/// Frame scaler seam (M2 implements `wgpu` primary + `fast_image_resize` fallback).
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

/// Default M1 stub loader cleanly reporting NDI availability without panicking.
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

/// Per-node runtime metadata stored by `src-tauri` for M2 NDI nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NdiNodeRuntimeStub {
    pub device_id: Uuid,
    pub config: VideoResizeConfig,
    pub runtime: NdiRuntimeStatus,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ndi_stub_reports_missing_runtime_cleanly() {
        let loader = NdiRuntimeStub;
        let status = loader.probe_runtime();
        assert!(!status.available);
        assert!(matches!(
            loader.discover_sources(),
            Err(VideoError::NdiRuntimeMissing)
        ));
    }
}
