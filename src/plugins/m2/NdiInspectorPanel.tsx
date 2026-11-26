import React, { useEffect, useState } from 'react';
import {
  AlertTriangle,
  CheckCircle2,
  ExternalLink,
  RefreshCw,
  Sliders,
} from 'lucide-react';
import { isTauriRuntime } from '../../lib/bridge';
import { useWorkspaceStore } from '../../store/workspaceStore';
import type { Device } from '../../types/kontrolfreak';
import {
  browserM2Mock,
  m2Bridge,
  type NdiNodeConfigPatch,
  type NdiNodeState,
  type NdiRuntimeStatus,
  type VideoFitMode,
  type VideoSourceInfo,
} from './m2Bridge';

const RESOLUTION_PRESETS: Array<{ label: string; w: number; h: number }> = [
  { label: '1920×1080', w: 1920, h: 1080 },
  { label: '1280×720', w: 1280, h: 720 },
  { label: '1280×360', w: 1280, h: 360 },
  { label: '512×512', w: 512, h: 512 },
  { label: '640×360', w: 640, h: 360 },
];

const FIT_MODES: Array<{ id: VideoFitMode; label: string }> = [
  { id: 'contain', label: 'Contain' },
  { id: 'cover', label: 'Cover' },
  { id: 'stretch', label: 'Stretch' },
  { id: 'crop', label: 'Crop' },
];

export const NdiInspectorPanel: React.FC<{ device?: Device }> = ({ device }) => {
  const updateDeviceKind = useWorkspaceStore((s) => s.updateDeviceKind);

  const [runtime, setRuntime] = useState<NdiRuntimeStatus>(() =>
    browserM2Mock.getNdiRuntimeStatus()
  );
  const [sources, setSources] = useState<VideoSourceInfo[]>(() =>
    browserM2Mock.discoverNdiSources()
  );
  const [state, setState] = useState<NdiNodeState | null>(() =>
    device ? browserM2Mock.getNdiNodeState(device.id, device) : null
  );

  useEffect(() => {
    if (!device) return;
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
      setState(st);
    })();

    const unsub = browserM2Mock.subscribe(() => {
      if (!isTauriRuntime()) {
        setState(browserM2Mock.getNdiNodeState(device.id, device));
      }
    });
    return () => {
      alive = false;
      unsub();
    };
  }, [device]);

  if (!device || !state) return null;

  const applyPatch = async (patch: NdiNodeConfigPatch) => {
    const next = await m2Bridge.configureNdiNode(device.id, patch, device);
    setState(next);

    if (
      device.kind.type === 'NdiVideoNode' &&
      (patch.source_name !== undefined ||
        patch.output_name !== undefined ||
        patch.out_width !== undefined ||
        patch.out_height !== undefined ||
        patch.out_fps !== undefined)
    ) {
      await updateDeviceKind(device.id, {
        ...device.kind,
        source_name: patch.source_name ?? device.kind.source_name,
        output_name: patch.output_name ?? device.kind.output_name,
        out_width: patch.out_width ?? device.kind.out_width,
        out_height: patch.out_height ?? device.kind.out_height,
        out_fps: patch.out_fps ?? device.kind.out_fps,
      });
    }
  };

  const refreshSources = async () => {
    const [rt, srcs] = await Promise.all([
      m2Bridge.getNdiRuntimeStatus(),
      m2Bridge.discoverNdiSources(),
    ]);
    setRuntime(rt);
    setSources(srcs);
  };

  const pendingResize = state.pending_width !== null || state.pending_height !== null;

  return (
    <div className="space-y-3 text-xs">
      {/* NDI Runtime Status Banner */}
      <div
        className={`p-2 rounded-lg border space-y-1 ${
          runtime.available
            ? 'bg-emerald-950/35 border-emerald-500/40'
            : 'bg-amber-950/30 border-amber-500/40'
        }`}
      >
        <div className="flex items-center justify-between gap-2">
          <span className="flex items-center gap-1.5 text-[11px] font-medium text-slate-100">
            {runtime.available ? (
              <CheckCircle2 className="w-3.5 h-3.5 text-emerald-400" />
            ) : (
              <AlertTriangle className="w-3.5 h-3.5 text-amber-400" />
            )}
            {runtime.available ? 'NDI Runtime v6 Loaded' : 'NDI Runtime Not Installed'}
          </span>
          <a
            href={runtime.install_url}
            target="_blank"
            rel="noreferrer"
            className="flex items-center gap-0.5 text-[10px] text-violet-300 hover:text-violet-200"
          >
            NDI Tools <ExternalLink className="w-2.5 h-2.5" />
          </a>
        </div>
        <div className="text-[10px] text-studio-muted">
          {runtime.available
            ? `${runtime.version ?? 'v6'} · ${runtime.library_path}`
            : 'Built-in Synthetic Test Patterns (SMPTE / Motion Grid / Gradient) are active for resize, crop & modulation.'}
        </div>
        <div className="text-[9px] font-mono text-violet-300">
          Scaler: {state.scaler_backend}
        </div>
      </div>

      {/* Source & Output Stream */}
      <div className="space-y-2">
        <div className="flex items-center justify-between">
          <span className="text-[11px] text-studio-muted">Video Source</span>
          <button
            onClick={() => void refreshSources()}
            className="flex items-center gap-1 text-[10px] text-violet-300 hover:text-violet-200"
          >
            <RefreshCw className="w-3 h-3" />
            Discover
          </button>
        </div>
        <select
          value={state.source_name}
          onChange={(e) => void applyPatch({ source_name: e.target.value })}
          className="w-full bg-studio-bg border border-studio-border rounded px-2 py-1.5 text-xs text-slate-200 outline-none focus:border-violet-400"
        >
          {sources.map((s) => (
            <option key={s.name} value={s.name}>
              {s.name}
            </option>
          ))}
        </select>

        <label className="block space-y-1">
          <span className="text-[11px] text-studio-muted">NDI Output Stream Name</span>
          <input
            value={state.output_name}
            onChange={(e) => void applyPatch({ output_name: e.target.value })}
            className="w-full bg-studio-bg border border-studio-border rounded px-2 py-1 text-xs text-slate-200 outline-none focus:border-violet-400"
          />
        </label>
      </div>

      {/* Custom Output Resolution (Debounced ~150ms) */}
      <div className="space-y-2 pt-2 border-t border-studio-border">
        <div className="flex items-center justify-between">
          <span className="text-[11px] font-medium text-slate-200">
            Output Resolution (150ms debounce)
          </span>
          <span
            className={`text-[9px] font-mono px-1.5 py-0.5 rounded ${
              pendingResize
                ? 'bg-amber-500/20 text-amber-300'
                : 'bg-violet-500/20 text-violet-200'
            }`}
          >
            {pendingResize
              ? `Settling ${state.pending_width}×${state.pending_height}`
              : `${state.active_width}×${state.active_height}`}
          </span>
        </div>

        <div className="grid grid-cols-2 gap-2">
          <label className="space-y-0.5">
            <span className="text-[10px] text-studio-muted">Width (px)</span>
            <input
              type="number"
              min={16}
              max={7680}
              step={16}
              value={state.transform.out_width}
              onChange={(e) =>
                void applyPatch({ out_width: Math.max(16, Number(e.target.value) || 1280) })
              }
              className="w-full bg-studio-bg border border-studio-border rounded px-2 py-1 font-mono text-slate-100"
            />
          </label>
          <label className="space-y-0.5">
            <span className="text-[10px] text-studio-muted">Height (px)</span>
            <input
              type="number"
              min={16}
              max={4320}
              step={16}
              value={state.transform.out_height}
              onChange={(e) =>
                void applyPatch({ out_height: Math.max(16, Number(e.target.value) || 720) })
              }
              className="w-full bg-studio-bg border border-studio-border rounded px-2 py-1 font-mono text-slate-100"
            />
          </label>
        </div>

        <div className="flex flex-wrap gap-1">
          {RESOLUTION_PRESETS.map((p) => (
            <button
              key={p.label}
              onClick={() => void applyPatch({ out_width: p.w, out_height: p.h })}
              className={`px-1.5 py-0.5 rounded text-[10px] font-mono border transition-colors ${
                state.transform.out_width === p.w && state.transform.out_height === p.h
                  ? 'bg-violet-500/30 text-violet-100 border-violet-400/60'
                  : 'bg-studio-bg text-studio-muted border-studio-border hover:text-slate-200'
              }`}
            >
              {p.label}
            </button>
          ))}
        </div>
      </div>

      {/* Aspect / Fit Mode & Target FPS */}
      <div className="space-y-2 pt-2 border-t border-studio-border">
        <div className="text-[11px] font-medium text-slate-200">Fit / Aspect Mode</div>
        <div className="grid grid-cols-4 gap-1">
          {FIT_MODES.map((m) => (
            <button
              key={m.id}
              onClick={() => void applyPatch({ fit_mode: m.id })}
              className={`py-1 rounded text-[10px] font-medium border ${
                state.transform.fit_mode === m.id
                  ? 'bg-violet-500/30 text-violet-100 border-violet-400/60'
                  : 'bg-studio-bg text-studio-muted border-studio-border hover:text-slate-200'
              }`}
            >
              {m.label}
            </button>
          ))}
        </div>

        <label className="block space-y-1">
          <div className="flex justify-between text-[10px]">
            <span className="text-studio-muted">Target Output FPS</span>
            <span className="font-mono text-violet-300">
              {Math.round(state.transform.out_fps)} fps
            </span>
          </div>
          <input
            type="range"
            min={5}
            max={60}
            step={1}
            value={state.transform.out_fps}
            onChange={(e) => void applyPatch({ out_fps: Number(e.target.value) })}
            className="w-full accent-violet-400 h-1.5 cursor-pointer"
          />
        </label>
      </div>

      {/* Instant Per-Frame Crop / Pan / Zoom / Opacity */}
      <div className="space-y-2 pt-2 border-t border-studio-border">
        <div className="flex items-center gap-1.5 text-[11px] font-medium text-slate-200">
          <Sliders className="w-3.5 h-3.5 text-violet-400" />
          Instant Frame Transform (OSC / MIDI Modulatable)
        </div>

        {(
          [
            { key: 'crop_x', label: 'Crop X', min: 0, max: 1, step: 0.01, val: state.transform.crop_x },
            { key: 'crop_y', label: 'Crop Y', min: 0, max: 1, step: 0.01, val: state.transform.crop_y },
            { key: 'crop_w', label: 'Crop W', min: 0.05, max: 1, step: 0.01, val: state.transform.crop_w },
            { key: 'crop_h', label: 'Crop H', min: 0.05, max: 1, step: 0.01, val: state.transform.crop_h },
            { key: 'zoom', label: 'Zoom', min: 1, max: 6, step: 0.05, val: state.transform.zoom },
            { key: 'opacity', label: 'Opacity', min: 0, max: 1, step: 0.01, val: state.transform.opacity },
          ] as const
        ).map((ctrl) => (
          <label key={ctrl.key} className="block space-y-0.5">
            <div className="flex justify-between text-[10px]">
              <span className="text-studio-muted">{ctrl.label}</span>
              <span className="font-mono text-amber-300">
                {ctrl.key === 'zoom' ? `${ctrl.val.toFixed(2)}×` : ctrl.val.toFixed(2)}
              </span>
            </div>
            <input
              type="range"
              min={ctrl.min}
              max={ctrl.max}
              step={ctrl.step}
              value={ctrl.val}
              onChange={(e) =>
                void applyPatch({ [ctrl.key]: Number(e.target.value) } as NdiNodeConfigPatch)
              }
              className="w-full accent-amber-400 h-1.5 cursor-pointer"
            />
          </label>
        ))}

        <div className="flex gap-2 pt-1">
          <label className="flex-1 flex items-center justify-center gap-1.5 py-1 rounded bg-studio-bg border border-studio-border text-[11px] cursor-pointer">
            <input
              type="checkbox"
              checked={state.transform.freeze}
              onChange={(e) => void applyPatch({ freeze: e.target.checked })}
              className="accent-cyan-400"
            />
            Freeze
          </label>
          <label className="flex-1 flex items-center justify-center gap-1.5 py-1 rounded bg-studio-bg border border-studio-border text-[11px] cursor-pointer">
            <input
              type="checkbox"
              checked={state.transform.bypass}
              onChange={(e) => void applyPatch({ bypass: e.target.checked })}
              className="accent-amber-400"
            />
            Bypass
          </label>
        </div>
      </div>
    </div>
  );
};
