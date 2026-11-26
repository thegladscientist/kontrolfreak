pub use kf_core::{DepthRoiConfig, DepthSampleSummary, PixelFormat, SidecarHealthStatus};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

pub const SIDECAR_PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum SidecarError {
    #[error("Sensor sidecar runtime not implemented yet (scheduled for Milestone 4)")]
    NotImplementedYet,
    #[error("Sidecar disconnected: {0}")]
    Disconnected(String),
    #[error("IPC protocol error: {0}")]
    Protocol(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Shared-memory ring buffer error: {0}")]
    Shm(String),
    #[error("Failed to launch sidecar process: {0}")]
    SpawnFailed(String),
    #[error("Sidecar operation timed out: {0}")]
    Timeout(String),
}

/// Fixed-layout shared-memory frame header for zero-copy 16-bit depth + RGB frame handoff.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShmFrameHeader {
    pub magic: u32,
    pub protocol_version: u32,
    pub frame_seq: u64,
    pub timestamp_ns: u64,
    pub width: u32,
    pub height: u32,
    pub stride_bytes: u32,
    pub pixel_format_code: u32,
}

impl ShmFrameHeader {
    pub const MAGIC: u32 = 0x4B46_4431; // "KFD1"

    pub fn new(
        frame_seq: u64,
        timestamp_ns: u64,
        width: u32,
        height: u32,
        stride_bytes: u32,
        format: PixelFormat,
    ) -> Self {
        Self {
            magic: Self::MAGIC,
            protocol_version: SIDECAR_PROTOCOL_VERSION,
            frame_seq,
            timestamp_ns,
            width,
            height,
            stride_bytes,
            pixel_format_code: Self::code_from_format(format),
        }
    }

    pub fn code_from_format(format: PixelFormat) -> u32 {
        match format {
            PixelFormat::Rgba8 => 1,
            PixelFormat::Bgra8 => 2,
            PixelFormat::Uyvy => 3,
            PixelFormat::Nv12 => 4,
            PixelFormat::Depth16 => 5,
            PixelFormat::Gray8 => 6,
        }
    }

    pub fn pixel_format(&self) -> Option<PixelFormat> {
        match self.pixel_format_code {
            1 => Some(PixelFormat::Rgba8),
            2 => Some(PixelFormat::Bgra8),
            3 => Some(PixelFormat::Uyvy),
            4 => Some(PixelFormat::Nv12),
            5 => Some(PixelFormat::Depth16),
            6 => Some(PixelFormat::Gray8),
            _ => None,
        }
    }

    pub fn payload_bytes(&self) -> usize {
        (self.stride_bytes as usize).saturating_mul(self.height as usize)
    }
}

/// Depth colormap palette for Depth-to-RGBA / Depth-to-NDI visualization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DepthColormap {
    Turbo,
    Viridis,
    Grayscale,
    DepthSlice,
}

impl Default for DepthColormap {
    fn default() -> Self {
        Self::Turbo
    }
}

/// Configurable 3D Region of Interest in normalized image space `(x, y, w, h)` + millimeter Z range.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NamedDepthRoi {
    pub id: String,
    pub name: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub near_mm: u16,
    pub far_mm: u16,
    #[serde(default = "default_occupancy_threshold")]
    pub occupancy_threshold: f32,
}

fn default_occupancy_threshold() -> f32 {
    0.05
}

impl Default for NamedDepthRoi {
    fn default() -> Self {
        Self {
            id: "roi-main".into(),
            name: "Main 3D Zone".into(),
            x: 0.15,
            y: 0.15,
            w: 0.70,
            h: 0.70,
            near_mm: 300,
            far_mm: 2500,
            occupancy_threshold: 0.05,
        }
    }
}

impl From<&DepthRoiConfig> for NamedDepthRoi {
    fn from(cfg: &DepthRoiConfig) -> Self {
        let x = cfg.x_min.clamp(0.0, 1.0);
        let y = cfg.y_min.clamp(0.0, 1.0);
        let w = (cfg.x_max - cfg.x_min).clamp(0.01, 1.0 - x);
        let h = (cfg.y_max - cfg.y_min).clamp(0.01, 1.0 - y);
        let near_mm = (cfg.min_z_meters.max(0.05) * 1000.0).round() as u16;
        let far_mm = (cfg.max_z_meters.max(cfg.min_z_meters + 0.05) * 1000.0).round() as u16;
        Self {
            id: "roi-main".into(),
            name: "Primary ROI".into(),
            x,
            y,
            w,
            h,
            near_mm,
            far_mm,
            occupancy_threshold: 0.05,
        }
    }
}

impl NamedDepthRoi {
    pub fn to_depth_roi_config(&self) -> DepthRoiConfig {
        DepthRoiConfig {
            x_min: self.x.clamp(0.0, 1.0),
            x_max: (self.x + self.w).clamp(0.0, 1.0),
            y_min: self.y.clamp(0.0, 1.0),
            y_max: (self.y + self.h).clamp(0.0, 1.0),
            min_z_meters: self.near_mm as f32 / 1000.0,
            max_z_meters: self.far_mm.max(self.near_mm.saturating_add(10)) as f32 / 1000.0,
        }
    }
}

/// Real-time 3D measurements extracted from a single `NamedDepthRoi` on a depth frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoiExtractionResult {
    pub roi_id: String,
    pub roi_name: String,
    pub min_depth_mm: u16,
    pub max_depth_mm: u16,
    pub mean_depth_mm: f32,
    pub presence: bool,
    pub fill_ratio: f32,
    pub valid_pixel_count: u32,
    pub total_pixel_count: u32,
    /// `[x_norm, y_norm, z_norm]` all in `0.0..=1.0` (z_norm = 0 at near_mm, 1 at far_mm).
    pub closest_point_norm: [f64; 3],
    /// Metric 3D coordinates `[x_m, y_m, z_m]` in camera space (meters).
    pub closest_point_meters: [f64; 3],
    /// Normalized 3D blob centroid `[x_norm, y_norm, z_norm]`.
    pub centroid_norm: [f64; 3],
    /// Metric 3D blob centroid `[x_m, y_m, z_m]` in meters.
    pub centroid_meters: [f64; 3],
    pub confidence: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum SidecarRequest {
    Hello {
        protocol_version: u32,
    },
    StartStream {
        device_id: Uuid,
        vendor: String,
        roi: DepthRoiConfig,
    },
    UpdateRoi {
        device_id: Uuid,
        roi: DepthRoiConfig,
    },
    UpdateRois {
        device_id: Uuid,
        rois: Vec<NamedDepthRoi>,
    },
    ConfigureColormap {
        device_id: Uuid,
        palette: DepthColormap,
    },
    StopStream {
        device_id: Uuid,
    },
    HealthPing,
    SimulateCrash,
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "resp", rename_all = "snake_case")]
pub enum SidecarResponse {
    HelloAck {
        protocol_version: u32,
        sidecar_name: String,
    },
    Status {
        health: SidecarHealthStatus,
        active_streams: usize,
    },
    Pong {
        uptime_ms: u64,
        pid: u32,
        frames_produced: u64,
    },
    StreamStarted {
        device_id: Uuid,
        shm_path: String,
        width: u32,
        height: u32,
        fps: u32,
    },
    StreamStopped {
        device_id: Uuid,
    },
    RoiUpdated {
        device_id: Uuid,
        roi_count: usize,
    },
    DepthSample(DepthSampleSummary),
    MultiRoiSamples {
        device_id: Uuid,
        frame_seq: u64,
        timestamp_ms: u64,
        samples: Vec<RoiExtractionResult>,
    },
    Error {
        message: String,
    },
}

/// Out-of-process Sensor Sidecar client seam.
pub trait SidecarClient: Send + Sync {
    fn status(&self) -> SidecarHealthStatus;
    fn send_request(&self, req: SidecarRequest) -> Result<SidecarResponse, SidecarError>;
}

#[derive(Debug, Default)]
pub struct SidecarClientStub;

impl SidecarClient for SidecarClientStub {
    fn status(&self) -> SidecarHealthStatus {
        SidecarHealthStatus::Stopped
    }

    fn send_request(&self, _req: SidecarRequest) -> Result<SidecarResponse, SidecarError> {
        Err(SidecarError::NotImplementedYet)
    }
}
