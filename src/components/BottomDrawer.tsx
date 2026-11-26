import React, { useEffect, useMemo, useRef, useState } from 'react';
import { Activity, ChevronDown, ChevronUp, Pause, Play, Terminal, Trash2 } from 'lucide-react';
import { listDrawerTabs, registerDrawerTab } from '../registries';
import { useWorkspaceStore } from '../store/workspaceStore';
import { telemetryStore } from '../lib/telemetryStore';
import { SIGNAL_COLORS } from '../lib/signalColors';

const SignalOscilloscopeTab: React.FC = () => {
  const graph = useWorkspaceStore((s) => s.graph);
  const selection = useWorkspaceStore((s) => s.selection);
  const calmMode = graph.settings.calm_mode;
  const canvasRef = useRef<HTMLCanvasElement | null>(null);

  // Pick up to 4 traces: selected device/wire ports first, otherwise top active ports
  const watchedPorts = useMemo(() => {
    const out: Array<{ portId: string; label: string; color: string }> = [];
    if (selection.kind === 'device') {
      const dev = graph.devices.find((d) => d.id === selection.id);
      if (dev) {
        for (const p of dev.ports.slice(0, 4)) {
          out.push({
            portId: p.id,
            label: `${dev.display_name} · ${p.alias}`,
            color: SIGNAL_COLORS[p.signal_kind].hex,
          });
        }
      }
    } else if (selection.kind === 'wire') {
      const w = graph.wires.find((x) => x.id === selection.id);
      if (w) {
        for (const d of graph.devices) {
          for (const p of d.ports) {
            if (p.id === w.from_port_id || p.id === w.to_port_id) {
              out.push({
                portId: p.id,
                label: `${d.display_name} · ${p.alias}`,
                color: SIGNAL_COLORS[p.signal_kind].hex,
              });
            }
          }
        }
      }
    }
    if (out.length === 0) {
      for (const d of graph.devices) {
        for (const p of d.ports) {
          if (out.length < 4) {
            out.push({
              portId: p.id,
              label: `${d.display_name} · ${p.alias}`,
              color: SIGNAL_COLORS[p.signal_kind].hex,
            });
          }
        }
      }
    }
    return out;
  }, [graph.devices, graph.wires, selection]);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const draw = () => {
      const w = canvas.width;
      const h = canvas.height;
      ctx.clearRect(0, 0, w, h);
      ctx.fillStyle = '#0b0e14';
      ctx.fillRect(0, 0, w, h);

      // Horizontal reference lines (0.0, 0.5, 1.0)
      ctx.strokeStyle = '#1e293b';
      ctx.lineWidth = 1;
      [0.15, 0.5, 0.85].forEach((ratio) => {
        const y = Math.round(h * ratio);
        ctx.beginPath();
        ctx.moveTo(0, y);
        ctx.lineTo(w, y);
        ctx.stroke();
      });

      for (const trace of watchedPorts) {
        const samples = telemetryStore.getPortHistory(trace.portId);
        if (samples.length < 2) continue;
        ctx.strokeStyle = trace.color;
        ctx.globalAlpha = calmMode ? 0.65 : 0.95;
        ctx.lineWidth = calmMode ? 1.5 : 2;
        ctx.beginPath();
        const stepX = w / Math.max(1, samples.length - 1);
        samples.forEach((v, i) => {
          const clamped = Math.min(1, Math.max(0, v));
          const x = i * stepX;
          const y = h - 10 - clamped * (h - 20);
          if (i === 0) ctx.moveTo(x, y);
          else ctx.lineTo(x, y);
        });
        ctx.stroke();
      }
      ctx.globalAlpha = 1;
    };

    draw();
    return telemetryStore.subscribe(() => draw());
  }, [watchedPorts, calmMode]);

  return (
    <div className="h-full flex gap-3 p-2.5">
      <div className="flex-1 rounded-lg border border-studio-border overflow-hidden bg-studio-bg">
        <canvas
          ref={canvasRef}
          width={760}
          height={120}
          className="w-full h-full block"
          aria-label="Live signal oscilloscope"
        />
      </div>
      <div className="w-56 shrink-0 space-y-1.5 overflow-y-auto pr-1">
        <div className="text-[10px] font-semibold uppercase tracking-wider text-studio-muted">
          Scope Traces (30 Hz)
        </div>
        {watchedPorts.map((t) => {
          const latest = telemetryStore.getPortLatest(t.portId);
          return (
            <div
              key={t.portId}
              className="flex items-center justify-between gap-2 px-2 py-1 rounded bg-studio-panel border border-studio-border text-[11px]"
            >
              <span className="flex items-center gap-1.5 truncate">
                <span
                  className="w-2 h-2 rounded-full shrink-0"
                  style={{ backgroundColor: t.color }}
                />
                <span className="truncate text-slate-200">{t.label}</span>
              </span>
              <span className="font-mono text-[10px] text-slate-300 shrink-0">
                {typeof latest?.numeric_value === 'number'
                  ? latest.numeric_value.toFixed(2)
                  : '--'}
              </span>
            </div>
          );
        })}
      </div>
    </div>
  );
};

const ProtocolTrafficLogTab: React.FC = () => {
  const [filter, setFilter] = useState<string>('ALL');
  const [paused, setPaused] = useState(() => telemetryStore.isLogPaused());
  const [, setTick] = useState(0);

  useEffect(() => {
    const id = setInterval(() => setTick((t) => t + 1), 250);
    return () => clearInterval(id);
  }, []);

  const logs = telemetryStore
    .getLogs()
    .filter((l) => filter === 'ALL' || l.protocol === filter)
    .slice(-60)
    .reverse();

  return (
    <div className="h-full flex flex-col p-2 gap-1.5">
      <div className="flex items-center justify-between gap-2">
        <div className="flex items-center gap-1">
          {['ALL', 'OSC', 'MIDI', 'WS', 'UDP'].map((proto) => (
            <button
              key={proto}
              onClick={() => setFilter(proto)}
              className={`px-2 py-0.5 rounded text-[10px] font-mono border ${
                filter === proto
                  ? 'bg-studio-elevated border-amber-500/50 text-amber-300'
                  : 'bg-studio-bg border-studio-border text-studio-muted hover:text-slate-200'
              }`}
            >
              {proto}
            </button>
          ))}
        </div>
        <div className="flex items-center gap-1.5">
          <button
            onClick={() => {
              const next = !paused;
              setPaused(next);
              telemetryStore.setLogPaused(next);
            }}
            className="flex items-center gap-1 px-2 py-0.5 rounded text-[10px] bg-studio-panel border border-studio-border text-slate-200 hover:bg-studio-elevated"
          >
            {paused ? <Play className="w-3 h-3 text-emerald-400" /> : <Pause className="w-3 h-3 text-amber-400" />}
            {paused ? 'Resume' : 'Pause'}
          </button>
          <button
            onClick={() => {
              telemetryStore.clearLogs();
              setTick((t) => t + 1);
            }}
            className="p-1 rounded bg-studio-panel border border-studio-border text-slate-400 hover:text-rose-300"
            title="Clear traffic log"
          >
            <Trash2 className="w-3 h-3" />
          </button>
        </div>
      </div>

      <div className="flex-1 overflow-y-auto rounded-md bg-studio-bg border border-studio-border p-1.5 font-mono text-[11px] space-y-0.5">
        {logs.length === 0 ? (
          <div className="text-studio-muted text-center py-4">
            Waiting for live OSC / MIDI / WebSocket / UDP traffic...
          </div>
        ) : (
          logs.map((entry, idx) => (
            <div key={`${entry.timestamp_ms}-${idx}`} className="flex items-center gap-2 text-[11px]">
              <span className="text-slate-500">
                {new Date(entry.timestamp_ms).toLocaleTimeString()}
              </span>
              <span className="px-1 rounded bg-studio-panel text-cyan-300 text-[10px]">
                {entry.protocol}
              </span>
              <span className="text-amber-400 text-[10px]">{entry.direction}</span>
              <span className="text-slate-200 truncate">{entry.summary}</span>
            </div>
          ))
        )}
      </div>
    </div>
  );
};

export function registerBuiltinDrawerTabs(): void {
  registerDrawerTab({
    id: 'scope',
    label: 'Signal Scope',
    order: 10,
    component: SignalOscilloscopeTab,
  });
  registerDrawerTab({
    id: 'traffic',
    label: 'Traffic Log',
    order: 20,
    component: ProtocolTrafficLogTab,
  });
}

export const BottomDrawer: React.FC = () => {
  const drawerOpen = useWorkspaceStore((s) => s.drawerOpen);
  const setDrawerOpen = useWorkspaceStore((s) => s.setDrawerOpen);
  const activeTab = useWorkspaceStore((s) => s.activeDrawerTab);
  const setActiveTab = useWorkspaceStore((s) => s.setActiveDrawerTab);

  const tabs = listDrawerTabs();
  const current = tabs.find((t) => t.id === activeTab) ?? tabs[0];
  const ActiveComp = current?.component;

  return (
    <section
      className={`shrink-0 bg-studio-surface border-t border-studio-border flex flex-col transition-all ${
        drawerOpen ? 'h-44' : 'h-8'
      }`}
    >
      <div className="h-8 shrink-0 px-3 flex items-center justify-between border-b border-studio-border bg-studio-bg/60">
        <div className="flex items-center gap-1">
          {tabs.map((tab) => (
            <button
              key={tab.id}
              onClick={() => setActiveTab(tab.id)}
              className={`flex items-center gap-1.5 px-2.5 py-1 rounded text-[11px] font-medium transition-colors ${
                drawerOpen && tab.id === current?.id
                  ? 'bg-studio-elevated text-amber-300'
                  : 'text-studio-muted hover:text-slate-200'
              }`}
            >
              {tab.id === 'scope' ? (
                <Activity className="w-3 h-3" />
              ) : (
                <Terminal className="w-3 h-3" />
              )}
              <span>{tab.label}</span>
              {tab.badge && (
                <span className="text-[9px] font-mono px-1 rounded bg-violet-500/20 text-violet-300">
                  {tab.badge}
                </span>
              )}
            </button>
          ))}
        </div>

        <button
          onClick={() => setDrawerOpen(!drawerOpen)}
          className="p-1 text-studio-muted hover:text-slate-200"
          title={drawerOpen ? 'Collapse drawer' : 'Expand drawer'}
        >
          {drawerOpen ? <ChevronDown className="w-4 h-4" /> : <ChevronUp className="w-4 h-4" />}
        </button>
      </div>

      {drawerOpen && ActiveComp && (
        <div className="flex-1 min-h-0 overflow-hidden">
          <ActiveComp />
        </div>
      )}
    </section>
  );
};
