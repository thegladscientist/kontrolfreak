import React, { useEffect, useRef, useState } from 'react';
import { AlertCircle, CheckCircle2, Film } from 'lucide-react';
import { isTauriRuntime } from '../../lib/bridge';
import { useWorkspaceStore } from '../../store/workspaceStore';
import type { Device } from '../../types/kontrolfreak';
import {
  browserM2Mock,
  m2Bridge,
  renderSyntheticPatternToCanvas,
  type NdiNodeState,
  type NdiRuntimeStatus,
  type VideoSourceInfo,
} from './m2Bridge';

export const NdiNodeBody: React.FC<{ device: Device; calmMode: boolean }> = ({
  device,
  calmMode,
}) => {
  const updateDeviceKind = useWorkspaceStore((s) => s.updateDeviceKind);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const phaseRef = useRef(0);

  const [nodeState, setNodeState] = useState<NdiNodeState>(() =>
    browserM2Mock.getNdiNodeState(device.id, device)
  );
  const [runtime, setRuntime] = useState<NdiRuntimeStatus>(() =>
    browserM2Mock.getNdiRuntimeStatus()
  );
  const [sources, setSources] = useState<VideoSourceInfo[]>(() =>
    browserM2Mock.discoverNdiSources()
  );

  // Sync initial runtime + node state + discovered sources
  useEffect(() => {
    let alive = true;
    void (async () => {
      const [rt, srcs, st] = await Promise.all([
        m2Bridge.getNdiRuntimeStatus(),
        m2Bridge.discoverNdiSources(),
        m2Bridge.getNdiNodeState(device.id, device),
      ]);
      if (!alive) return;
      setRuntime(rt);
      setSources(srcs);
      setNodeState(st);
    })();

    const unsub = browserM2Mock.subscribe(() => {
      if (!isTauriRuntime()) {
        setNodeState(browserM2Mock.getNdiNodeState(device.id, device));
      }
    });
    return () => {
      alive = false;
      unsub();
    };
  }, [device]);

  // Live preview loop (10-15 fps throttled, or 4 fps in Calm Mode, zero React re-renders per frame)
  useEffect(() => {
    const intervalMs = calmMode ? 250 : 80;
    let cancelled = false;
    let inFlight = false;

    const tick = async () => {
      if (cancelled || inFlight) return;
      const canvas = canvasRef.current;
      if (!canvas) return;
      inFlight = true;

      try {
        if (isTauriRuntime()) {
          const payload = await m2Bridge.getNdiPreviewFrame(device.id, 224, 126);
          if (payload && !cancelled) {
            const img = new Image();
            img.onload = () => {
              const ctx = canvas.getContext('2d');
              if (!ctx) return;
              ctx.fillStyle = '#05070c';
              ctx.fillRect(0, 0, canvas.width, canvas.height);
              const dx = Math.round((canvas.width - payload.thumb_width) / 2);
              const dy = Math.round((canvas.height - payload.thumb_height) / 2);
              ctx.drawImage(img, dx, dy, payload.thumb_width, payload.thumb_height);
            };
            img.src = `data:image/jpeg;base64,${payload.jpeg_base64}`;
          }
        } else {
          const st = browserM2Mock.getNdiNodeState(device.id, device);
          const effective = browserM2Mock.getEffectiveTransform(device);
          if (!effective.freeze) {
            phaseRef.current += calmMode ? 0.04 : 0.09;
          }
          renderSyntheticPatternToCanvas(canvas, st.test_pattern, effective, phaseRef.current);
        }
      } finally {
        inFlight = false;
      }
    };

    void tick();
    const timer = setInterval(() => {
      void tick();
    }, intervalMs);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, [device, calmMode]);

  const handleSelectSource = async (nextSource: string) => {
    const updated = await m2Bridge.configureNdiNode(
      device.id,
      { source_name: nextSource },
      device
    );
    setNodeState(updated);
    if (device.kind.type === 'NdiVideoNode') {
      await updateDeviceKind(device.id, {
        ...device.kind,
        source_name: nextSource,
      });
    }
  };

  const pendingResize = nodeState.pending_width !== null || nodeState.pending_height !== null;

  return (
    <div className="space-y-2 nodrag">
      {/* Live Video Preview Canvas */}
      <div className="relative rounded-lg overflow-hidden border border-violet-500/40 bg-[#05070c]">
        <canvas
          ref={canvasRef}
          width={224}
          height={126}
          className="w-full h-[116px] block object-contain"
        />

        {/* Resolution + FPS Badge */}
        <div className="absolute top-1.5 left-1.5 flex items-center gap-1 px-1.5 py-0.5 rounded bg-black/75 border border-violet-400/40 text-[9px] font-mono text-violet-200">
          <Film className="w-2.5 h-2.5 text-violet-400" />
          <span>
            {nodeState.active_width}×{nodeState.active_height}
          </span>
          <span className="text-slate-400">@</span>
          <span>{Math.round(nodeState.transform.out_fps)}fps</span>
          {pendingResize && (
            <span
              className="ml-0.5 w-1.5 h-1.5 rounded-full bg-amber-400"
              title="Debouncing resolution reallocation (~150ms)"
            />
          )}
        </div>

        {/* Fit Mode + Freeze Badge */}
        <div className="absolute bottom-1.5 right-1.5 flex items-center gap-1">
          {nodeState.transform.freeze && (
            <span className="px-1 py-0.5 rounded bg-cyan-500/80 text-[8px] font-mono font-bold text-slate-950">
              FREEZE
            </span>
          )}
          <span className="px-1.5 py-0.5 rounded bg-black/75 border border-slate-700 text-[9px] font-mono uppercase text-slate-300">
            {nodeState.transform.fit_mode}
          </span>
        </div>
      </div>

      {/* Quick Source Dropdown */}
      <select
        value={nodeState.source_name}
        onChange={(e) => {
          e.stopPropagation();
          void handleSelectSource(e.target.value);
        }}
        onMouseDown={(e) => e.stopPropagation()}
        aria-label="NDI Source"
        className="w-full bg-studio-bg border border-studio-border rounded px-1.5 py-1 text-[10px] text-slate-200 outline-none focus:border-violet-400"
      >
        {sources.map((src) => (
          <option key={src.name} value={src.name}>
            {src.name}
          </option>
        ))}
      </select>

      {/* Compact NDI Runtime Status Banner */}
      <div
        className={`flex items-center justify-between gap-1.5 px-2 py-1 rounded border text-[9px] ${
          runtime.available
            ? 'bg-emerald-950/35 border-emerald-500/35 text-emerald-200'
            : 'bg-violet-950/35 border-violet-500/35 text-violet-200'
        }`}
      >
        <span className="flex items-center gap-1 truncate">
          {runtime.available ? (
            <CheckCircle2 className="w-3 h-3 text-emerald-400 shrink-0" />
          ) : (
            <AlertCircle className="w-3 h-3 text-violet-400 shrink-0" />
          )}
          <span className="truncate">
            {runtime.available
              ? runtime.version ?? 'NDI Runtime v6 Active'
              : 'Test Pattern · NDI Runtime Optional'}
          </span>
        </span>
      </div>
    </div>
  );
};
