import type { DepthColormap, NamedDepthRoi, RoiExtractionResult } from './types';

const HFOV_RAD = (69.4 * Math.PI) / 180;
const VFOV_RAD = (42.5 * Math.PI) / 180;
const HALF_W_TAN = Math.tan(HFOV_RAD * 0.5);
const HALF_H_TAN = Math.tan(VFOV_RAD * 0.5);

export function createDefaultRoi(): NamedDepthRoi {
  return {
    id: 'roi-main',
    name: 'Main 3D Zone',
    x: 0.15,
    y: 0.15,
    w: 0.7,
    h: 0.7,
    near_mm: 300,
    far_mm: 2500,
    occupancy_threshold: 0.05,
  };
}

/** Generates a 16-bit millimeter synthetic depth scene matching Rust `SyntheticDepthBackend`. */
export function generateSyntheticDepthFrame(
  width: number,
  height: number,
  phase: number
): Uint16Array {
  const buf = new Uint16Array(width * height);
  const orb1Cx = 0.5 + 0.24 * Math.cos(phase * 0.9);
  const orb1Cy = 0.48 + 0.18 * Math.sin(phase * 1.3);
  const orb1Z = 920 + 360 * Math.sin(phase * 0.7);
  const orb1R = 0.155;
  const orb1RSq = orb1R * orb1R;

  const orb2Cx = 0.5 - 0.28 * Math.sin(phase * 0.65);
  const orb2Cy = 0.55 + 0.2 * Math.cos(phase * 0.85);
  const orb2Z = 1480 + 300 * Math.cos(phase * 0.5 + 1.2);
  const orb2R = 0.125;
  const orb2RSq = orb2R * orb2R;

  const aspect = height / Math.max(1, width);
  const invW = 1 / Math.max(1, width - 1);
  const invH = 1 / Math.max(1, height - 1);

  for (let py = 0; py < height; py++) {
    const yn = py * invH;
    const rowOff = py * width;
    const sinYn = Math.sin(yn * Math.PI);
    for (let px = 0; px < width; px++) {
      if (px === 0 || py === 0) {
        buf[rowOff + px] = 0;
        continue;
      }
      const xn = px * invW;
      const wave = 75 * Math.sin(xn * 2 * Math.PI * 1.5 + phase) * sinYn;
      let z = 2680 + (yn - 0.5) * 260 + wave;

      const dx1 = xn - orb1Cx;
      const dy1 = (yn - orb1Cy) * aspect;
      const d1Sq = dx1 * dx1 + dy1 * dy1;
      if (d1Sq < orb1RSq) {
        const bulge = (Math.sqrt(orb1RSq - d1Sq) / orb1R) * 210;
        const surf = Math.max(320, orb1Z - bulge);
        if (surf < z) z = surf;
      }

      const dx2 = xn - orb2Cx;
      const dy2 = (yn - orb2Cy) * aspect;
      const d2Sq = dx2 * dx2 + dy2 * dy2;
      if (d2Sq < orb2RSq) {
        const bulge = (Math.sqrt(orb2RSq - d2Sq) / orb2R) * 165;
        const surf = Math.max(400, orb2Z - bulge);
        if (surf < z) z = surf;
      }

      buf[rowOff + px] = Math.min(6000, Math.max(200, Math.round(z)));
    }
  }
  return buf;
}

function unprojectMeters(u: number, v: number, zMm: number): [number, number, number] {
  const zM = zMm / 1000;
  const xM = (u * 2 - 1) * HALF_W_TAN * zM;
  const yM = (0.5 - v) * 2 * HALF_H_TAN * zM;
  return [xM, yM, zM];
}

export function extractRoiJs(
  depthMm: Uint16Array,
  width: number,
  height: number,
  roi: NamedDepthRoi
): RoiExtractionResult {
  const x0 = Math.min(width - 1, Math.max(0, Math.floor(roi.x * width)));
  const y0 = Math.min(height - 1, Math.max(0, Math.floor(roi.y * height)));
  const x1 = Math.min(width, Math.max(x0 + 1, Math.ceil((roi.x + roi.w) * width)));
  const y1 = Math.min(height, Math.max(y0 + 1, Math.ceil((roi.y + roi.h) * height)));

  const near = Math.max(1, roi.near_mm);
  const far = Math.max(near + 1, roi.far_mm);
  const zSpan = far - near;
  const totalPixels = (x1 - x0) * (y1 - y0);

  let validCount = 0;
  let minD = 65535;
  let maxD = 0;
  let sumD = 0;
  let closestXSum = 0;
  let closestYSum = 0;
  let closestTies = 0;
  let centXSum = 0;
  let centYSum = 0;
  let centZSum = 0;

  const invW = 1 / Math.max(1, width - 1);
  const invH = 1 / Math.max(1, height - 1);

  for (let py = y0; py < y1; py++) {
    const rowOff = py * width;
    const yn = py * invH;
    for (let px = x0; px < x1; px++) {
      const d = depthMm[rowOff + px];
      if (d === 0 || d < near || d > far) continue;
      const xn = px * invW;
      validCount++;
      sumD += d;
      if (d < minD) {
        minD = d;
        closestXSum = xn;
        closestYSum = yn;
        closestTies = 1;
      } else if (d === minD) {
        closestXSum += xn;
        closestYSum += yn;
        closestTies++;
      }
      if (d > maxD) maxD = d;
      centXSum += xn;
      centYSum += yn;
      centZSum += d;
    }
  }

  if (validCount === 0) {
    return {
      roi_id: roi.id,
      roi_name: roi.name,
      min_depth_mm: 0,
      max_depth_mm: 0,
      mean_depth_mm: 0,
      presence: false,
      fill_ratio: 0,
      valid_pixel_count: 0,
      total_pixel_count: totalPixels,
      closest_point_norm: [0.5, 0.5, 1.0],
      closest_point_meters: [0, 0, 0],
      centroid_norm: [0.5, 0.5, 1.0],
      centroid_meters: [0, 0, 0],
      confidence: 0,
    };
  }

  const fillRatio = validCount / Math.max(1, totalPixels);
  const closestU = closestXSum / Math.max(1, closestTies);
  const closestV = closestYSum / Math.max(1, closestTies);
  const closestZNorm = Math.min(1, Math.max(0, (minD - near) / zSpan));

  const centU = centXSum / validCount;
  const centV = centYSum / validCount;
  const centZMm = centZSum / validCount;
  const centZNorm = Math.min(1, Math.max(0, (centZMm - near) / zSpan));

  return {
    roi_id: roi.id,
    roi_name: roi.name,
    min_depth_mm: minD,
    max_depth_mm: maxD,
    mean_depth_mm: sumD / validCount,
    presence: fillRatio >= roi.occupancy_threshold,
    fill_ratio: fillRatio,
    valid_pixel_count: validCount,
    total_pixel_count: totalPixels,
    closest_point_norm: [closestU, closestV, closestZNorm],
    closest_point_meters: unprojectMeters(closestU, closestV, minD),
    centroid_norm: [centU, centV, centZNorm],
    centroid_meters: unprojectMeters(centU, centV, centZMm),
    confidence: Math.min(1, Math.max(0, fillRatio * 4)),
  };
}

function turboRgb(t: number): [number, number, number] {
  const x = Math.min(1, Math.max(0, t));
  const r = Math.min(
    1,
    Math.max(
      0,
      0.13572138 +
        x * (4.6153926 + x * (-42.66032258 + x * (132.13108234 + x * (-152.94239396 + x * 59.28637943))))
    )
  );
  const g = Math.min(
    1,
    Math.max(
      0,
      0.09140261 +
        x * (2.19418839 + x * (4.84296658 + x * (-14.18503333 + x * (4.27729857 + x * 2.82956604))))
    )
  );
  const b = Math.min(
    1,
    Math.max(
      0,
      0.1066733 +
        x * (12.64194608 + x * (-60.58204836 + x * (110.36276771 + x * (-89.90310912 + x * 27.34824973))))
    )
  );
  return [Math.round(r * 255), Math.round(g * 255), Math.round(b * 255)];
}

const VIRIDIS_STOPS: Array<[number, number, number]> = [
  [68, 1, 84],
  [59, 82, 139],
  [33, 145, 140],
  [94, 201, 98],
  [253, 231, 37],
];

function viridisRgb(t: number): [number, number, number] {
  const x = Math.min(1, Math.max(0, t)) * 4;
  const idx = Math.min(3, Math.floor(x));
  const f = x - idx;
  const a = VIRIDIS_STOPS[idx];
  const b = VIRIDIS_STOPS[idx + 1];
  return [
    Math.round(a[0] + (b[0] - a[0]) * f),
    Math.round(a[1] + (b[1] - a[1]) * f),
    Math.round(a[2] + (b[2] - a[2]) * f),
  ];
}

export function colorizeDepthToRgbaJs(
  depthMm: Uint16Array,
  width: number,
  height: number,
  nearMm: number,
  farMm: number,
  palette: DepthColormap
): Uint8ClampedArray {
  const count = width * height;
  const out = new Uint8ClampedArray(count * 4);
  const near = Math.max(1, nearMm);
  const far = Math.max(near + 1, farMm);
  const invSpan = 1 / (far - near);

  for (let i = 0; i < count; i++) {
    const d = depthMm[i];
    const base = i * 4;
    if (d === 0) {
      out[base] = 11;
      out[base + 1] = 14;
      out[base + 2] = 20;
      out[base + 3] = 255;
      continue;
    }

    let r = 0;
    let g = 0;
    let b = 0;

    if (palette === 'turbo') {
      const t = 1 - Math.min(1, Math.max(0, (d - near) * invSpan));
      [r, g, b] = turboRgb(t);
    } else if (palette === 'viridis') {
      const t = 1 - Math.min(1, Math.max(0, (d - near) * invSpan));
      [r, g, b] = viridisRgb(t);
    } else if (palette === 'grayscale') {
      const t = 1 - Math.min(1, Math.max(0, (d - near) * invSpan));
      const v = Math.round(t * 255);
      r = v;
      g = v;
      b = v;
    } else {
      if (d < near || d > far) {
        r = 15;
        g = 20;
        b = 30;
      } else {
        const t = 1 - Math.min(1, Math.max(0, (d - near) * invSpan));
        const band = (t * 8) % 1 < 0.08 ? 1.15 : 1.0;
        r = Math.min(255, Math.round((18 + t * 55) * band));
        g = Math.min(255, Math.round((130 + t * 125) * band));
        b = Math.min(255, Math.round((165 + (1 - t) * 70) * band));
      }
    }

    out[base] = r;
    out[base + 1] = g;
    out[base + 2] = b;
    out[base + 3] = 255;
  }

  return out;
}

export function decodeBase64ToRgba(b64: string): Uint8ClampedArray {
  const bin = atob(b64);
  const arr = new Uint8ClampedArray(bin.length);
  for (let i = 0; i < bin.length; i++) {
    arr[i] = bin.charCodeAt(i);
  }
  return arr;
}
