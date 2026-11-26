import React, { useEffect, useState } from 'react';
import { Activity, Clock, Play, Plus, Radio, Send, Square, Wifi } from 'lucide-react';
import { isTauriRuntime } from '../../lib/bridge';
import { useWorkspaceStore } from '../../store/workspaceStore';
import type { Device } from '../../types/kontrolfreak';
import { browserM2Mock, m2Bridge, type RtpMidiSessionStatus } from './m2Bridge';

export const RtpMidiInspectorPanel: React.FC<{ device?: Device }> = ({ device }) => {
  const updateDeviceKind = useWorkspaceStore((s) => s.updateDeviceKind);
  const pushToast = useWorkspaceStore((s) => s.pushToast);

  const [status, setStatus] = useState<RtpMidiSessionStatus | null>(() =>
    device ? browserM2Mock.getRtpMidiStatus(device.id, device) : null
  );
  const [peerAddrInput, setPeerAddrInput] = useState('127.0.0.1:5006');

  useEffect(() => {
    if (!device) return;
    let alive = true;
    const refresh = async () => {
      const st = await m2Bridge.getRtpMidiStatus(device.id, device);
      if (alive) setStatus(st);
    };
    void refresh();

    const unsub = browserM2Mock.subscribe(() => {
      if (!isTauriRuntime()) {
        setStatus(browserM2Mock.getRtpMidiStatus(device.id, device));
      }
    });
    return () => {
      alive = false;
      unsub();
    };
  }, [device]);

  if (!device || device.kind.type !== 'RtpMidiSession') return null;
  const rtpKind = device.kind;
  const running = status?.running ?? false;

  const refresh = async () => {
    const st = await m2Bridge.getRtpMidiStatus(device.id, device);
    setStatus(st);
  };

  const handleToggleSession = async () => {
    try {
      if (running) {
        await m2Bridge.stopRtpMidiSession(device.id);
      } else {
        await m2Bridge.startRtpMidiSession(device.id, device);
      }
      await refresh();
    } catch (err) {
      pushToast({
        title: 'RTPMIDI session error',
        description: err instanceof Error ? err.message : String(err),
        accent: 'rose',
      });
    }
  };

  const handleConnect = async (addr: string) => {
    const trimmed = addr.trim();
    if (!trimmed) return;
    try {
      await m2Bridge.connectRtpMidiPeer(device.id, trimmed, device);
      await refresh();
      pushToast({
        title: 'RTPMIDI invitation sent',
        description: `Initiated RFC 6295 dual-port handshake with ${trimmed}`,
        accent: 'cyan',
      });
    } catch (err) {
      pushToast({
        title: 'Peer connect failed',
        description: err instanceof Error ? err.message : String(err),
        accent: 'rose',
      });
    }
  };

  const handleSyncClock = async () => {
    await m2Bridge.syncRtpMidiClock(device.id, device);
    await refresh();
    pushToast({ title: 'Clock Sync (CK) complete', accent: 'cyan' });
  };

  const handleSendTest = async () => {
    await m2Bridge.sendRtpMidiTestMidi(device.id, [0x90, 60, 100], device);
    await refresh();
    pushToast({
      title: 'Sent RTPMIDI Test Note',
      description: 'Channel 1 · Note C4 (60) Vel 100',
      accent: 'emerald',
    });
  };

  return (
    <div className="space-y-3 text-xs">
      {/* Session Controls */}
      <div className="p-2 rounded-lg bg-studio-bg border border-studio-border space-y-2">
        <div className="flex items-center justify-between">
          <span className="flex items-center gap-1.5 text-[11px] font-medium text-cyan-200">
            <Radio className="w-3.5 h-3.5 text-cyan-400" />
            RFC 6295 Session
          </span>
          <span
            className={`text-[10px] font-mono px-1.5 py-0.5 rounded ${
              running
                ? 'bg-emerald-500/20 text-emerald-300'
                : 'bg-slate-800 text-slate-400'
            }`}
          >
            {running ? 'Running' : 'Stopped'}
          </span>
        </div>

        <label className="block space-y-0.5">
          <span className="text-[10px] text-studio-muted">Bonjour / Session Name</span>
          <input
            value={rtpKind.session_name}
            onChange={(e) =>
              void updateDeviceKind(device.id, {
                ...rtpKind,
                session_name: e.target.value,
              })
            }
            className="w-full bg-studio-surface border border-studio-border rounded px-2 py-1 text-xs text-slate-100"
          />
        </label>

        <div className="grid grid-cols-2 gap-2">
          <label className="space-y-0.5">
            <span className="text-[10px] text-studio-muted">Control Port (N)</span>
            <input
              type="number"
              min={1024}
              max={65534}
              value={rtpKind.control_port}
              onChange={(e) =>
                void updateDeviceKind(device.id, {
                  ...rtpKind,
                  control_port: Number(e.target.value) || 5004,
                })
              }
              className="w-full bg-studio-surface border border-studio-border rounded px-2 py-1 font-mono text-slate-100"
            />
          </label>
          <div className="space-y-0.5">
            <span className="text-[10px] text-studio-muted">Data Port (N+1)</span>
            <div className="px-2 py-1 rounded bg-studio-panel border border-studio-border font-mono text-cyan-300">
              {rtpKind.control_port + 1}
            </div>
          </div>
        </div>

        <label className="flex items-center gap-2 text-[11px] text-slate-300 cursor-pointer">
          <input
            type="checkbox"
            checked={rtpKind.mdns_discover}
            onChange={(e) =>
              void updateDeviceKind(device.id, {
                ...rtpKind,
                mdns_discover: e.target.checked,
              })
            }
            className="accent-cyan-400"
          />
          Announce &amp; discover `_apple-midi._udp.local.`
        </label>

        <div className="flex gap-1.5 pt-1">
          <button
            onClick={() => void handleToggleSession()}
            className={`flex-1 flex items-center justify-center gap-1 py-1.5 rounded text-xs font-medium border ${
              running
                ? 'bg-rose-950/50 text-rose-200 border-rose-500/40 hover:bg-rose-900/50'
                : 'bg-cyan-500/25 text-cyan-100 border-cyan-400/40 hover:bg-cyan-500/35'
            }`}
          >
            {running ? <Square className="w-3 h-3" /> : <Play className="w-3 h-3" />}
            {running ? 'Stop Session' : 'Start Session'}
          </button>
          <button
            onClick={() => void handleSyncClock()}
            disabled={!running}
            title="Trigger 3-way CK clock synchronization"
            className="px-2 py-1.5 rounded bg-studio-panel hover:bg-studio-elevated disabled:opacity-40 border border-studio-border text-cyan-300"
          >
            <Clock className="w-3.5 h-3.5" />
          </button>
          <button
            onClick={() => void handleSendTest()}
            disabled={!running}
            title="Send test MIDI Note C4 over RTPMIDI"
            className="px-2 py-1.5 rounded bg-studio-panel hover:bg-studio-elevated disabled:opacity-40 border border-studio-border text-emerald-300"
          >
            <Send className="w-3.5 h-3.5" />
          </button>
        </div>
      </div>

      {/* Connect Direct Peer */}
      <div className="space-y-1.5">
        <div className="text-[11px] font-medium text-slate-200">Connect Remote Peer</div>
        <div className="flex gap-1.5">
          <input
            value={peerAddrInput}
            onChange={(e) => setPeerAddrInput(e.target.value)}
            placeholder="192.168.1.42:5004"
            className="flex-1 bg-studio-bg border border-studio-border rounded px-2 py-1 font-mono text-[11px] text-slate-100 outline-none"
          />
          <button
            onClick={() => void handleConnect(peerAddrInput)}
            className="px-2 py-1 rounded bg-cyan-500/20 hover:bg-cyan-500/30 border border-cyan-500/40 text-cyan-200"
            title="Send AppleMIDI Invitation (IN)"
          >
            <Plus className="w-3.5 h-3.5" />
          </button>
        </div>
      </div>

      {/* Connected Peers & Latency */}
      <div className="space-y-1.5 pt-2 border-t border-studio-border">
        <div className="flex items-center justify-between text-[11px]">
          <span className="font-medium text-slate-200">
            Connected Peers ({status?.connected_peers.length ?? 0})
          </span>
          {status && (
            <span className="font-mono text-[10px] text-studio-muted">
              TX {status.tx_packets} · RX {status.rx_packets}
            </span>
          )}
        </div>

        {!status || status.connected_peers.length === 0 ? (
          <div className="text-[11px] text-studio-muted">
            No active peers connected yet. Click Connect on a discovered peer below or enter host:port.
          </div>
        ) : (
          status.connected_peers.map((peer) => (
            <div
              key={peer.control_addr}
              className="p-2 rounded bg-studio-panel border border-studio-border space-y-1"
            >
              <div className="flex items-center justify-between">
                <span className="font-medium text-slate-100 truncate">{peer.name}</span>
                <span className="flex items-center gap-1 font-mono text-[10px] text-emerald-300">
                  <Activity className="w-3 h-3" />
                  {peer.rtt_ms !== null ? `${peer.rtt_ms.toFixed(1)} ms` : 'syncing'}
                </span>
              </div>
              <div className="flex items-center justify-between text-[10px] font-mono text-studio-muted">
                <span>{peer.control_addr}</span>
                <span>CK×{peer.sync_count}</span>
              </div>
            </div>
          ))
        )}
      </div>

      {/* mDNS-SD Discovered Peers */}
      <div className="space-y-1.5 pt-2 border-t border-studio-border">
        <div className="flex items-center gap-1.5 text-[11px] font-medium text-slate-200">
          <Wifi className="w-3.5 h-3.5 text-cyan-400" />
          mDNS Discovered Peers ({status?.discovered_peers.length ?? 0})
        </div>
        {status?.mdns.fallback_reason && (
          <div className="text-[10px] text-amber-300 bg-amber-950/30 border border-amber-500/30 rounded p-1.5">
            {status.mdns.fallback_reason}
          </div>
        )}
        <div className="space-y-1">
          {(status?.discovered_peers ?? []).map((peer) => (
            <div
              key={`${peer.host}:${peer.control_port}`}
              className="flex items-center justify-between px-2 py-1.5 rounded bg-studio-bg border border-studio-border"
            >
              <div className="min-w-0">
                <div className="text-[11px] text-slate-200 truncate">{peer.name}</div>
                <div className="text-[9px] font-mono text-studio-muted">
                  {peer.host}:{peer.control_port}
                </div>
              </div>
              <button
                onClick={() => void handleConnect(`${peer.host}:${peer.control_port}`)}
                className="px-2 py-0.5 rounded bg-cyan-500/20 hover:bg-cyan-500/35 border border-cyan-500/40 text-[10px] text-cyan-200"
              >
                Connect
              </button>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
};
