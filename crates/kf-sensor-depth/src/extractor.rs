//! Real-time 3D depth parameter extractors turning raw 16-bit millimeter depth buffers
//! into hackable `kf-core` signals (min/max/mean depth, presence/occupancy & fill ratio,
//! closest-point XYZ, and 3D blob centroid tracking across multiple configurable ROIs).

use kf_core::{DepthSampleSummary, SignalValue};
use kf_sidecar_ipc::{NamedDepthRoi, RoiExtractionResult};
use uuid::Uuid;

/// Pinhole camera field-of-view parameters used for unprojecting `(u, v, depth_mm)` to metric XYZ.
#[derive(Debug, Clone, Copy)]
pub struct CameraIntrinsics {
    pub hfov_deg: f64,
    pub vfov_deg: f64,
}

impl Default for CameraIntrinsics {
    fn default() -> Self {
        // Matches Intel RealSense D435 depth sensor FOV
        Self {
            hfov_deg: 69.4,
            vfov_deg: 42.5,
        }
    }
}

impl CameraIntrinsics {
    /// Unprojects normalized image coordinates `(u, v)` in `0..=1` and millimeter depth `z_mm`
    /// into camera-space metric coordinates `[x_m, y_m, z_m]`.
    pub fn unproject_meters(&self, u: f64, v: f64, z_mm: f64) -> [f64; 3] {
        let z_m = z_mm / 1000.0;
        let half_w = (self.hfov_deg.to_radians() * 0.5).tan();
        let half_h = (self.vfov_deg.to_radians() * 0.5).tan();
        let x_m = (u * 2.0 - 1.0) * half_w * z_m;
        let y_m = (0.5 - v) * 2.0 * half_h * z_m; // +Y up
        [x_m, y_m, z_m]
    }
}

/// Extracts 3D depth measurements for a single `NamedDepthRoi` from a 16-bit millimeter depth frame.
pub fn extract_roi(
    depth_mm: &[u16],
    width: u32,
    height: u32,
    roi: &NamedDepthRoi,
    intrinsics: CameraIntrinsics,
) -> RoiExtractionResult {
    let w = width as usize;
    let h = height as usize;
    if w == 0 || h == 0 || depth_mm.len() < w * h {
        return empty_result(roi);
    }

    let x0 = ((roi.x.clamp(0.0, 1.0)) * w as f32).floor() as usize;
    let y0 = ((roi.y.clamp(0.0, 1.0)) * h as f32).floor() as usize;
    let x1 = (((roi.x + roi.w).clamp(0.0, 1.0)) * w as f32)
        .ceil()
        .min(w as f32) as usize;
    let y1 = (((roi.y + roi.h).clamp(0.0, 1.0)) * h as f32)
        .ceil()
        .min(h as f32) as usize;

    let x0 = x0.min(w.saturating_sub(1));
    let y0 = y0.min(h.saturating_sub(1));
    let x1 = x1.max(x0 + 1).min(w);
    let y1 = y1.max(y0 + 1).min(h);

    let near = roi.near_mm.max(1);
    let far = roi.far_mm.max(near.saturating_add(1));
    let z_span = (far - near) as f64;

    let total_pixels = ((x1 - x0) * (y1 - y0)) as u32;
    let mut valid_count: u32 = 0;
    let mut min_d: u16 = u16::MAX;
    let mut max_d: u16 = 0;
    let mut sum_d: u64 = 0;

    let mut closest_x_sum: f64 = 0.0;
    let mut closest_y_sum: f64 = 0.0;
    let mut closest_tie_count: u32 = 0;

    let mut centroid_x_sum: f64 = 0.0;
    let mut centroid_y_sum: f64 = 0.0;
    let mut centroid_z_sum: f64 = 0.0;

    let inv_w = 1.0 / (w.saturating_sub(1).max(1) as f64);
    let inv_h = 1.0 / (h.saturating_sub(1).max(1) as f64);

    for py in y0..y1 {
        let row_off = py * w;
        let yn = py as f64 * inv_h;
        for px in x0..x1 {
            let d = depth_mm[row_off + px];
            if d == 0 || d < near || d > far {
                continue;
            }
            let xn = px as f64 * inv_w;

            valid_count += 1;
            sum_d += d as u64;
            if d < min_d {
                min_d = d;
                closest_x_sum = xn;
                closest_y_sum = yn;
                closest_tie_count = 1;
            } else if d == min_d {
                closest_x_sum += xn;
                closest_y_sum += yn;
                closest_tie_count += 1;
            }
            if d > max_d {
                max_d = d;
            }

            centroid_x_sum += xn;
            centroid_y_sum += yn;
            centroid_z_sum += d as f64;
        }
    }

    if valid_count == 0 {
        let mut r = empty_result(roi);
        r.total_pixel_count = total_pixels;
        return r;
    }

    let valid_f = valid_count as f64;
    let mean_depth_mm = (sum_d as f64 / valid_f) as f32;
    let fill_ratio = (valid_count as f32) / (total_pixels.max(1) as f32);
    let presence = fill_ratio >= roi.occupancy_threshold;

    let closest_u = closest_x_sum / (closest_tie_count.max(1) as f64);
    let closest_v = closest_y_sum / (closest_tie_count.max(1) as f64);
    let closest_z_norm = ((min_d as f64 - near as f64) / z_span).clamp(0.0, 1.0);
    let closest_point_meters = intrinsics.unproject_meters(closest_u, closest_v, min_d as f64);

    let cent_u = centroid_x_sum / valid_f;
    let cent_v = centroid_y_sum / valid_f;
    let cent_z_mm = centroid_z_sum / valid_f;
    let cent_z_norm = ((cent_z_mm - near as f64) / z_span).clamp(0.0, 1.0);
    let centroid_meters = intrinsics.unproject_meters(cent_u, cent_v, cent_z_mm);

    // Smooth confidence score saturating around 25% ROI fill
    let confidence = (fill_ratio as f64 * 4.0).clamp(0.0, 1.0);

    RoiExtractionResult {
        roi_id: roi.id.clone(),
        roi_name: roi.name.clone(),
        min_depth_mm: min_d,
        max_depth_mm: max_d,
        mean_depth_mm,
        presence,
        fill_ratio,
        valid_pixel_count: valid_count,
        total_pixel_count: total_pixels,
        closest_point_norm: [closest_u, closest_v, closest_z_norm],
        closest_point_meters,
        centroid_norm: [cent_u, cent_v, cent_z_norm],
        centroid_meters,
        confidence,
    }
}

/// Extracts measurements for all configured 3D ROIs in one call.
pub fn extract_all_rois(
    depth_mm: &[u16],
    width: u32,
    height: u32,
    rois: &[NamedDepthRoi],
    intrinsics: CameraIntrinsics,
) -> Vec<RoiExtractionResult> {
    rois.iter()
        .map(|roi| extract_roi(depth_mm, width, height, roi, intrinsics))
        .collect()
}

/// Converts a primary `RoiExtractionResult` into `kf-core`'s `DepthSampleSummary`.
pub fn to_depth_sample_summary(
    device_id: Uuid,
    res: &RoiExtractionResult,
    timestamp_ms: u64,
) -> DepthSampleSummary {
    DepthSampleSummary {
        device_id,
        closest_point: res.closest_point_norm,
        closest_confidence: res.confidence,
        centroid: res.centroid_norm,
        foreground_mass: res.fill_ratio as f64,
        timestamp_ms,
    }
}

/// Produces standard `(logical_key, SignalValue)` pairs for routing into `kf-core::SignalRouter`.
pub fn roi_to_signal_bindings(res: &RoiExtractionResult) -> Vec<(&'static str, SignalValue)> {
    vec![
        ("depth.closest_z", SignalValue::Float(res.closest_point_norm[2])),
        ("depth.closest_xyz", SignalValue::Vec3(res.closest_point_norm)),
        ("depth.centroid_xyz", SignalValue::Vec3(res.centroid_norm)),
        ("depth.presence", SignalValue::Bool(res.presence)),
        ("depth.fill_ratio", SignalValue::Float(res.fill_ratio as f64)),
    ]
}

fn empty_result(roi: &NamedDepthRoi) -> RoiExtractionResult {
    RoiExtractionResult {
        roi_id: roi.id.clone(),
        roi_name: roi.name.clone(),
        min_depth_mm: 0,
        max_depth_mm: 0,
        mean_depth_mm: 0.0,
        presence: false,
        fill_ratio: 0.0,
        valid_pixel_count: 0,
        total_pixel_count: 0,
        closest_point_norm: [0.5, 0.5, 1.0],
        closest_point_meters: [0.0, 0.0, 0.0],
        centroid_norm: [0.5, 0.5, 1.0],
        centroid_meters: [0.0, 0.0, 0.0],
        confidence: 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roi_closest_point_centroid_and_presence_accuracy() {
        let width = 100u32;
        let height = 100u32;
        // Background at 3500mm (outside ROI near..far 500..2000mm)
        let mut depth = vec![3500u16; (width * height) as usize];

        // Place a 20x20 foreground block centered at (30..50, 40..60) at depth 1200mm
        for y in 40..60 {
            for x in 30..50 {
                depth[(y * width + x) as usize] = 1200;
            }
        }
        // Place the single closest point at (35, 45) at depth 800mm
        depth[(45 * width + 35) as usize] = 800;

        let roi = NamedDepthRoi {
            id: "zone-a".into(),
            name: "Zone A".into(),
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
            near_mm: 500,
            far_mm: 2000,
            occupancy_threshold: 0.03,
        };

        let res = extract_roi(&depth, width, height, &roi, CameraIntrinsics::default());
        assert_eq!(res.min_depth_mm, 800);
        assert_eq!(res.max_depth_mm, 1200);
        assert_eq!(res.valid_pixel_count, 400);
        assert_eq!(res.total_pixel_count, 10_000);
        assert!((res.fill_ratio - 0.04).abs() < 1e-4);
        assert!(res.presence);

        // Closest point at pixel (35, 45) on 100x100 -> u = 35/99, v = 45/99, z_norm = (800-500)/(2000-500) = 0.2
        assert!((res.closest_point_norm[0] - (35.0 / 99.0)).abs() < 1e-3);
        assert!((res.closest_point_norm[1] - (45.0 / 99.0)).abs() < 1e-3);
        assert!((res.closest_point_norm[2] - 0.2).abs() < 1e-3);
        assert!((res.closest_point_meters[2] - 0.8).abs() < 1e-4);

        // Centroid of 30..50 (mean 39.5) and 40..60 (mean 49.5)
        assert!((res.centroid_norm[0] - (39.5 / 99.0)).abs() < 1e-3);
        assert!((res.centroid_norm[1] - (49.5 / 99.0)).abs() < 1e-3);
        assert!((res.centroid_meters[2] - 1.199).abs() < 0.01);

        // Empty ROI placed in bottom-right corner where background is 3500mm -> presence = false
        let empty_roi = NamedDepthRoi {
            id: "zone-empty".into(),
            name: "Empty Corner".into(),
            x: 0.7,
            y: 0.7,
            w: 0.25,
            h: 0.25,
            near_mm: 500,
            far_mm: 2000,
            occupancy_threshold: 0.05,
        };
        let empty_res = extract_roi(&depth, width, height, &empty_roi, CameraIntrinsics::default());
        assert!(!empty_res.presence);
        assert_eq!(empty_res.valid_pixel_count, 0);
        assert_eq!(empty_res.fill_ratio, 0.0);
    }
}
