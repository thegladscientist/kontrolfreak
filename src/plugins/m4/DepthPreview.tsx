import React, { useCallback, useEffect, useRef } from 'react';
import type { DepthPreviewPacket, NamedDepthRoi } from './types';

interface Props {
  packet: DepthPreviewPacket | null;
  roi: NamedDepthRoi | null;
  interactive?: boolean;
  onRoiChange?: (roi: NamedDepthRoi) => void;
  className?: string;
}

const MIN_SIZE = 0.05;
const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

type DragMode = 'move' | 'resize' | null;

/** Live colorized depth `<canvas>` with interactive 3D ROI box (front face = XY, back face offset = Z span). */
export const DepthPreviewCanvas: React.FC<Props> = ({
  packet,
  roi,
  interactive = false,
  onRoiChange,
  className = '',
}) => {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const hostRef = useRef<HTMLDivElement | null>(null);
  const drag = useRef<{ mode: DragMode; startX: number; startY: number; roi: NamedDepthRoi } | null>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || !packet?.rgba_pixels) return;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;
    if (canvas.width !== packet.width) canvas.width = packet.width;
    if (canvas.height !== packet.height) canvas.height = packet.height;
    const pixels = new Uint8ClampedArray(packet.rgba_pixels.length);
    pixels.set(packet.rgba_pixels);
    ctx.putImageData(new ImageData(pixels, packet.width, packet.height), 0, 0);
  }, [packet]);

  const onPointerDown = useCallback(
    (mode: DragMode) => (e: React.PointerEvent) => {
      if (!interactive || !roi) return;
      e.stopPropagation();
      e.preventDefault();
      (e.target as Element).setPointerCapture?.(e.pointerId);
      drag.current = { mode, startX: e.clientX, startY: e.clientY, roi: { ...roi } };
    },
    [interactive, roi]
  );

  const onPointerMove = useCallback(
    (e: React.PointerEvent) => {
      const d = drag.current;
      const host = hostRef.current;
      if (!d || !host || !onRoiChange) return;
      const rect = host.getBoundingClientRect();
      if (rect.width === 0 || rect.height === 0) return;
      const dx = (e.clientX - d.startX) / rect.width;
      const dy = (e.clientY - d.startY) / rect.height;
      if (d.mode === 'move') {
        onRoiChange({
          ...d.roi,
          x: clamp(d.roi.x + dx, 0, 1 - d.roi.w),
          y: clamp(d.roi.y + dy, 0, 1 - d.roi.h),
        });
      } else if (d.mode === 'resize') {
        onRoiChange({
          ...d.roi,
          w: clamp(d.roi.w + dx, MIN_SIZE, 1 - d.roi.x),
          h: clamp(d.roi.h + dy, MIN_SIZE, 1 - d.roi.y),
        });
      }
    },
    [onRoiChange]
  );

  const endDrag = useCallback(() => {
    drag.current = null;
  }, []);

  const result = packet?.roi_results[0];
  const aspect = packet ? packet.width / packet.height : 4 / 3;
  const zSpan = roi ? roi.far_mm - roi.near_mm : 0;
  const off = clamp(zSpan / 20000, 0.015, 0.09);

  return (
    <div
      ref={hostRef}
      className={`relative w-full overflow-hidden rounded-md border border-studio-border bg-black select-none ${className}`}
      style={{ aspectRatio: String(aspect) }}
      onPointerMove={onPointerMove}
      onPointerUp={endDrag}
      onPointerCancel={endDrag}
      data-testid="depth-preview"
    >
      <canvas
        ref={canvasRef}
        width={packet?.width ?? 160}
        height={packet?.height ?? 120}
        className="absolute inset-0 w-full h-full"
        style={{ imageRendering: 'pixelated' }}
      />

      {roi && (
        <svg
          className="absolute inset-0 w-full h-full pointer-events-none"
          viewBox="0 0 100 100"
          preserveAspectRatio="none"
        >
          {(() => {
            const fx = roi.x * 100;
            const fy = roi.y * 100;
            const fw = roi.w * 100;
            const fh = roi.h * 100;
            const bx = Math.min(100 - fw, fx + off * 100);
            const by = Math.max(0, fy - off * 100);
            return (
              <g fill="none" strokeWidth={0.5} vectorEffect="non-scaling-stroke">
                <rect x={bx} y={by} width={fw} height={fh} stroke="#a78bfa" strokeDasharray="2 1.5" opacity={0.8} />
                <line x1={fx} y1={fy} x2={bx} y2={by} stroke="#a78bfa" opacity={0.7} />
                <line x1={fx + fw} y1={fy} x2={bx + fw} y2={by} stroke="#a78bfa" opacity={0.7} />
                <line x1={fx} y1={fy + fh} x2={bx} y2={by + fh} stroke="#a78bfa" opacity={0.7} />
                <line x1={fx + fw} y1={fy + fh} x2={bx + fw} y2={by + fh} stroke="#a78bfa" opacity={0.7} />
                <rect
                  x={fx}
                  y={fy}
                  width={fw}
                  height={fh}
                  stroke={result?.presence ? '#34d399' : '#fbbf24'}
                  strokeWidth={0.9}
                  fill={result?.presence ? 'rgba(52,211,153,0.10)' : 'rgba(251,191,36,0.06)'}
                />
                {result && result.valid_pixel_count > 0 && (
                  <>
                    <circle
                      cx={result.closest_point_norm[0] * 100}
                      cy={result.closest_point_norm[1] * 100}
                      r={1.6}
                      stroke="#ffffff"
                      strokeWidth={0.6}
                    />
                    <circle
                      cx={result.centroid_norm[0] * 100}
                      cy={result.centroid_norm[1] * 100}
                      r={1.2}
                      stroke="#22d3ee"
                      strokeWidth={0.6}
                    />
                  </>
                )}
              </g>
            );
          })()}
        </svg>
      )}

      {roi && interactive && (
        <>
          <div
            data-testid="roi-move-handle"
            onPointerDown={onPointerDown('move')}
            className="absolute cursor-move"
            style={{
              left: `${roi.x * 100}%`,
              top: `${roi.y * 100}%`,
              width: `${roi.w * 100}%`,
              height: `${roi.h * 100}%`,
            }}
          />
          <div
            data-testid="roi-resize-handle"
            onPointerDown={onPointerDown('resize')}
            className="absolute w-3 h-3 -ml-1.5 -mt-1.5 rounded-sm bg-emerald-400 border border-black cursor-nwse-resize"
            style={{ left: `${(roi.x + roi.w) * 100}%`, top: `${(roi.y + roi.h) * 100}%` }}
          />
        </>
      )}

      {roi && (
        <div className="absolute left-1 bottom-1 text-[9px] font-mono px-1 rounded bg-black/70 text-slate-200">
          {result && result.valid_pixel_count > 0
            ? `${(result.min_depth_mm / 1000).toFixed(2)} m · fill ${(result.fill_ratio * 100).toFixed(0)}%`
            : 'no depth in ROI'}
        </div>
      )}
    </div>
  );
};
