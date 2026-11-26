import { create } from 'zustand';
import { temporal } from 'zundo';
import type {
  Device,
  DeviceFingerprint,
  DeviceKind,
  DeviceStatus,
  MacroTarget,
  MidiPortInfo,
  Modifier,
  Port,
  SelectionTarget,
  SignalValue,
  ToastNotice,
  VirtualMidiGuidance,
  WorkspaceGraph,
} from '../types/kontrolfreak';
import { bridge, createStarterGraph } from '../lib/bridge';

export interface WorkspaceStoreState {
  graph: WorkspaceGraph;
  selection: SelectionTarget;
  selectedNodeIds: string[];
  drawerOpen: boolean;
  activeDrawerTab: string;
  activeSidebarSection: string;
  commandPaletteOpen: boolean;
  midiPorts: MidiPortInfo[];
  virtualMidiGuidance: VirtualMidiGuidance | null;
  toasts: ToastNotice[];
  initialized: boolean;

  // Bootstrap & hardware refresh
  bootstrap: () => Promise<void>;
  refreshHardware: () => Promise<void>;

  // Selection & UI chrome
  setSelection: (target: SelectionTarget) => void;
  setSelectedNodeIds: (ids: string[]) => void;
  setDrawerOpen: (open: boolean) => void;
  setActiveDrawerTab: (tabId: string) => void;
  setActiveSidebarSection: (sectionId: string) => void;
  setCommandPaletteOpen: (open: boolean) => void;
  pushToast: (toast: Omit<ToastNotice, 'id'>) => void;
  dismissToast: (id: string) => void;

  // Project & Calm Mode
  setProjectName: (name: string) => void;
  toggleCalmMode: () => Promise<void>;
  loadProjectJson: (json: string, reconcileHardware?: boolean) => Promise<void>;
  exportProjectJson: () => Promise<string>;
  importDevicePresetJson: (json: string) => Promise<void>;
  importGroupPresetJson: (json: string) => Promise<void>;

  // Graph mutations (tracked by zundo temporal undo/redo)
  addDevice: (device: Device) => Promise<Device>;
  removeDevice: (deviceId: string) => Promise<void>;
  updateDevicePosition: (deviceId: string, x: number, y: number) => Promise<void>;
  renameDevice: (deviceId: string, displayName: string) => Promise<void>;
  renamePort: (deviceId: string, portId: string, alias: string) => Promise<void>;
  addPort: (deviceId: string, port: Port) => Promise<void>;
  removePort: (deviceId: string, portId: string) => Promise<void>;
  updateDeviceKind: (deviceId: string, kind: DeviceKind) => Promise<void>;
  setDeviceStatus: (deviceId: string, status: DeviceStatus) => Promise<void>;
  rebindOfflineDevice: (
    deviceId: string,
    fingerprint?: DeviceFingerprint | null,
    portName?: string | null
  ) => Promise<void>;

  // Wires & modifiers
  addWire: (fromPortId: string, toPortId: string) => Promise<void>;
  removeWire: (wireId: string) => Promise<void>;
  toggleWire: (wireId: string, enabled: boolean) => Promise<void>;
  updateWireModifiers: (wireId: string, modifiers: Modifier[]) => Promise<void>;

  // Racks & macros
  createRackFromSelection: (displayName?: string) => Promise<void>;
  createRack: (displayName: string, memberIds: string[]) => Promise<void>;
  ungroupRack: (groupId: string) => Promise<void>;
  renameRack: (groupId: string, displayName: string) => Promise<void>;
  setRackCollapsed: (groupId: string, collapsed: boolean) => Promise<void>;
  setAllRacksCollapsed: (collapsed: boolean) => Promise<void>;
  exposeMacro: (groupId: string, alias: string, targets: MacroTarget[]) => Promise<void>;
  removeMacro: (groupId: string, macroId: string) => Promise<void>;
  setMacroValue: (groupId: string, macroId: string, value: number) => Promise<void>;

  // Signal testing & overrides
  setManualOverride: (deviceId: string, portId: string, value: SignalValue | null) => Promise<void>;
  injectSignal: (portId: string, value: SignalValue) => Promise<void>;
}

function makeToastId(): string {
  return `toast-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
}

export const useWorkspaceStore = create<WorkspaceStoreState>()(
  temporal(
    (set, get) => ({
      graph: createStarterGraph(),
      selection: { kind: 'none' },
      selectedNodeIds: [],
      drawerOpen: true,
      activeDrawerTab: 'scope',
      activeSidebarSection: 'palette',
      commandPaletteOpen: false,
      midiPorts: [],
      virtualMidiGuidance: null,
      toasts: [],
      initialized: false,

      bootstrap: async () => {
        const [graph, midiPorts, guidance] = await Promise.all([
          bridge.getWorkspace(),
          bridge.listMidiPorts(),
          bridge.getVirtualMidiGuidance(),
        ]);
        set({
          graph,
          midiPorts,
          virtualMidiGuidance: guidance,
          initialized: true,
        });
      },

      refreshHardware: async () => {
        const [midiPorts, guidance] = await Promise.all([
          bridge.listMidiPorts(),
          bridge.getVirtualMidiGuidance(),
        ]);
        set({ midiPorts, virtualMidiGuidance: guidance });
      },

      setSelection: (selection) => set({ selection }),
      setSelectedNodeIds: (selectedNodeIds) => set({ selectedNodeIds }),
      setDrawerOpen: (drawerOpen) => set({ drawerOpen }),
      setActiveDrawerTab: (activeDrawerTab) => set({ activeDrawerTab, drawerOpen: true }),
      setActiveSidebarSection: (activeSidebarSection) => set({ activeSidebarSection }),
      setCommandPaletteOpen: (commandPaletteOpen) => set({ commandPaletteOpen }),

      pushToast: (toast) => {
        const id = makeToastId();
        set((s) => ({ toasts: [...s.toasts.slice(-3), { ...toast, id }] }));
        setTimeout(() => {
          get().dismissToast(id);
        }, 6500);
      },

      dismissToast: (id) => set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })),

      setProjectName: (name) =>
        set((s) => ({ graph: { ...s.graph, name } })),

      toggleCalmMode: async () => {
        const next = !get().graph.settings.calm_mode;
        const settings = await bridge.setCalmMode(next);
        set((s) => ({ graph: { ...s.graph, settings } }));
        get().pushToast({
          title: next ? 'Calm Mode enabled' : 'Calm Mode disabled',
          description: next
            ? 'Reduced motion active: wire animations and meter flashing softened.'
            : 'Full live telemetry motion restored.',
          accent: 'cyan',
        });
      },

      loadProjectJson: async (json, reconcileHardware = true) => {
        const graph = await bridge.importProjectJson(json, reconcileHardware);
        const ghosts = graph.devices.filter((d) => d.status.state === 'offline_ghost').length;
        set({ graph, selection: { kind: 'none' }, selectedNodeIds: [] });
        get().pushToast({
          title: `Loaded "${graph.name}"`,
          description:
            ghosts > 0
              ? `${ghosts} disconnected device(s) preserved as Offline Ghosts with wires intact.`
              : `${graph.devices.length} nodes and ${graph.wires.length} wires restored.`,
          accent: ghosts > 0 ? 'amber' : 'emerald',
        });
      },

      exportProjectJson: () => bridge.exportProjectJson(),

      importDevicePresetJson: async (json) => {
        const dev = await bridge.importDevicePreset(json);
        set((s) => ({
          graph: { ...s.graph, devices: [...s.graph.devices, dev] },
          selection: { kind: 'device', id: dev.id },
        }));
        get().pushToast({
          title: `Imported device "${dev.display_name}"`,
          accent: 'emerald',
        });
      },

      importGroupPresetJson: async (json) => {
        const graph = await bridge.importGroupPreset(json);
        set({ graph });
        get().pushToast({
          title: 'Imported Rack Preset',
          description: 'Subgraph instantiated with fresh immutable UUIDs.',
          accent: 'violet',
        });
      },

      addDevice: async (device) => {
        const added = await bridge.addDevice(device);
        set((s) => ({
          graph: { ...s.graph, devices: [...s.graph.devices, added] },
          selection: { kind: 'device', id: added.id },
        }));
        return added;
      },

      removeDevice: async (deviceId) => {
        await bridge.removeDevice(deviceId);
        const graph = await bridge.getWorkspace();
        set((s) => ({
          graph,
          selection:
            s.selection.kind === 'device' && s.selection.id === deviceId
              ? { kind: 'none' }
              : s.selection,
        }));
      },

      updateDevicePosition: async (deviceId, x, y) => {
        await bridge.updateDevicePosition(deviceId, x, y);
        set((s) => ({
          graph: {
            ...s.graph,
            devices: s.graph.devices.map((d) =>
              d.id === deviceId ? { ...d, ui: { ...d.ui, x, y } } : d
            ),
          },
        }));
      },

      renameDevice: async (deviceId, displayName) => {
        const updated = await bridge.renameDevice(deviceId, displayName);
        set((s) => ({
          graph: {
            ...s.graph,
            devices: s.graph.devices.map((d) => (d.id === deviceId ? updated : d)),
          },
        }));
      },

      renamePort: async (deviceId, portId, alias) => {
        const updated = await bridge.renamePort(deviceId, portId, alias);
        set((s) => ({
          graph: {
            ...s.graph,
            devices: s.graph.devices.map((d) =>
              d.id === deviceId
                ? { ...d, ports: d.ports.map((p) => (p.id === portId ? updated : p)) }
                : d
            ),
          },
        }));
      },

      addPort: async (deviceId, port) => {
        const added = await bridge.addPort(deviceId, port);
        set((s) => ({
          graph: {
            ...s.graph,
            devices: s.graph.devices.map((d) =>
              d.id === deviceId && !d.ports.some((p) => p.id === added.id)
                ? { ...d, ports: [...d.ports, added] }
                : d
            ),
          },
        }));
      },

      removePort: async (deviceId, portId) => {
        await bridge.removePort(deviceId, portId);
        const graph = await bridge.getWorkspace();
        set({ graph });
      },

      updateDeviceKind: async (deviceId, kind) => {
        const updated = await bridge.updateDeviceKind(deviceId, kind);
        set((s) => ({
          graph: {
            ...s.graph,
            devices: s.graph.devices.map((d) => (d.id === deviceId ? updated : d)),
          },
        }));
      },

      setDeviceStatus: async (deviceId, status) => {
        const updated = await bridge.setDeviceStatus(deviceId, status);
        set((s) => ({
          graph: {
            ...s.graph,
            devices: s.graph.devices.map((d) => (d.id === deviceId ? updated : d)),
          },
        }));
      },

      rebindOfflineDevice: async (deviceId, fingerprint, portName) => {
        const updated = await bridge.rebindOfflineDevice(deviceId, fingerprint, portName);
        set((s) => ({
          graph: {
            ...s.graph,
            devices: s.graph.devices.map((d) => (d.id === deviceId ? updated : d)),
          },
        }));
        get().pushToast({
          title: `Rebound "${updated.display_name}"`,
          description: 'Node is back Online with all wires and modifiers intact.',
          accent: 'emerald',
        });
      },

      addWire: async (fromPortId, toPortId) => {
        try {
          const wire = await bridge.addWire(fromPortId, toPortId);
          set((s) => ({
            graph: { ...s.graph, wires: [...s.graph.wires, wire] },
            selection: { kind: 'wire', id: wire.id },
          }));
        } catch (err) {
          get().pushToast({
            title: 'Cannot connect ports',
            description: err instanceof Error ? err.message : String(err),
            accent: 'rose',
          });
        }
      },

      removeWire: async (wireId) => {
        await bridge.removeWire(wireId);
        set((s) => ({
          graph: { ...s.graph, wires: s.graph.wires.filter((w) => w.id !== wireId) },
          selection:
            s.selection.kind === 'wire' && s.selection.id === wireId
              ? { kind: 'none' }
              : s.selection,
        }));
      },

      toggleWire: async (wireId, enabled) => {
        const updated = await bridge.toggleWire(wireId, enabled);
        set((s) => ({
          graph: {
            ...s.graph,
            wires: s.graph.wires.map((w) => (w.id === wireId ? updated : w)),
          },
        }));
      },

      updateWireModifiers: async (wireId, modifiers) => {
        const updated = await bridge.updateWireModifiers(wireId, modifiers);
        set((s) => ({
          graph: {
            ...s.graph,
            wires: s.graph.wires.map((w) => (w.id === wireId ? updated : w)),
          },
        }));
      },

      createRackFromSelection: async (displayName) => {
        const ids = get().selectedNodeIds.filter((id) =>
          get().graph.devices.some((d) => d.id === id && !d.group_id)
        );
        if (ids.length < 1) {
          get().pushToast({
            title: 'Select one or more ungrouped nodes first',
            description: 'Shift-click or drag-select nodes, then press Cmd/Ctrl+G.',
            accent: 'amber',
          });
          return;
        }
        await get().createRack(displayName ?? `Rack ${get().graph.groups.length + 1}`, ids);
      },

      createRack: async (displayName, memberIds) => {
        const rack = await bridge.createRack(displayName, memberIds);
        const graph = await bridge.getWorkspace();
        set({
          graph,
          selection: { kind: 'rack', id: rack.id },
          selectedNodeIds: [],
        });
        get().pushToast({
          title: `Created Rack "${rack.display_name}"`,
          description: 'Promote internal parameters to Macros in the Inspector.',
          accent: 'violet',
        });
      },

      ungroupRack: async (groupId) => {
        await bridge.ungroupRack(groupId);
        const graph = await bridge.getWorkspace();
        set((s) => ({
          graph,
          selection:
            s.selection.kind === 'rack' && s.selection.id === groupId
              ? { kind: 'none' }
              : s.selection,
        }));
      },

      renameRack: async (groupId, displayName) => {
        const updated = await bridge.renameRack(groupId, displayName);
        set((s) => ({
          graph: {
            ...s.graph,
            groups: s.graph.groups.map((g) => (g.id === groupId ? updated : g)),
          },
        }));
      },

      setRackCollapsed: async (groupId, collapsed) => {
        const updated = await bridge.setRackCollapsed(groupId, collapsed);
        set((s) => ({
          graph: {
            ...s.graph,
            groups: s.graph.groups.map((g) => (g.id === groupId ? updated : g)),
          },
        }));
      },

      setAllRacksCollapsed: async (collapsed) => {
        const groups = get().graph.groups;
        for (const g of groups) {
          await bridge.setRackCollapsed(g.id, collapsed);
        }
        const graph = await bridge.getWorkspace();
        set({ graph });
      },

      exposeMacro: async (groupId, alias, targets) => {
        await bridge.exposeMacro(groupId, alias, targets);
        const graph = await bridge.getWorkspace();
        set({ graph });
      },

      removeMacro: async (groupId, macroId) => {
        await bridge.removeMacro(groupId, macroId);
        const graph = await bridge.getWorkspace();
        set({ graph });
      },

      setMacroValue: async (groupId, macroId, value) => {
        const updated = await bridge.setMacroValue(groupId, macroId, value);
        set((s) => ({
          graph: {
            ...s.graph,
            groups: s.graph.groups.map((g) =>
              g.id === groupId
                ? { ...g, macros: g.macros.map((m) => (m.id === macroId ? updated : m)) }
                : g
            ),
          },
        }));
      },

      setManualOverride: async (deviceId, portId, value) => {
        await bridge.setManualOverride(deviceId, portId, value);
        set((s) => ({
          graph: {
            ...s.graph,
            devices: s.graph.devices.map((d) => {
              if (d.id !== deviceId) return d;
              const next = { ...d.manual_override };
              if (value) next[portId] = value;
              else delete next[portId];
              return { ...d, manual_override: next };
            }),
          },
        }));
      },

      injectSignal: async (portId, value) => {
        await bridge.injectSignal(portId, value);
      },
    }),
    {
      // Only snapshot canonical graph edits in Undo/Redo history (keeps selection/toasts crisp)
      partialize: (state) => ({ graph: state.graph }),
      limit: 60,
    }
  )
);
