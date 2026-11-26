import React, { useEffect, useRef } from 'react';
import { Camera, Crosshair, Eye, Zap } from 'lucide-react';
import type { Device } from '../../types/kontrolfreak';
import { useM3Store } from './m3Store';
import { renderSyntheticCameraFrame } from './m3Bridge';
import { DEFAULT_RGB_CV_CONFIG } from './m3Types';

/**
 * Compact live <canvas> preview inside an `RgbCamera` node on the canvas.
 * Draws the synthetic RGB frame plus ROI box, target-color blob centroid crosshair,
 * brightest-point marker, and active motion zone highlights.
 */
export const RgbCameraNodeBody: React.FC<{ device: Device; calmMode: boolean }> = ({
  device,
  calmMode,
}) => {
  const cameraId =
    device.kind.type === 'RgbCamera' ? device.kind.camera_id : 'sim://camera/studio-cv';
  const stepRgbCamera = useM3Store((s) => s.stepRgbCamera);
  const cvConfig = useM3Store((s) => s.cvConfigs[cameraId] ?? DEFAULT_RGB_CV_CONFIG);
  const cvOut = useM3Store((s) => s.cvLatest[cameraId]);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);

  useEffect(() => {
    if (device.status.state !== 'online') return;
    const intervalMs = calmMode ? 220 : 100;
    const timer = setInterval(() => {
      void stepRgbCamera(cameraId, device.id);
    }, intervalMs);
    return () => clearInterval(timer);
  }, [cameraId, device.id, device.status.state, calmMode, stepRgbCamera]);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const w = canvas.width;
    const h = canvas.height;
    const frameIndex = cvOut?.frame_index ?? 0;
    const rgba = renderSyntheticCameraFrame(w, h, frameIndex);

    if (typeof ImageData !== 'undefined') {
      const img = new ImageData(new Uint8ClampedArray(rgba), w, h);
      ctx.putImageData(img, 0, 0);
    } else {
      ctx.fillStyle = '#0f172a';
      ctx.fillRect(0, 0, w, h);
    }

    // Motion zone grid overlay
    const zCols = Math.max(1, cvConfig.zone_cols);
    const zRows = Math.max(1, cvConfig.zone_rows);
    const zw = w / zCols;
    const zh = h / zRows;
    if (cvOut?.cv.zones) {
      for (const z of cvOut.cv.zones) {
        if (z.triggered) {
          ctx.fillStyle = calmMode ? 'rgba(245, 158, 11, 0.14)' : 'rgba(245, 158, 11, 0.28)';
          ctx.fillRect(z.col * zw, z.row * zh, zw, zh);
        }
        ctx.strokeStyle = 'rgba(148, 163, 184, 0.22)';
        ctx.lineWidth = 1;
        ctx.strokeRect(z.col * zw, z.row * zh, zw, zh);
      }
    }

    // ROI dashed rectangle
    ctx.save();
    ctx.strokeStyle = '#38bdf8';
    ctx.lineWidth = 1;
    ctx.setLineDash([3, 2]);
    ctx.strokeRect(
      cvConfig.roi.x * w,
      cvConfig.roi.y * h,
      cvConfig.roi.w * w,
      cvConfig.roi.h * h
    );
    ctx.restore();

    // Blob centroid crosshair
    if (cvOut && cvOut.cv.blob_area > 0.002) {
      const bx = cvOut.cv.blob_centroid[0] * w;
      const by = cvOut.cv.blob_centroid[1] * h;
      ctx.strokeStyle = '#10b981';
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      ctx.moveTo(bx - 7, by);
      ctx.lineTo(bx + 7, by);
      ctx.moveTo(bx, by - 7);
      ctx.lineTo(bx, by + 7);
      ctx.stroke();
      ctx.beginPath();
      ctx.arc(bx, by, 5, 0, Math.PI * 2);
      ctx.stroke();
    }
  }, [cvOut, cvConfig, calmMode]);

  const luma = cvOut?.cv.global_luma ?? 0;
  const motion = cvOut?.cv.motion_energy ?? 0;
  const blobArea = cvOut?.cv.blob_area ?? 0;

  return (
    <div className="space-y-1.5">
      <div className="relative rounded-md overflow-hidden border border-studio-border bg-studio-bg">
        <canvas
          ref={canvasRef}
          width={160}
          height={96}
          className="w-full h-24 block"
          aria-label="RGB Camera CV preview"
        />
        <div className="absolute top-1 left-1 px-1.5 py-0.5 rounded bg-black/65 text-[9px] font-mono text-emerald-300 flex items-center gap-1">
          <Camera className="w-2.5 h-2.5" />
          CV Live
        </div>
      </div>

      <div className="grid grid-cols-3 gap-1 text-[9px] font-mono text-slate-300">
        <div className="px-1.5 py-0.5 rounded bg-studio-bg border border-studio-border">
          Luma {(luma * 100).toFixed(0)}%
        </div>
        <div className="px-1.5 py-0.5 rounded bg-studio-bg border border-studio-border">
          Mot {(motion * 100).toFixed(0)}%
        </div>
        <div className="px-1.5 py-0.5 rounded bg-studio-bg border border-studio-border">
          Area {(blobArea * 100).toFixed(1)}%
        </div>
      </div>
    </div>
  );
};

/**
 * Single-focus Inspector extension panel for `RgbCamera` nodes:
 * shows live CV telemetry bars, dominant color swatch, motion zone grid, and ROI/HSV controls.
 */
export const RgbCameraInspectorPanel: React.FC<{ device?: Device }> = ({ device }) => {
  const cameraId =
    device?.kind.type === 'RgbCamera' ? device.kind.camera_id : 'sim://camera/studio-cv';
  const config = useM3Store((s) => s.cvConfigs[cameraId] ?? DEFAULT_RGB_CV_CONFIG);
  const latest = useM3Store((s) => s.cvLatest[cameraId]);
  const updateRgbCvConfig = useM3Store((s) => s.updateRgbCvConfig);
  const stepRgbCamera = useM3Store((s) => s.stepRgbCamera);

  const cv = latest?.cv;
  const domRgb = cv
    ? `rgb(${Math.round(cv.dominant_rgb[0] * 255)}, ${Math.round(cv.dominant_rgb[1] * 255)}, ${Math.round(cv.dominant_rgb[2] * 255)})`
    : 'rgb(30, 41, 59)';

  const patchConfig = (partial: Partial<typeof config>) => {
    void updateRgbCvConfig(cameraId, { ...config, ...partial });
  };

  return (
    <div className="space-y-3 text-xs">
      {/* Live CV Meters */}
      <div className="p-2 rounded-lg bg-studio-bg border border-studio-border space-y-2">
        <div className="flex items-center justify-between text-[11px]">
          <span className="flex items-center gap-1 text-emerald-300 font-medium">
            <Eye className="w-3.5 h-3.5" />
            CPU CV Telemetry
          </span>
          <button
            onClick={() => void stepRgbCamera(cameraId, device?.id)}
            className="text-[10px] text-cyan-300 hover:text-cyan-200 underline"
          >
            Step Frame
          </button>
        </div>

        {[
          { label: 'Global Luma', val: cv?.global_luma ?? 0, color: 'bg-amber-400' },
          { label: 'ROI Luma', val: cv?.roi_luma ?? 0, color: 'bg-amber-300' },
          { label: 'Motion Energy', val: cv?.motion_energy ?? 0, color: 'bg-cyan-400' },
          { label: 'Blob Area', val: Math.min(1, (cv?.blob_area ?? 0) * 5), color: 'bg-emerald-400' },
        ].map((m) => (
          <div key={m.label} className="space-y-0.5">
            <div className="flex justify-between text-[10px]">
              <span className="text-studio-muted">{m.label}</span>
              <span className="font-mono text-slate-200">{(m.val * 100).toFixed(1)}%</span>
            </div>
            <div className="h-1.5 rounded-full bg-studio-panel overflow-hidden">
              <div
                className={`h-full ${m.color} transition-all duration-100`}
                style={{ width: `${Math.round(Math.min(1, Math.max(0, m.val)) * 100)}%` }}
              />
            </div>
          </div>
        ))}

        <div className="flex items-center justify-between pt-1 border-t border-studio-border/60 text-[10px]">
          <span className="flex items-center gap-1.5 text-studio-muted">
            <span
              className="w-3.5 h-3.5 rounded border border-white/20 inline-block"
              style={{ backgroundColor: domRgb }}
            />
            Dominant Color
          </span>
          <span className="font-mono text-slate-300">
            <Crosshair className="w-3 h-3 inline mr-1 text-emerald-400" />
            ({(cv?.blob_centroid[0] ?? 0.5).toFixed(2)}, {(cv?.blob_centroid[1] ?? 0.5).toFixed(2)})
          </span>
        </div>
      </div>

      {/* Motion Zone Trigger Grid */}
      <div className="space-y-1">
        <div className="text-[10px] font-semibold uppercase tracking-wider text-studio-muted flex items-center gap-1">
          <Zap className="w-3 h-3 text-amber-400" />
          Motion Zone Triggers ({config.zone_cols}×{config.zone_rows})
        </div>
        <div
          className="grid gap-1"
          style={{ gridTemplateColumns: `repeat(${config.zone_cols}, minmax(0, 1fr))` }}
        >
          {(cv?.zones ?? [
            { index: 0, col: 0, row: 0, energy: 0, triggered: false },
            { index: 1, col: 1, row: 0, energy: 0, triggered: false },
            { index: 2, col: 0, row: 1, energy: 0, triggered: false },
            { index: 3, col: 1, row: 1, energy: 0, triggered: false },
          ]).map((z) => (
            <div
              key={z.index}
              className={`px-2 py-1.5 rounded border text-center font-mono text-[10px] ${
                z.triggered
                  ? 'bg-amber-500/25 border-amber-400 text-amber-200 font-semibold'
                  : 'bg-studio-bg border-studio-border text-studio-muted'
              }`}
            >
              Z{z.index} · {(z.energy * 100).toFixed(0)}%
            </div>
          ))}
        </div>
      </div>

      {/* Target Hue & Motion Threshold Tuning */}
      <div className="space-y-2 p-2 rounded-lg bg-studio-panel border border-studio-border text-[11px]">
        <label className="block space-y-0.5">
          <div className="flex justify-between text-[10px]">
            <span className="text-studio-muted">Target Blob Hue</span>
            <span className="font-mono text-emerald-300">{Math.round(config.target_hue_deg)}°</span>
          </div>
          <input
            type="range"
            min={0}
            max={360}
            value={config.target_hue_deg}
            onChange={(e) => patchConfig({ target_hue_deg: Number(e.target.value) })}
            className="w-full accent-emerald-400 h-1.5"
          />
        </label>

        <label className="block space-y-0.5">
          <div className="flex justify-between text-[10px]">
            <span className="text-studio-muted">Zone Trigger Threshold</span>
            <span className="font-mono text-amber-300">
              {(config.zone_trigger_threshold * 100).toFixed(0)}%
            </span>
          </div>
          <input
            type="range"
            min={0.01}
            max={0.35}
            step={0.01}
            value={config.zone_trigger_threshold}
            onChange={(e) => patchConfig({ zone_trigger_threshold: Number(e.target.value) })}
            className="w-full accent-amber-400 h-1.5"
          />
        </label>
      </div>
    </div>
  );
};
