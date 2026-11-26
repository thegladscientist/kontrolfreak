//! Depth-to-Video / Depth-to-NDI Colormap Bridge converting 16-bit millimeter depth maps
//! into RGBA8 frames (`Turbo`, `Viridis`, `Grayscale`, `DepthSlice`) with `kf_core::FrameRef`.

use kf_core::{FrameRef, PixelFormat};
use kf_sidecar_ipc::DepthColormap;
use uuid::Uuid;

/// Colorizes a 16-bit millimeter depth buffer into packed RGBA8 bytes (`width * height * 4`).
/// Pixels with `0` (invalid) render as deep studio slate `(11, 14, 20, 255)`.
pub fn colorize_depth_to_rgba(
    depth_mm: &[u16],
    width: u32,
    height: u32,
    near_mm: u16,
    far_mm: u16,
    palette: DepthColormap,
    out_rgba: &mut Vec<u8>,
) {
    let pixel_count = (width as usize).saturating_mul(height as usize);
    out_rgba.resize(pixel_count * 4, 0);

    let near = near_mm.max(1) as f32;
    let far = far_mm.max(near_mm.saturating_add(1)) as f32;
    let inv_span = 1.0 / (far - near);

    for (i, &d) in depth_mm.iter().take(pixel_count).enumerate() {
        let base = i * 4;
        if d == 0 {
            out_rgba[base] = 11;
            out_rgba[base + 1] = 14;
            out_rgba[base + 2] = 20;
            out_rgba[base + 3] = 255;
            continue;
        }

        let df = d as f32;
        let (r, g, b, a) = match palette {
            DepthColormap::Turbo => {
                // Near objects map to warm reds/yellows (t -> 1.0), far objects map to cool blues (t -> 0.0)
                let t = 1.0 - ((df - near) * inv_span).clamp(0.0, 1.0);
                let (r, g, b) = turbo_rgb(t);
                (r, g, b, 255)
            }
            DepthColormap::Viridis => {
                let t = 1.0 - ((df - near) * inv_span).clamp(0.0, 1.0);
                let (r, g, b) = viridis_rgb(t);
                (r, g, b, 255)
            }
            DepthColormap::Grayscale => {
                let t = 1.0 - ((df - near) * inv_span).clamp(0.0, 1.0);
                let v = (t * 255.0).round().clamp(0.0, 255.0) as u8;
                (v, v, v, 255)
            }
            DepthColormap::DepthSlice => {
                if df < near || df > far {
                    // Out-of-slice background dimmed for stage matting / keying
                    (15, 20, 30, 255)
                } else {
                    let t = 1.0 - ((df - near) * inv_span).clamp(0.0, 1.0);
                    // Subtle contour bands every ~12.5% of slice depth
                    let band = if ((t * 8.0).fract()) < 0.08 { 1.15 } else { 1.0 };
                    let r = ((18.0 + t * 55.0) * band).min(255.0) as u8;
                    let g = ((130.0 + t * 125.0) * band).min(255.0) as u8;
                    let b = ((165.0 + (1.0 - t) * 70.0) * band).min(255.0) as u8;
                    (r, g, b, 255)
                }
            }
        };

        out_rgba[base] = r;
        out_rgba[base + 1] = g;
        out_rgba[base + 2] = b;
        out_rgba[base + 3] = a;
    }
}

/// Builds an NDI / video-pipeline ready `FrameRef` for a colorized RGBA8 depth frame.
pub fn build_depth_ndi_frame_ref(
    source_id: Uuid,
    frame_seq: u64,
    width: u32,
    height: u32,
    timestamp_ns: u64,
) -> FrameRef {
    FrameRef {
        frame_id: frame_seq,
        width,
        height,
        format: PixelFormat::Rgba8,
        source_id,
        timestamp_ns,
    }
}

/// Smooth Google Turbo colormap polynomial approximation (`t` in `0.0..=1.0`).
pub fn turbo_rgb(t: f32) -> (u8, u8, u8) {
    let x = t.clamp(0.0, 1.0);
    let r = (0.13572138
        + x * (4.61539260
            + x * (-42.66032258
                + x * (132.13108234 + x * (-152.94239396 + x * 59.28637943)))))
        .clamp(0.0, 1.0);
    let g = (0.09140261
        + x * (2.19418839
            + x * (4.84296658
                + x * (-14.18503333 + x * (4.27729857 + x * 2.82956604)))))
        .clamp(0.0, 1.0);
    let b = (0.10667330
        + x * (12.64194608
            + x * (-60.58204836
                + x * (110.36276771 + x * (-89.90310912 + x * 27.34824973)))))
        .clamp(0.0, 1.0);
    (
        (r * 255.0).round() as u8,
        (g * 255.0).round() as u8,
        (b * 255.0).round() as u8,
    )
}

/// Perceptually uniform Viridis palette interpolation (`t` in `0.0..=1.0`).
pub fn viridis_rgb(t: f32) -> (u8, u8, u8) {
    const STOPS: [(f32, f32, f32); 5] = [
        (68.0, 1.0, 84.0),
        (59.0, 82.0, 139.0),
        (33.0, 145.0, 140.0),
        (94.0, 201.0, 98.0),
        (253.0, 231.0, 37.0),
    ];
    let x = t.clamp(0.0, 1.0) * 4.0;
    let idx = (x.floor() as usize).min(3);
    let frac = x - idx as f32;
    let a = STOPS[idx];
    let b = STOPS[idx + 1];
    (
        (a.0 + (b.0 - a.0) * frac).round() as u8,
        (a.1 + (b.1 - a.1) * frac).round() as u8,
        (a.2 + (b.2 - a.2) * frac).round() as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depth_colormap_palettes_produce_expected_rgba_and_frame_ref() {
        // 4 pixels: invalid(0), near(500), mid(1500), far(2500), out-of-slice(4000)
        let depth = vec![0u16, 500, 1500, 2500, 4000, 1000];
        let mut rgba = Vec::new();

        // Grayscale: near -> white (255), far -> black (0)
        colorize_depth_to_rgba(&depth, 3, 2, 500, 2500, DepthColormap::Grayscale, &mut rgba);
        assert_eq!(rgba.len(), 6 * 4);
        assert_eq!(&rgba[0..4], &[11, 14, 20, 255]); // invalid
        assert_eq!(&rgba[4..8], &[255, 255, 255, 255]); // near=500 -> white
        assert_eq!(&rgba[12..16], &[0, 0, 0, 255]); // far=2500 -> black

        // Turbo: near should have warmer red than far
        colorize_depth_to_rgba(&depth, 3, 2, 500, 2500, DepthColormap::Turbo, &mut rgba);
        assert!(rgba[4] > rgba[12]);

        // Viridis: near should be bright yellow-green (high R & G), far dark purple (low G)
        colorize_depth_to_rgba(&depth, 3, 2, 500, 2500, DepthColormap::Viridis, &mut rgba);
        assert!(rgba[5] > 200);
        assert!(rgba[13] < 20);

        // DepthSlice: out-of-slice (4000mm) is dimmed matte, in-slice (1500mm) has vivid cyan/emerald G/B
        colorize_depth_to_rgba(&depth, 3, 2, 500, 2500, DepthColormap::DepthSlice, &mut rgba);
        assert_eq!(&rgba[16..20], &[15, 20, 30, 255]);
        assert!(rgba[9] > 150);

        let fref = build_depth_ndi_frame_ref(Uuid::new_v4(), 42, 3, 2, 999);
        assert_eq!(fref.format, PixelFormat::Rgba8);
        assert_eq!(fref.frame_id, 42);
    }
}
