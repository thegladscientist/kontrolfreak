import React, { useState } from 'react';
import {
  ChevronDown,
  ChevronRight,
  Download,
  Layers,
  Link2,
  Plus,
  Sliders,
  Trash2,
  Unplug,
} from 'lucide-react';
import { useWorkspaceStore } from '../store/workspaceStore';
import { getInspectorPanelsFor } from '../registries';
import { SIGNAL_COLORS } from '../lib/signalColors';
import { TransferCurveCanvas } from './TransferCurveCanvas';
import { bridge } from '../lib/bridge';
import type { CurveKind, Device, Modifier, RackGroup, Wire } from '../types/kontrolfreak';

const MODIFIER_TEMPLATES: Array<{ label: string; create: () => Modifier }> = [
  { label: 'Invert', create: () => ({ type: 'Invert' }) },
  {
    label: 'Range Map',
    create: () => ({
      type: 'RangeMap',
      in_min: 0,
      in_max: 1,
      out_min: 0,
      out_max: 1,
      clamp: true,
    }),
  },
  { label: 'Clamp', create: () => ({ type: 'Clamp', min: 0.1, max: 0.9 }) },
  {
    label: 'Deadzone',
    create: () => ({ type: 'Deadzone', center: 0.5, radius: 0.06, rescale: true }),
  },
  {
    label: 'Curve (S-Curve / Exp / Log)',
    create: () => ({ type: 'Curve', kind: { kind: 's_curve', steepness: 5 } }),
  },
  {
    label: '1-Euro Smooth',
    create: () => ({ type: 'SmoothOneEuro', min_cutoff: 1.2, beta: 0.05, d_cutoff: 1.0 }),
  },
  {
    label: 'Hysteresis Gate',
    create: () => ({
      type: 'Hysteresis',
      low_threshold: 0.35,
      high_threshold: 0.65,
      low_value: 0,
      high_value: 1,
    }),
  },
  {
    label: 'Threshold / Trigger',
    create: () => ({
      type: 'Threshold',
      threshold: 0.5,
      rising_only: false,
      emit_trigger: false,
    }),
  },
  {
    label: 'Math Expression',
    create: () => ({ type: 'Expression', expr: 'clamp(x * 1.2 - 0.1, 0, 1)' }),
  },
];

const DeviceInspector: React.FC<{ device: Device }> = ({ device }) => {
  const graph = useWorkspaceStore((s) => s.graph);
  const renameDevice = useWorkspaceStore((s) => s.renameDevice);
  const renamePort = useWorkspaceStore((s) => s.renamePort);
  const addPort = useWorkspaceStore((s) => s.addPort);
  const removeDevice = useWorkspaceStore((s) => s.removeDevice);
  const updateDeviceKind = useWorkspaceStore((s) => s.updateDeviceKind);
  const setManualOverride = useWorkspaceStore((s) => s.setManualOverride);
  const rebindOfflineDevice = useWorkspaceStore((s) => s.rebindOfflineDevice);
  const exposeMacro = useWorkspaceStore((s) => s.exposeMacro);
  const pushToast = useWorkspaceStore((s) => s.pushToast);
  const midiPorts = useWorkspaceStore((s) => s.midiPorts);

  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [newPortKey, setNewPortKey] = useState('');
  const [newPortAlias, setNewPortAlias] = useState('');

  const parentRack = device.group_id
    ? graph.groups.find((g) => g.id === device.group_id)
    : undefined;
  const extensionPanels = getInspectorPanelsFor(device.kind.type);

  const handleExportDevice = async () => {
    const json = await bridge.exportDevicePreset(device.id);
    const blob = new Blob([json], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `${device.display_name.toLowerCase().replace(/[^a-z0-9]+/g, '-')}.kfdevice.json`;
    a.click();
    URL.revokeObjectURL(url);
    pushToast({ title: 'Exported .kfdevice.json', accent: 'emerald' });
  };

  const handleAddPort = () => {
    const key = newPortKey.trim();
    if (!key) return;
    void addPort(device.id, {
      id: '',
      logical_key: key,
      alias: newPortAlias.trim() || key,
      direction: 'bidir',
      signal_kind: device.kind.type === 'MidiPort' ? 'midi' : 'control',
      min_value: 0,
      max_value: 1,
      modifiers: [],
    });
    setNewPortKey('');
    setNewPortAlias('');
  };

  return (
    <div className="space-y-4">
      {/* Header & Alias */}
      <div className="space-y-2">
        <div className="flex items-center justify-between">
          <span className="text-[10px] font-mono uppercase text-studio-muted">
            {device.kind.type}
          </span>
          <span
            className={`text-[10px] font-mono px-1.5 py-0.5 rounded ${
              device.status.state === 'online'
                ? 'bg-emerald-500/20 text-emerald-300'
                : 'bg-amber-500/20 text-amber-300'
            }`}
          >
            {device.status.state === 'online' ? 'Online' : 'Offline Ghost'}
          </span>
        </div>

        <label className="block space-y-1">
          <span className="text-[11px] text-studio-muted">Display Alias (UUID-safe)</span>
          <input
            value={device.display_name}
            onChange={(e) => void renameDevice(device.id, e.target.value)}
            className="w-full bg-studio-bg border border-studio-border rounded-md px-2.5 py-1.5 text-xs text-white focus:border-amber-500/60 outline-none"
          />
        </label>

        {device.status.state === 'offline_ghost' && (
          <div className="p-2.5 rounded-lg bg-amber-950/35 border border-dashed border-amber-500/50 space-y-2">
            <div className="flex items-center gap-1.5 text-xs text-amber-200 font-medium">
              <Unplug className="w-3.5 h-3.5" />
              Disconnected — wires preserved
            </div>
            <p className="text-[11px] text-studio-muted">
              Rebind to an available port, or scrub a manual override slider below to test downstream
              routing offline.
            </p>
            <button
              onClick={() =>
                void rebindOfflineDevice(
                  device.id,
                  device.fingerprint,
                  midiPorts[0]?.name ?? 'Kontrolfreak Loopback 1'
                )
              }
              className="w-full flex items-center justify-center gap-1.5 py-1.5 rounded bg-amber-500/25 hover:bg-amber-500/35 text-xs font-medium text-amber-100 border border-amber-400/40"
            >
              <Link2 className="w-3.5 h-3.5" />
              Rebind Device Online
            </button>
          </div>
        )}
      </div>

      {/* Ports & Manual Test Overrides */}
      <div className="space-y-2 pt-2 border-t border-studio-border">
        <div className="text-[11px] font-semibold uppercase tracking-wider text-studio-muted">
          Ports &amp; Live Test Override ({device.ports.length})
        </div>

        {device.ports.map((port) => {
          const sig = SIGNAL_COLORS[port.signal_kind];
          const override = device.manual_override[port.id];
          const sliderVal =
            override && override.type === 'Float' ? override.value : 0.5;
          return (
            <div
              key={port.id}
              className="p-2 rounded-lg bg-studio-panel border border-studio-border space-y-1.5"
            >
              <div className="flex items-center justify-between gap-2">
                <input
                  value={port.alias}
                  onChange={(e) => void renamePort(device.id, port.id, e.target.value)}
                  className="bg-transparent text-xs font-medium text-slate-100 outline-none border-b border-transparent focus:border-amber-500/60 flex-1 min-w-0"
                />
                <span
                  className={`text-[9px] font-mono px-1.5 py-0.5 rounded border ${sig.bgClass} ${sig.textClass} ${sig.borderClass}`}
                >
                  {port.direction.toUpperCase()}
                </span>
              </div>
              <div className="text-[10px] font-mono text-studio-muted">{port.logical_key}</div>

              {/* Manual test slider */}
              <div className="flex items-center gap-2 pt-0.5">
                <input
                  type="range"
                  min={0}
                  max={1}
                  step={0.01}
                  value={sliderVal}
                  onChange={(e) =>
                    void setManualOverride(device.id, port.id, {
                      type: 'Float',
                      value: Number(e.target.value),
                    })
                  }
                  className="flex-1 accent-amber-400 h-1.5 cursor-pointer"
                  aria-label={`Test override for ${port.alias}`}
                />
                <span className="text-[10px] font-mono text-amber-300 w-8 text-right">
                  {sliderVal.toFixed(2)}
                </span>
              </div>

              {parentRack && (
                <button
                  onClick={() =>
                    void exposeMacro(parentRack.id, `${port.alias} Macro`, [
                      {
                        port_id: port.id,
                        min: 0,
                        max: 1,
                        invert: false,
                        curve: { kind: 'linear' },
                      },
                    ])
                  }
                  className="text-[10px] text-violet-300 hover:text-violet-200 flex items-center gap-1 pt-0.5"
                >
                  <Sliders className="w-3 h-3" />
                  Expose as Macro on &quot;{parentRack.display_name}&quot;
                </button>
              )}
            </div>
          );
        })}

        {/* Add Port */}
        <div className="flex gap-1.5 pt-1">
          <input
            value={newPortKey}
            onChange={(e) => setNewPortKey(e.target.value)}
            placeholder="/osc/key or cc.1.1"
            className="flex-1 bg-studio-bg border border-studio-border rounded px-2 py-1 text-[11px] font-mono text-slate-200 outline-none"
          />
          <input
            value={newPortAlias}
            onChange={(e) => setNewPortAlias(e.target.value)}
            placeholder="Alias"
            className="w-20 bg-studio-bg border border-studio-border rounded px-2 py-1 text-[11px] text-slate-200 outline-none"
          />
          <button
            onClick={handleAddPort}
            className="px-2 py-1 rounded bg-studio-elevated hover:bg-slate-700 text-amber-300 border border-studio-border"
            title="Add parameter port"
          >
            <Plus className="w-3.5 h-3.5" />
          </button>
        </div>
      </div>

      {/* Progressive Disclosure: Advanced Protocol Config Accordion */}
      <div className="pt-2 border-t border-studio-border">
        <button
          onClick={() => setAdvancedOpen((v) => !v)}
          className="w-full flex items-center justify-between py-1 text-xs font-medium text-slate-300 hover:text-white"
        >
          <span>Advanced / Protocol Config</span>
          {advancedOpen ? (
            <ChevronDown className="w-4 h-4 text-studio-muted" />
          ) : (
            <ChevronRight className="w-4 h-4 text-studio-muted" />
          )}
        </button>

        {advancedOpen && (
          <div className="mt-2 p-2.5 rounded-lg bg-studio-bg border border-studio-border space-y-2 text-[11px]">
            <div className="font-mono text-[10px] text-studio-muted truncate">
              UUID: {device.id}
            </div>

            {device.kind.type === 'OscEndpoint' && (() => {
              const oscKind = device.kind;
              return (
                <>
                  <label className="block space-y-1">
                    <span className="text-studio-muted">Bind UDP Address</span>
                    <input
                      value={oscKind.bind_addr}
                      onChange={(e) =>
                        void updateDeviceKind(device.id, {
                          ...oscKind,
                          bind_addr: e.target.value,
                        })
                      }
                      className="w-full bg-studio-surface border border-studio-border rounded px-2 py-1 font-mono text-slate-200"
                    />
                  </label>
                  <label className="block space-y-1">
                    <span className="text-studio-muted">Target UDP Address</span>
                    <input
                      value={oscKind.target_addr ?? ''}
                      onChange={(e) =>
                        void updateDeviceKind(device.id, {
                          ...oscKind,
                          target_addr: e.target.value || null,
                        })
                      }
                      className="w-full bg-studio-surface border border-studio-border rounded px-2 py-1 font-mono text-slate-200"
                    />
                  </label>
                </>
              );
            })()}

            {device.kind.type === 'WebSocketEndpoint' && (() => {
              const wsKind = device.kind;
              return (
                <>
                  <label className="block space-y-1">
                    <span className="text-studio-muted">URL / Bind Address</span>
                    <input
                      value={wsKind.url_or_bind}
                      onChange={(e) =>
                        void updateDeviceKind(device.id, {
                          ...wsKind,
                          url_or_bind: e.target.value,
                        })
                      }
                      className="w-full bg-studio-surface border border-studio-border rounded px-2 py-1 font-mono text-slate-200"
                    />
                  </label>
                  <label className="block space-y-1">
                    <span className="text-studio-muted">Framing</span>
                    <select
                      value={wsKind.framing}
                      onChange={(e) =>
                        void updateDeviceKind(device.id, {
                          ...wsKind,
                          framing: e.target.value as 'json' | 'text' | 'binary' | 'osc_over_ws',
                        })
                      }
                      className="w-full bg-studio-surface border border-studio-border rounded px-2 py-1 text-slate-200"
                    >
                      <option value="json">JSON</option>
                      <option value="text">Text</option>
                      <option value="binary">Binary</option>
                      <option value="osc_over_ws">OSC-over-WS</option>
                    </select>
                  </label>
                </>
              );
            })()}

            {device.kind.type === 'UdpEndpoint' && (() => {
              const udpKind = device.kind;
              return (
                <>
                  <label className="block space-y-1">
                    <span className="text-studio-muted">UDP Format</span>
                    <select
                      value={udpKind.format}
                      onChange={(e) =>
                        void updateDeviceKind(device.id, {
                          ...udpKind,
                          format: e.target.value as 'utf8' | 'json' | 'csv' | 'hex',
                        })
                      }
                      className="w-full bg-studio-surface border border-studio-border rounded px-2 py-1 text-slate-200"
                    >
                      <option value="json">JSON</option>
                      <option value="utf8">UTF-8</option>
                      <option value="csv">CSV</option>
                      <option value="hex">Hex</option>
                    </select>
                  </label>
                  <label className="block space-y-1">
                    <span className="text-studio-muted">Target Address</span>
                    <input
                      value={udpKind.target_addr ?? ''}
                      onChange={(e) =>
                        void updateDeviceKind(device.id, {
                          ...udpKind,
                          target_addr: e.target.value || null,
                        })
                      }
                      className="w-full bg-studio-surface border border-studio-border rounded px-2 py-1 font-mono text-slate-200"
                    />
                  </label>
                </>
              );
            })()}

            {device.kind.type === 'MidiPort' && (() => {
              const midiKind = device.kind;
              return (
                <label className="block space-y-1">
                  <span className="text-studio-muted">MIDI Port Name</span>
                  <input
                    value={midiKind.port_name}
                    onChange={(e) =>
                      void updateDeviceKind(device.id, {
                        ...midiKind,
                        port_name: e.target.value,
                      })
                    }
                    className="w-full bg-studio-surface border border-studio-border rounded px-2 py-1 font-mono text-slate-200"
                  />
                </label>
              );
            })()}
          </div>
        )}
      </div>

      {/* M2/M3/M4 Registered Inspector Extensions */}
      {extensionPanels.map((panel) => {
        const PanelComp = panel.component;
        return (
          <div key={panel.id} className="pt-2 border-t border-studio-border">
            <div className="text-[11px] font-semibold uppercase tracking-wider text-studio-muted mb-2">
              {panel.title}
            </div>
            <PanelComp device={device} />
          </div>
        );
      })}

      {/* Actions */}
      <div className="pt-2 border-t border-studio-border flex gap-2">
        <button
          onClick={() => void handleExportDevice()}
          className="flex-1 flex items-center justify-center gap-1.5 py-1.5 rounded-md bg-studio-panel hover:bg-studio-elevated border border-studio-border text-xs text-slate-200"
        >
          <Download className="w-3.5 h-3.5 text-emerald-400" />
          Export Preset
        </button>
        <button
          onClick={() => void removeDevice(device.id)}
          className="px-2.5 py-1.5 rounded-md bg-rose-950/50 hover:bg-rose-900/60 border border-rose-500/40 text-rose-300"
          title="Delete node"
        >
          <Trash2 className="w-3.5 h-3.5" />
        </button>
      </div>
    </div>
  );
};

const WireInspector: React.FC<{ wire: Wire }> = ({ wire }) => {
  const graph = useWorkspaceStore((s) => s.graph);
  const toggleWire = useWorkspaceStore((s) => s.toggleWire);
  const removeWire = useWorkspaceStore((s) => s.removeWire);
  const updateWireModifiers = useWorkspaceStore((s) => s.updateWireModifiers);
  const pushToast = useWorkspaceStore((s) => s.pushToast);

  const [addTypeIdx, setAddTypeIdx] = useState(0);

  const findPortLabel = (pid: string) => {
    for (const d of graph.devices) {
      const p = d.ports.find((x) => x.id === pid);
      if (p) return `${d.display_name} · ${p.alias}`;
    }
    return pid.slice(0, 8);
  };

  const updateModifierAt = (idx: number, next: Modifier) => {
    const updated = wire.modifiers.map((m, i) => (i === idx ? next : m));
    void updateWireModifiers(wire.id, updated);
  };

  const removeModifierAt = (idx: number) => {
    const updated = wire.modifiers.filter((_, i) => i !== idx);
    void updateWireModifiers(wire.id, updated);
  };

  const handleAddModifier = () => {
    const tpl = MODIFIER_TEMPLATES[addTypeIdx];
    if (!tpl) return;
    void updateWireModifiers(wire.id, [...wire.modifiers, tpl.create()]);
  };

  const handleExportMapping = async () => {
    const json = await bridge.exportMappingPreset('Wire Mapping', null, wire.modifiers);
    const blob = new Blob([json], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = 'wire-mapping.kfmapping.json';
    a.click();
    URL.revokeObjectURL(url);
    pushToast({ title: 'Exported .kfmapping.json', accent: 'amber' });
  };

  return (
    <div className="space-y-4">
      <div className="space-y-1">
        <div className="flex items-center justify-between">
          <span className="text-[10px] font-mono uppercase text-amber-400">Signal Wire</span>
          <label className="flex items-center gap-1.5 text-xs text-slate-200 cursor-pointer">
            <input
              type="checkbox"
              checked={wire.enabled}
              onChange={(e) => void toggleWire(wire.id, e.target.checked)}
              className="accent-amber-400"
            />
            Active
          </label>
        </div>
        <div className="text-xs text-slate-200 leading-snug">
          <div>{findPortLabel(wire.from_port_id)}</div>
          <div className="text-studio-muted">↓</div>
          <div>{findPortLabel(wire.to_port_id)}</div>
        </div>
      </div>

      {/* Interactive Transfer Curve */}
      <div className="space-y-1.5">
        <div className="text-[11px] font-semibold uppercase tracking-wider text-studio-muted">
          Transfer Curve (Live)
        </div>
        <TransferCurveCanvas
          modifiers={wire.modifiers}
          sourcePortId={wire.from_port_id}
          targetPortId={wire.to_port_id}
          calmMode={graph.settings.calm_mode}
        />
      </div>

      {/* Modifier Chain */}
      <div className="space-y-2 pt-2 border-t border-studio-border">
        <div className="text-[11px] font-semibold uppercase tracking-wider text-studio-muted">
          Modifier Stack ({wire.modifiers.length})
        </div>

        {wire.modifiers.map((mod, idx) => (
          <div
            key={`${mod.type}-${idx}`}
            className="p-2 rounded-lg bg-studio-panel border border-studio-border space-y-2 text-[11px]"
          >
            <div className="flex items-center justify-between">
              <span className="font-semibold text-amber-300">
                {idx + 1}. {mod.type}
              </span>
              <button
                onClick={() => removeModifierAt(idx)}
                className="text-slate-400 hover:text-rose-400"
                title="Remove modifier"
              >
                <Trash2 className="w-3.5 h-3.5" />
              </button>
            </div>

            {mod.type === 'RangeMap' && (
              <div className="grid grid-cols-2 gap-1.5">
                {(['in_min', 'in_max', 'out_min', 'out_max'] as const).map((field) => (
                  <label key={field} className="space-y-0.5">
                    <span className="text-[10px] text-studio-muted">{field}</span>
                    <input
                      type="number"
                      step={0.1}
                      value={mod[field]}
                      onChange={(e) =>
                        updateModifierAt(idx, { ...mod, [field]: Number(e.target.value) })
                      }
                      className="w-full bg-studio-bg border border-studio-border rounded px-1.5 py-0.5 font-mono text-slate-200"
                    />
                  </label>
                ))}
              </div>
            )}

            {mod.type === 'Clamp' && (
              <div className="grid grid-cols-2 gap-1.5">
                <label className="space-y-0.5">
                  <span className="text-[10px] text-studio-muted">min</span>
                  <input
                    type="number"
                    step={0.05}
                    value={mod.min}
                    onChange={(e) => updateModifierAt(idx, { ...mod, min: Number(e.target.value) })}
                    className="w-full bg-studio-bg border border-studio-border rounded px-1.5 py-0.5 font-mono"
                  />
                </label>
                <label className="space-y-0.5">
                  <span className="text-[10px] text-studio-muted">max</span>
                  <input
                    type="number"
                    step={0.05}
                    value={mod.max}
                    onChange={(e) => updateModifierAt(idx, { ...mod, max: Number(e.target.value) })}
                    className="w-full bg-studio-bg border border-studio-border rounded px-1.5 py-0.5 font-mono"
                  />
                </label>
              </div>
            )}

            {mod.type === 'Deadzone' && (
              <div className="grid grid-cols-2 gap-1.5">
                <label className="space-y-0.5">
                  <span className="text-[10px] text-studio-muted">center</span>
                  <input
                    type="number"
                    step={0.05}
                    value={mod.center}
                    onChange={(e) =>
                      updateModifierAt(idx, { ...mod, center: Number(e.target.value) })
                    }
                    className="w-full bg-studio-bg border border-studio-border rounded px-1.5 py-0.5 font-mono"
                  />
                </label>
                <label className="space-y-0.5">
                  <span className="text-[10px] text-studio-muted">radius</span>
                  <input
                    type="number"
                    step={0.02}
                    value={mod.radius}
                    onChange={(e) =>
                      updateModifierAt(idx, { ...mod, radius: Number(e.target.value) })
                    }
                    className="w-full bg-studio-bg border border-studio-border rounded px-1.5 py-0.5 font-mono"
                  />
                </label>
              </div>
            )}

            {mod.type === 'Curve' && (
              <div className="space-y-1.5">
                <select
                  value={mod.kind.kind}
                  onChange={(e) => {
                    const k = e.target.value;
                    let nextKind: CurveKind = { kind: 'linear' };
                    if (k === 'exponential') nextKind = { kind: 'exponential', exponent: 2.2 };
                    if (k === 'logarithmic') nextKind = { kind: 'logarithmic', base: 10 };
                    if (k === 's_curve') nextKind = { kind: 's_curve', steepness: 5 };
                    if (k === 'bezier')
                      nextKind = { kind: 'bezier', p1: [0.25, 0.85], p2: [0.75, 0.15] };
                    updateModifierAt(idx, { type: 'Curve', kind: nextKind });
                  }}
                  className="w-full bg-studio-bg border border-studio-border rounded px-2 py-1 text-slate-200"
                >
                  <option value="linear">Linear</option>
                  <option value="exponential">Exponential</option>
                  <option value="logarithmic">Logarithmic</option>
                  <option value="s_curve">S-Curve</option>
                  <option value="bezier">Cubic Bezier</option>
                </select>
                {mod.kind.kind === 'exponential' && (
                  <input
                    type="range"
                    min={0.2}
                    max={5}
                    step={0.1}
                    value={mod.kind.exponent}
                    onChange={(e) =>
                      updateModifierAt(idx, {
                        type: 'Curve',
                        kind: { kind: 'exponential', exponent: Number(e.target.value) },
                      })
                    }
                    className="w-full accent-amber-400"
                  />
                )}
                {mod.kind.kind === 's_curve' && (
                  <input
                    type="range"
                    min={1}
                    max={14}
                    step={0.5}
                    value={mod.kind.steepness}
                    onChange={(e) =>
                      updateModifierAt(idx, {
                        type: 'Curve',
                        kind: { kind: 's_curve', steepness: Number(e.target.value) },
                      })
                    }
                    className="w-full accent-amber-400"
                  />
                )}
              </div>
            )}

            {mod.type === 'SmoothOneEuro' && (
              <div className="grid grid-cols-2 gap-1.5">
                <label className="space-y-0.5">
                  <span className="text-[10px] text-studio-muted">min_cutoff</span>
                  <input
                    type="number"
                    step={0.1}
                    value={mod.min_cutoff}
                    onChange={(e) =>
                      updateModifierAt(idx, { ...mod, min_cutoff: Number(e.target.value) })
                    }
                    className="w-full bg-studio-bg border border-studio-border rounded px-1.5 py-0.5 font-mono"
                  />
                </label>
                <label className="space-y-0.5">
                  <span className="text-[10px] text-studio-muted">beta</span>
                  <input
                    type="number"
                    step={0.01}
                    value={mod.beta}
                    onChange={(e) =>
                      updateModifierAt(idx, { ...mod, beta: Number(e.target.value) })
                    }
                    className="w-full bg-studio-bg border border-studio-border rounded px-1.5 py-0.5 font-mono"
                  />
                </label>
              </div>
            )}

            {mod.type === 'Hysteresis' && (
              <div className="grid grid-cols-2 gap-1.5">
                <label className="space-y-0.5">
                  <span className="text-[10px] text-studio-muted">low</span>
                  <input
                    type="number"
                    step={0.05}
                    value={mod.low_threshold}
                    onChange={(e) =>
                      updateModifierAt(idx, { ...mod, low_threshold: Number(e.target.value) })
                    }
                    className="w-full bg-studio-bg border border-studio-border rounded px-1.5 py-0.5 font-mono"
                  />
                </label>
                <label className="space-y-0.5">
                  <span className="text-[10px] text-studio-muted">high</span>
                  <input
                    type="number"
                    step={0.05}
                    value={mod.high_threshold}
                    onChange={(e) =>
                      updateModifierAt(idx, { ...mod, high_threshold: Number(e.target.value) })
                    }
                    className="w-full bg-studio-bg border border-studio-border rounded px-1.5 py-0.5 font-mono"
                  />
                </label>
              </div>
            )}

            {mod.type === 'Threshold' && (
              <label className="block space-y-0.5">
                <span className="text-[10px] text-studio-muted">threshold</span>
                <input
                  type="number"
                  step={0.05}
                  value={mod.threshold}
                  onChange={(e) =>
                    updateModifierAt(idx, { ...mod, threshold: Number(e.target.value) })
                  }
                  className="w-full bg-studio-bg border border-studio-border rounded px-1.5 py-0.5 font-mono"
                />
              </label>
            )}

            {mod.type === 'Expression' && (
              <label className="block space-y-0.5">
                <span className="text-[10px] text-studio-muted">expr (use x)</span>
                <input
                  value={mod.expr}
                  onChange={(e) => updateModifierAt(idx, { type: 'Expression', expr: e.target.value })}
                  className="w-full bg-studio-bg border border-studio-border rounded px-1.5 py-1 font-mono text-amber-200"
                />
              </label>
            )}
          </div>
        ))}

        {/* Add Modifier Row */}
        <div className="flex gap-1.5">
          <select
            value={addTypeIdx}
            onChange={(e) => setAddTypeIdx(Number(e.target.value))}
            className="flex-1 bg-studio-bg border border-studio-border rounded-md px-2 py-1.5 text-xs text-slate-200 outline-none"
          >
            {MODIFIER_TEMPLATES.map((tpl, i) => (
              <option key={tpl.label} value={i}>
                {tpl.label}
              </option>
            ))}
          </select>
          <button
            onClick={handleAddModifier}
            className="px-2.5 py-1.5 rounded-md bg-amber-500/20 hover:bg-amber-500/30 border border-amber-500/40 text-amber-200 text-xs font-medium"
          >
            Add
          </button>
        </div>
      </div>

      {/* Wire actions */}
      <div className="pt-2 border-t border-studio-border flex gap-2">
        <button
          onClick={() => void handleExportMapping()}
          className="flex-1 flex items-center justify-center gap-1.5 py-1.5 rounded-md bg-studio-panel hover:bg-studio-elevated border border-studio-border text-xs text-slate-200"
        >
          <Download className="w-3.5 h-3.5 text-amber-400" />
          Save Mapping
        </button>
        <button
          onClick={() => void removeWire(wire.id)}
          className="px-2.5 py-1.5 rounded-md bg-rose-950/50 hover:bg-rose-900/60 border border-rose-500/40 text-rose-300"
          title="Delete wire"
        >
          <Trash2 className="w-3.5 h-3.5" />
        </button>
      </div>
    </div>
  );
};

const RackInspector: React.FC<{ rack: RackGroup }> = ({ rack }) => {
  const graph = useWorkspaceStore((s) => s.graph);
  const renameRack = useWorkspaceStore((s) => s.renameRack);
  const setRackCollapsed = useWorkspaceStore((s) => s.setRackCollapsed);
  const ungroupRack = useWorkspaceStore((s) => s.ungroupRack);
  const exposeMacro = useWorkspaceStore((s) => s.exposeMacro);
  const removeMacro = useWorkspaceStore((s) => s.removeMacro);
  const setMacroValue = useWorkspaceStore((s) => s.setMacroValue);
  const pushToast = useWorkspaceStore((s) => s.pushToast);

  const memberDevices = graph.devices.filter((d) => rack.member_node_ids.includes(d.id));
  const internalPorts = memberDevices.flatMap((d) =>
    d.ports.map((p) => ({ device: d, port: p }))
  );

  const [targetPortId, setTargetPortId] = useState(internalPorts[0]?.port.id ?? '');
  const [macroName, setMacroName] = useState('Master Macro');

  const handleAddMacro = () => {
    const pid = targetPortId || internalPorts[0]?.port.id;
    if (!pid) return;
    void exposeMacro(rack.id, macroName.trim() || 'Macro', [
      { port_id: pid, min: 0, max: 1, invert: false, curve: { kind: 'linear' } },
    ]);
  };

  const handleExportRack = async () => {
    const json = await bridge.exportGroupPreset(rack.id);
    const blob = new Blob([json], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `${rack.display_name.toLowerCase().replace(/[^a-z0-9]+/g, '-')}.kfgroup.json`;
    a.click();
    URL.revokeObjectURL(url);
    pushToast({ title: 'Exported .kfgroup.json', accent: 'violet' });
  };

  return (
    <div className="space-y-4">
      <div className="space-y-2">
        <div className="flex items-center justify-between">
          <span className="flex items-center gap-1.5 text-[10px] font-mono uppercase text-violet-300">
            <Layers className="w-3.5 h-3.5" />
            Collapsible Rack
          </span>
          <button
            onClick={() => void setRackCollapsed(rack.id, !rack.collapsed)}
            className="text-xs text-violet-300 hover:text-violet-100 underline"
          >
            {rack.collapsed ? 'Expand' : 'Collapse'}
          </button>
        </div>

        <input
          value={rack.display_name}
          onChange={(e) => void renameRack(rack.id, e.target.value)}
          className="w-full bg-studio-bg border border-studio-border rounded-md px-2.5 py-1.5 text-xs text-white focus:border-violet-400 outline-none"
        />
      </div>

      {/* Macros */}
      <div className="space-y-2 pt-2 border-t border-studio-border">
        <div className="text-[11px] font-semibold uppercase tracking-wider text-studio-muted">
          Exposed Macros ({rack.macros.length})
        </div>

        {rack.macros.map((m) => (
          <div
            key={m.id}
            className="p-2 rounded-lg bg-studio-panel border border-studio-border space-y-1.5"
          >
            <div className="flex items-center justify-between">
              <span className="text-xs font-medium text-slate-100">{m.alias}</span>
              <button
                onClick={() => void removeMacro(rack.id, m.id)}
                className="text-slate-400 hover:text-rose-400"
              >
                <Trash2 className="w-3.5 h-3.5" />
              </button>
            </div>
            <input
              type="range"
              min={0}
              max={1}
              step={0.01}
              value={m.value}
              onChange={(e) => void setMacroValue(rack.id, m.id, Number(e.target.value))}
              className="w-full accent-amber-400"
            />
            <div className="text-[10px] text-studio-muted">
              Bound to {m.targets.length} internal target(s) · Value {m.value.toFixed(2)}
            </div>
          </div>
        ))}

        {internalPorts.length > 0 && (
          <div className="space-y-1.5 p-2 rounded-lg bg-studio-bg border border-studio-border">
            <input
              value={macroName}
              onChange={(e) => setMacroName(e.target.value)}
              placeholder="Macro name"
              className="w-full bg-studio-surface border border-studio-border rounded px-2 py-1 text-xs text-slate-200 outline-none"
            />
            <select
              value={targetPortId || internalPorts[0].port.id}
              onChange={(e) => setTargetPortId(e.target.value)}
              className="w-full bg-studio-surface border border-studio-border rounded px-2 py-1 text-xs text-slate-200 outline-none"
            >
              {internalPorts.map(({ device, port }) => (
                <option key={port.id} value={port.id}>
                  {device.display_name} · {port.alias}
                </option>
              ))}
            </select>
            <button
              onClick={handleAddMacro}
              className="w-full flex items-center justify-center gap-1 py-1.5 rounded bg-violet-500/20 hover:bg-violet-500/30 border border-violet-500/40 text-xs text-violet-200 font-medium"
            >
              <Sliders className="w-3.5 h-3.5" />
              Expose Macro
            </button>
          </div>
        )}
      </div>

      <div className="pt-2 border-t border-studio-border flex gap-2">
        <button
          onClick={() => void handleExportRack()}
          className="flex-1 flex items-center justify-center gap-1.5 py-1.5 rounded-md bg-studio-panel hover:bg-studio-elevated border border-studio-border text-xs text-slate-200"
        >
          <Download className="w-3.5 h-3.5 text-violet-400" />
          Export Rack
        </button>
        <button
          onClick={() => void ungroupRack(rack.id)}
          className="px-2.5 py-1.5 rounded-md bg-studio-panel hover:bg-studio-elevated border border-studio-border text-xs text-slate-300"
        >
          Ungroup
        </button>
      </div>
    </div>
  );
};

export const RightInspector: React.FC = () => {
  const graph = useWorkspaceStore((s) => s.graph);
  const selection = useWorkspaceStore((s) => s.selection);

  const selectedDevice =
    selection.kind === 'device'
      ? graph.devices.find((d) => d.id === selection.id)
      : undefined;
  const selectedWire =
    selection.kind === 'wire'
      ? graph.wires.find((w) => w.id === selection.id)
      : undefined;
  const selectedRack =
    selection.kind === 'rack'
      ? graph.groups.find((g) => g.id === selection.id)
      : undefined;

  return (
    <aside className="w-80 shrink-0 bg-studio-surface border-l border-studio-border flex flex-col min-h-0">
      <div className="px-3 py-2 border-b border-studio-border flex items-center justify-between bg-studio-bg/50">
        <span className="text-xs font-semibold tracking-wide text-slate-200">Inspector</span>
        <span className="text-[10px] font-mono text-studio-muted uppercase">
          {selection.kind === 'none' ? 'Idle' : selection.kind}
        </span>
      </div>

      <div className="flex-1 overflow-y-auto p-3">
        {selectedDevice && <DeviceInspector device={selectedDevice} />}
        {selectedWire && <WireInspector wire={selectedWire} />}
        {selectedRack && <RackInspector rack={selectedRack} />}
        {!selectedDevice && !selectedWire && !selectedRack && (
          <div className="h-full flex flex-col items-center justify-center text-center px-4 space-y-2 text-studio-muted">
            <Sliders className="w-7 h-7 opacity-40" />
            <div className="text-xs font-medium text-slate-300">Single-Focus Inspector</div>
            <p className="text-[11px] leading-relaxed">
              Select a node, signal wire, or collapsible rack on the canvas to edit aliases, tune
              modifier transfer curves, or expose macros.
            </p>
          </div>
        )}
      </div>
    </aside>
  );
};
