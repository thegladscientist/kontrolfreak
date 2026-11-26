import React, { useMemo, useState } from 'react';
import {
  AlertTriangle,
  Cpu,
  Download,
  FolderOpen,
  Layers,
  Link2,
  Plus,
  Radar,
  RefreshCw,
  Search,
  Unplug,
  Upload,
  Video,
} from 'lucide-react';
import {
  listNodeDefinitions,
  listSidebarSections,
  registerSidebarSection,
} from '../registries';
import { SIGNAL_COLORS } from '../lib/signalColors';
import { useWorkspaceStore } from '../store/workspaceStore';
import type { DeviceKind } from '../types/kontrolfreak';

const DevicePaletteSection: React.FC = () => {
  const [query, setQuery] = useState('');
  const addDevice = useWorkspaceStore((s) => s.addDevice);
  const defs = useMemo(() => listNodeDefinitions(), []);

  const filtered = defs.filter(
    (d) =>
      d.label.toLowerCase().includes(query.toLowerCase()) ||
      d.shortBadge.toLowerCase().includes(query.toLowerCase()) ||
      d.description.toLowerCase().includes(query.toLowerCase())
  );

  const onDragStart = (event: React.DragEvent, kindType: DeviceKind['type']) => {
    event.dataTransfer.setData('application/kontrolfreak-node', kindType);
    event.dataTransfer.effectAllowed = 'move';
  };

  const handleQuickAdd = (kindType: DeviceKind['type']) => {
    const def = defs.find((d) => d.kindType === kindType);
    if (!def) return;
    const existing = useWorkspaceStore.getState().graph.devices;
    const addedIndex = Math.max(0, existing.length - 4);
    const col = addedIndex % 3;
    const row = Math.floor(addedIndex / 3);
    const baseY = existing.length >= 4 ? 500 : 120;
    void addDevice(def.createDevice({ x: 80 + col * 360, y: baseY + row * 430 }));
  };

  return (
    <div className="space-y-3">
      <div className="relative">
        <Search className="w-3.5 h-3.5 text-studio-muted absolute left-2.5 top-2.5" />
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search nodes..."
          className="w-full bg-studio-bg border border-studio-border rounded-md pl-8 pr-2.5 py-1.5 text-xs text-studio-text placeholder:text-studio-muted focus:border-amber-500/60 outline-none"
        />
      </div>

      <p className="text-[11px] text-studio-muted">
        Drag any node onto the canvas, or click <span className="text-slate-300">+</span> to spawn.
      </p>

      <div className="space-y-1.5">
        {filtered.map((def) => {
          const color = SIGNAL_COLORS[def.primarySignal];
          const isFutureSeam = def.milestoneTag && def.milestoneTag !== 'M1';
          return (
            <div
              key={def.kindType}
              draggable
              onDragStart={(e) => onDragStart(e, def.kindType)}
              className="group p-2 rounded-lg bg-studio-panel hover:bg-studio-elevated border border-studio-border hover:border-slate-500/60 cursor-grab active:cursor-grabbing transition-colors"
            >
              <div className="flex items-center justify-between gap-2">
                <div className="flex items-center gap-2 min-w-0">
                  <span
                    className={`text-[10px] font-mono font-semibold px-1.5 py-0.5 rounded border ${color.bgClass} ${color.textClass} ${color.borderClass}`}
                  >
                    {def.shortBadge}
                  </span>
                  <span className="text-xs font-medium text-slate-100 truncate">{def.label}</span>
                </div>
                <div className="flex items-center gap-1 shrink-0">
                  {isFutureSeam && (
                    <span className="text-[9px] font-mono px-1 py-0.5 rounded bg-slate-800 text-slate-400 border border-slate-700">
                      {def.milestoneTag}
                    </span>
                  )}
                  <button
                    onClick={() => handleQuickAdd(def.kindType)}
                    title={`Add ${def.label} to canvas`}
                    className="p-1 rounded hover:bg-slate-700/70 text-slate-400 hover:text-white"
                  >
                    <Plus className="w-3.5 h-3.5" />
                  </button>
                </div>
              </div>
              <p className="text-[11px] text-studio-muted mt-1 leading-snug">{def.description}</p>
            </div>
          );
        })}
      </div>
    </div>
  );
};

const HardwareStatusSection: React.FC = () => {
  const devices = useWorkspaceStore((s) => s.graph.devices);
  const midiPorts = useWorkspaceStore((s) => s.midiPorts);
  const guidance = useWorkspaceStore((s) => s.virtualMidiGuidance);
  const refreshHardware = useWorkspaceStore((s) => s.refreshHardware);
  const setSelection = useWorkspaceStore((s) => s.setSelection);
  const rebindOfflineDevice = useWorkspaceStore((s) => s.rebindOfflineDevice);
  const setDeviceStatus = useWorkspaceStore((s) => s.setDeviceStatus);

  const [showLoopMidiGuide, setShowLoopMidiGuide] = useState(false);

  const onlineDevices = devices.filter((d) => d.status.state === 'online');
  const offlineDevices = devices.filter((d) => d.status.state === 'offline_ghost');

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <span className="text-[11px] font-semibold uppercase tracking-wider text-studio-muted">
          Live &amp; Offline Nodes
        </span>
        <button
          onClick={() => void refreshHardware()}
          className="flex items-center gap-1 text-[11px] text-cyan-300 hover:text-cyan-200 px-1.5 py-0.5 rounded hover:bg-studio-elevated"
        >
          <RefreshCw className="w-3 h-3" />
          Refresh
        </button>
      </div>

      {/* Online Devices */}
      <div className="space-y-1">
        <div className="text-[11px] text-emerald-400 font-medium">
          ● Active ({onlineDevices.length})
        </div>
        {onlineDevices.map((dev) => (
          <div
            key={dev.id}
            onClick={() => setSelection({ kind: 'device', id: dev.id })}
            className="flex items-center justify-between px-2 py-1.5 rounded-md bg-studio-panel hover:bg-studio-elevated border border-studio-border cursor-pointer"
          >
            <div className="min-w-0">
              <div className="text-xs text-slate-100 truncate">{dev.display_name}</div>
              <div className="text-[10px] font-mono text-studio-muted truncate">
                {dev.logical_key}
              </div>
            </div>
            <button
              onClick={(e) => {
                e.stopPropagation();
                void setDeviceStatus(dev.id, {
                  state: 'offline_ghost',
                  reason: 'Simulated unplug',
                });
              }}
              title="Simulate disconnect (enter Offline Ghost)"
              className="p-1 text-slate-400 hover:text-amber-300 rounded hover:bg-slate-800"
            >
              <Unplug className="w-3.5 h-3.5" />
            </button>
          </div>
        ))}
      </div>

      {/* Offline Ghost Devices */}
      <div className="space-y-1">
        <div className="text-[11px] text-amber-400 font-medium">
          ◌ Offline Ghosts ({offlineDevices.length})
        </div>
        {offlineDevices.length === 0 ? (
          <div className="text-[11px] text-studio-muted px-2 py-1">
            No disconnected devices. Unplugging hardware keeps all wires intact here.
          </div>
        ) : (
          offlineDevices.map((dev) => (
            <div
              key={dev.id}
              onClick={() => setSelection({ kind: 'device', id: dev.id })}
              className="p-2 rounded-md bg-studio-panel/60 border border-dashed border-amber-500/40 hover:border-amber-400 cursor-pointer space-y-1.5"
            >
              <div className="flex items-center justify-between gap-2">
                <span className="text-xs font-medium text-amber-200 truncate">
                  {dev.display_name}
                </span>
                <button
                  onClick={(e) => {
                    e.stopPropagation();
                    void rebindOfflineDevice(
                      dev.id,
                      dev.fingerprint,
                      midiPorts[0]?.name ?? 'Kontrolfreak Loopback 1'
                    );
                  }}
                  className="flex items-center gap-1 px-2 py-0.5 rounded text-[10px] font-medium bg-amber-500/20 hover:bg-amber-500/30 text-amber-200 border border-amber-400/40"
                >
                  <Link2 className="w-3 h-3" />
                  Rebind
                </button>
              </div>
              {dev.status.state === 'offline_ghost' && dev.status.reason && (
                <div className="text-[10px] text-studio-muted">{dev.status.reason}</div>
              )}
            </div>
          ))
        )}
      </div>

      {/* Detected MIDI / Virtual MIDI Ports & Windows loopMIDI Setup Helper */}
      <div className="pt-2 border-t border-studio-border space-y-2">
        <div className="text-[11px] font-semibold uppercase tracking-wider text-studio-muted">
          MIDI &amp; Virtual Ports ({midiPorts.length})
        </div>
        <div className="space-y-1">
          {midiPorts.map((p, idx) => (
            <div
              key={`${p.name}-${idx}`}
              className="flex items-center justify-between px-2 py-1 rounded bg-studio-bg border border-studio-border text-[11px]"
            >
              <span className="truncate text-slate-200">{p.name}</span>
              <span
                className={`text-[9px] font-mono px-1.5 py-0.5 rounded ${
                  p.is_loopmidi
                    ? 'bg-cyan-500/20 text-cyan-300'
                    : p.is_virtual
                    ? 'bg-violet-500/20 text-violet-300'
                    : 'bg-slate-800 text-slate-300'
                }`}
              >
                {p.is_loopmidi ? 'loopMIDI' : p.is_virtual ? 'Virtual' : 'HW'}
              </span>
            </div>
          ))}
        </div>

        {/* Windows loopMIDI Setup Helper */}
        {guidance && (
          <div className="rounded-lg bg-cyan-950/30 border border-cyan-500/30 p-2 space-y-1.5">
            <button
              onClick={() => setShowLoopMidiGuide((v) => !v)}
              className="w-full flex items-center justify-between text-left text-[11px] font-medium text-cyan-200"
            >
              <span className="flex items-center gap-1.5">
                <AlertTriangle className="w-3.5 h-3.5 text-cyan-400 shrink-0" />
                Virtual MIDI / loopMIDI Guide
              </span>
              <span className="text-[10px] text-cyan-400">
                {showLoopMidiGuide ? 'Hide' : 'Show'}
              </span>
            </button>
            {showLoopMidiGuide && (
              <div className="space-y-1.5 pt-1 text-[11px] text-slate-300">
                <div className="text-[10px] font-mono text-cyan-300">
                  {guidance.platform} · loopMIDI:{' '}
                  {guidance.loopmidi_detected ? 'Detected' : 'Not Detected'}
                </div>
                <ol className="list-decimal list-inside space-y-1 text-[11px] text-slate-300">
                  {guidance.steps.map((step, i) => (
                    <li key={i}>{step}</li>
                  ))}
                </ol>
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
};

const PresetsProjectsSection: React.FC = () => {
  const graph = useWorkspaceStore((s) => s.graph);
  const exportProjectJson = useWorkspaceStore((s) => s.exportProjectJson);
  const loadProjectJson = useWorkspaceStore((s) => s.loadProjectJson);
  const importDevicePresetJson = useWorkspaceStore((s) => s.importDevicePresetJson);
  const importGroupPresetJson = useWorkspaceStore((s) => s.importGroupPresetJson);
  const pushToast = useWorkspaceStore((s) => s.pushToast);

  const [jsonBuffer, setJsonBuffer] = useState('');

  const handleCopyProjectJson = async () => {
    const json = await exportProjectJson();
    setJsonBuffer(json);
    pushToast({
      title: 'Project JSON snapshot ready',
      description: 'Inspect or copy below, or save as .kfproj.json.',
      accent: 'amber',
    });
  };

  const handleApplyJson = async () => {
    const trimmed = jsonBuffer.trim();
    if (!trimmed) return;
    try {
      const parsed = JSON.parse(trimmed) as Record<string, unknown>;
      if ('graph' in parsed) {
        await loadProjectJson(trimmed, true);
      } else if ('member_devices' in parsed) {
        await importGroupPresetJson(trimmed);
      } else if ('device' in parsed) {
        await importDevicePresetJson(trimmed);
      } else {
        throw new Error('Unrecognized Kontrolfreak JSON artifact');
      }
    } catch (err) {
      pushToast({
        title: 'Import failed',
        description: err instanceof Error ? err.message : String(err),
        accent: 'rose',
      });
    }
  };

  const handleUploadPresetFile = (kind: 'project' | 'device' | 'group') => {
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = '.json';
    input.onchange = async () => {
      const file = input.files?.[0];
      if (!file) return;
      try {
        const text = await file.text();
        if (kind === 'project') await loadProjectJson(text, true);
        if (kind === 'device') await importDevicePresetJson(text);
        if (kind === 'group') await importGroupPresetJson(text);
      } catch (err) {
        pushToast({
          title: 'Import failed',
          description: err instanceof Error ? err.message : String(err),
          accent: 'rose',
        });
      }
    };
    input.click();
  };

  return (
    <div className="space-y-3">
      <div className="text-[11px] font-semibold uppercase tracking-wider text-studio-muted">
        Presets &amp; Projects (v1 JSON)
      </div>

      <div className="grid grid-cols-1 gap-1.5">
        <button
          onClick={() => void handleCopyProjectJson()}
          className="flex items-center justify-between px-2.5 py-1.5 rounded-md bg-studio-panel hover:bg-studio-elevated border border-studio-border text-xs text-slate-200"
        >
          <span>Snapshot Project (.kfproj.json)</span>
          <Download className="w-3.5 h-3.5 text-amber-400" />
        </button>
        <button
          onClick={() => handleUploadPresetFile('device')}
          className="flex items-center justify-between px-2.5 py-1.5 rounded-md bg-studio-panel hover:bg-studio-elevated border border-studio-border text-xs text-slate-200"
        >
          <span>Import Device (.kfdevice.json)</span>
          <Upload className="w-3.5 h-3.5 text-emerald-400" />
        </button>
        <button
          onClick={() => handleUploadPresetFile('group')}
          className="flex items-center justify-between px-2.5 py-1.5 rounded-md bg-studio-panel hover:bg-studio-elevated border border-studio-border text-xs text-slate-200"
        >
          <span>Import Rack (.kfgroup.json)</span>
          <Upload className="w-3.5 h-3.5 text-violet-400" />
        </button>
      </div>

      <div className="space-y-1.5 pt-2 border-t border-studio-border">
        <div className="text-[11px] text-studio-muted">
          Quick JSON Inspector ({graph.devices.length} nodes · {graph.wires.length} wires ·{' '}
          {graph.groups.length} racks)
        </div>
        <textarea
          value={jsonBuffer}
          onChange={(e) => setJsonBuffer(e.target.value)}
          rows={7}
          placeholder="Click 'Snapshot Project' or paste .kfproj.json / .kfdevice.json / .kfgroup.json..."
          className="w-full bg-studio-bg border border-studio-border rounded-md p-2 text-[10px] font-mono text-slate-300 outline-none focus:border-amber-500/50"
        />
        <button
          onClick={() => void handleApplyJson()}
          disabled={!jsonBuffer.trim()}
          className="w-full py-1.5 rounded-md text-xs font-medium bg-amber-500/20 hover:bg-amber-500/30 disabled:opacity-40 text-amber-200 border border-amber-500/40"
        >
          Load / Import JSON
        </button>
      </div>
    </div>
  );
};

export function registerBuiltinSidebarSections(): void {
  registerSidebarSection({
    id: 'palette',
    label: 'Nodes',
    order: 10,
    iconName: 'layers',
    component: DevicePaletteSection,
  });
  registerSidebarSection({
    id: 'hardware',
    label: 'Devices',
    order: 20,
    iconName: 'cpu',
    component: HardwareStatusSection,
  });
  registerSidebarSection({
    id: 'presets',
    label: 'Presets',
    order: 30,
    iconName: 'folder',
    component: PresetsProjectsSection,
  });
}

export const LeftSidebar: React.FC = () => {
  const activeId = useWorkspaceStore((s) => s.activeSidebarSection);
  const setActive = useWorkspaceStore((s) => s.setActiveSidebarSection);
  const sections = listSidebarSections();
  const activeSection = sections.find((s) => s.id === activeId) ?? sections[0];
  const ActiveComponent = activeSection?.component;

  const renderIcon = (name: string) => {
    switch (name) {
      case 'cpu':
        return <Cpu className="w-3.5 h-3.5" />;
      case 'folder':
        return <FolderOpen className="w-3.5 h-3.5" />;
      case 'radar':
        return <Radar className="w-3.5 h-3.5" />;
      case 'video':
        return <Video className="w-3.5 h-3.5" />;
      default:
        return <Layers className="w-3.5 h-3.5" />;
    }
  };

  return (
    <aside className="w-64 shrink-0 bg-studio-surface border-r border-studio-border flex flex-col min-h-0">
      <div className="grid grid-cols-3 border-b border-studio-border p-1 gap-1 bg-studio-bg/50">
        {sections.map((sec) => (
          <button
            key={sec.id}
            onClick={() => setActive(sec.id)}
            className={`flex items-center justify-center gap-1 px-1.5 py-1.5 rounded-md text-[11px] font-medium transition-colors truncate ${
              sec.id === activeSection?.id
                ? 'bg-studio-elevated text-amber-300 border border-studio-border'
                : 'text-studio-muted hover:text-slate-200 hover:bg-studio-panel'
            }`}
          >
            {renderIcon(sec.iconName)}
            <span className="truncate">{sec.label}</span>
          </button>
        ))}
      </div>

      <div className="flex-1 overflow-y-auto p-3">
        {ActiveComponent ? <ActiveComponent /> : null}
      </div>
    </aside>
  );
};
