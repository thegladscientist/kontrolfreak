import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  Download,
  Play,
  Plus,
  RefreshCw,
  ShieldAlert,
  Square,
  Upload,
} from 'lucide-react';
import {
  getNodeDefinition,
  listSidebarSections,
  registerCommandAction,
  registerDrawerTab,
  registerInspectorPanel,
  registerNodeType,
  registerSidebarSection,
} from '../registries';
import { useWorkspaceStore } from '../store/workspaceStore';
import { telemetryStore } from '../lib/telemetryStore';
import type { Device, Port } from '../types/kontrolfreak';
import { createDefaultRoi } from './m4/colormap';
import { DepthPreviewCanvas } from './m4/DepthPreview';
import { subscribeDepthFeed, type DepthFeedState } from './m4/depthPoller';
import {
  exportKfreakToDownload,
  isDepthDevice,
  pickAndImportKfreak,
  runCrashIsolationTest,
} from './m4/kfreakActions';
import { DEPTH_PORT_KEYS, m4Bridge } from './m4/m4Bridge';
import type {
  DepthColormap,
  DiagnosticsSnapshot,
  NamedDepthRoi,
  SidecarHealthStatus,
} from './m4/types';

export const M4_NODE_KIND = 'DepthSensor' as const;
export const M4_SIDEBAR_ID = 'm4-depth';
export const M4_INSPECTOR_ID = 'm4-depth-inspector';
export const M4_DRAWER_TAB_ID = 'm4-diagnostics';
export const M4_COMMAND_IDS = {
  export: 'm4.export-kfreak',
  import: 'm4.import-kfreak',
  diagnostics: 'm4.open-diagnostics',
  crashTest: 'm4.test-crash-isolation',
  addSensor: 'm4.add-depth-sensor',
} as const;

const COLORMAPS: Array<{ id: DepthColormap; label: string }> = [
  { id: 'turbo', label: 'Turbo' },
  { id: 'viridis', label: 'Viridis' },
  { id: 'grayscale', label: 'Grayscale' },
  { id: 'depth_slice', label: 'Depth Slice' },
];

const HEALTH_STYLE: Record<SidecarHealthStatus, { label: string; cls: string; dot: string }> = {
  running: { label: 'Healthy', cls: 'text-emerald-300 border-emerald-500/40 bg-emerald-500/10', dot: 'bg-emerald-400' },
  starting: { label: 'Starting', cls: 'text-cyan-300 border-cyan-500/40 bg-cyan-500/10', dot: 'bg-cyan-400' },
  degraded: { label: 'Degraded', cls: 'text-amber-300 border-amber-500/40 bg-amber-500/10', dot: 'bg-amber-400' },
  crashed_restarting: {
    label: 'Crashed · restarting',
    cls: 'text-rose-300 border-rose-500/40 bg-rose-500/10',
    dot: 'bg-rose-400 animate-pulse',
  },
  stopped: { label: 'Stopped', cls: 'text-slate-300 border-slate-500/40 bg-slate-500/10', dot: 'bg-slate-400' },
};

export function useDepthFeed(deviceId: string): DepthFeedState {
  const [state, setState] = useState<DepthFeedState>({ packet: null, status: null, error: null });
  useEffect(() => subscribeDepthFeed(deviceId, setState), [deviceId]);
  return state;
}

export const SidecarHealthBadge: React.FC<{ status: SidecarHealthStatus; restarts?: number; pid?: number | null }> = ({
  status,
  restarts = 0,
  pid,
}) => {
  const s = HEALTH_STYLE[status];
  return (
    <span
      data-testid="sidecar-health-badge"
      data-status={status}
      className={`inline-flex items-center gap-1.5 px-2 py-0.5 rounded-full border text-[10px] font-medium ${s.cls}`}
    >
      <span className={`w-1.5 h-1.5 rounded-full ${s.dot}`} />
      {s.label}
      {pid ? <span className="font-mono opacity-70">pid {pid}</span> : null}
      {restarts > 0 ? <span className="font-mono opacity-70">↻{restarts}</span> : null}
    </span>
  );
};

// ---------- Node body (canvas mini preview) ----------

const DepthNodeBody: React.FC<{ device: Device; calmMode: boolean }> = ({ device, calmMode }) => {
  const { packet, status } = useDepthFeed(device.id);
  const health = status?.health.status ?? 'stopped';
  return (
    <div className="px-3 pb-2 pt-1 space-y-1.5">
      <DepthPreviewCanvas packet={calmMode ? null : packet} roi={status?.rois[0] ?? null} />
      <div className="flex items-center justify-between">
        <SidecarHealthBadge status={health} restarts={status?.health.restart_count} />
        <span className="text-[10px] font-mono text-studio-muted">
          {status?.health.fps ? `${status.health.fps.toFixed(0)} fps` : '—'}
        </span>
      </div>
    </div>
  );
};

// ---------- Inspector panel ----------

function useThrottledCommit<T>(fn: (v: T) => void, ms = 80): (v: T) => void {
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const latest = useRef<T | null>(null);
  const fnRef = useRef(fn);
  fnRef.current = fn;
  useEffect(
    () => () => {
      if (timer.current) clearTimeout(timer.current);
    },
    []
  );
  return useCallback(
    (v: T) => {
      latest.current = v;
      if (timer.current) return;
      timer.current = setTimeout(() => {
        timer.current = null;
        if (latest.current !== null) fnRef.current(latest.current);
      }, ms);
    },
    [ms]
  );
}

const DepthSensorInspector: React.FC<{ device?: Device }> = ({ device }) => {
  if (!device) return null;
  return <DepthSensorInspectorBody device={device} />;
};

const DepthSensorInspectorBody: React.FC<{ device: Device }> = ({ device }) => {
  const { packet, status, error } = useDepthFeed(device.id);
  const pushToast = useWorkspaceStore((s) => s.pushToast);
  const updateDeviceKind = useWorkspaceStore((s) => s.updateDeviceKind);

  const [roi, setRoi] = useState<NamedDepthRoi>(createDefaultRoi());
  const [palette, setPalette] = useState<DepthColormap>('turbo');
  const [vendor, setVendor] = useState(device.kind.type === 'DepthSensor' ? device.kind.vendor : 'synthetic');
  const [vendors, setVendors] = useState<Array<{ vendor_id: string; display_name: string; hardware_detected: boolean }>>([]);
  const [busy, setBusy] = useState(false);
  const seeded = useRef(false);

  useEffect(() => {
    void m4Bridge.listVendors().then(setVendors).catch(() => undefined);
  }, []);

  useEffect(() => {
    if (seeded.current || !status) return;
    seeded.current = true;
    if (status.rois[0]) setRoi(status.rois[0]);
    setPalette(status.colormap.palette);
  }, [status]);

  const commitRoi = useThrottledCommit<NamedDepthRoi>((r) => {
    void m4Bridge.configureRois(device.id, [r]);
    if (device.kind.type === 'DepthSensor') {
      void updateDeviceKind(device.id, {
        ...device.kind,
        roi_min_z: r.near_mm / 1000,
        roi_max_z: r.far_mm / 1000,
      }).catch(() => undefined);
    }
  });

  const applyRoi = (next: NamedDepthRoi) => {
    setRoi(next);
    commitRoi(next);
  };

  const applyZ = (nearMm: number, farMm: number) => {
    const near = Math.max(100, Math.min(nearMm, farMm - 100));
    const far = Math.max(near + 100, farMm);
    applyRoi({ ...roi, near_mm: near, far_mm: far });
    void m4Bridge.setColormap(device.id, palette, near, far);
  };

  const applyPalette = (p: DepthColormap) => {
    setPalette(p);
    void m4Bridge.setColormap(device.id, p, roi.near_mm, roi.far_mm);
  };

  const health = status?.health;
  const running = health?.status === 'running';
  const result = packet?.roi_results[0];

  const act = async (fn: () => Promise<unknown>) => {
    setBusy(true);
    try {
      await fn();
    } catch (err) {
      pushToast({
        title: 'Depth sidecar error',
        description: err instanceof Error ? err.message : String(err),
        accent: 'rose',
      });
    } finally {
      setBusy(false);
    }
  };

  const start = () =>
    act(async () => {
      await m4Bridge.startSidecar(device.id, vendor);
      await m4Bridge.configureRois(device.id, [roi]);
      await m4Bridge.setColormap(device.id, palette, roi.near_mm, roi.far_mm);
    });

  return (
    <div className="space-y-2.5" data-testid="m4-depth-inspector">
      <div className="flex items-center justify-between gap-2">
        <SidecarHealthBadge
          status={health?.status ?? 'stopped'}
          restarts={health?.restart_count}
          pid={health?.pid}
        />
        <div className="flex gap-1">
          {running ? (
            <button
              disabled={busy}
              onClick={() => void act(() => m4Bridge.stopSidecar(device.id))}
              className="p-1.5 rounded border border-studio-border bg-studio-panel hover:bg-studio-elevated text-slate-200"
              title="Stop sidecar"
            >
              <Square className="w-3 h-3" />
            </button>
          ) : (
            <button
              disabled={busy}
              onClick={() => void start()}
              className="p-1.5 rounded border border-emerald-500/40 bg-emerald-500/15 hover:bg-emerald-500/25 text-emerald-200"
              title="Start sidecar"
            >
              <Play className="w-3 h-3" />
            </button>
          )}
          <button
            disabled={busy || !health || health.status === 'stopped'}
            onClick={() => void act(() => m4Bridge.restartSidecar(device.id))}
            className="p-1.5 rounded border border-studio-border bg-studio-panel hover:bg-studio-elevated text-slate-200 disabled:opacity-40"
            title="Restart sidecar"
          >
            <RefreshCw className="w-3 h-3" />
          </button>
        </div>
      </div>

      <DepthPreviewCanvas packet={packet} roi={roi} interactive onRoiChange={applyRoi} />

      {error && <div className="text-[10px] text-rose-300 font-mono">{error}</div>}
      {health?.last_crash_reason && (
        <div className="text-[10px] text-amber-300/90 font-mono leading-snug">
          Last crash: {health.last_crash_reason}
        </div>
      )}

      <div className="grid grid-cols-2 gap-2 text-[10px]">
        <label className="space-y-1">
          <span className="text-studio-muted uppercase tracking-wider">Vendor</span>
          <select
            value={vendor}
            onChange={(e) => setVendor(e.target.value)}
            className="w-full bg-studio-bg border border-studio-border rounded px-1.5 py-1 text-xs text-slate-200"
          >
            {(vendors.length ? vendors : [{ vendor_id: vendor, display_name: vendor, hardware_detected: true }]).map((v) => (
              <option key={v.vendor_id} value={v.vendor_id}>
                {v.display_name}
              </option>
            ))}
          </select>
        </label>
        <label className="space-y-1">
          <span className="text-studio-muted uppercase tracking-wider">Colormap</span>
          <select
            data-testid="colormap-select"
            value={palette}
            onChange={(e) => applyPalette(e.target.value as DepthColormap)}
            className="w-full bg-studio-bg border border-studio-border rounded px-1.5 py-1 text-xs text-slate-200"
          >
            {COLORMAPS.map((c) => (
              <option key={c.id} value={c.id}>
                {c.label}
              </option>
            ))}
          </select>
        </label>
      </div>

      <div className="space-y-1.5">
        <div className="flex justify-between text-[10px] text-studio-muted">
          <span>Near Z</span>
          <span className="font-mono text-slate-200">{(roi.near_mm / 1000).toFixed(2)} m</span>
        </div>
        <input
          data-testid="near-z-slider"
          type="range"
          min={100}
          max={5000}
          step={50}
          value={roi.near_mm}
          onChange={(e) => applyZ(Number(e.target.value), roi.far_mm)}
          className="w-full accent-emerald-400"
        />
        <div className="flex justify-between text-[10px] text-studio-muted">
          <span>Far Z</span>
          <span className="font-mono text-slate-200">{(roi.far_mm / 1000).toFixed(2)} m</span>
        </div>
        <input
          data-testid="far-z-slider"
          type="range"
          min={200}
          max={6000}
          step={50}
          value={roi.far_mm}
          onChange={(e) => applyZ(roi.near_mm, Number(e.target.value))}
          className="w-full accent-violet-400"
        />
      </div>

      <div className="grid grid-cols-3 gap-1.5 text-[10px] font-mono">
        <Readout label="closest z" value={result ? `${(result.min_depth_mm / 1000).toFixed(2)}m` : '—'} />
        <Readout label="presence" value={result ? (result.presence ? 'YES' : 'no') : '—'} />
        <Readout label="fill" value={result ? `${(result.fill_ratio * 100).toFixed(0)}%` : '—'} />
        <Readout
          label="closest xyz"
          value={result ? result.closest_point_meters.map((v) => v.toFixed(2)).join(',') : '—'}
        />
        <Readout
          label="centroid xyz"
          value={result ? result.centroid_meters.map((v) => v.toFixed(2)).join(',') : '—'}
        />
        <Readout label="frames" value={health ? String(health.frames_received) : '0'} />
      </div>

      <button
        data-testid="crash-isolation-button"
        disabled={busy || !running}
        onClick={() => void act(() => runCrashIsolationTest(device.id))}
        className="w-full flex items-center justify-center gap-1.5 py-1.5 rounded-md text-xs font-medium bg-rose-500/15 hover:bg-rose-500/25 disabled:opacity-40 text-rose-200 border border-rose-500/40"
      >
        <ShieldAlert className="w-3.5 h-3.5" />
        Test Crash Isolation
      </button>
      <p className="text-[10px] text-studio-muted leading-snug">
        Kills the sensor sidecar process. The core router keeps routing OSC/MIDI while the supervisor
        auto-restarts the sidecar with exponential backoff.
      </p>
    </div>
  );
};

const Readout: React.FC<{ label: string; value: string }> = ({ label, value }) => (
  <div className="rounded border border-studio-border bg-studio-bg px-1.5 py-1 min-w-0">
    <div className="text-[8px] uppercase tracking-wider text-studio-muted">{label}</div>
    <div className="text-slate-200 truncate">{value}</div>
  </div>
);

// ---------- Sidebar ----------

export function createDepthSensorDevice(position = { x: 140, y: 140 }): Device | null {
  const def = getNodeDefinition(M4_NODE_KIND);
  return def ? def.createDevice(position) : null;
}

const KfreakBundleActions: React.FC = () => (
  <div className="space-y-1.5 pt-2 border-t border-studio-border" data-testid="kfreak-actions">
    <div className="text-[11px] font-semibold uppercase tracking-wider text-studio-muted">
      Project Bundle (.kfreak)
    </div>
    <button
      onClick={() => void exportKfreakToDownload().catch(() => undefined)}
      className="w-full flex items-center justify-between px-2.5 py-1.5 rounded-md bg-studio-panel hover:bg-studio-elevated border border-studio-border text-xs text-slate-200"
    >
      <span>Export Bundle (.kfreak)</span>
      <Download className="w-3.5 h-3.5 text-violet-400" />
    </button>
    <button
      onClick={() => pickAndImportKfreak()}
      className="w-full flex items-center justify-between px-2.5 py-1.5 rounded-md bg-studio-panel hover:bg-studio-elevated border border-studio-border text-xs text-slate-200"
    >
      <span>Import Bundle (.kfreak)</span>
      <Upload className="w-3.5 h-3.5 text-emerald-400" />
    </button>
    <p className="text-[10px] text-studio-muted leading-snug">
      ZIP of project + device / rack / mapping presets with SHA-256 integrity checks.
    </p>
  </div>
);

const DepthSidebarSection: React.FC = () => {
  const devices = useWorkspaceStore((s) => s.graph.devices);
  const addDevice = useWorkspaceStore((s) => s.addDevice);
  const setSelection = useWorkspaceStore((s) => s.setSelection);
  const depthDevices = useMemo(() => devices.filter(isDepthDevice), [devices]);

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <div className="text-[11px] font-semibold uppercase tracking-wider text-studio-muted">
          Depth Sensors ({depthDevices.length})
        </div>
        <button
          onClick={() => {
            const dev = createDepthSensorDevice({ x: 160 + depthDevices.length * 30, y: 160 + depthDevices.length * 30 });
            if (dev) void addDevice(dev);
          }}
          className="flex items-center gap-1 px-2 py-1 rounded-md bg-emerald-500/15 hover:bg-emerald-500/25 border border-emerald-500/40 text-[11px] text-emerald-200"
        >
          <Plus className="w-3 h-3" /> Add
        </button>
      </div>

      {depthDevices.length === 0 && (
        <div className="text-[11px] text-studio-muted leading-snug">
          No depth sensors yet. Add one to stream a crash-isolated sidecar (RealSense / Orbbec / Kinect or the
          built-in synthetic simulator).
        </div>
      )}

      {depthDevices.map((d) => (
        <DepthSidebarRow key={d.id} device={d} onSelect={() => setSelection({ kind: 'device', id: d.id })} />
      ))}

      <KfreakBundleActions />
    </div>
  );
};

const DepthSidebarRow: React.FC<{ device: Device; onSelect: () => void }> = ({ device, onSelect }) => {
  const { status } = useDepthFeed(device.id);
  return (
    <button
      onClick={onSelect}
      className="w-full text-left rounded-md border border-studio-border bg-studio-panel hover:bg-studio-elevated px-2.5 py-2 space-y-1"
    >
      <div className="text-xs text-slate-200 truncate">{device.display_name}</div>
      <SidecarHealthBadge status={status?.health.status ?? 'stopped'} restarts={status?.health.restart_count} />
    </button>
  );
};

// ---------- Drawer: Diagnostics & Profiler ----------

const HISTORY = 120;

const DiagnosticsTab: React.FC = () => {
  const [snap, setSnap] = useState<DiagnosticsSnapshot | null>(null);
  const history = useRef<{ rate: number[]; p95: number[] }>({ rate: [], p95: [] });
  const canvasRef = useRef<HTMLCanvasElement | null>(null);

  useEffect(() => {
    let alive = true;
    const tick = async () => {
      try {
        const s = await m4Bridge.getDiagnostics([
          { protocol: 'ROUTER', rate: telemetryStore.getTotalMsgPerSec() },
        ]);
        if (!alive) return;
        history.current.rate.push(s.total_msg_per_sec);
        history.current.p95.push(s.latency.p95_ms);
        if (history.current.rate.length > HISTORY) history.current.rate.shift();
        if (history.current.p95.length > HISTORY) history.current.p95.shift();
        setSnap(s);
      } catch {
        /* diagnostics are best-effort */
      }
    };
    void tick();
    const t = setInterval(() => void tick(), 500);
    return () => {
      alive = false;
      clearInterval(t);
    };
  }, []);

  useEffect(() => {
    const canvas = canvasRef.current;
    const ctx = canvas?.getContext('2d');
    if (!canvas || !ctx) return;
    const { width: w, height: h } = canvas;
    ctx.clearRect(0, 0, w, h);
    ctx.fillStyle = '#0b0e14';
    ctx.fillRect(0, 0, w, h);
    const plot = (data: number[], color: string) => {
      if (data.length < 2) return;
      const max = Math.max(1e-6, ...data) * 1.15;
      ctx.strokeStyle = color;
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      data.forEach((v, i) => {
        const x = (i / (HISTORY - 1)) * w;
        const y = h - (v / max) * (h - 6) - 3;
        if (i === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      });
      ctx.stroke();
    };
    plot(history.current.rate, '#22d3ee');
    plot(history.current.p95, '#f59e0b');
  }, [snap]);

  const maxRate = Math.max(1, ...(snap?.protocol_rates.map((r) => r.msg_per_sec) ?? [1]));

  return (
    <div className="h-full overflow-y-auto p-3 space-y-3" data-testid="m4-diagnostics">
      <div className="grid grid-cols-2 md:grid-cols-5 gap-2">
        <Stat label="Total msg/s" value={snap ? snap.total_msg_per_sec.toFixed(1) : '—'} accent="text-cyan-300" />
        <Stat label="Latency P50" value={snap ? `${snap.latency.p50_ms.toFixed(2)} ms` : '—'} accent="text-emerald-300" />
        <Stat label="Latency P95" value={snap ? `${snap.latency.p95_ms.toFixed(2)} ms` : '—'} accent="text-amber-300" />
        <Stat label="Latency P99" value={snap ? `${snap.latency.p99_ms.toFixed(2)} ms` : '—'} accent="text-rose-300" />
        <Stat label="Dropped frames" value={snap ? String(snap.dropped_frames) : '—'} accent="text-violet-300" />
      </div>

      <div className="grid grid-cols-1 lg:grid-cols-2 gap-3">
        <div className="space-y-1.5">
          <div className="text-[11px] uppercase tracking-wider text-studio-muted flex items-center gap-2">
            Throughput <span className="text-cyan-300">msg/s</span> · <span className="text-amber-300">P95 latency</span>
          </div>
          <canvas
            ref={canvasRef}
            width={480}
            height={90}
            className="w-full h-[90px] rounded border border-studio-border"
          />
          <div className="space-y-1">
            {(snap?.protocol_rates ?? []).map((r) => (
              <div key={r.protocol} className="flex items-center gap-2 text-[10px] font-mono">
                <span className="w-16 text-slate-300">{r.protocol}</span>
                <div className="flex-1 h-1.5 bg-studio-bg rounded overflow-hidden">
                  <div className="h-full bg-cyan-400/70" style={{ width: `${Math.min(100, (r.msg_per_sec / maxRate) * 100)}%` }} />
                </div>
                <span className="w-24 text-right text-studio-muted">{r.msg_per_sec.toFixed(1)} /s</span>
              </div>
            ))}
          </div>
        </div>

        <div className="space-y-1.5">
          <div className="text-[11px] uppercase tracking-wider text-studio-muted">Sensor sidecars</div>
          {(snap?.sidecars ?? []).length === 0 && (
            <div className="text-[11px] text-studio-muted">No sidecars running. Add a Depth Sensor and press Start.</div>
          )}
          {(snap?.sidecars ?? []).map((s) => (
            <div key={s.sidecar_id} className="rounded border border-studio-border bg-studio-panel px-2.5 py-2 space-y-1">
              <div className="flex items-center justify-between">
                <span className="text-xs text-slate-200 font-mono truncate">{s.vendor} · {s.sidecar_id.slice(0, 8)}</span>
                <SidecarHealthBadge status={s.status} restarts={s.restart_count} pid={s.pid} />
              </div>
              <div className="grid grid-cols-4 gap-1 text-[10px] font-mono text-studio-muted">
                <span>{s.fps.toFixed(0)} fps</span>
                <span>{(s.uptime_ms / 1000).toFixed(0)}s up</span>
                <span>ring {(s.ring_buffer_utilization * 100).toFixed(0)}%</span>
                <span>drop {s.ring_buffer_dropped}</span>
              </div>
              {s.last_crash_reason && (
                <div className="text-[10px] text-amber-300/90 font-mono">{s.last_crash_reason}</div>
              )}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
};

const Stat: React.FC<{ label: string; value: string; accent: string }> = ({ label, value, accent }) => (
  <div className="rounded border border-studio-border bg-studio-panel px-2.5 py-1.5">
    <div className="text-[9px] uppercase tracking-wider text-studio-muted">{label}</div>
    <div className={`text-sm font-mono ${accent}`}>{value}</div>
  </div>
);

// ---------- Presets sidebar wrapper (adds .kfreak actions to built-in Presets & Projects) ----------

function extendPresetsSection(): void {
  const presets = listSidebarSections().find((s) => s.id === 'presets');
  if (!presets || (presets.component as { __m4?: boolean }).__m4) return;
  const Base = presets.component;
  const Extended: React.FC = () => (
    <div className="space-y-3">
      <Base />
      <KfreakBundleActions />
    </div>
  );
  (Extended as { __m4?: boolean }).__m4 = true;
  registerSidebarSection({ ...presets, component: Extended });
}

// ---------- Registration ----------

function firstDepthDeviceId(): string | null {
  const st = useWorkspaceStore.getState();
  if (st.selection.kind === 'device') {
    const sel = st.graph.devices.find((d) => d.id === (st.selection as { id: string }).id);
    if (sel && isDepthDevice(sel)) return sel.id;
  }
  return st.graph.devices.find(isDepthDevice)?.id ?? null;
}

function portUid(): string {
  return typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function'
    ? crypto.randomUUID()
    : `port-${Date.now()}-${Math.random().toString(36).slice(2, 9)}`;
}

let registered = false;

export function registerM4DepthSidecarPlugin(): void {
  if (registered) return;
  registered = true;

  const base = getNodeDefinition(M4_NODE_KIND);
  if (base) {
    registerNodeType({
      ...base,
      description:
        'Crash-isolated depth sidecar (RealSense / Orbbec / Kinect) with 3D ROI extraction, shared-memory frames and NDI depth matte.',
      createDevice: (pos) => {
        const dev = base.createDevice(pos);
        const byKey = new Map(dev.ports.map((p) => [p.logical_key, p]));
        const ensure = (key: string, alias: string, kind: 'sensor' | 'video'): Port =>
          byKey.get(key) ?? {
            id: portUid(),
            logical_key: key,
            alias,
            direction: 'out',
            signal_kind: kind,
            min_value: kind === 'video' ? null : 0,
            max_value: kind === 'video' ? null : 1,
            modifiers: [],
          };
        return {
          ...dev,
          display_name: 'Depth Camera',
          kind: { type: 'DepthSensor', vendor: 'synthetic', sidecar_id: null, roi_min_z: 0.3, roi_max_z: 2.5 },
          status: { state: 'offline_ghost', reason: 'Depth sidecar not started' },
          ports: [
            ensure(DEPTH_PORT_KEYS.closestZ, 'Closest Z', 'sensor'),
            ensure(DEPTH_PORT_KEYS.closestXyz, 'Closest XYZ', 'sensor'),
            ensure(DEPTH_PORT_KEYS.centroidXyz, 'Centroid XYZ', 'sensor'),
            ensure(DEPTH_PORT_KEYS.presence, 'Presence', 'sensor'),
            ensure(DEPTH_PORT_KEYS.fillRatio, 'ROI Fill', 'sensor'),
            ensure(DEPTH_PORT_KEYS.ndiOut, 'Depth Matte', 'video'),
          ],
        };
      },
      renderBody: DepthNodeBody,
    });
  }

  registerSidebarSection({
    id: M4_SIDEBAR_ID,
    label: 'Depth',
    order: 25,
    iconName: 'radar',
    component: DepthSidebarSection,
  });
  extendPresetsSection();

  registerInspectorPanel({
    id: M4_INSPECTOR_ID,
    title: 'Depth Sensor · 3D ROI',
    order: 5,
    target: M4_NODE_KIND,
    component: DepthSensorInspector,
  });

  registerDrawerTab({
    id: M4_DRAWER_TAB_ID,
    label: 'Diagnostics & Profiler',
    order: 40,
    component: DiagnosticsTab,
  });

  registerCommandAction({
    id: M4_COMMAND_IDS.export,
    label: 'Export Project Bundle (.kfreak)',
    group: 'Project',
    keywords: ['kfreak', 'bundle', 'zip', 'export', 'presets'],
    run: async () => {
      await exportKfreakToDownload().catch(() => undefined);
    },
  });
  registerCommandAction({
    id: M4_COMMAND_IDS.import,
    label: 'Import Project Bundle (.kfreak)',
    group: 'Project',
    keywords: ['kfreak', 'bundle', 'zip', 'import', 'presets'],
    run: () => pickAndImportKfreak(),
  });
  registerCommandAction({
    id: M4_COMMAND_IDS.diagnostics,
    label: 'Open Diagnostics & Profiler',
    group: 'View',
    keywords: ['latency', 'profiler', 'diagnostics', 'p95', 'sidecar'],
    run: () => useWorkspaceStore.getState().setActiveDrawerTab(M4_DRAWER_TAB_ID),
  });
  registerCommandAction({
    id: M4_COMMAND_IDS.crashTest,
    label: 'Test Depth Sidecar Crash Isolation',
    group: 'Depth',
    keywords: ['crash', 'sigkill', 'sidecar', 'isolation', 'depth'],
    run: async () => {
      const id = firstDepthDeviceId();
      const st = useWorkspaceStore.getState();
      if (!id) {
        st.pushToast({ title: 'No depth sensor in project', description: 'Add a Depth Sensor first.', accent: 'amber' });
        return;
      }
      await runCrashIsolationTest(id).catch((err) =>
        st.pushToast({ title: 'Crash test failed', description: String(err), accent: 'rose' })
      );
    },
  });
  registerCommandAction({
    id: M4_COMMAND_IDS.addSensor,
    label: 'Add Depth Sensor Node',
    group: 'Depth',
    keywords: ['depth', 'realsense', 'orbbec', 'kinect', 'sensor'],
    run: async () => {
      const dev = createDepthSensorDevice();
      if (dev) await useWorkspaceStore.getState().addDevice(dev);
    },
  });
}
