//! Milestone 4 seam crate for out-of-process depth camera sensor sidecars
//! (Named Pipe / Unix Socket control IPC + zero-copy shared-memory ring buffer).

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
}

/// Fixed-layout shared-memory ring buffer header for zero-copy 16-bit depth + RGB frame handoff.
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
    StopStream {
        device_id: Uuid,
    },
    HealthPing,
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
    DepthSample(DepthSampleSummary),
    Error {
        message: String,
    },
}

/// Out-of-process Sensor Sidecar client seam (M4 implements with `interprocess` + `memmap2`).
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidecar_protocol_messages_roundtrip_json() {
        let req = SidecarRequest::StartStream {
            device_id: Uuid::new_v4(),
            vendor: "realsense".into(),
            roi: DepthRoiConfig::default(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: SidecarRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, back);
        assert_eq!(ShmFrameHeader::MAGIC, 0x4B46_4431);
    }
}
