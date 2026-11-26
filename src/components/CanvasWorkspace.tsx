import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  Background,
  BackgroundVariant,
  BaseEdge,
  Controls,
  EdgeLabelRenderer,
  getBezierPath,
  Handle,
  MiniMap,
  Position,
  ReactFlow,
  ReactFlowProvider,
  useReactFlow,
  type Connection,
  type Edge,
  type EdgeProps,
  type Node,
  type NodeProps,
} from '@xyflow/react';
import '@xyflow/react/dist/style.css';
import { ChevronDown, ChevronRight, Layers, Link2, Sliders, Unplug } from 'lucide-react';
import { useWorkspaceStore } from '../store/workspaceStore';
import { getNodeDefinition } from '../registries';
import { areSignalKindsCompatible, SIGNAL_COLORS } from '../lib/signalColors';
import { telemetryStore } from '../lib/telemetryStore';
import type { Device, DeviceKind, Port, RackGroup, SignalKind, Wire } from '../types/kontrolfreak';

/** Tiny zero-re-render HTML5 <canvas> horizontal value bar for a single port. */
const PortMeterCanvas: React.FC<{
  portId: string;
  colorHex: string;
  calmMode: boolean;
}> = ({ portId, colorHex, calmMode }) => {
  const ref = useRef<HTMLCanvasElement | null>(null);

  useEffect(() => {
    const canvas = ref.current;
    if (!canvas) return;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const draw = () => {
      const w = canvas.width;
      const h = canvas.height;
      ctx.clearRect(0, 0, w, h);
      ctx.fillStyle = '#0b0e14';
      ctx.fillRect(0, 0, w, h);

      const entry = telemetryStore.getPortLatest(portId);
      const val =
        entry && typeof entry.numeric_value === 'number'
          ? Math.min(1, Math.max(0, entry.numeric_value))
          : 0;
      ctx.globalAlpha = calmMode ? 0.55 : entry?.active ? 1.0 : 0.65;
      ctx.fillStyle = colorHex;
      ctx.fillRect(0, 0, Math.round(w * val), h);
      ctx.globalAlpha = 1;
    };

    draw();
    return telemetryStore.subscribe(() => draw());
  }, [portId, colorHex, calmMode]);

  return (
    <canvas
      ref={ref}
      width={48}
      height={6}
      className="w-12 h-1.5 rounded-full border border-studio-border shrink-0"
    />
  );
};

interface DeviceNodeData extends Record<string, unknown> {
  device: Device;
  calmMode: boolean;
  connectingFromKind: SignalKind | null;
}

const DeviceNodeComponent: React.FC<NodeProps<Node<DeviceNodeData>>> = ({ data, selected }) => {
  const { device, calmMode, connectingFromKind } = data;
  const renameDevice = useWorkspaceStore((s) => s.renameDevice);
  const rebindOfflineDevice = useWorkspaceStore((s) => s.rebindOfflineDevice);
  const midiPorts = useWorkspaceStore((s) => s.midiPorts);

  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(device.display_name);

  useEffect(() => {
    setDraft(device.display_name);
  }, [device.display_name]);

  const def = getNodeDefinition(device.kind.type);
  const badgeColor = SIGNAL_COLORS[def?.primarySignal ?? 'control'];
  const isOffline = device.status.state === 'offline_ghost';
  const CustomBody = def?.renderBody;

  const commitName = () => {
    const trimmed = draft.trim();
    if (trimmed && trimmed !== device.display_name) {
      void renameDevice(device.id, trimmed);
    } else {
      setDraft(device.display_name);
    }
    setEditing(false);
  };

  return (
    <div
      className={`min-w-[235px] max-w-[270px] rounded-xl border transition-all shadow-lg ${
        isOffline
          ? 'bg-studio-surface/75 border-dashed border-amber-500/60 opacity-85'
          : 'bg-studio-surface border-studio-border'
      } ${selected ? 'ring-2 ring-amber-400/70 border-amber-400/60' : ''}`}
    >
      {/* Header */}
      <div className="px-3 py-2 border-b border-studio-border flex items-center justify-between gap-2 bg-studio-panel/80 rounded-t-xl">
        <div className="flex items-center gap-2 min-w-0 flex-1">
          <span
            className={`text-[10px] font-mono font-bold px-1.5 py-0.5 rounded border shrink-0 ${badgeColor.bgClass} ${badgeColor.textClass} ${badgeColor.borderClass}`}
          >
            {def?.shortBadge ?? 'NODE'}
          </span>

          {editing ? (
            <input
              autoFocus
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              onBlur={commitName}
              onKeyDown={(e) => {
                if (e.key === 'Enter') commitName();
                if (e.key === 'Escape') {
                  setDraft(device.display_name);
                  setEditing(false);
                }
              }}
              onClick={(e) => e.stopPropagation()}
              className="nodrag bg-studio-bg border border-amber-500/70 rounded px-1.5 py-0.5 text-xs text-white w-full outline-none"
            />
          ) : (
            <button
              onClick={(e) => {
                e.stopPropagation();
                setEditing(true);
              }}
              title="Click to rename alias (never breaks wires)"
              className="nodrag text-xs font-semibold text-slate-100 hover:text-amber-300 truncate text-left"
            >
              {device.display_name}
            </button>
          )}
        </div>

        <span
          className={`w-2 h-2 rounded-full shrink-0 ${
            isOffline ? 'bg-amber-400/70' : 'bg-emerald-400'
          }`}
          title={isOffline ? 'Offline Ghost — wires preserved' : 'Online'}
        />
      </div>

      {/* Offline Ghost Banner */}
      {isOffline && (
        <div className="px-3 py-1.5 bg-amber-950/40 border-b border-amber-500/30 flex items-center justify-between gap-2">
          <span className="flex items-center gap-1 text-[10px] text-amber-200 truncate">
            <Unplug className="w-3 h-3 shrink-0" />
            Offline Ghost
          </span>
          <button
            onClick={(e) => {
              e.stopPropagation();
              void rebindOfflineDevice(
                device.id,
                device.fingerprint,
                midiPorts[0]?.name ?? 'Kontrolfreak Loopback 1'
              );
            }}
            className="nodrag flex items-center gap-1 px-1.5 py-0.5 rounded bg-amber-500/20 hover:bg-amber-500/35 text-[10px] font-medium text-amber-200 border border-amber-400/40"
          >
            <Link2 className="w-2.5 h-2.5" />
            Rebind
          </button>
        </div>
      )}

      {/* Optional M2/M3/M4 Custom Body Extension */}
      {CustomBody && (
        <div className="px-3 py-2 border-b border-studio-border">
          <CustomBody device={device} calmMode={calmMode} />
        </div>
      )}

      {/* Ports */}
      <div className="py-1.5 divide-y divide-studio-border/40">
        {device.ports.map((port) => {
          const sig = SIGNAL_COLORS[port.signal_kind];
          const canIn = port.direction === 'in' || port.direction === 'bidir';
          const canOut = port.direction === 'out' || port.direction === 'bidir';
          const isCompatibleHighlight =
            connectingFromKind !== null &&
            canIn &&
            areSignalKindsCompatible(connectingFromKind, port.signal_kind);

          return (
            <div
              key={port.id}
              className={`relative px-3 py-1.5 flex items-center justify-between gap-2 text-xs transition-colors ${
                isCompatibleHighlight ? 'bg-amber-500/10' : ''
              }`}
            >
              {canIn && (
                <Handle
                  type="target"
                  position={Position.Left}
                  id={`in:${port.id}`}
                  style={{
                    background: sig.hex,
                    width: isCompatibleHighlight ? 13 : 10,
                    height: isCompatibleHighlight ? 13 : 10,
                    border: '2px solid #0b0e14',
                    boxShadow: isCompatibleHighlight ? `0 0 8px ${sig.hex}` : undefined,
                  }}
                />
              )}

              <div className="min-w-0 flex-1 pl-1">
                <div className="text-[11px] text-slate-200 truncate">{port.alias}</div>
                <div className="text-[9px] font-mono text-studio-muted truncate">
                  {port.logical_key}
                </div>
              </div>

              <PortMeterCanvas portId={port.id} colorHex={sig.hex} calmMode={calmMode} />

              {canOut && (
                <Handle
                  type="source"
                  position={Position.Right}
                  id={`out:${port.id}`}
                  style={{
                    background: sig.hex,
                    width: 10,
                    height: 10,
                    border: '2px solid #0b0e14',
                  }}
                />
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
};

interface RackNodeData extends Record<string, unknown> {
  rack: RackGroup;
  memberDevices: Device[];
  exposedInPorts: Port[];
  exposedOutPorts: Port[];
}

const RackNodeComponent: React.FC<NodeProps<Node<RackNodeData>>> = ({ data, selected }) => {
  const { rack, memberDevices, exposedInPorts, exposedOutPorts } = data;
  const setRackCollapsed = useWorkspaceStore((s) => s.setRackCollapsed);
  const renameRack = useWorkspaceStore((s) => s.renameRack);
  const setMacroValue = useWorkspaceStore((s) => s.setMacroValue);

  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(rack.display_name);

  useEffect(() => {
    setDraft(rack.display_name);
  }, [rack.display_name]);

  const commitName = () => {
    const trimmed = draft.trim();
    if (trimmed && trimmed !== rack.display_name) {
      void renameRack(rack.id, trimmed);
    } else {
      setDraft(rack.display_name);
    }
    setEditing(false);
  };

  return (
    <div
      className={`rounded-xl border transition-all ${
        rack.collapsed
          ? 'min-w-[270px] bg-studio-panel border-violet-500/50 shadow-xl'
          : 'min-w-[300px] bg-violet-950/15 border-violet-500/40 backdrop-blur-[1px]'
      } ${selected ? 'ring-2 ring-violet-400/70' : ''}`}
    >
      {/* Rack Header */}
      <div className="px-3 py-2 border-b border-violet-500/30 bg-violet-950/35 rounded-t-xl flex items-center justify-between gap-2">
        <div className="flex items-center gap-1.5 min-w-0">
          <button
            onClick={(e) => {
              e.stopPropagation();
              void setRackCollapsed(rack.id, !rack.collapsed);
            }}
            title={rack.collapsed ? 'Expand Rack' : 'Collapse Rack'}
            className="nodrag p-0.5 rounded hover:bg-violet-500/20 text-violet-300"
          >
            {rack.collapsed ? (
              <ChevronRight className="w-4 h-4" />
            ) : (
              <ChevronDown className="w-4 h-4" />
            )}
          </button>
          <Layers className="w-3.5 h-3.5 text-violet-400 shrink-0" />
          {editing ? (
            <input
              autoFocus
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              onBlur={commitName}
              onKeyDown={(e) => {
                if (e.key === 'Enter') commitName();
                if (e.key === 'Escape') {
                  setDraft(rack.display_name);
                  setEditing(false);
                }
              }}
              onClick={(e) => e.stopPropagation()}
              className="nodrag bg-studio-bg border border-violet-400 rounded px-1.5 py-0.5 text-xs text-white outline-none w-36"
            />
          ) : (
            <button
              onClick={(e) => {
                e.stopPropagation();
                setEditing(true);
              }}
              className="nodrag text-xs font-semibold text-violet-100 hover:text-white truncate"
            >
              {rack.display_name}
            </button>
          )}
        </div>

        <span className="text-[10px] font-mono text-violet-300 shrink-0">
          {memberDevices.length} node{memberDevices.length === 1 ? '' : 's'}
        </span>
      </div>

      {/* Exposed Macro Knobs / Sliders */}
      {rack.macros.length > 0 && (
        <div className="px-3 py-2 space-y-2 border-b border-violet-500/20 bg-studio-surface/70">
          <div className="flex items-center gap-1 text-[10px] font-semibold uppercase tracking-wider text-violet-300">
            <Sliders className="w-3 h-3" />
            Macros
          </div>
          {rack.macros.map((m) => (
            <div key={m.id} className="space-y-0.5">
              <div className="flex items-center justify-between text-[11px]">
                <span className="text-slate-200 truncate">{m.alias}</span>
                <span className="font-mono text-amber-300">{m.value.toFixed(2)}</span>
              </div>
              <input
                type="range"
                min={0}
                max={1}
                step={0.01}
                value={m.value}
                onChange={(e) => {
                  e.stopPropagation();
                  void setMacroValue(rack.id, m.id, Number(e.target.value));
                }}
                onMouseDown={(e) => e.stopPropagation()}
                className="nodrag w-full accent-amber-400 h-1.5 bg-studio-bg rounded-lg cursor-pointer"
              />
            </div>
          ))}
        </div>
      )}

      {/* Collapsed summary & external handles */}
      {rack.collapsed ? (
        <div className="p-2.5 space-y-1.5 relative">
          <div className="text-[10px] text-studio-muted">
            Collapsed — internal routing &amp; macros active
          </div>
          <div className="flex justify-between gap-4">
            <div className="space-y-1">
              {exposedInPorts.map((p) => {
                const sig = SIGNAL_COLORS[p.signal_kind];
                return (
                  <div key={p.id} className="relative flex items-center text-[10px] text-slate-300 pl-2">
                    <Handle
                      type="target"
                      position={Position.Left}
                      id={`in:${p.id}`}
                      style={{
                        background: sig.hex,
                        width: 10,
                        height: 10,
                        left: -14,
                        border: '2px solid #0b0e14',
                      }}
                    />
                    <span className="truncate max-w-[100px]">{p.alias}</span>
                  </div>
                );
              })}
            </div>
            <div className="space-y-1 text-right">
              {exposedOutPorts.map((p) => {
                const sig = SIGNAL_COLORS[p.signal_kind];
                return (
                  <div
                    key={p.id}
                    className="relative flex items-center justify-end text-[10px] text-slate-300 pr-2"
                  >
                    <span className="truncate max-w-[100px]">{p.alias}</span>
                    <Handle
                      type="source"
                      position={Position.Right}
                      id={`out:${p.id}`}
                      style={{
                        background: sig.hex,
                        width: 10,
                        height: 10,
                        right: -14,
                        border: '2px solid #0b0e14',
                      }}
                    />
                  </div>
                );
              })}
            </div>
          </div>
        </div>
      ) : (
        <div className="px-3 py-2 text-[10px] text-violet-300/70">
          Expanded Rack · Members: {memberDevices.map((d) => d.display_name).join(', ')}
        </div>
      )}
    </div>
  );
};

interface SignalEdgeData extends Record<string, unknown> {
  wire: Wire;
  colorHex: string;
  calmMode: boolean;
}

const SignalEdgeComponent: React.FC<EdgeProps<Edge<SignalEdgeData>>> = ({
  id,
  sourceX,
  sourceY,
  targetX,
  targetY,
  sourcePosition,
  targetPosition,
  selected,
  data,
}) => {
  const [edgePath, labelX, labelY] = getBezierPath({
    sourceX,
    sourceY,
    sourcePosition,
    targetX,
    targetY,
    targetPosition,
  });

  const wire = data?.wire;
  const colorHex = data?.colorHex ?? '#f59e0b';
  const calmMode = data?.calmMode ?? false;
  const enabled = wire?.enabled ?? true;
  const modCount = wire?.modifiers.length ?? 0;

  return (
    <>
      <BaseEdge
        id={id}
        path={edgePath}
        className={enabled && !calmMode ? 'kf-edge-flowing' : undefined}
        style={{
          stroke: enabled ? colorHex : '#475569',
          strokeWidth: selected ? 3 : 2,
          opacity: enabled ? (calmMode ? 0.72 : 0.95) : 0.4,
        }}
      />
      {modCount > 0 && (
        <EdgeLabelRenderer>
          <div
            style={{
              position: 'absolute',
              transform: `translate(-50%, -50%) translate(${labelX}px,${labelY}px)`,
              pointerEvents: 'all',
            }}
            className={`px-1.5 py-0.5 rounded-full text-[9px] font-mono border ${
              selected
                ? 'bg-amber-500 text-slate-950 border-amber-300 font-bold'
                : 'bg-studio-panel text-amber-300 border-studio-border'
            }`}
          >
            {modCount} mod{modCount === 1 ? '' : 's'}
          </div>
        </EdgeLabelRenderer>
      )}
    </>
  );
};

const nodeTypes = {
  kfDevice: DeviceNodeComponent,
  kfRack: RackNodeComponent,
};

const edgeTypes = {
  kfSignal: SignalEdgeComponent,
};

const CanvasInner: React.FC = () => {
  const graph = useWorkspaceStore((s) => s.graph);
  const selection = useWorkspaceStore((s) => s.selection);
  const addDevice = useWorkspaceStore((s) => s.addDevice);
  const addWire = useWorkspaceStore((s) => s.addWire);
  const updateDevicePosition = useWorkspaceStore((s) => s.updateDevicePosition);
  const setSelection = useWorkspaceStore((s) => s.setSelection);
  const setSelectedNodeIds = useWorkspaceStore((s) => s.setSelectedNodeIds);
  const createRackFromSelection = useWorkspaceStore((s) => s.createRackFromSelection);

  const [connectingFromKind, setConnectingFromKind] = useState<SignalKind | null>(null);
  const { screenToFlowPosition } = useReactFlow();
  const calmMode = graph.settings.calm_mode;

  // Global keyboard shortcuts: Cmd/Ctrl+G (create rack), Cmd/Ctrl+Z (undo), Cmd/Ctrl+Shift+Z (redo)
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement | null;
      if (target && (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA')) return;

      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'g') {
        e.preventDefault();
        void createRackFromSelection();
      } else if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'z') {
        e.preventDefault();
        if (e.shiftKey) useWorkspaceStore.temporal.getState().redo();
        else useWorkspaceStore.temporal.getState().undo();
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [createRackFromSelection]);

  const portLookup = useMemo(() => {
    const map = new Map<string, { device: Device; port: Port }>();
    for (const dev of graph.devices) {
      for (const port of dev.ports) {
        map.set(port.id, { device: dev, port });
      }
    }
    return map;
  }, [graph.devices]);

  const collapsedGroupIds = useMemo(
    () => new Set(graph.groups.filter((g) => g.collapsed).map((g) => g.id)),
    [graph.groups]
  );

  // Build React Flow nodes (Rack nodes + non-collapsed Device nodes)
  const rfNodes = useMemo<Node[]>(() => {
    const list: Node[] = [];

    for (const rack of graph.groups) {
      const members = graph.devices.filter((d) => rack.member_node_ids.includes(d.id));
      const memberPortIds = new Set(members.flatMap((d) => d.ports.map((p) => p.id)));
      // Expose any port that connects across the rack boundary (or all member ports if none yet)
      const exposedInPorts: Port[] = [];
      const exposedOutPorts: Port[] = [];
      for (const dev of members) {
        for (const p of dev.ports) {
          const hasExternalIn = graph.wires.some(
            (w) => w.to_port_id === p.id && !memberPortIds.has(w.from_port_id)
          );
          const hasExternalOut = graph.wires.some(
            (w) => w.from_port_id === p.id && !memberPortIds.has(w.to_port_id)
          );
          if (hasExternalIn || (rack.collapsed && p.direction !== 'out' && exposedInPorts.length < 3)) {
            exposedInPorts.push(p);
          }
          if (hasExternalOut || (rack.collapsed && p.direction !== 'in' && exposedOutPorts.length < 3)) {
            exposedOutPorts.push(p);
          }
        }
      }

      list.push({
        id: `rack:${rack.id}`,
        type: 'kfRack',
        position: { x: rack.ui.x, y: rack.ui.y },
        selected: selection.kind === 'rack' && selection.id === rack.id,
        data: {
          rack,
          memberDevices: members,
          exposedInPorts,
          exposedOutPorts,
        },
      });
    }

    for (const dev of graph.devices) {
      if (dev.group_id && collapsedGroupIds.has(dev.group_id)) {
        continue;
      }
      list.push({
        id: dev.id,
        type: 'kfDevice',
        position: { x: dev.ui.x, y: dev.ui.y },
        selected: selection.kind === 'device' && selection.id === dev.id,
        data: {
          device: dev,
          calmMode,
          connectingFromKind,
        },
      });
    }

    return list;
  }, [graph.devices, graph.groups, graph.wires, collapsedGroupIds, selection, calmMode, connectingFromKind]);

  // Build React Flow edges, automatically rerouting to collapsed Rack boundary handles
  const rfEdges = useMemo<Edge[]>(() => {
    const edges: Edge[] = [];
    for (const wire of graph.wires) {
      const fromEntry = portLookup.get(wire.from_port_id);
      const toEntry = portLookup.get(wire.to_port_id);
      if (!fromEntry || !toEntry) continue;

      const fromCollapsed =
        fromEntry.device.group_id && collapsedGroupIds.has(fromEntry.device.group_id);
      const toCollapsed =
        toEntry.device.group_id && collapsedGroupIds.has(toEntry.device.group_id);

      // Hide purely internal wires while their Rack is collapsed (they keep running in Data Plane!)
      if (
        fromCollapsed &&
        toCollapsed &&
        fromEntry.device.group_id === toEntry.device.group_id
      ) {
        continue;
      }

      const sourceNodeId = fromCollapsed ? `rack:${fromEntry.device.group_id}` : fromEntry.device.id;
      const targetNodeId = toCollapsed ? `rack:${toEntry.device.group_id}` : toEntry.device.id;
      const sigColor = SIGNAL_COLORS[fromEntry.port.signal_kind].hex;

      edges.push({
        id: wire.id,
        type: 'kfSignal',
        source: sourceNodeId,
        sourceHandle: `out:${wire.from_port_id}`,
        target: targetNodeId,
        targetHandle: `in:${wire.to_port_id}`,
        selected: selection.kind === 'wire' && selection.id === wire.id,
        data: {
          wire,
          colorHex: sigColor,
          calmMode,
        },
      });
    }
    return edges;
  }, [graph.wires, portLookup, collapsedGroupIds, selection, calmMode]);

  const onConnect = useCallback(
    (connection: Connection) => {
      setConnectingFromKind(null);
      if (!connection.sourceHandle || !connection.targetHandle) return;
      const fromPortId = connection.sourceHandle.replace(/^out:/, '');
      const toPortId = connection.targetHandle.replace(/^in:/, '');
      void addWire(fromPortId, toPortId);
    },
    [addWire]
  );

  const onConnectStart = useCallback(
    (_: unknown, params: { handleId?: string | null }) => {
      if (!params.handleId) return;
      const pid = params.handleId.replace(/^(out|in):/, '');
      const found = portLookup.get(pid);
      if (found) setConnectingFromKind(found.port.signal_kind);
    },
    [portLookup]
  );

  const onConnectEnd = useCallback(() => {
    setConnectingFromKind(null);
  }, []);

  const onDragOver = useCallback((event: React.DragEvent) => {
    event.preventDefault();
    event.dataTransfer.dropEffect = 'move';
  }, []);

  const onDrop = useCallback(
    (event: React.DragEvent) => {
      event.preventDefault();
      const kindType = event.dataTransfer.getData(
        'application/kontrolfreak-node'
      ) as DeviceKind['type'];
      if (!kindType) return;
      const def = getNodeDefinition(kindType);
      if (!def) return;
      const pos = screenToFlowPosition({ x: event.clientX, y: event.clientY });
      void addDevice(def.createDevice(pos));
    },
    [screenToFlowPosition, addDevice]
  );

  return (
    <div className="flex-1 h-full relative bg-studio-bg" onDragOver={onDragOver} onDrop={onDrop}>
      <ReactFlow
        nodes={rfNodes}
        edges={rfEdges}
        nodeTypes={nodeTypes}
        edgeTypes={edgeTypes}
        onConnect={onConnect}
        onConnectStart={onConnectStart}
        onConnectEnd={onConnectEnd}
        onNodeDragStop={(_, node) => {
          if (!node.id.startsWith('rack:')) {
            void updateDevicePosition(node.id, Math.round(node.position.x), Math.round(node.position.y));
          }
        }}
        onNodeClick={(_, node) => {
          if (node.id.startsWith('rack:')) {
            setSelection({ kind: 'rack', id: node.id.replace(/^rack:/, '') });
          } else {
            setSelection({ kind: 'device', id: node.id });
          }
        }}
        onEdgeClick={(_, edge) => setSelection({ kind: 'wire', id: edge.id })}
        onPaneClick={() => setSelection({ kind: 'none' })}
        onSelectionChange={({ nodes }) => {
          setSelectedNodeIds(nodes.filter((n) => !n.id.startsWith('rack:')).map((n) => n.id));
        }}
        fitView
        fitViewOptions={{ padding: 0.25 }}
        deleteKeyCode={null}
      >
        <Background
          variant={BackgroundVariant.Dots}
          gap={22}
          size={1.2}
          color="#1e293b"
        />
        <Controls position="bottom-left" />
        <MiniMap
          position="bottom-right"
          nodeColor={(n) => (n.id.startsWith('rack:') ? '#7c3aed' : '#334155')}
          maskColor="rgba(11, 14, 20, 0.72)"
        />
      </ReactFlow>

      {/* Subtle floating helper hint */}
      <div className="pointer-events-none absolute top-2.5 left-3 flex items-center gap-2 text-[10px] font-mono text-studio-muted bg-studio-surface/80 backdrop-blur px-2.5 py-1 rounded-md border border-studio-border">
        <span>Drag ports to wire</span>
        <span>·</span>
        <span>Click title to rename</span>
        <span>·</span>
        <span>Multi-select + ⌘G for Rack</span>
      </div>
    </div>
  );
};

export const CanvasWorkspace: React.FC = () => (
  <ReactFlowProvider>
    <CanvasInner />
  </ReactFlowProvider>
);
