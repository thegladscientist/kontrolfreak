import React, { useEffect, useRef } from 'react';
import type { Modifier } from '../types/kontrolfreak';
import { sampleTransferCurveLocal } from '../lib/modifierMath';
import { telemetryStore } from '../lib/telemetryStore';

interface TransferCurveCanvasProps {
  modifiers: Modifier[];
  sourcePortId?: string;
  targetPortId?: string;
  calmMode: boolean;
}

export const TransferCurveCanvas: React.FC<TransferCurveCanvasProps> = ({
  modifiers,
  sourcePortId,
  targetPortId,
  calmMode,
}) => {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const points = sampleTransferCurveLocal(modifiers, 96);

    const draw = () => {
      const w = canvas.width;
      const h = canvas.height;
      const pad = 16;
      const plotW = w - pad * 2;
      const plotH = h - pad * 2;

      ctx.clearRect(0, 0, w, h);
      ctx.fillStyle = '#0b0e14';
      ctx.fillRect(0, 0, w, h);

      // Subtle grid
      ctx.strokeStyle = '#1e293b';
      ctx.lineWidth = 1;
      for (let i = 0; i <= 4; i++) {
        const gx = pad + (plotW * i) / 4;
        const gy = pad + (plotH * i) / 4;
        ctx.beginPath();
        ctx.moveTo(gx, pad);
        ctx.lineTo(gx, h - pad);
        ctx.stroke();

        ctx.beginPath();
        ctx.moveTo(pad, gy);
        ctx.lineTo(w - pad, gy);
        ctx.stroke();
      }

      // Linear reference diagonal
      ctx.strokeStyle = '#334155';
      ctx.setLineDash([4, 4]);
      ctx.beginPath();
      ctx.moveTo(pad, h - pad);
      ctx.lineTo(w - pad, pad);
      ctx.stroke();
      ctx.setLineDash([]);

      // Normalize Y bounds cleanly (default 0..1, expand if RangeMap maps outside 0..1)
      let yMin = 0;
      let yMax = 1;
      for (const [, y] of points) {
        if (y < yMin) yMin = y;
        if (y > yMax) yMax = y;
      }
      const ySpan = Math.max(1e-4, yMax - yMin);

      // Transfer curve stroke
      ctx.strokeStyle = '#f59e0b';
      ctx.lineWidth = 2.25;
      ctx.beginPath();
      points.forEach(([x, y], idx) => {
        const px = pad + x * plotW;
        const py = h - pad - ((y - yMin) / ySpan) * plotH;
        if (idx === 0) ctx.moveTo(px, py);
        else ctx.lineTo(px, py);
      });
      ctx.stroke();

      // Live input/output crosshair dot from high-frequency telemetryStore
      if (sourcePortId) {
        const src = telemetryStore.getPortLatest(sourcePortId);
        const dst = targetPortId ? telemetryStore.getPortLatest(targetPortId) : undefined;
        if (src && typeof src.numeric_value === 'number') {
          const ix = Math.min(1, Math.max(0, src.numeric_value));
          const oy =
            dst && typeof dst.numeric_value === 'number'
              ? dst.numeric_value
              : points[Math.min(points.length - 1, Math.round(ix * (points.length - 1)))][1];
          const px = pad + ix * plotW;
          const py = h - pad - ((oy - yMin) / ySpan) * plotH;

          ctx.fillStyle = calmMode ? 'rgba(34, 211, 238, 0.7)' : '#22d3ee';
          ctx.beginPath();
          ctx.arc(px, py, calmMode ? 4 : 5, 0, Math.PI * 2);
          ctx.fill();
        }
      }

      // Axis labels
      ctx.fillStyle = '#64748b';
      ctx.font = '10px ui-monospace, monospace';
      ctx.fillText(`OUT ${yMax.toFixed(1)}`, pad + 3, pad + 10);
      ctx.fillText(`0 → 1 IN`, w - pad - 46, h - 4);
    };

    draw();
    const unsub = telemetryStore.subscribe(() => draw());
    return unsub;
  }, [modifiers, sourcePortId, targetPortId, calmMode]);

  return (
    <div className="rounded-lg overflow-hidden border border-studio-border bg-studio-bg">
      <canvas
        ref={canvasRef}
        width={260}
        height={138}
        className="w-full h-[138px] block"
        aria-label="Wire modifier transfer curve preview"
      />
    </div>
  );
};
