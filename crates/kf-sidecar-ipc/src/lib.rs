//! Milestone 4 crate for crash-isolated out-of-process depth camera sensor sidecars:
//! - Cross-platform local control IPC (`interprocess` Unix sockets / Windows named pipes)
//! - Zero-copy file-backed shared-memory ring buffer (`memmap2`) with atomic sequence counters
//! - `SidecarSupervisor` with heartbeat monitoring, crash isolation, and exponential backoff restart
//! - Portable `.kfreak` ZIP project bundle exporter/importer with SHA-256 manifest verification
//! - Real-time Diagnostics & Performance Profiler engine

pub mod bundle;
pub mod diagnostics;
pub mod ipc;
pub mod protocol;
pub mod shm;
pub mod supervisor;

pub use bundle::{
    export_kfreak_bytes, export_kfreak_file, import_kfreak_bytes, import_kfreak_file, sha256_hex,
    BundleError, KfreakBundle, KfreakManifest, KFREAK_BUNDLE_VERSION, MANIFEST_PATH,
    PROJECT_ENTRY_PATH,
};
pub use diagnostics::{
    DiagnosticsProfiler, DiagnosticsSnapshot, LatencyPercentilesMs, ProtocolRateEntry,
    SidecarDiagnosticEntry,
};
pub use ipc::{
    cleanup_socket_endpoint, read_framed_json, to_socket_name, write_framed_json, IpcClient,
    IpcServer,
};
pub use protocol::{
    DepthColormap, DepthRoiConfig, DepthSampleSummary, NamedDepthRoi, PixelFormat,
    RoiExtractionResult, ShmFrameHeader, SidecarClient, SidecarClientStub, SidecarError,
    SidecarHealthStatus, SidecarRequest, SidecarResponse, SIDECAR_PROTOCOL_VERSION,
};
pub use shm::{ShmRingHealth, ShmRingReader, ShmRingWriter};
pub use supervisor::{
    locate_depth_sidecar_binary, DepthFrameSnapshot, FrameCallback, SidecarSupervisor,
    SidecarSupervisorSnapshot, StatusCallback, SupervisorConfig,
};

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

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
