import React, { useEffect, useState } from 'react';
import { AlertCircle, CheckCircle2, Film, Plus, Radio, RefreshCw } from 'lucide-react';
import { getNodeDefinition } from '../../registries';
import { useWorkspaceStore } from '../../store/workspaceStore';
import {
  browserM2Mock,
  m2Bridge,
  type NdiRuntimeStatus,
  type RtpMidiPeer,
  type VideoSourceInfo,
} from './m2Bridge';

export const M2SidebarSection: React.FC = () => {
  const addDevice = useWorkspaceStore((s) => s.addDevice);
  const pushToast = useWorkspaceStore((s) => s.pushToast);

  const [runtime, setRuntime] = useState<NdiRuntimeStatus>(() =>
    browserM2Mock.getNdiRuntimeStatus()
  );
  const [sources, setSources] = useState<VideoSourceInfo[]>(() =>
    browserM2Mock.discoverNdiSources()
  );
  const [peers, setPeers] = useState<RtpMidiPeer[]>(() =>
    browserM2Mock.discoverRtpMidiPeers()
  );

  const refresh = async () => {
    const [rt, srcs, discovered] = await Promise.all([
      m2Bridge.getNdiRuntimeStatus(),
      m2Bridge.discoverNdiSources(),
      m2Bridge.discoverRtpMidiPeers(),
    ]);
    setRuntime(rt);
    setSources(srcs);
    setPeers(discovered);
  };

  useEffect(() => {
    void refresh();
  }, []);

  const spawnNdiWithSource = async (src: VideoSourceInfo) => {
    const def = getNodeDefinition('NdiVideoNode');
    if (!def) return;
    const offset = Math.round(Math.random() * 110);
    const dev = def.createDevice({ x: 260 + offset, y: 140 + offset });
    if (dev.kind.type === 'NdiVideoNode') {
      dev.kind.source_name = src.name;
      dev.display_name = src.name.replace(/^Test Pattern:\s*/i, 'NDI ');
    }
    const added = await addDevice(dev);
    await m2Bridge.configureNdiNode(added.id, { source_name: src.name }, added);
    pushToast({
      title: `Added ${added.display_name}`,
      description: 'Wire any OSC/MIDI/WS/UDP port to crop, zoom, or resize live.',
      accent: 'violet',
    });
  };

  const spawnRtpPeer = async (peer: RtpMidiPeer) => {
    const def = getNodeDefinition('RtpMidiSession');
    if (!def) return;
    const offset = Math.round(Math.random() * 110);
    const dev = def.createDevice({ x: 200 + offset, y: 220 + offset });
    if (dev.kind.type === 'RtpMidiSession') {
      dev.kind.remote_host = `${peer.host}:${peer.control_port}`;
      dev.display_name = `RTP · ${peer.name}`;
    }
    const added = await addDevice(dev);
    await m2Bridge.startRtpMidiSession(added.id, added);
    await m2Bridge.connectRtpMidiPeer(added.id, `${peer.host}:${peer.control_port}`, added);
    pushToast({
      title: `Connected RTPMIDI "${peer.name}"`,
      description: `Dual-UDP control :${peer.control_port} / data :${peer.control_port + 1}`,
      accent: 'cyan',
    });
  };

  return (
    <div className="space-y-4">
      {/* Header + Refresh */}
      <div className="flex items-center justify-between">
        <span className="text-[11px] font-semibold uppercase tracking-wider text-studio-muted">
          NDI &amp; RTPMIDI Network
        </span>
        <button
          onClick={() => void refresh()}
          className="flex items-center gap-1 text-[11px] text-violet-300 hover:text-violet-200 px-1.5 py-0.5 rounded hover:bg-studio-elevated"
        >
          <RefreshCw className="w-3 h-3" />
          Scan
        </button>
      </div>

      {/* NDI Runtime Banner */}
      <div
        className={`p-2 rounded-lg border text-[11px] space-y-1 ${
          runtime.available
            ? 'bg-emerald-950/30 border-emerald-500/35'
            : 'bg-violet-950/30 border-violet-500/35'
        }`}
      >
        <div className="flex items-center gap-1.5 font-medium text-slate-100">
          {runtime.available ? (
            <CheckCircle2 className="w-3.5 h-3.5 text-emerald-400 shrink-0" />
          ) : (
            <AlertCircle className="w-3.5 h-3.5 text-violet-400 shrink-0" />
          )}
          <span>{runtime.available ? 'NDI Runtime v6 Ready' : 'NDI Synthetic Mode Ready'}</span>
        </div>
        <p className="text-[10px] text-studio-muted leading-snug">
          {runtime.available
            ? runtime.version ?? 'Dynamic NDI v6 library loaded.'
            : 'Install NDI Runtime for LAN streams, or spawn a built-in Test Pattern below.'}
        </p>
      </div>

      {/* Discovered NDI Video Sources */}
      <div className="space-y-1.5">
        <div className="flex items-center gap-1.5 text-[11px] font-medium text-violet-300">
          <Film className="w-3.5 h-3.5" />
          NDI Video Sources ({sources.length})
        </div>
        {sources.map((src) => (
          <div
            key={src.name}
            className="flex items-center justify-between gap-2 px-2 py-1.5 rounded-md bg-studio-panel hover:bg-studio-elevated border border-studio-border"
          >
            <div className="min-w-0">
              <div className="text-[11px] text-slate-100 truncate">{src.name}</div>
              <div className="text-[9px] font-mono text-studio-muted">
                {src.width && src.height ? `${src.width}×${src.height}` : 'Auto'} ·{' '}
                {src.fps ? `${src.fps}fps` : 'LAN NDI'}
              </div>
            </div>
            <button
              onClick={() => void spawnNdiWithSource(src)}
              title="Spawn NDI Video Scaler node with this source"
              className="p-1 rounded bg-violet-500/20 hover:bg-violet-500/35 text-violet-200 border border-violet-500/40 shrink-0"
            >
              <Plus className="w-3 h-3" />
            </button>
          </div>
        ))}
      </div>

      {/* Discovered RTPMIDI Peers */}
      <div className="space-y-1.5 pt-2 border-t border-studio-border">
        <div className="flex items-center gap-1.5 text-[11px] font-medium text-cyan-300">
          <Radio className="w-3.5 h-3.5" />
          RTPMIDI Peers · mDNS ({peers.length})
        </div>
        {peers.map((peer) => (
          <div
            key={`${peer.host}:${peer.control_port}`}
            className="flex items-center justify-between gap-2 px-2 py-1.5 rounded-md bg-studio-panel hover:bg-studio-elevated border border-studio-border"
          >
            <div className="min-w-0">
              <div className="text-[11px] text-slate-100 truncate">{peer.name}</div>
              <div className="text-[9px] font-mono text-studio-muted">
                {peer.host}:{peer.control_port}
              </div>
            </div>
            <button
              onClick={() => void spawnRtpPeer(peer)}
              title="Spawn RTPMIDI session connected to this peer"
              className="p-1 rounded bg-cyan-500/20 hover:bg-cyan-500/35 text-cyan-200 border border-cyan-500/40 shrink-0"
            >
              <Plus className="w-3 h-3" />
            </button>
          </div>
        ))}
      </div>
    </div>
  );
};
