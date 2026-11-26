//! RGB Camera backend trait, deterministic synthetic animated RGB camera source,
//! and cleanly gated `nokhwa` native capture hook for desktop OS targets.

use crate::HardwareError;
use kf_core::SignalEvent;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RgbCameraInfo {
    pub id: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub is_synthetic: bool,
}

#[derive(Debug, Clone)]
pub struct RgbFrame {
    pub width: u32,
    pub height: u32,
    /// Tightly packed RGBA8 pixel data (`width * height * 4` bytes).
    pub rgba: Vec<u8>,
    pub timestamp_ns: u64,
    pub frame_index: u64,
}

/// Cross-platform RGB camera capture backend.
pub trait RgbCameraBackend: Send + Sync {
    fn list_cameras(&self) -> Result<Vec<RgbCameraInfo>, HardwareError>;
    fn capture_frame(&self, camera_id: &str) -> Result<RgbFrame, HardwareError>;
}

/// Configurable synthetic scene parameters for deterministic CV testing and live demos.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyntheticSceneParams {
    pub width: u32,
    pub height: u32,
    /// Normalized background luma (0.0..=1.0)
    pub bg_brightness: f32,
    /// Moving colored target orb RGB (0..=255)
    pub orb_rgb: [u8; 3],
    /// Orb radius as fraction of min(width, height)
    pub orb_radius_norm: f32,
    /// Optional fixed normalized orb position `[x, y]`; when `None`, animates smoothly along a Lissajous path.
    pub fixed_orb_pos: Option<[f32; 2]>,
    /// Optional fixed bright highlight `[x, y]` for brightest-point tracking tests.
    pub bright_spot_pos: Option<[f32; 2]>,
}

impl Default for SyntheticSceneParams {
    fn default() -> Self {
        Self {
            width: 160,
            height: 120,
            bg_brightness: 0.14,
            orb_rgb: [30, 235, 165], // Vibrant emerald-cyan orb (~158 deg hue)
            orb_radius_norm: 0.14,
            fixed_orb_pos: None,
            bright_spot_pos: None,
        }
    }
}

/// Deterministic animated synthetic RGB camera source.
/// Works identically in headless Linux CI, unit tests, and browser/desktop demos.
pub struct SyntheticRgbCamera {
    info: RgbCameraInfo,
    params: Mutex<SyntheticSceneParams>,
    frame_counter: Mutex<u64>,
}

impl SyntheticRgbCamera {
    pub fn new(id: impl Into<String>, name: impl Into<String>, params: SyntheticSceneParams) -> Self {
        let info = RgbCameraInfo {
            id: id.into(),
            name: name.into(),
            width: params.width,
            height: params.height,
            fps: 30,
            is_synthetic: true,
        };
        Self {
            info,
            params: Mutex::new(params),
            frame_counter: Mutex::new(0),
        }
    }

    pub fn default_demo() -> Self {
        Self::new(
            "sim://camera/studio-cv",
            "Synthetic Studio RGB Camera",
            SyntheticSceneParams::default(),
        )
    }

    pub fn set_params(&self, next: SyntheticSceneParams) {
        *self.params.lock() = next;
    }

    pub fn params(&self) -> SyntheticSceneParams {
        self.params.lock().clone()
    }

    pub fn info(&self) -> RgbCameraInfo {
        self.info.clone()
    }

    /// Renders a specific deterministic frame index without mutating internal counter.
    pub fn render_frame(&self, frame_index: u64) -> RgbFrame {
        let p = self.params.lock().clone();
        render_synthetic_frame(&p, frame_index)
    }

    /// Advances frame index by 1 and renders the next RGBA8 frame.
    pub fn next_frame(&self) -> RgbFrame {
        let mut idx = self.frame_counter.lock();
        let cur = *idx;
        *idx = cur.wrapping_add(1);
        drop(idx);
        self.render_frame(cur)
    }
}

/// Pure deterministic synthetic RGBA8 frame generator.
pub fn render_synthetic_frame(p: &SyntheticSceneParams, frame_index: u64) -> RgbFrame {
    let w = p.width.max(8);
    let h = p.height.max(8);
    let mut rgba = vec![0u8; (w * h * 4) as usize];

    let t = frame_index as f32 * 0.14;
    let [orb_x_norm, orb_y_norm] = p.fixed_orb_pos.unwrap_or_else(|| {
        [
            0.5 + 0.34 * (t * 0.9).sin(),
            0.5 + 0.30 * (t * 1.35 + 0.6).cos(),
        ]
    });

    let orb_cx = orb_x_norm.clamp(0.05, 0.95) * (w as f32 - 1.0);
    let orb_cy = orb_y_norm.clamp(0.05, 0.95) * (h as f32 - 1.0);
    let radius = (p.orb_radius_norm * w.min(h) as f32).max(2.0);
    let r2 = radius * radius;

    let bright_spot = p.bright_spot_pos.map(|[bx, by]| {
        (
            bx.clamp(0.0, 1.0) * (w as f32 - 1.0),
            by.clamp(0.0, 1.0) * (h as f32 - 1.0),
            (w.min(h) as f32 * 0.05).max(2.0),
        )
    });

    let bg_base = (p.bg_brightness.clamp(0.0, 1.0) * 255.0) as f32;

    for y in 0..h {
        let fy = y as f32;
        for x in 0..w {
            let fx = x as f32;
            let idx = ((y * w + x) * 4) as usize;

            // Subtle dark studio gradient background
            let grad = 1.0 + 0.08 * (fy / h as f32);
            let mut r = (bg_base * grad).clamp(0.0, 255.0);
            let mut g = (bg_base * grad * 1.04).clamp(0.0, 255.0);
            let mut b = (bg_base * grad * 1.15).clamp(0.0, 255.0);

            // Moving colored target orb
            let dx = fx - orb_cx;
            let dy = fy - orb_cy;
            let dist2 = dx * dx + dy * dy;
            if dist2 <= r2 {
                let falloff = (1.0 - (dist2 / r2) * 0.25).clamp(0.0, 1.0);
                r = p.orb_rgb[0] as f32 * falloff;
                g = p.orb_rgb[1] as f32 * falloff;
                b = p.orb_rgb[2] as f32 * falloff;
            }

            // Optional pure-white brightest highlight
            if let Some((bx, by, br)) = bright_spot {
                let bdx = fx - bx;
                let bdy = fy - by;
                if bdx * bdx + bdy * bdy <= br * br {
                    r = 255.0;
                    g = 255.0;
                    b = 255.0;
                }
            }

            rgba[idx] = r.round().clamp(0.0, 255.0) as u8;
            rgba[idx + 1] = g.round().clamp(0.0, 255.0) as u8;
            rgba[idx + 2] = b.round().clamp(0.0, 255.0) as u8;
            rgba[idx + 3] = 255;
        }
    }

    RgbFrame {
        width: w,
        height: h,
        rgba,
        timestamp_ns: SignalEvent::now_ns(),
        frame_index,
    }
}

impl RgbCameraBackend for SyntheticRgbCamera {
    fn list_cameras(&self) -> Result<Vec<RgbCameraInfo>, HardwareError> {
        Ok(vec![self.info.clone()])
    }

    fn capture_frame(&self, camera_id: &str) -> Result<RgbFrame, HardwareError> {
        if camera_id != self.info.id && camera_id != "default" {
            return Err(HardwareError::Backend(format!(
                "Unknown synthetic camera id: {camera_id}"
            )));
        }
        Ok(self.next_frame())
    }
}

/// Optional native desktop RGB camera backend using `nokhwa`, gated behind `native-camera`
/// so headless Linux builds compile cleanly without V4L2/clang/AVFoundation native toolchains.
#[cfg(all(
    feature = "native-camera",
    any(target_os = "windows", target_os = "macos", target_os = "linux")
))]
pub struct NokhwaCameraBackend;

#[cfg(all(
    feature = "native-camera",
    any(target_os = "windows", target_os = "macos", target_os = "linux")
))]
impl RgbCameraBackend for NokhwaCameraBackend {
    fn list_cameras(&self) -> Result<Vec<RgbCameraInfo>, HardwareError> {
        let backend = nokhwa::native_api_backend()
            .ok_or_else(|| HardwareError::Backend("No native camera API backend available".into()))?;
        let devices = nokhwa::query(backend)
            .map_err(|e| HardwareError::Backend(format!("nokhwa query failed: {e}")))?;
        Ok(devices
            .into_iter()
            .enumerate()
            .map(|(i, info)| RgbCameraInfo {
                id: format!("native://camera/{i}"),
                name: info.human_name(),
                width: 640,
                height: 480,
                fps: 30,
                is_synthetic: false,
            })
            .collect())
    }

    fn capture_frame(&self, camera_id: &str) -> Result<RgbFrame, HardwareError> {
        let idx: u32 = camera_id
            .rsplit('/')
            .next()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let requested = nokhwa::utils::RequestedFormat::new::<nokhwa::pixel_format::RgbAFormat>(
            nokhwa::utils::RequestedFormatType::AbsoluteHighestFrameRate,
        );
        let mut cam = nokhwa::Camera::new(nokhwa::utils::CameraIndex::Index(idx), requested)
            .map_err(|e| HardwareError::Backend(format!("nokhwa open failed: {e}")))?;
        cam.open_stream()
            .map_err(|e| HardwareError::Backend(format!("nokhwa stream failed: {e}")))?;
        let frame = cam
            .frame()
            .map_err(|e| HardwareError::Backend(format!("nokhwa frame failed: {e}")))?;
        let decoded = frame
            .decode_image::<nokhwa::pixel_format::RgbAFormat>()
            .map_err(|e| HardwareError::Backend(format!("nokhwa decode failed: {e}")))?;
        let (width, height) = decoded.dimensions();
        Ok(RgbFrame {
            width,
            height,
            rgba: decoded.into_raw(),
            timestamp_ns: SignalEvent::now_ns(),
            frame_index: 0,
        })
    }
}
