//! Pluggable `DepthCameraBackend` trait with vendor adapter stubs/feature gates for
//! Intel RealSense D400, Orbbec Femto/Astra, and Azure Kinect, plus a default high-fidelity
//! `SyntheticDepthBackend` producing realistic animated 16-bit millimeter depth maps.

use kf_core::SignalEvent;
use serde::{Deserialize, Serialize};
use std::f32::consts::PI;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BackendError {
    #[error("Depth camera backend '{vendor}' unavailable: {reason}")]
    Unavailable { vendor: String, reason: String },
    #[error("Depth capture error: {0}")]
    Capture(String),
}

/// Raw 16-bit depth frame in millimeters (`0` = invalid/no return).
#[derive(Debug, Clone)]
pub struct DepthFrame {
    pub width: u32,
    pub height: u32,
    pub timestamp_ns: u64,
    pub depth_mm: Vec<u16>,
}

/// Metadata describing a depth camera adapter available to the sidecar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VendorAdapterInfo {
    pub vendor_id: String,
    pub display_name: String,
    pub sdk_compiled: bool,
    pub hardware_detected: bool,
    pub notes: String,
}

/// Pluggable backend interface implemented by hardware SDK adapters and the synthetic generator.
pub trait DepthCameraBackend: Send {
    fn vendor_id(&self) -> &'static str;
    fn display_name(&self) -> &'static str;
    fn is_hardware_available(&self) -> bool;
    fn resolution(&self) -> (u32, u32);
    fn next_frame(&mut self, frame_index: u64) -> Result<DepthFrame, BackendError>;
}

/// High-fidelity synthetic 3D depth scene generator producing moving 3D objects,
/// a performer hand/orb sweeping through space, and subtle depth waves in millimeters.
pub struct SyntheticDepthBackend {
    width: u32,
    height: u32,
    vendor_tag: &'static str,
    phase_step: f32,
}

impl SyntheticDepthBackend {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width: width.max(16),
            height: height.max(16),
            vendor_tag: "synthetic",
            phase_step: 0.065,
        }
    }

    pub fn with_vendor_tag(mut self, vendor_tag: &'static str) -> Self {
        self.vendor_tag = vendor_tag;
        self
    }

    /// Generates a deterministic 16-bit millimeter depth map for the given phase `t` (radians).
    pub fn render_scene_at_phase(&self, t: f32) -> Vec<u16> {
        let w = self.width as usize;
        let h = self.height as usize;
        let mut buf = vec![0u16; w * h];

        // Primary 3D foreground sphere ("hand / performer"): orbits smoothly in X, Y, Z.
        let orb1_cx = 0.50 + 0.24 * (t * 0.9).cos();
        let orb1_cy = 0.48 + 0.18 * (t * 1.3).sin();
        let orb1_z = 920.0 + 360.0 * (t * 0.7).sin(); // 560mm .. 1280mm
        let orb1_r = 0.155_f32;

        // Secondary 3D object ("second hand / prop"): counter-orbit slightly deeper.
        let orb2_cx = 0.50 - 0.28 * (t * 0.65).sin();
        let orb2_cy = 0.55 + 0.20 * (t * 0.85).cos();
        let orb2_z = 1480.0 + 300.0 * (t * 0.5 + 1.2).cos();
        let orb2_r = 0.125_f32;

        let inv_w = 1.0 / (w.saturating_sub(1).max(1) as f32);
        let inv_h = 1.0 / (h.saturating_sub(1).max(1) as f32);

        for py in 0..h {
            let yn = py as f32 * inv_h;
            let row_off = py * w;
            for px in 0..w {
                let xn = px as f32 * inv_w;

                // Subtle sloped studio backdrop + gentle traveling wave (2450..2950mm)
                let wave = 75.0 * (xn * 2.0 * PI * 1.5 + t).sin() * (yn * PI).sin();
                let mut z = 2680.0 + (yn - 0.5) * 260.0 + wave;

                // Primary 3D sphere hemisphere
                let dx1 = xn - orb1_cx;
                let dy1 = (yn - orb1_cy) * (h as f32 / w as f32);
                let d1_sq = dx1 * dx1 + dy1 * dy1;
                if d1_sq < orb1_r * orb1_r {
                    let bulge = ((orb1_r * orb1_r - d1_sq).sqrt() / orb1_r) * 210.0;
                    let surface_z = (orb1_z - bulge).max(320.0);
                    if surface_z < z {
                        z = surface_z;
                    }
                }

                // Secondary 3D sphere hemisphere
                let dx2 = xn - orb2_cx;
                let dy2 = (yn - orb2_cy) * (h as f32 / w as f32);
                let d2_sq = dx2 * dx2 + dy2 * dy2;
                if d2_sq < orb2_r * orb2_r {
                    let bulge = ((orb2_r * orb2_r - d2_sq).sqrt() / orb2_r) * 165.0;
                    let surface_z = (orb2_z - bulge).max(400.0);
                    if surface_z < z {
                        z = surface_z;
                    }
                }

                // Tiny border invalid-depth edge (2px) mimicking real IR stereo occlusion borders
                if px == 0 || py == 0 {
                    buf[row_off + px] = 0;
                } else {
                    buf[row_off + px] = z.clamp(200.0, 6000.0) as u16;
                }
            }
        }
        buf
    }
}

impl DepthCameraBackend for SyntheticDepthBackend {
    fn vendor_id(&self) -> &'static str {
        self.vendor_tag
    }

    fn display_name(&self) -> &'static str {
        "Synthetic 3D Depth Simulator"
    }

    fn is_hardware_available(&self) -> bool {
        true
    }

    fn resolution(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn next_frame(&mut self, frame_index: u64) -> Result<DepthFrame, BackendError> {
        let phase = frame_index as f32 * self.phase_step;
        let depth_mm = self.render_scene_at_phase(phase);
        Ok(DepthFrame {
            width: self.width,
            height: self.height,
            timestamp_ns: SignalEvent::now_ns(),
            depth_mm,
        })
    }
}

/// Intel RealSense D400 series (`librealsense2`) adapter stub with feature gate & synthetic fallback.
pub struct RealSenseBackend {
    width: u32,
    height: u32,
    fallback: Option<SyntheticDepthBackend>,
}

impl RealSenseBackend {
    pub fn new(width: u32, height: u32, allow_fallback: bool) -> Self {
        Self {
            width,
            height,
            fallback: allow_fallback
                .then(|| SyntheticDepthBackend::new(width, height).with_vendor_tag("realsense")),
        }
    }

    pub fn sdk_compiled() -> bool {
        cfg!(feature = "realsense")
    }
}

impl DepthCameraBackend for RealSenseBackend {
    fn vendor_id(&self) -> &'static str {
        "realsense"
    }

    fn display_name(&self) -> &'static str {
        "Intel RealSense D400 (librealsense2)"
    }

    fn is_hardware_available(&self) -> bool {
        false
    }

    fn resolution(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn next_frame(&mut self, frame_index: u64) -> Result<DepthFrame, BackendError> {
        if let Some(ref mut fb) = self.fallback {
            return fb.next_frame(frame_index);
        }
        Err(BackendError::Unavailable {
            vendor: "realsense".into(),
            reason: "No RealSense D400 hardware detected (enable `realsense` feature with librealsense2)".into(),
        })
    }
}

/// Orbbec Femto Bolt / Astra 2 adapter stub with feature gate & synthetic fallback.
pub struct OrbbecBackend {
    width: u32,
    height: u32,
    fallback: Option<SyntheticDepthBackend>,
}

impl OrbbecBackend {
    pub fn new(width: u32, height: u32, allow_fallback: bool) -> Self {
        Self {
            width,
            height,
            fallback: allow_fallback
                .then(|| SyntheticDepthBackend::new(width, height).with_vendor_tag("orbbec")),
        }
    }

    pub fn sdk_compiled() -> bool {
        cfg!(feature = "orbbec")
    }
}

impl DepthCameraBackend for OrbbecBackend {
    fn vendor_id(&self) -> &'static str {
        "orbbec"
    }

    fn display_name(&self) -> &'static str {
        "Orbbec Femto / Astra (OrbbecSDK)"
    }

    fn is_hardware_available(&self) -> bool {
        false
    }

    fn resolution(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn next_frame(&mut self, frame_index: u64) -> Result<DepthFrame, BackendError> {
        if let Some(ref mut fb) = self.fallback {
            return fb.next_frame(frame_index);
        }
        Err(BackendError::Unavailable {
            vendor: "orbbec".into(),
            reason: "No Orbbec Femto/Astra device detected (enable `orbbec` feature with OrbbecSDK)".into(),
        })
    }
}

/// Microsoft Azure Kinect DK / K4A adapter stub with feature gate & synthetic fallback.
pub struct AzureKinectBackend {
    width: u32,
    height: u32,
    fallback: Option<SyntheticDepthBackend>,
}

impl AzureKinectBackend {
    pub fn new(width: u32, height: u32, allow_fallback: bool) -> Self {
        Self {
            width,
            height,
            fallback: allow_fallback
                .then(|| SyntheticDepthBackend::new(width, height).with_vendor_tag("kinect")),
        }
    }

    pub fn sdk_compiled() -> bool {
        cfg!(feature = "kinect")
    }
}

impl DepthCameraBackend for AzureKinectBackend {
    fn vendor_id(&self) -> &'static str {
        "kinect"
    }

    fn display_name(&self) -> &'static str {
        "Azure Kinect DK (k4a)"
    }

    fn is_hardware_available(&self) -> bool {
        false
    }

    fn resolution(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn next_frame(&mut self, frame_index: u64) -> Result<DepthFrame, BackendError> {
        if let Some(ref mut fb) = self.fallback {
            return fb.next_frame(frame_index);
        }
        Err(BackendError::Unavailable {
            vendor: "kinect".into(),
            reason: "No Azure Kinect DK detected (enable `kinect` feature with k4a SDK)".into(),
        })
    }
}

/// Instantiates the requested vendor depth backend (falling back to synthetic simulation when allowed).
pub fn create_backend(
    vendor: &str,
    width: u32,
    height: u32,
    allow_synthetic_fallback: bool,
) -> Box<dyn DepthCameraBackend> {
    match vendor.to_ascii_lowercase().as_str() {
        "realsense" | "realsense-d400" | "d435" => {
            Box::new(RealSenseBackend::new(width, height, allow_synthetic_fallback))
        }
        "orbbec" | "femto" | "astra" => {
            Box::new(OrbbecBackend::new(width, height, allow_synthetic_fallback))
        }
        "kinect" | "azure-kinect" | "k4a" => {
            Box::new(AzureKinectBackend::new(width, height, allow_synthetic_fallback))
        }
        _ => Box::new(SyntheticDepthBackend::new(width, height)),
    }
}

pub fn list_vendor_adapters() -> Vec<VendorAdapterInfo> {
    vec![
        VendorAdapterInfo {
            vendor_id: "synthetic".into(),
            display_name: "Synthetic 3D Depth Simulator".into(),
            sdk_compiled: true,
            hardware_detected: true,
            notes: "Built-in animated 3D depth generator (30-60 Hz) for testing without physical cameras.".into(),
        },
        VendorAdapterInfo {
            vendor_id: "realsense".into(),
            display_name: "Intel RealSense D400".into(),
            sdk_compiled: RealSenseBackend::sdk_compiled(),
            hardware_detected: false,
            notes: "Isolated sidecar adapter for librealsense2 (falls back to synthetic stream if disconnected).".into(),
        },
        VendorAdapterInfo {
            vendor_id: "orbbec".into(),
            display_name: "Orbbec Femto / Astra".into(),
            sdk_compiled: OrbbecBackend::sdk_compiled(),
            hardware_detected: false,
            notes: "Isolated sidecar adapter for OrbbecSDK (falls back to synthetic stream if disconnected).".into(),
        },
        VendorAdapterInfo {
            vendor_id: "kinect".into(),
            display_name: "Azure Kinect DK".into(),
            sdk_compiled: AzureKinectBackend::sdk_compiled(),
            hardware_detected: false,
            notes: "Isolated sidecar adapter for Azure Kinect k4a (falls back to synthetic stream if disconnected).".into(),
        },
    ]
}
