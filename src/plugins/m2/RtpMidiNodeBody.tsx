import React, { useEffect, useState } from 'react';
import { Activity, Play, Radio, Square, Wifi } from 'lucide-react';
import { isTauriRuntime } from '../../lib/bridge';
import type { Device } from '../../types/kontrolfreak';
import { browserM2Mock, m2Bridge, type RtpMidiSessionStatus } from './m2Bridge';

export const RtpMidiNodeBody: React.FC<{ device: Device; calmMode: boolean }> = ({ device }) => {
  const [status, setStatus] = useState<RtpMidiSessionStatus | null>(() =>
    browserM2Mock.getRtpMidiStatus(device.id, device)
  );

  useEffect(() => {
    let alive = true;
    const poll = async () => {
      const st = await m2Bridge.getRtpMidiStatus(device.id, device);
      if (alive && st) setStatus(st);
    };
    void poll();

    const unsub = browserM2Mock.subscribe(() => {
      if (!isTauriRuntime()) {
        setStatus(browserM2Mock.getRtpMidiStatus(device.id, device));
      }
    });
    const timer = setInterval(() => {
      if (isTauriRuntime()) void poll();
    }, 1500);

    return () => {
      alive = false;
      unsub();
      clearInterval(timer);
    };
  }, [device]);

  const kind = device.kind.type === 'RtpMidiSession' ? device.kind : null;
  const controlPort = status?.control_port ?? kind?.control_port ?? 5004;
  const dataPort = status?.data_port ?? controlPort + 1;
  const running = status?.running ?? false;
  const connected = status?.connected_peers ?? [];
  const discoveredCount = status?.discovered_peers.length ?? 0;

  const avgRtt =
    connected.length > 0
      ? connected.reduce((acc, p) => acc + (p.rtt_ms ?? 0), 0) / connected.length
      : null;

  const toggleRun = async (e: React.MouseEvent) => {
    e.stopPropagation();
    if (running) {
      await m2Bridge.stopRtpMidiSession(device.id);
    } else {
      await m2Bridge.startRtpMidiSession(device.id, device);
    }
    const next = await m2Bridge.getRtpMidiStatus(device.id, device);
    setStatus(next);
  };

  return (
    <div className="space-y-1.5 nodrag text-[10px]">
      {/* Session State + Ports + Start/Stop */}
      <div className="flex items-center justify-between gap-2">
        <div className="flex items-center gap-1.5 min-w-0">
          <Radio
            className={`w-3 h-3 shrink-0 ${
              running ? 'text-cyan-400' : 'text-slate-500'
            }`}
          />
          <span className="font-mono text-slate-200 truncate">
            UDP :{controlPort}/{dataPort}
          </span>
        </div>

        <button
          onClick={(e) => void toggleRun(e)}
          className={`flex items-center gap-1 px-1.5 py-0.5 rounded border text-[9px] font-medium ${
            running
              ? 'bg-emerald-500/20 text-emerald-200 border-emerald-500/40 hover:bg-rose-500/20 hover:text-rose-200'
              : 'bg-cyan-500/20 text-cyan-200 border-cyan-500/40 hover:bg-cyan-500/30'
          }`}
        >
          {running ? <Square className="w-2.5 h-2.5" /> : <Play className="w-2.5 h-2.5" />}
          {running ? 'Live' : 'Start'}
        </button>
      </div>

      {/* Latency & Peer Summary */}
      <div className="flex items-center justify-between px-2 py-1 rounded bg-studio-bg border border-studio-border">
        <span className="flex items-center gap-1 text-slate-300">
          <Wifi className="w-3 h-3 text-cyan-400" />
          {connected.length} peer{connected.length === 1 ? '' : 's'}
          {discoveredCount > 0 && (
            <span className="text-studio-muted">· {discoveredCount} mDNS</span>
          )}
        </span>
        <span className="flex items-center gap-1 font-mono text-emerald-300">
          <Activity className="w-2.5 h-2.5" />
          {avgRtt !== null ? `${avgRtt.toFixed(1)} ms` : '-- ms'}
        </span>
      </div>

      {/* Connected Peer Chips */}
      {connected.length > 0 && (
        <div className="flex flex-wrap gap-1">
          {connected.slice(0, 2).map((p) => (
            <span
              key={p.control_addr}
              className="px-1.5 py-0.5 rounded bg-cyan-950/50 border border-cyan-500/30 text-[9px] text-cyan-200 truncate max-w-[200px]"
            >
              {p.name}
            </span>
          ))}
        </div>
      )}
    </div>
  );
};
