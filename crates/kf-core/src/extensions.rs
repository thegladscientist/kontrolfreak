use crate::signal::PixelFormat;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Shared extension capabilities exposed across M2, M3, and M4 crates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionCapability {
    RtpMidi,
    NdiVideo,
    HidLearn,
    RgbCamera,
    DepthSidecar,
}

// ---------- M2: Video / NDI Shared DTOs ----------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoSourceInfo {
    pub name: String,
    pub url_address: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AspectScaleMode {
    Fit,
    Fill,
    Stretch,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoResizeConfig {
    pub crop_x: f32,
    pub crop_y: f32,
    pub crop_w: f32,
    pub crop_h: f32,
    pub out_width: u32,
    pub out_height: u32,
    pub out_fps: f32,
    pub scale_mode: AspectScaleMode,
    pub freeze: bool,
    pub bypass: bool,
}

impl Default for VideoResizeConfig {
    fn default() -> Self {
        Self {
            crop_x: 0.0,
            crop_y: 0.0,
            crop_w: 1.0,
            crop_h: 1.0,
            out_width: 1280,
            out_height: 720,
            out_fps: 30.0,
            scale_mode: AspectScaleMode::Fit,
            freeze: false,
            bypass: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoPreviewFrame {
    pub device_id: Uuid,
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
    pub jpeg_bytes: Vec<u8>,
    pub timestamp_ms: u64,
}

// ---------- M3: Hardware / HID Learn Shared DTOs ----------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HidDeviceInfo {
    pub path: String,
    pub vid: u16,
    pub pid: u16,
    pub serial: Option<String>,
    pub product_name: Option<String>,
    pub manufacturer: Option<String>,
    pub usage_page: u16,
    pub usage: u16,
    pub interface_number: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HidControlClass {
    Button,
    Toggle,
    AbsoluteAxis,
    RelativeEncoder,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LearnedHidMapping {
    pub logical_key: String,
    pub alias: String,
    pub report_id: u8,
    pub byte_offset: usize,
    pub bit_mask: u8,
    pub bit_width: u8,
    pub signed: bool,
    pub little_endian: bool,
    pub control_class: HidControlClass,
    pub observed_min: i64,
    pub observed_max: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HidLearnStep {
    Idle,
    CapturingBaseline,
    RecordingControl,
    ReviewAndTune,
    Completed,
}

// ---------- M4: Depth Sensor Sidecar Shared DTOs ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SidecarHealthStatus {
    Stopped,
    Starting,
    Running,
    Degraded,
    CrashedRestarting,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DepthRoiConfig {
    pub x_min: f32,
    pub x_max: f32,
    pub y_min: f32,
    pub y_max: f32,
    pub min_z_meters: f32,
    pub max_z_meters: f32,
}

impl Default for DepthRoiConfig {
    fn default() -> Self {
        Self {
            x_min: 0.0,
            x_max: 1.0,
            y_min: 0.0,
            y_max: 1.0,
            min_z_meters: 0.3,
            max_z_meters: 3.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DepthSampleSummary {
    pub device_id: Uuid,
    pub closest_point: [f64; 3],
    pub closest_confidence: f64,
    pub centroid: [f64; 3],
    pub foreground_mass: f64,
    pub timestamp_ms: u64,
}
