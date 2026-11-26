//! Flexible-resolution & transform video scaler:
//! - SIMD CPU backend via `fast_image_resize`
//! - Optional GPU backend via `wgpu` (`--features gpu`) with automatic CPU fallback
//! - Arbitrary custom output resolutions, fit modes (`Contain`, `Cover`, `Stretch`, `Crop`),
//!   normalized crop + zoom, opacity, pixel format conversion, and fast JPEG/RGBA thumbnails.

use crate::{VideoError, VideoFrame, VideoScaler};
use fast_image_resize::images::Image;
use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer};
use kf_core::{AspectScaleMode, FrameRef, PixelFormat, SignalEvent, VideoPreviewFrame, VideoResizeConfig};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoFitMode {
    /// Letterbox / pillarbox inside output bounds preserving aspect ratio.
    Contain,
    /// Scale & center-crop to fill output bounds preserving aspect ratio.
    Cover,
    /// Stretch to fill output bounds ignoring aspect ratio.
    Stretch,
    /// Explicit normalized `crop_x/y/w/h` + `zoom` region scaled to output bounds.
    Crop,
}

impl Default for VideoFitMode {
    fn default() -> Self {
        Self::Contain
    }
}

impl From<AspectScaleMode> for VideoFitMode {
    fn from(mode: AspectScaleMode) -> Self {
        match mode {
            AspectScaleMode::Fit => VideoFitMode::Contain,
            AspectScaleMode::Fill => VideoFitMode::Cover,
            AspectScaleMode::Stretch => VideoFitMode::Stretch,
        }
    }
}

impl From<VideoFitMode> for AspectScaleMode {
    fn from(mode: VideoFitMode) -> Self {
        match mode {
            VideoFitMode::Contain => AspectScaleMode::Fit,
            VideoFitMode::Cover => AspectScaleMode::Fill,
            VideoFitMode::Stretch | VideoFitMode::Crop => AspectScaleMode::Stretch,
        }
    }
}

/// Full M2 video transform configuration supporting arbitrary output dimensions, fit modes,
/// normalized crop + zoom, FPS throttling, opacity, freeze, and bypass.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NdiTransformConfig {
    pub fit_mode: VideoFitMode,
    pub crop_x: f32,
    pub crop_y: f32,
    pub crop_w: f32,
    pub crop_h: f32,
    pub zoom: f32,
    pub out_width: u32,
    pub out_height: u32,
    pub out_fps: f32,
    pub opacity: f32,
    pub freeze: bool,
    pub bypass: bool,
}

impl Default for NdiTransformConfig {
    fn default() -> Self {
        Self {
            fit_mode: VideoFitMode::Contain,
            crop_x: 0.0,
            crop_y: 0.0,
            crop_w: 1.0,
            crop_h: 1.0,
            zoom: 1.0,
            out_width: 1280,
            out_height: 720,
            out_fps: 30.0,
            opacity: 1.0,
            freeze: false,
            bypass: false,
        }
    }
}

impl From<&VideoResizeConfig> for NdiTransformConfig {
    fn from(cfg: &VideoResizeConfig) -> Self {
        let is_cropped = cfg.crop_x > 0.001
            || cfg.crop_y > 0.001
            || cfg.crop_w < 0.999
            || cfg.crop_h < 0.999;
        Self {
            fit_mode: if is_cropped {
                VideoFitMode::Crop
            } else {
                VideoFitMode::from(cfg.scale_mode)
            },
            crop_x: cfg.crop_x,
            crop_y: cfg.crop_y,
            crop_w: cfg.crop_w,
            crop_h: cfg.crop_h,
            zoom: 1.0,
            out_width: cfg.out_width,
            out_height: cfg.out_height,
            out_fps: cfg.out_fps,
            opacity: 1.0,
            freeze: cfg.freeze,
            bypass: cfg.bypass,
        }
    }
}

impl NdiTransformConfig {
    pub fn sanitized(&self) -> Self {
        let crop_w = self.crop_w.clamp(0.05, 1.0);
        let crop_h = self.crop_h.clamp(0.05, 1.0);
        let crop_x = self.crop_x.clamp(0.0, 1.0);
        let crop_y = self.crop_y.clamp(0.0, 1.0);
        Self {
            fit_mode: self.fit_mode,
            crop_x,
            crop_y,
            crop_w,
            crop_h,
            zoom: self.zoom.clamp(1.0, 8.0),
            out_width: self.out_width.clamp(16, 7680),
            out_height: self.out_height.clamp(16, 4320),
            out_fps: self.out_fps.clamp(1.0, 120.0),
            opacity: self.opacity.clamp(0.0, 1.0),
            freeze: self.freeze,
            bypass: self.bypass,
        }
    }
}

/// Computed source crop rectangle (in source pixels) and destination placement rectangle
/// (in output pixels, used for letterboxing/pillarboxing in `Contain` mode).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComputedRects {
    pub src_x: u32,
    pub src_y: u32,
    pub src_w: u32,
    pub src_h: u32,
    pub dst_x: u32,
    pub dst_y: u32,
    pub dst_w: u32,
    pub dst_h: u32,
}

/// Computes exact source crop and destination placement rectangles for any resolution and fit mode.
pub fn compute_transform_rects(
    in_w: u32,
    in_h: u32,
    cfg: &NdiTransformConfig,
) -> ComputedRects {
    let cfg = cfg.sanitized();
    let in_wf = in_w.max(1) as f64;
    let in_hf = in_h.max(1) as f64;
    let out_w = cfg.out_width.max(1);
    let out_h = cfg.out_height.max(1);
    let out_wf = out_w as f64;
    let out_hf = out_h as f64;

    // Start from user crop region and clamp into [0..1] after applying centered zoom
    let mut norm_w = cfg.crop_w as f64;
    let mut norm_h = cfg.crop_h as f64;
    let mut norm_x = (cfg.crop_x as f64).min((1.0 - norm_w).max(0.0));
    let mut norm_y = (cfg.crop_y as f64).min((1.0 - norm_h).max(0.0));

    // Apply zoom centered within the crop box (and allow smooth panning when zoomed)
    let zoom = (cfg.zoom as f64).max(1.0);
    if zoom > 1.0001 {
        let zoomed_w = norm_w / zoom;
        let zoomed_h = norm_h / zoom;
        let cx = if norm_w >= 0.999 {
            if cfg.crop_x <= 0.0001 {
                0.5
            } else {
                zoomed_w * 0.5 + (cfg.crop_x as f64).clamp(0.0, 1.0) * (1.0 - zoomed_w)
            }
        } else {
            norm_x + norm_w * 0.5
        };
        let cy = if norm_h >= 0.999 {
            if cfg.crop_y <= 0.0001 {
                0.5
            } else {
                zoomed_h * 0.5 + (cfg.crop_y as f64).clamp(0.0, 1.0) * (1.0 - zoomed_h)
            }
        } else {
            norm_y + norm_h * 0.5
        };
        norm_w = zoomed_w;
        norm_h = zoomed_h;
        norm_x = (cx - norm_w * 0.5).clamp(0.0, (1.0 - norm_w).max(0.0));
        norm_y = (cy - norm_h * 0.5).clamp(0.0, (1.0 - norm_h).max(0.0));
    }

    let mut src_x = (norm_x * in_wf).floor().max(0.0);
    let mut src_y = (norm_y * in_hf).floor().max(0.0);
    let mut src_w = (norm_w * in_wf).round().clamp(1.0, in_wf - src_x);
    let mut src_h = (norm_h * in_hf).round().clamp(1.0, in_hf - src_y);

    let mut dst_x: u32 = 0;
    let mut dst_y: u32 = 0;
    let mut dst_w: u32 = out_w;
    let mut dst_h: u32 = out_h;

    let src_aspect = src_w / src_h.max(1.0);
    let dst_aspect = out_wf / out_hf.max(1.0);

    match cfg.fit_mode {
        VideoFitMode::Contain => {
            if src_aspect > dst_aspect {
                // Source is wider -> letterbox top & bottom
                let scaled_h = (out_wf / src_aspect).round().clamp(1.0, out_hf) as u32;
                dst_w = out_w;
                dst_h = scaled_h;
                dst_x = 0;
                dst_y = (out_h.saturating_sub(scaled_h)) / 2;
            } else {
                // Source is taller -> pillarbox left & right
                let scaled_w = (out_hf * src_aspect).round().clamp(1.0, out_wf) as u32;
                dst_w = scaled_w;
                dst_h = out_h;
                dst_x = (out_w.saturating_sub(scaled_w)) / 2;
                dst_y = 0;
            }
        }
        VideoFitMode::Cover => {
            if src_aspect > dst_aspect {
                // Source is wider than destination -> trim left & right equally
                let new_src_w = (src_h * dst_aspect).round().clamp(1.0, src_w);
                src_x += ((src_w - new_src_w) * 0.5).floor();
                src_w = new_src_w;
            } else {
                // Source is taller -> trim top & bottom equally
                let new_src_h = (src_w / dst_aspect).round().clamp(1.0, src_h);
                src_y += ((src_h - new_src_h) * 0.5).floor();
                src_h = new_src_h;
            }
        }
        VideoFitMode::Stretch | VideoFitMode::Crop => {
            // Map entire selected source crop rect to full output dimensions
        }
    }

    let src_x_u = (src_x as u32).min(in_w.saturating_sub(1));
    let src_y_u = (src_y as u32).min(in_h.saturating_sub(1));
    let src_w_u = (src_w as u32).clamp(1, in_w.saturating_sub(src_x_u).max(1));
    let src_h_u = (src_h as u32).clamp(1, in_h.saturating_sub(src_y_u).max(1));

    ComputedRects {
        src_x: src_x_u,
        src_y: src_y_u,
        src_w: src_w_u,
        src_h: src_h_u,
        dst_x,
        dst_y,
        dst_w: dst_w.max(1),
        dst_h: dst_h.max(1),
    }
}

/// Converts any supported `VideoFrame` pixel format into tightly packed `Rgba8` bytes.
pub fn frame_to_rgba8(frame: &VideoFrame) -> Result<Vec<u8>, VideoError> {
    let w = frame.meta.width as usize;
    let h = frame.meta.height as usize;
    let expected_rgba = w * h * 4;

    match frame.meta.format {
        PixelFormat::Rgba8 => {
            if frame.stride == w * 4 && frame.data.len() >= expected_rgba {
                return Ok(frame.data[..expected_rgba].to_vec());
            }
            let mut out = vec![0u8; expected_rgba];
            for y in 0..h {
                let src_row = y * frame.stride;
                let dst_row = y * w * 4;
                if src_row + w * 4 > frame.data.len() {
                    return Err(VideoError::Backend("truncated RGBA8 frame buffer".into()));
                }
                out[dst_row..dst_row + w * 4]
                    .copy_from_slice(&frame.data[src_row..src_row + w * 4]);
            }
            Ok(out)
        }
        PixelFormat::Bgra8 => {
            let mut out = vec![0u8; expected_rgba];
            for y in 0..h {
                let src_row = y * frame.stride;
                let dst_row = y * w * 4;
                if src_row + w * 4 > frame.data.len() {
                    return Err(VideoError::Backend("truncated BGRA8 frame buffer".into()));
                }
                for x in 0..w {
                    let s = src_row + x * 4;
                    let d = dst_row + x * 4;
                    out[d] = frame.data[s + 2];
                    out[d + 1] = frame.data[s + 1];
                    out[d + 2] = frame.data[s];
                    out[d + 3] = frame.data[s + 3];
                }
            }
            Ok(out)
        }
        PixelFormat::Uyvy => {
            let mut out = vec![0u8; expected_rgba];
            for y in 0..h {
                let src_row = y * frame.stride;
                let dst_row = y * w * 4;
                for pair in 0..(w / 2) {
                    let s = src_row + pair * 4;
                    if s + 3 >= frame.data.len() {
                        break;
                    }
                    let u = frame.data[s] as f32 - 128.0;
                    let y0 = frame.data[s + 1] as f32;
                    let v = frame.data[s + 2] as f32 - 128.0;
                    let y1 = frame.data[s + 3] as f32;

                    let (r0, g0, b0) = yuv_to_rgb(y0, u, v);
                    let (r1, g1, b1) = yuv_to_rgb(y1, u, v);
                    let d0 = dst_row + pair * 8;
                    out[d0] = r0;
                    out[d0 + 1] = g0;
                    out[d0 + 2] = b0;
                    out[d0 + 3] = 255;
                    out[d0 + 4] = r1;
                    out[d0 + 5] = g1;
                    out[d0 + 6] = b1;
                    out[d0 + 7] = 255;
                }
            }
            Ok(out)
        }
        PixelFormat::Nv12 => {
            let y_plane_len = frame.stride * h;
            if frame.data.len() < y_plane_len + frame.stride * (h / 2) {
                return Err(VideoError::Backend("truncated NV12 frame buffer".into()));
            }
            let mut out = vec![0u8; expected_rgba];
            for y in 0..h {
                let y_row = y * frame.stride;
                let uv_row = y_plane_len + (y / 2) * frame.stride;
                let dst_row = y * w * 4;
                for x in 0..w {
                    let luma = frame.data[y_row + x] as f32;
                    let uv_idx = uv_row + (x & !1);
                    let u = frame.data[uv_idx] as f32 - 128.0;
                    let v = frame.data[uv_idx + 1] as f32 - 128.0;
                    let (r, g, b) = yuv_to_rgb(luma, u, v);
                    let d = dst_row + x * 4;
                    out[d] = r;
                    out[d + 1] = g;
                    out[d + 2] = b;
                    out[d + 3] = 255;
                }
            }
            Ok(out)
        }
        PixelFormat::Gray8 => {
            let mut out = vec![0u8; expected_rgba];
            for y in 0..h {
                let src_row = y * frame.stride;
                let dst_row = y * w * 4;
                for x in 0..w {
                    let g = frame.data[src_row + x];
                    let d = dst_row + x * 4;
                    out[d] = g;
                    out[d + 1] = g;
                    out[d + 2] = g;
                    out[d + 3] = 255;
                }
            }
            Ok(out)
        }
        PixelFormat::Depth16 => {
            let mut out = vec![0u8; expected_rgba];
            for y in 0..h {
                let src_row = y * frame.stride;
                let dst_row = y * w * 4;
                for x in 0..w {
                    let s = src_row + x * 2;
                    let raw = u16::from_le_bytes([frame.data[s], frame.data[s + 1]]);
                    let g = (raw >> 8) as u8;
                    let d = dst_row + x * 4;
                    out[d] = g;
                    out[d + 1] = g;
                    out[d + 2] = g;
                    out[d + 3] = 255;
                }
            }
            Ok(out)
        }
    }
}

#[inline]
fn yuv_to_rgb(y: f32, u: f32, v: f32) -> (u8, u8, u8) {
    let r = (y + 1.402 * v).clamp(0.0, 255.0) as u8;
    let g = (y - 0.344136 * u - 0.714136 * v).clamp(0.0, 255.0) as u8;
    let b = (y + 1.772 * u).clamp(0.0, 255.0) as u8;
    (r, g, b)
}

/// SIMD-accelerated CPU scaler powered by `fast_image_resize` + `jpeg-encoder`.
pub struct CpuVideoScaler {
    resizer: Resizer,
    jpeg_quality: u8,
}

impl Default for CpuVideoScaler {
    fn default() -> Self {
        Self::new()
    }
}

impl CpuVideoScaler {
    pub fn new() -> Self {
        Self {
            resizer: Resizer::new(),
            jpeg_quality: 75,
        }
    }

    pub fn with_jpeg_quality(mut self, quality: u8) -> Self {
        self.jpeg_quality = quality.clamp(20, 95);
        self
    }

    pub fn transform_frame(
        &mut self,
        input: &VideoFrame,
        config: &NdiTransformConfig,
    ) -> Result<VideoFrame, VideoError> {
        let cfg = config.sanitized();
        if cfg.bypass {
            return Ok(input.clone());
        }

        let rgba = frame_to_rgba8(input)?;
        let rects = compute_transform_rects(input.meta.width, input.meta.height, &cfg);

        let src_image = Image::from_vec_u8(
            input.meta.width,
            input.meta.height,
            rgba,
            PixelType::U8x4,
        )
        .map_err(|e| VideoError::Backend(format!("source image error: {e}")))?;

        let mut scaled_inner = Image::new(rects.dst_w, rects.dst_h, PixelType::U8x4);
        let options = ResizeOptions::new()
            .resize_alg(ResizeAlg::Convolution(FilterType::Bilinear))
            .crop(
                rects.src_x as f64,
                rects.src_y as f64,
                rects.src_w as f64,
                rects.src_h as f64,
            );
        self.resizer
            .resize(&src_image, &mut scaled_inner, &options)
            .map_err(|e| VideoError::Backend(format!("fast_image_resize failed: {e}")))?;

        let mut out_data = if rects.dst_x == 0
            && rects.dst_y == 0
            && rects.dst_w == cfg.out_width
            && rects.dst_h == cfg.out_height
        {
            scaled_inner.into_vec()
        } else {
            // Composite letterboxed/pillarboxed inner frame onto opaque black canvas
            let out_w = cfg.out_width as usize;
            let out_h = cfg.out_height as usize;
            let mut canvas = vec![0u8; out_w * out_h * 4];
            for px in canvas.chunks_exact_mut(4) {
                px[3] = 255;
            }
            let inner = scaled_inner.buffer();
            let inner_stride = (rects.dst_w as usize) * 4;
            let out_stride = out_w * 4;
            let dx_bytes = (rects.dst_x as usize) * 4;
            for row in 0..(rects.dst_h as usize) {
                let dst_y = (rects.dst_y as usize) + row;
                if dst_y >= out_h {
                    break;
                }
                let dst_start = dst_y * out_stride + dx_bytes;
                let src_start = row * inner_stride;
                canvas[dst_start..dst_start + inner_stride]
                    .copy_from_slice(&inner[src_start..src_start + inner_stride]);
            }
            canvas
        };

        // Apply opacity attenuation (premultiplied dark fade) if < 1.0
        if cfg.opacity < 0.999 {
            let alpha = cfg.opacity.clamp(0.0, 1.0);
            for px in out_data.chunks_exact_mut(4) {
                px[0] = (px[0] as f32 * alpha).round() as u8;
                px[1] = (px[1] as f32 * alpha).round() as u8;
                px[2] = (px[2] as f32 * alpha).round() as u8;
                px[3] = 255;
            }
        }

        Ok(VideoFrame {
            meta: FrameRef {
                frame_id: input.meta.frame_id,
                width: cfg.out_width,
                height: cfg.out_height,
                format: PixelFormat::Rgba8,
                source_id: input.meta.source_id,
                timestamp_ns: input.meta.timestamp_ns,
            },
            stride: (cfg.out_width as usize) * 4,
            data: out_data,
        })
    }

    /// Downscales `input` proportionally into `(max_width, max_height)` returning raw RGBA8 bytes.
    pub fn encode_preview_rgba(
        &mut self,
        input: &VideoFrame,
        max_width: u32,
        max_height: u32,
    ) -> Result<VideoFrame, VideoError> {
        let in_w = input.meta.width.max(1) as f64;
        let in_h = input.meta.height.max(1) as f64;
        let scale = (max_width.max(16) as f64 / in_w)
            .min(max_height.max(16) as f64 / in_h)
            .min(1.0);
        let tw = (in_w * scale).round().max(16.0) as u32;
        let th = (in_h * scale).round().max(16.0) as u32;

        let cfg = NdiTransformConfig {
            fit_mode: VideoFitMode::Stretch,
            out_width: tw,
            out_height: th,
            ..Default::default()
        };
        self.transform_frame(input, &cfg)
    }

    /// Fast JPEG thumbnail encoder for viewport-scoped UI previews.
    pub fn encode_preview_jpeg_frame(
        &mut self,
        input: &VideoFrame,
        max_width: u32,
        max_height: u32,
    ) -> Result<VideoPreviewFrame, VideoError> {
        let thumb = self.encode_preview_rgba(input, max_width, max_height)?;
        let mut jpeg_bytes = Vec::with_capacity((thumb.meta.width * thumb.meta.height / 3) as usize);
        let encoder = jpeg_encoder::Encoder::new(&mut jpeg_bytes, self.jpeg_quality);
        encoder
            .encode(
                &thumb.data,
                thumb.meta.width as u16,
                thumb.meta.height as u16,
                jpeg_encoder::ColorType::Rgba,
            )
            .map_err(|e| VideoError::Backend(format!("JPEG thumbnail encode failed: {e}")))?;

        Ok(VideoPreviewFrame {
            device_id: input.meta.source_id,
            width: thumb.meta.width,
            height: thumb.meta.height,
            format: PixelFormat::Rgba8,
            jpeg_bytes,
            timestamp_ms: SignalEvent::now_ms(),
        })
    }
}

impl VideoScaler for CpuVideoScaler {
    fn scale_frame(
        &mut self,
        input: &VideoFrame,
        config: &VideoResizeConfig,
    ) -> Result<VideoFrame, VideoError> {
        let m2_cfg = NdiTransformConfig::from(config);
        self.transform_frame(input, &m2_cfg)
    }

    fn encode_preview_jpeg(
        &mut self,
        input: &VideoFrame,
        max_width: u32,
        max_height: u32,
    ) -> Result<VideoPreviewFrame, VideoError> {
        self.encode_preview_jpeg_frame(input, max_width, max_height)
    }
}

// ---------- Optional WGPU GPU Scaler Backend (`--features gpu`) ----------

#[cfg(feature = "gpu")]
pub struct WgpuVideoScaler {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    adapter_name: String,
    cpu_fallback: CpuVideoScaler,
}

#[cfg(feature = "gpu")]
const WGPU_SCALE_SHADER: &str = r#"
struct Params {
    in_w: u32,
    in_h: u32,
    out_w: u32,
    out_h: u32,
    src_x: u32,
    src_y: u32,
    src_w: u32,
    src_h: u32,
    dst_x: u32,
    dst_y: u32,
    dst_w: u32,
    dst_h: u32,
    opacity_u32: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};

@group(0) @binding(0) var<storage, read> src_pixels: array<u32>;
@group(0) @binding(1) var<storage, read_write> dst_pixels: array<u32>;
@group(0) @binding(2) var<uniform> params: Params;

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.out_w || gid.y >= params.out_h) {
        return;
    }
    let out_idx = gid.y * params.out_w + gid.x;
    if (gid.x < params.dst_x || gid.x >= params.dst_x + params.dst_w ||
        gid.y < params.dst_y || gid.y >= params.dst_y + params.dst_h) {
        dst_pixels[out_idx] = 0xFF000000u;
        return;
    }
    let rel_x = f32(gid.x - params.dst_x) / max(f32(params.dst_w), 1.0);
    let rel_y = f32(gid.y - params.dst_y) / max(f32(params.dst_h), 1.0);
    let sx = min(params.src_x + u32(rel_x * f32(params.src_w)), params.in_w - 1u);
    let sy = min(params.src_y + u32(rel_y * f32(params.src_h)), params.in_h - 1u);
    let px = src_pixels[sy * params.in_w + sx];
    let alpha = f32(params.opacity_u32) / 1000.0;
    let r = u32(f32(px & 0xFFu) * alpha);
    let g = u32(f32((px >> 8u) & 0xFFu) * alpha);
    let b = u32(f32((px >> 16u) & 0xFFu) * alpha);
    dst_pixels[out_idx] = (r & 0xFFu) | ((g & 0xFFu) << 8u) | ((b & 0xFFu) << 16u) | 0xFF000000u;
}
"#;

#[cfg(feature = "gpu")]
impl WgpuVideoScaler {
    pub fn try_new() -> Option<Self> {
        let instance = wgpu::Instance::default();
        let adapter = pollster_block(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))?;
        let adapter_name = adapter.get_info().name;
        let (device, queue) = pollster_block(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("kontrolfreak-ndi-scaler"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_defaults(),
                memory_hints: wgpu::MemoryHints::Performance,
            },
            None,
        ))
        .ok()?;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("kf-ndi-scale-wgsl"),
            source: wgpu::ShaderSource::Wgsl(WGPU_SCALE_SHADER.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("kf-ndi-scale-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("kf-ndi-scale-pl"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("kf-ndi-scale-pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        Some(Self {
            device,
            queue,
            pipeline,
            bind_group_layout,
            adapter_name,
            cpu_fallback: CpuVideoScaler::new(),
        })
    }

    pub fn adapter_name(&self) -> &str {
        &self.adapter_name
    }

    pub fn transform_frame(
        &mut self,
        input: &VideoFrame,
        config: &NdiTransformConfig,
    ) -> Result<VideoFrame, VideoError> {
        let cfg = config.sanitized();
        if cfg.bypass {
            return Ok(input.clone());
        }
        let rgba = frame_to_rgba8(input)?;
        let rects = compute_transform_rects(input.meta.width, input.meta.height, &cfg);

        let src_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("kf-src"),
            size: rgba.len() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&src_buf, 0, &rgba);

        let out_bytes_len = (cfg.out_width as u64) * (cfg.out_height as u64) * 4;
        let dst_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("kf-dst"),
            size: out_bytes_len,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("kf-readback"),
            size: out_bytes_len,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let params_words: [u32; 16] = [
            input.meta.width,
            input.meta.height,
            cfg.out_width,
            cfg.out_height,
            rects.src_x,
            rects.src_y,
            rects.src_w,
            rects.src_h,
            rects.dst_x,
            rects.dst_y,
            rects.dst_w,
            rects.dst_h,
            (cfg.opacity * 1000.0).round() as u32,
            0,
            0,
            0,
        ];
        let mut params_bytes = Vec::with_capacity(64);
        for w in params_words {
            params_bytes.extend_from_slice(&w.to_le_bytes());
        }
        let params_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("kf-params"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&params_buf, 0, &params_bytes);

        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("kf-ndi-bg"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: src_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: dst_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: params_buf.as_entire_binding(),
                },
            ],
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("kf-ndi-enc"),
            });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("kf-ndi-pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(cfg.out_width.div_ceil(16), cfg.out_height.div_ceil(16), 1);
        }
        encoder.copy_buffer_to_buffer(&dst_buf, 0, &readback_buf, 0, out_bytes_len);
        self.queue.submit(Some(encoder.finish()));

        let slice = readback_buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |res| {
            let _ = tx.send(res);
        });
        self.device.poll(wgpu::Maintain::Wait);
        match rx.recv() {
            Ok(Ok(())) => {
                let data = slice.get_mapped_range().to_vec();
                readback_buf.unmap();
                Ok(VideoFrame {
                    meta: FrameRef {
                        frame_id: input.meta.frame_id,
                        width: cfg.out_width,
                        height: cfg.out_height,
                        format: PixelFormat::Rgba8,
                        source_id: input.meta.source_id,
                        timestamp_ns: input.meta.timestamp_ns,
                    },
                    stride: (cfg.out_width as usize) * 4,
                    data,
                })
            }
            _ => self.cpu_fallback.transform_frame(input, &cfg),
        }
    }
}

#[cfg(feature = "gpu")]
fn pollster_block<F: std::future::Future>(future: F) -> F::Output {
    use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
    fn noop(_: *const ()) {}
    fn clone(p: *const ()) -> RawWaker {
        RawWaker::new(p, &VTABLE)
    }
    static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, noop, noop, noop);
    let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) };
    let mut cx = Context::from_waker(&waker);
    let mut pinned = Box::pin(future);
    loop {
        if let Poll::Ready(val) = pinned.as_mut().poll(&mut cx) {
            return val;
        }
        std::thread::yield_now();
    }
}

/// Adaptive scaler choosing GPU (`wgpu`) when enabled and available, falling back automatically
/// to SIMD CPU (`fast_image_resize`) so headless/CI environments never fail.
pub struct AdaptiveVideoScaler {
    #[cfg(feature = "gpu")]
    gpu: Option<WgpuVideoScaler>,
    cpu: CpuVideoScaler,
}

impl Default for AdaptiveVideoScaler {
    fn default() -> Self {
        Self::new()
    }
}

impl AdaptiveVideoScaler {
    pub fn new() -> Self {
        #[cfg(feature = "gpu")]
        {
            let gpu = WgpuVideoScaler::try_new();
            if let Some(ref g) = gpu {
                tracing::info!("VideoScaler using wgpu GPU backend: {}", g.adapter_name());
            } else {
                tracing::info!("No GPU adapter found; VideoScaler using fast_image_resize SIMD CPU backend");
            }
            Self {
                gpu,
                cpu: CpuVideoScaler::new(),
            }
        }
        #[cfg(not(feature = "gpu"))]
        {
            Self {
                cpu: CpuVideoScaler::new(),
            }
        }
    }

    pub fn backend_name(&self) -> String {
        #[cfg(feature = "gpu")]
        if let Some(ref g) = self.gpu {
            return format!("wgpu ({})", g.adapter_name());
        }
        "cpu-simd (fast_image_resize)".to_string()
    }

    pub fn transform_frame(
        &mut self,
        input: &VideoFrame,
        config: &NdiTransformConfig,
    ) -> Result<VideoFrame, VideoError> {
        #[cfg(feature = "gpu")]
        if let Some(ref mut g) = self.gpu {
            if let Ok(frame) = g.transform_frame(input, config) {
                return Ok(frame);
            }
        }
        self.cpu.transform_frame(input, config)
    }

    pub fn encode_preview_rgba(
        &mut self,
        input: &VideoFrame,
        max_width: u32,
        max_height: u32,
    ) -> Result<VideoFrame, VideoError> {
        self.cpu.encode_preview_rgba(input, max_width, max_height)
    }
}

impl VideoScaler for AdaptiveVideoScaler {
    fn scale_frame(
        &mut self,
        input: &VideoFrame,
        config: &VideoResizeConfig,
    ) -> Result<VideoFrame, VideoError> {
        let m2_cfg = NdiTransformConfig::from(config);
        self.transform_frame(input, &m2_cfg)
    }

    fn encode_preview_jpeg(
        &mut self,
        input: &VideoFrame,
        max_width: u32,
        max_height: u32,
    ) -> Result<VideoPreviewFrame, VideoError> {
        self.cpu.encode_preview_jpeg_frame(input, max_width, max_height)
    }
}
