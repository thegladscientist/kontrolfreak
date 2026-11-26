//! Milestone 3 seam crate for USB/Bluetooth HID hot-plug detection, 2-tier HID Learn Engine,
//! and RGB camera CV extractors.

pub use kf_core::{
    DeviceFingerprint, HidControlClass, HidDeviceInfo, HidLearnStep, LearnedHidMapping, SignalValue,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum HardwareError {
    #[error("Hardware backend not implemented yet (scheduled for Milestone 3)")]
    NotImplementedYet,
    #[error("OS permission required (e.g. macOS Input Monitoring): {0}")]
    PermissionRequired(String),
    #[error("Hardware error: {0}")]
    Backend(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HidReportSample {
    pub report_id: u8,
    pub bytes: Vec<u8>,
    pub timestamp_ns: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum HardwareHotplugEvent {
    Connected {
        info: HidDeviceInfo,
        fingerprint: DeviceFingerprint,
    },
    Disconnected {
        path: String,
        fingerprint: DeviceFingerprint,
    },
}

/// Hot-plug watcher seam (M3 implements with `hidapi` / `nusb`).
pub trait HardwareWatcher: Send + Sync {
    fn list_hid_devices(&self) -> Result<Vec<HidDeviceInfo>, HardwareError>;
    fn check_os_permissions(&self) -> Result<(), HardwareError>;
}

/// 2-Tier HID Learn Engine seam (M3 implements descriptor inspection + baseline diff capture).
pub trait HidLearnEngine: Send + Sync {
    fn step(&self) -> HidLearnStep;
    fn start_baseline(&mut self, device_path: &str) -> Result<(), HardwareError>;
    fn ingest_report(&mut self, sample: &HidReportSample) -> Result<(), HardwareError>;
    fn finalize_control(&mut self, alias: &str) -> Result<LearnedHidMapping, HardwareError>;
}

/// RGB camera CV extractor seam (M3 implements via `nokhwa`).
pub trait RgbCameraExtractor: Send + Sync {
    fn extractor_name(&self) -> &'static str;
    fn extract_signals(&mut self, rgba: &[u8], width: u32, height: u32) -> Vec<(String, SignalValue)>;
}

#[derive(Debug, Default)]
pub struct HardwareStub;

impl HardwareWatcher for HardwareStub {
    fn list_hid_devices(&self) -> Result<Vec<HidDeviceInfo>, HardwareError> {
        Ok(Vec::new())
    }

    fn check_os_permissions(&self) -> Result<(), HardwareError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hardware_stub_lists_empty_cleanly() {
        let watcher = HardwareStub;
        assert!(watcher.list_hid_devices().unwrap().is_empty());
        assert!(watcher.check_os_permissions().is_ok());
    }
}
