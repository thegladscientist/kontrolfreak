import type { CropRect, FitMode } from './model';

export interface FittedRect {
  sx: number;
  sy: number;
  sw: number;
  sh: number;
  dx: number;
  dy: number;
  dw: number;
  dh: number;
}

/** Places a cropped source inside the output buffer using contain, cover, or stretch. */
export function fitRect(
  srcW: number,
  srcH: number,
  dstW: number,
  dstH: number,
  fit: FitMode,
  crop: CropRect
): FittedRect {
  const safeSrcW = Math.max(1, srcW);
  const safeSrcH = Math.max(1, srcH);
  const safeDstW = Math.max(1, dstW);
  const safeDstH = Math.max(1, dstH);
  const sx = clamp01(crop.x) * safeSrcW;
  const sy = clamp01(crop.y) * safeSrcH;
  let sw = clamp(crop.w, 0.05, 1) * safeSrcW;
  let sh = clamp(crop.h, 0.05, 1) * safeSrcH;
  if (sx + sw > safeSrcW) sw = safeSrcW - sx;
  if (sy + sh > safeSrcH) sh = safeSrcH - sy;
  sw = Math.max(1, sw);
  sh = Math.max(1, sh);

  if (fit === 'stretch') {
    return { sx, sy, sw, sh, dx: 0, dy: 0, dw: safeDstW, dh: safeDstH };
  }

  const scale =
    fit === 'cover'
      ? Math.max(safeDstW / sw, safeDstH / sh)
      : Math.min(safeDstW / sw, safeDstH / sh);
  const dw = sw * scale;
  const dh = sh * scale;
  return {
    sx,
    sy,
    sw,
    sh,
    dx: (safeDstW - dw) / 2,
    dy: (safeDstH - dh) / 2,
    dw,
    dh,
  };
}

function clamp01(value: number): number {
  return clamp(value, 0, 1);
}

function clamp(value: number, min: number, max: number): number {
  if (Number.isNaN(value)) return min;
  return Math.min(max, Math.max(min, value));
}
