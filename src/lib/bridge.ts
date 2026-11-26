import type {
  Device,
  DeviceFingerprint,
  DeviceKind,
  DeviceStatus,
  EndpointStatus,
  GroupPreset,
  MacroBinding,
  MacroTarget,
  MappingPreset,
  MidiPortInfo,
  Modifier,
  Port,
  ProjectFile,
  RackGroup,
  SignalValue,
  TelemetryBatch,
  TelemetryEntry,
  TrafficLogEntry,
  VirtualMidiGuidance,
  Wire,
  WorkspaceGraph,
  WorkspaceSettings,
} from '../types/kontrolfreak';
import {
  applyCurve,
  applyModifierChain,
  formatSignalSummary,
  sampleTransferCurveLocal,
  signalToNumber,
  type ModifierSlotState,
} from './modifierMath';
import { areSignalKindsCompatible } from './signalColors';

export function isTauriRuntime(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

function makeUuid(): string {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
    return crypto.randomUUID();
  }
  return 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, (c) => {
    const r = (Math.random() * 16) | 0;
    const v = c === 'x' ? r : (r & 0x3) | 0x8;
    return v.toString(16);
  });
}

function clone<T>(v: T): T {
  return JSON.parse(JSON.stringify(v));
}

export function createStarterGraph(): WorkspaceGraph {
  const oscFaderId = makeUuid();
  const oscPadId = makeUuid();
  const midiCcId = makeUuid();
  const midiNoteId = makeUuid();
  const wsInId = makeUuid();
  const udpOutId = makeUuid();

  const oscDev: Device = {
    id: makeUuid(),
    logical_key: 'osc.ipad',
    display_name: 'TouchOSC iPad',
    kind: {
      type: 'OscEndpoint',
      bind_addr: '127.0.0.1:9000',
      target_addr: '127.0.0.1:9001',
      auto_discover: true,
    },
    fingerprint: { endpoint: '127.0.0.1:9000' },
    status: { state: 'online' },
    ports: [
      {
        id: oscFaderId,
        logical_key: '/fader/1',
        alias: 'Main Fader',
        direction: 'out',
        signal_kind: 'control',
        min_value: 0,
        max_value: 1,
        modifiers: [],
      },
      {
        id: oscPadId,
        logical_key: '/xy/pad',
        alias: 'XY Pad',
        direction: 'out',
        signal_kind: 'control',
        min_value: 0,
        max_value: 1,
        modifiers: [],
      },
    ],
    ui: { x: 80, y: 100, collapsed: false },
    group_id: null,
    manual_override: {},
  };

  const midiDev: Device = {
    id: makeUuid(),
    logical_key: 'midi.synth',
    display_name: 'Virtual Synth Bus',
    kind: {
      type: 'MidiPort',
      port_name: 'Kontrolfreak Loopback 1',
      is_virtual: true,
      channel_filter: null,
      backend: 'loopback_mock',
    },
    fingerprint: { port_name: 'Kontrolfreak Loopback 1' },
    status: { state: 'online' },
    ports: [
      {
        id: midiCcId,
        logical_key: 'cc.1.7',
        alias: 'Master Volume CC7',
        direction: 'in',
        signal_kind: 'midi',
        min_value: 0,
        max_value: 1,
        modifiers: [],
      },
      {
        id: midiNoteId,
        logical_key: 'note.1.60',
        alias: 'Trigger Note C4',
        direction: 'in',
        signal_kind: 'trigger',
        min_value: 0,
        max_value: 1,
        modifiers: [],
      },
    ],
    ui: { x: 470, y: 80, collapsed: false },
    group_id: null,
    manual_override: {},
  };

  const wsDev: Device = {
    id: makeUuid(),
    logical_key: 'ws.bridge',
    display_name: 'WebSocket HUD',
    kind: {
      type: 'WebSocketEndpoint',
      mode: 'server',
      url_or_bind: '127.0.0.1:8765',
      framing: 'json',
    },
    fingerprint: { endpoint: 'ws://127.0.0.1:8765' },
    status: { state: 'online' },
    ports: [
      {
        id: wsInId,
        logical_key: '/hud/level',
        alias: 'HUD Level',
        direction: 'bidir',
        signal_kind: 'control',
        min_value: 0,
        max_value: 1,
        modifiers: [],
      },
    ],
    ui: { x: 470, y: 310, collapsed: false },
    group_id: null,
    manual_override: {},
  };

  const udpDev: Device = {
    id: makeUuid(),
    logical_key: 'udp.lights',
    display_name: 'ESP32 DMX Node',
    kind: {
      type: 'UdpEndpoint',
      bind_addr: '0.0.0.0:7000',
      target_addr: '127.0.0.1:7001',
      multicast_group: null,
      broadcast: false,
      format: 'json',
    },
    fingerprint: { endpoint: '127.0.0.1:7001' },
    status: { state: 'offline_ghost', reason: 'Unplugged USB/Wi-Fi lighting rig' },
    ports: [
      {
        id: udpOutId,
        logical_key: 'dimmer.1',
        alias: 'Wash Dimmer',
        direction: 'in',
        signal_kind: 'control',
        min_value: 0,
        max_value: 1,
        modifiers: [],
      },
    ],
    ui: { x: 840, y: 190, collapsed: false },
    group_id: null,
    manual_override: {},
  };

  return {
    project_id: makeUuid(),
    name: 'Studio Starter Rig',
    devices: [oscDev, midiDev, wsDev, udpDev],
    wires: [
      {
        id: makeUuid(),
        from_port_id: oscFaderId,
        to_port_id: midiCcId,
        enabled: true,
        modifiers: [
          { type: 'Curve', kind: { kind: 's_curve', steepness: 4.5 } },
          { type: 'Clamp', min: 0, max: 1 },
        ],
      },
      {
        id: makeUuid(),
        from_port_id: oscPadId,
        to_port_id: wsInId,
        enabled: true,
        modifiers: [],
      },
      {
        id: makeUuid(),
        from_port_id: wsInId,
        to_port_id: udpOutId,
        enabled: true,
        modifiers: [],
      },
    ],
    groups: [],
    settings: {
      calm_mode: false,
      telemetry_hz: 30,
      theme: 'dark-studio',
    },
  };
}

/**
 * Full in-browser loopback engine mirroring Rust `ControlPlane` + `SignalRouter` + 30 Hz telemetry
 * so every UI workflow works seamlessly in `vite dev` and headless Vitest tests.
 */
export class BrowserLoopbackEngine {
  private graph: WorkspaceGraph;
  private wireStates = new Map<string, ModifierSlotState[]>();
  private portSlots = new Map<
    string,
    { deviceId: string; value: SignalValue; count: number; updatedMs: number }
  >();
  private pendingLogs: TrafficLogEntry[] = [];
  private totalEvents = 0;
  private lastFlushMs = Date.now();
  private runningEndpoints = new Map<string, EndpointStatus>();
  private telemetryTimer: ReturnType<typeof setInterval> | null = null;
  private demoTimer: ReturnType<typeof setInterval> | null = null;
  private demoPhase = 0;
  private availableMidiPorts: MidiPortInfo[] = [
    {
      name: 'Kontrolfreak Loopback 1',
      direction: 'bidir',
      is_virtual: true,
      is_loopmidi: false,
      backend: 'loopback_mock',
    },
    {
      name: 'loopMIDI Port 1',
      direction: 'out',
      is_virtual: true,
      is_loopmidi: true,
      backend: 'loop_midi_windows',
    },
    {
      name: 'USB MIDI Controller (Simulated)',
      direction: 'in',
      is_virtual: false,
      is_loopmidi: false,
      backend: 'hardware',
    },
  ];

  constructor(initial?: WorkspaceGraph) {
    this.graph = initial ? clone(initial) : createStarterGraph();
    for (const dev of this.graph.devices) {
      if (dev.status.state === 'online') {
        this.runningEndpoints.set(dev.id, {
          device_id: dev.id,
          protocol: dev.kind.type,
          running: true,
          bound_address: dev.fingerprint?.endpoint ?? dev.fingerprint?.port_name ?? 'loopback',
          detail: 'Browser loopback active',
        });
      }
    }
  }

  getWorkspace(): WorkspaceGraph {
    return clone(this.graph);
  }

  replaceWorkspace(next: WorkspaceGraph): WorkspaceGraph {
    this.graph = clone(next);
    return clone(this.graph);
  }

  addDevice(device: Device): Device {
    const d = clone(device);
    if (!d.id) d.id = makeUuid();
    this.graph.devices.push(d);
    return clone(d);
  }

  removeDevice(deviceId: string): void {
    const dev = this.graph.devices.find((d) => d.id === deviceId);
    const removedPorts = new Set(dev?.ports.map((p) => p.id) ?? []);
    this.graph.devices = this.graph.devices.filter((d) => d.id !== deviceId);
    this.graph.wires = this.graph.wires.filter(
      (w) => !removedPorts.has(w.from_port_id) && !removedPorts.has(w.to_port_id)
    );
    for (const g of this.graph.groups) {
      g.member_node_ids = g.member_node_ids.filter((id) => id !== deviceId);
      for (const m of g.macros) {
        m.targets = m.targets.filter((t) => !removedPorts.has(t.port_id));
      }
    }
    this.runningEndpoints.delete(deviceId);
  }

  updateDevicePosition(deviceId: string, x: number, y: number): void {
    const dev = this.graph.devices.find((d) => d.id === deviceId);
    if (dev) {
      dev.ui.x = x;
      dev.ui.y = y;
    }
  }

  renameDevice(deviceId: string, displayName: string): Device {
    const dev = this.graph.devices.find((d) => d.id === deviceId);
    if (!dev) throw new Error(`Device not found: ${deviceId}`);
    dev.display_name = displayName;
    return clone(dev);
  }

  renamePort(deviceId: string, portId: string, alias: string): Port {
    const dev = this.graph.devices.find((d) => d.id === deviceId);
    const port = dev?.ports.find((p) => p.id === portId);
    if (!port) throw new Error(`Port not found: ${portId}`);
    port.alias = alias;
    return clone(port);
  }

  addPort(deviceId: string, port: Port): Port {
    const dev = this.graph.devices.find((d) => d.id === deviceId);
    if (!dev) throw new Error(`Device not found: ${deviceId}`);
    const existing = dev.ports.find((p) => p.logical_key === port.logical_key);
    if (existing) return clone(existing);
    const p = clone(port);
    if (!p.id) p.id = makeUuid();
    dev.ports.push(p);
    return clone(p);
  }

  removePort(deviceId: string, portId: string): void {
    const dev = this.graph.devices.find((d) => d.id === deviceId);
    if (!dev) return;
    dev.ports = dev.ports.filter((p) => p.id !== portId);
    this.graph.wires = this.graph.wires.filter(
      (w) => w.from_port_id !== portId && w.to_port_id !== portId
    );
  }

  updateDeviceKind(deviceId: string, kind: DeviceKind): Device {
    const dev = this.graph.devices.find((d) => d.id === deviceId);
    if (!dev) throw new Error(`Device not found: ${deviceId}`);
    dev.kind = clone(kind);
    return clone(dev);
  }

  addWire(fromPortId: string, toPortId: string): Wire {
    if (fromPortId === toPortId) throw new Error('Cannot wire port to itself');
    const findPort = (pid: string) => {
      for (const d of this.graph.devices) {
        const p = d.ports.find((x) => x.id === pid);
        if (p) return p;
      }
      return undefined;
    };
    const fp = findPort(fromPortId);
    const tp = findPort(toPortId);
    if (!fp || !tp) throw new Error('Port not found');
    if (fp.direction === 'in') throw new Error('Source port cannot emit');
    if (tp.direction === 'out') throw new Error('Destination port cannot receive');
    if (!areSignalKindsCompatible(fp.signal_kind, tp.signal_kind)) {
      throw new Error(`Incompatible signals: ${fp.signal_kind} -> ${tp.signal_kind}`);
    }
    const dup = this.graph.wires.find(
      (w) => w.from_port_id === fromPortId && w.to_port_id === toPortId
    );
    if (dup) throw new Error('Duplicate wire');
    const wire: Wire = {
      id: makeUuid(),
      from_port_id: fromPortId,
      to_port_id: toPortId,
      enabled: true,
      modifiers: [],
    };
    this.graph.wires.push(wire);
    return clone(wire);
  }

  removeWire(wireId: string): void {
    this.graph.wires = this.graph.wires.filter((w) => w.id !== wireId);
    this.wireStates.delete(wireId);
  }

  toggleWire(wireId: string, enabled: boolean): Wire {
    const wire = this.graph.wires.find((w) => w.id === wireId);
    if (!wire) throw new Error(`Wire not found: ${wireId}`);
    wire.enabled = enabled;
    return clone(wire);
  }

  updateWireModifiers(wireId: string, modifiers: Modifier[]): Wire {
    const wire = this.graph.wires.find((w) => w.id === wireId);
    if (!wire) throw new Error(`Wire not found: ${wireId}`);
    wire.modifiers = clone(modifiers);
    this.wireStates.delete(wireId);
    return clone(wire);
  }

  createRack(displayName: string, memberIds: string[]): RackGroup {
    let minX = Infinity;
    let minY = Infinity;
    const id = makeUuid();
    for (const dev of this.graph.devices) {
      if (memberIds.includes(dev.id)) {
        dev.group_id = id;
        minX = Math.min(minX, dev.ui.x);
        minY = Math.min(minY, dev.ui.y);
      }
    }
    const group: RackGroup = {
      id,
      display_name: displayName,
      collapsed: false,
      member_node_ids: [...memberIds],
      macros: [],
      ui: {
        x: Number.isFinite(minX) ? Math.max(0, minX - 28) : 120,
        y: Number.isFinite(minY) ? Math.max(0, minY - 56) : 120,
        collapsed: false,
      },
    };
    this.graph.groups.push(group);
    return clone(group);
  }

  ungroupRack(groupId: string): void {
    this.graph.groups = this.graph.groups.filter((g) => g.id !== groupId);
    for (const dev of this.graph.devices) {
      if (dev.group_id === groupId) dev.group_id = null;
    }
  }

  renameRack(groupId: string, displayName: string): RackGroup {
    const g = this.graph.groups.find((x) => x.id === groupId);
    if (!g) throw new Error(`Rack not found: ${groupId}`);
    g.display_name = displayName;
    return clone(g);
  }

  setRackCollapsed(groupId: string, collapsed: boolean): RackGroup {
    const g = this.graph.groups.find((x) => x.id === groupId);
    if (!g) throw new Error(`Rack not found: ${groupId}`);
    g.collapsed = collapsed;
    return clone(g);
  }

  exposeMacro(groupId: string, alias: string, targets: MacroTarget[]): MacroBinding {
    const g = this.graph.groups.find((x) => x.id === groupId);
    if (!g) throw new Error(`Rack not found: ${groupId}`);
    const binding: MacroBinding = {
      id: makeUuid(),
      alias,
      value: 0,
      targets: clone(targets),
    };
    g.macros.push(binding);
    return clone(binding);
  }

  removeMacro(groupId: string, macroId: string): void {
    const g = this.graph.groups.find((x) => x.id === groupId);
    if (!g) return;
    g.macros = g.macros.filter((m) => m.id !== macroId);
  }

  setMacroValue(groupId: string, macroId: string, value: number): MacroBinding {
    const g = this.graph.groups.find((x) => x.id === groupId);
    const m = g?.macros.find((x) => x.id === macroId);
    if (!m) throw new Error(`Macro not found: ${macroId}`);
    m.value = Math.min(1, Math.max(0, value));
    for (const t of m.targets) {
      let norm = m.value;
      if (t.invert) norm = 1 - norm;
      norm = applyCurve(t.curve, norm);
      const out = t.min + norm * (t.max - t.min);
      this.injectSignal(t.port_id, { type: 'Float', value: out });
    }
    return clone(m);
  }

  setDeviceStatus(deviceId: string, status: DeviceStatus): Device {
    const dev = this.graph.devices.find((d) => d.id === deviceId);
    if (!dev) throw new Error(`Device not found: ${deviceId}`);
    dev.status = clone(status);
    return clone(dev);
  }

  rebindOfflineDevice(
    deviceId: string,
    newFingerprint?: DeviceFingerprint | null,
    newPortName?: string | null
  ): Device {
    const dev = this.graph.devices.find((d) => d.id === deviceId);
    if (!dev) throw new Error(`Device not found: ${deviceId}`);
    if (newFingerprint) dev.fingerprint = clone(newFingerprint);
    if (newPortName) {
      if (dev.kind.type === 'MidiPort') {
        dev.kind.port_name = newPortName;
      }
      dev.fingerprint = { ...(dev.fingerprint ?? {}), port_name: newPortName };
    }
    dev.status = { state: 'online' };
    return clone(dev);
  }

  setManualOverride(deviceId: string, portId: string, value: SignalValue | null): void {
    const dev = this.graph.devices.find((d) => d.id === deviceId);
    if (!dev) return;
    if (value) {
      dev.manual_override[portId] = value;
      this.injectSignal(portId, value);
    } else {
      delete dev.manual_override[portId];
    }
  }

  injectSignal(portId: string, value: SignalValue): number {
    const now = Date.now();
    const portToDevice = new Map<string, Device>();
    const portMap = new Map<string, Port>();
    for (const d of this.graph.devices) {
      for (const p of d.ports) {
        portToDevice.set(p.id, d);
        portMap.set(p.id, p);
      }
    }

    const queue: Array<{ pid: string; val: SignalValue; visited: Set<string>; hops: number }> = [
      { pid: portId, val: value, visited: new Set(), hops: 0 },
    ];
    let delivered = 0;

    while (queue.length > 0) {
      const ev = queue.shift()!;
      if (ev.hops >= 16) continue;
      const srcDev = portToDevice.get(ev.pid);
      if (srcDev) {
        this.recordSignal(srcDev.id, ev.pid, ev.val, now);
        ev.visited.add(srcDev.id);
      }

      const wires = this.graph.wires.filter((w) => w.enabled && w.from_port_id === ev.pid);
      for (const wire of wires) {
        const dstDev = portToDevice.get(wire.to_port_id);
        const dstPort = portMap.get(wire.to_port_id);
        if (!dstDev || !dstPort || ev.visited.has(dstDev.id)) continue;

        let slots = this.wireStates.get(wire.id);
        if (!slots) {
          slots = [];
          this.wireStates.set(wire.id, slots);
        }
        const shaped = applyModifierChain(wire.modifiers, ev.val, now, slots);
        if (!shaped) continue;

        this.recordSignal(dstDev.id, wire.to_port_id, shaped, now);

        if (dstDev.status.state === 'online') {
          delivered++;
          this.recordLog(
            protocolLabel(dstDev.kind),
            'TX',
            `${dstDev.display_name} · ${dstPort.logical_key} = ${formatSignalSummary(shaped)}`,
            dstDev.id,
            dstPort.id
          );
        }

        const nextVisited = new Set(ev.visited);
        nextVisited.add(dstDev.id);
        queue.push({
          pid: wire.to_port_id,
          val: shaped,
          visited: nextVisited,
          hops: ev.hops + 1,
        });
      }
    }

    return delivered;
  }

  private recordSignal(deviceId: string, portId: string, value: SignalValue, now: number): void {
    this.totalEvents++;
    const cur = this.portSlots.get(portId);
    if (cur) {
      cur.deviceId = deviceId;
      cur.value = value;
      cur.count++;
      cur.updatedMs = now;
    } else {
      this.portSlots.set(portId, { deviceId, value, count: 1, updatedMs: now });
    }
  }

  private recordLog(
    protocol: string,
    direction: string,
    summary: string,
    deviceId?: string,
    portId?: string
  ): void {
    this.pendingLogs.push({
      timestamp_ms: Date.now(),
      protocol,
      direction,
      summary,
      device_id: deviceId ?? null,
      port_id: portId ?? null,
    });
    if (this.pendingLogs.length > 80) {
      this.pendingLogs.shift();
    }
  }

  flushTelemetry(): TelemetryBatch {
    const now = Date.now();
    const dtSecs = Math.max(0.001, (now - this.lastFlushMs) / 1000);
    this.lastFlushMs = now;
    const totalRate = this.totalEvents / dtSecs;
    this.totalEvents = 0;

    const values: TelemetryEntry[] = [];
    for (const [portId, slot] of this.portSlots.entries()) {
      const count = slot.count;
      slot.count = 0;
      values.push({
        port_id: portId,
        device_id: slot.deviceId,
        latest_value: slot.value,
        numeric_value: signalToNumber(slot.value),
        active: count > 0 || now - slot.updatedMs < 180,
        packets_per_sec: count / dtSecs,
        last_updated_ms: slot.updatedMs,
      });
    }

    const recent_logs = this.pendingLogs.splice(0, this.pendingLogs.length);
    return {
      timestamp_ms: now,
      values,
      recent_logs,
      total_msg_per_sec: totalRate,
    };
  }

  startTelemetryStream(onBatch: (batch: TelemetryBatch) => void): () => void {
    this.stopTelemetryStream();
    const hz = Math.min(60, Math.max(10, this.graph.settings.telemetry_hz || 30));
    const intervalMs = Math.round(1000 / hz);
    this.telemetryTimer = setInterval(() => {
      onBatch(this.flushTelemetry());
    }, intervalMs);

    // Gentle live loopback oscillator on the first OSC device's primary port when online
    this.demoTimer = setInterval(() => {
      this.demoPhase += 0.09;
      const osc = this.graph.devices.find(
        (d) => d.kind.type === 'OscEndpoint' && d.status.state === 'online'
      );
      if (osc && osc.ports.length > 0 && !osc.manual_override[osc.ports[0].id]) {
        const val = 0.5 + 0.45 * Math.sin(this.demoPhase);
        this.injectSignal(osc.ports[0].id, { type: 'Float', value: val });
      }
    }, 80);

    return () => this.stopTelemetryStream();
  }

  stopTelemetryStream(): void {
    if (this.telemetryTimer) {
      clearInterval(this.telemetryTimer);
      this.telemetryTimer = null;
    }
    if (this.demoTimer) {
      clearInterval(this.demoTimer);
      this.demoTimer = null;
    }
  }

  listMidiPorts(): MidiPortInfo[] {
    return clone(this.availableMidiPorts);
  }

  getVirtualMidiGuidance(): VirtualMidiGuidance {
    const loopDetected = this.availableMidiPorts.some((p) => p.is_loopmidi);
    return {
      platform: 'Windows / macOS (Browser Loopback)',
      native_supported: false,
      loopmidi_detected: loopDetected,
      recommended_tool: 'loopMIDI by Tobias Erichsen',
      download_url: 'https://www.tobias-erichsen.de/software/loopmidi.html',
      steps: [
        'On macOS: native CoreMIDI virtual ports are created automatically in-app.',
        'On Windows: install loopMIDI and create a port named "Kontrolfreak Virtual 1".',
        'Kontrolfreak detects loopMIDI ports automatically and guards against self-loopback.',
      ],
    };
  }

  exportProjectJson(): string {
    const file: ProjectFile = { schema_version: 1, graph: this.graph };
    return JSON.stringify(file, null, 2);
  }

  importProjectJson(json: string, reconcileHardware = true): WorkspaceGraph {
    const parsed = JSON.parse(json) as ProjectFile;
    if (!parsed || typeof parsed.schema_version !== 'number' || !parsed.graph) {
      throw new Error('Invalid .kfproj.json format');
    }
    const next = clone(parsed.graph);
    if (reconcileHardware) {
      const availNames = new Set(
        this.availableMidiPorts.map((p) => p.name.toLowerCase())
      );
      for (const dev of next.devices) {
        if (dev.kind.type === 'MidiPort') {
          const pname = (dev.fingerprint?.port_name ?? dev.kind.port_name).toLowerCase();
          dev.status = availNames.has(pname)
            ? { state: 'online' }
            : { state: 'offline_ghost', reason: 'MIDI port not present on load' };
        }
      }
    }
    this.graph = next;
    return clone(this.graph);
  }

  exportDevicePreset(deviceId: string): string {
    const dev = this.graph.devices.find((d) => d.id === deviceId);
    if (!dev) throw new Error(`Device not found: ${deviceId}`);
    return JSON.stringify(
      {
        schema_version: 1,
        preset_id: makeUuid(),
        name: dev.display_name,
        device: dev,
      },
      null,
      2
    );
  }

  importDevicePreset(json: string): Device {
    const parsed = JSON.parse(json) as { device: Device };
    const dev = clone(parsed.device);
    dev.id = makeUuid();
    dev.ui = { ...dev.ui, x: dev.ui.x + 36, y: dev.ui.y + 36 };
    dev.group_id = null;
    dev.ports = dev.ports.map((p) => ({ ...p, id: makeUuid() }));
    this.graph.devices.push(dev);
    return clone(dev);
  }

  exportGroupPreset(groupId: string): string {
    const group = this.graph.groups.find((g) => g.id === groupId);
    if (!group) throw new Error(`Rack not found: ${groupId}`);
    const members = this.graph.devices.filter((d) => group.member_node_ids.includes(d.id));
    const portIds = new Set(members.flatMap((d) => d.ports.map((p) => p.id)));
    const internalWires = this.graph.wires.filter(
      (w) => portIds.has(w.from_port_id) && portIds.has(w.to_port_id)
    );
    const preset: GroupPreset = {
      schema_version: 1,
      preset_id: makeUuid(),
      name: group.display_name,
      group,
      member_devices: members,
      internal_wires: internalWires,
    };
    return JSON.stringify(preset, null, 2);
  }

  importGroupPreset(json: string): WorkspaceGraph {
    const preset = JSON.parse(json) as GroupPreset;
    const newGroupId = makeUuid();
    const devMap = new Map<string, string>();
    const portMap = new Map<string, string>();

    const newDevs = preset.member_devices.map((d) => {
      const nd = clone(d);
      nd.id = makeUuid();
      devMap.set(d.id, nd.id);
      nd.group_id = newGroupId;
      nd.ui.x += 40;
      nd.ui.y += 40;
      nd.ports = nd.ports.map((p) => {
        const nid = makeUuid();
        portMap.set(p.id, nid);
        return { ...p, id: nid };
      });
      return nd;
    });

    const newWires = preset.internal_wires
      .map((w) => {
        const fromId = portMap.get(w.from_port_id);
        const toId = portMap.get(w.to_port_id);
        if (!fromId || !toId) return null;
        return { ...clone(w), id: makeUuid(), from_port_id: fromId, to_port_id: toId };
      })
      .filter((w): w is Wire => w !== null);

    const newGroup = clone(preset.group);
    newGroup.id = newGroupId;
    newGroup.ui.x += 40;
    newGroup.ui.y += 40;
    newGroup.member_node_ids = newGroup.member_node_ids
      .map((id) => devMap.get(id))
      .filter((id): id is string => Boolean(id));
    newGroup.macros = newGroup.macros.map((m) => ({
      ...m,
      id: makeUuid(),
      targets: m.targets.map((t) => ({ ...t, port_id: portMap.get(t.port_id) ?? t.port_id })),
    }));

    this.graph.devices.push(...newDevs);
    this.graph.wires.push(...newWires);
    this.graph.groups.push(newGroup);
    return clone(this.graph);
  }

  exportMappingPreset(name: string, description: string | null, modifiers: Modifier[]): string {
    const preset: MappingPreset = {
      schema_version: 1,
      preset_id: makeUuid(),
      name,
      description,
      modifiers,
    };
    return JSON.stringify(preset, null, 2);
  }

  importMappingPreset(json: string): MappingPreset {
    return JSON.parse(json) as MappingPreset;
  }

  updateSettings(settings: WorkspaceSettings): WorkspaceSettings {
    this.graph.settings = clone(settings);
    return clone(this.graph.settings);
  }

  getProtocolStatuses(): EndpointStatus[] {
    return Array.from(this.runningEndpoints.values());
  }

  startProtocolEndpoint(deviceId: string): EndpointStatus {
    const dev = this.graph.devices.find((d) => d.id === deviceId);
    const status: EndpointStatus = {
      device_id: deviceId,
      protocol: dev ? protocolLabel(dev.kind) : 'Unknown',
      running: true,
      bound_address: dev?.fingerprint?.endpoint ?? 'loopback',
      detail: 'Running (loopback)',
    };
    this.runningEndpoints.set(deviceId, status);
    return status;
  }

  stopProtocolEndpoint(deviceId: string): void {
    this.runningEndpoints.delete(deviceId);
  }
}

function protocolLabel(kind: DeviceKind): string {
  switch (kind.type) {
    case 'OscEndpoint':
      return 'OSC';
    case 'MidiPort':
      return 'MIDI';
    case 'WebSocketEndpoint':
      return 'WS';
    case 'UdpEndpoint':
      return 'UDP';
    case 'VirtualControl':
      return 'CTRL';
    case 'RtpMidiSession':
      return 'RTPMIDI';
    case 'NdiVideoNode':
      return 'NDI';
    case 'HidDevice':
      return 'HID';
    case 'RgbCamera':
      return 'CAM';
    case 'DepthSensor':
      return 'DEPTH';
  }
}

const loopback = new BrowserLoopbackEngine();

async function tauriInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import('@tauri-apps/api/core');
  return invoke<T>(cmd, args);
}

/**
 * Unified dual-runtime bridge: uses typed Tauri v2 IPC commands & `Channel` when inside the
 * desktop app, and transparently falls back to `BrowserLoopbackEngine` in `vite dev` / tests.
 */
export const bridge = {
  isTauri: isTauriRuntime,
  loopback,

  async getWorkspace(): Promise<WorkspaceGraph> {
    return isTauriRuntime() ? tauriInvoke('get_workspace') : loopback.getWorkspace();
  },

  async addDevice(device: Device): Promise<Device> {
    return isTauriRuntime() ? tauriInvoke('add_device', { device }) : loopback.addDevice(device);
  },

  async removeDevice(deviceId: string): Promise<void> {
    if (isTauriRuntime()) return tauriInvoke('remove_device', { deviceId });
    loopback.removeDevice(deviceId);
  },

  async updateDevicePosition(deviceId: string, x: number, y: number): Promise<void> {
    if (isTauriRuntime()) return tauriInvoke('update_device_position', { deviceId, x, y });
    loopback.updateDevicePosition(deviceId, x, y);
  },

  async renameDevice(deviceId: string, displayName: string): Promise<Device> {
    return isTauriRuntime()
      ? tauriInvoke('rename_device', { deviceId, displayName })
      : loopback.renameDevice(deviceId, displayName);
  },

  async renamePort(deviceId: string, portId: string, alias: string): Promise<Port> {
    return isTauriRuntime()
      ? tauriInvoke('rename_port', { deviceId, portId, alias })
      : loopback.renamePort(deviceId, portId, alias);
  },

  async addPort(deviceId: string, port: Port): Promise<Port> {
    return isTauriRuntime()
      ? tauriInvoke('add_port', { deviceId, port })
      : loopback.addPort(deviceId, port);
  },

  async removePort(deviceId: string, portId: string): Promise<void> {
    if (isTauriRuntime()) return tauriInvoke('remove_port', { deviceId, portId });
    loopback.removePort(deviceId, portId);
  },

  async updateDeviceKind(deviceId: string, kind: DeviceKind): Promise<Device> {
    return isTauriRuntime()
      ? tauriInvoke('update_device_kind', { deviceId, kind })
      : loopback.updateDeviceKind(deviceId, kind);
  },

  async addWire(fromPortId: string, toPortId: string): Promise<Wire> {
    return isTauriRuntime()
      ? tauriInvoke('add_wire', { fromPortId, toPortId })
      : loopback.addWire(fromPortId, toPortId);
  },

  async removeWire(wireId: string): Promise<void> {
    if (isTauriRuntime()) return tauriInvoke('remove_wire', { wireId });
    loopback.removeWire(wireId);
  },

  async toggleWire(wireId: string, enabled: boolean): Promise<Wire> {
    return isTauriRuntime()
      ? tauriInvoke('toggle_wire', { wireId, enabled })
      : loopback.toggleWire(wireId, enabled);
  },

  async updateWireModifiers(wireId: string, modifiers: Modifier[]): Promise<Wire> {
    return isTauriRuntime()
      ? tauriInvoke('update_wire_modifiers', { wireId, modifiers })
      : loopback.updateWireModifiers(wireId, modifiers);
  },

  async computeTransferCurve(modifiers: Modifier[], steps = 65): Promise<Array<[number, number]>> {
    return isTauriRuntime()
      ? tauriInvoke('compute_transfer_curve', { modifiers, steps })
      : sampleTransferCurveLocal(modifiers, steps);
  },

  async createRack(displayName: string, memberNodeIds: string[]): Promise<RackGroup> {
    return isTauriRuntime()
      ? tauriInvoke('create_rack', { displayName, memberNodeIds })
      : loopback.createRack(displayName, memberNodeIds);
  },

  async ungroupRack(groupId: string): Promise<void> {
    if (isTauriRuntime()) return tauriInvoke('ungroup_rack', { groupId });
    loopback.ungroupRack(groupId);
  },

  async renameRack(groupId: string, displayName: string): Promise<RackGroup> {
    return isTauriRuntime()
      ? tauriInvoke('rename_rack', { groupId, displayName })
      : loopback.renameRack(groupId, displayName);
  },

  async setRackCollapsed(groupId: string, collapsed: boolean): Promise<RackGroup> {
    return isTauriRuntime()
      ? tauriInvoke('set_rack_collapsed', { groupId, collapsed })
      : loopback.setRackCollapsed(groupId, collapsed);
  },

  async exposeMacro(groupId: string, alias: string, targets: MacroTarget[]): Promise<MacroBinding> {
    return isTauriRuntime()
      ? tauriInvoke('expose_macro', { groupId, alias, targets })
      : loopback.exposeMacro(groupId, alias, targets);
  },

  async removeMacro(groupId: string, macroId: string): Promise<void> {
    if (isTauriRuntime()) return tauriInvoke('remove_macro', { groupId, macroId });
    loopback.removeMacro(groupId, macroId);
  },

  async setMacroValue(groupId: string, macroId: string, value: number): Promise<MacroBinding> {
    return isTauriRuntime()
      ? tauriInvoke('set_macro_value', { groupId, macroId, value })
      : loopback.setMacroValue(groupId, macroId, value);
  },

  async injectSignal(portId: string, value: SignalValue): Promise<number> {
    return isTauriRuntime()
      ? tauriInvoke('inject_signal', { portId, value })
      : loopback.injectSignal(portId, value);
  },

  async setManualOverride(
    deviceId: string,
    portId: string,
    value: SignalValue | null
  ): Promise<void> {
    if (isTauriRuntime()) return tauriInvoke('set_manual_override', { deviceId, portId, value });
    loopback.setManualOverride(deviceId, portId, value);
  },

  async setDeviceStatus(deviceId: string, status: DeviceStatus): Promise<Device> {
    return isTauriRuntime()
      ? tauriInvoke('set_device_status', { deviceId, status })
      : loopback.setDeviceStatus(deviceId, status);
  },

  async rebindOfflineDevice(
    deviceId: string,
    newFingerprint?: DeviceFingerprint | null,
    newPortName?: string | null
  ): Promise<Device> {
    return isTauriRuntime()
      ? tauriInvoke('rebind_offline_device', { deviceId, newFingerprint, newPortName })
      : loopback.rebindOfflineDevice(deviceId, newFingerprint, newPortName);
  },

  async startProtocolEndpoint(deviceId: string): Promise<EndpointStatus> {
    return isTauriRuntime()
      ? tauriInvoke('start_protocol_endpoint', { deviceId })
      : loopback.startProtocolEndpoint(deviceId);
  },

  async stopProtocolEndpoint(deviceId: string): Promise<void> {
    if (isTauriRuntime()) return tauriInvoke('stop_protocol_endpoint', { deviceId });
    loopback.stopProtocolEndpoint(deviceId);
  },

  async getProtocolStatuses(): Promise<EndpointStatus[]> {
    return isTauriRuntime()
      ? tauriInvoke('get_protocol_statuses')
      : loopback.getProtocolStatuses();
  },

  async listMidiPorts(): Promise<MidiPortInfo[]> {
    return isTauriRuntime() ? tauriInvoke('list_midi_ports') : loopback.listMidiPorts();
  },

  async getVirtualMidiGuidance(): Promise<VirtualMidiGuidance> {
    return isTauriRuntime()
      ? tauriInvoke('get_virtual_midi_guidance')
      : loopback.getVirtualMidiGuidance();
  },

  async exportProjectJson(): Promise<string> {
    return isTauriRuntime() ? tauriInvoke('export_project_json') : loopback.exportProjectJson();
  },

  async importProjectJson(json: string, reconcileHardware = true): Promise<WorkspaceGraph> {
    return isTauriRuntime()
      ? tauriInvoke('import_project_json', { json, reconcileHardware })
      : loopback.importProjectJson(json, reconcileHardware);
  },

  async exportDevicePreset(deviceId: string): Promise<string> {
    return isTauriRuntime()
      ? tauriInvoke('export_device_preset', { deviceId })
      : loopback.exportDevicePreset(deviceId);
  },

  async importDevicePreset(json: string): Promise<Device> {
    return isTauriRuntime()
      ? tauriInvoke('import_device_preset', { json })
      : loopback.importDevicePreset(json);
  },

  async exportGroupPreset(groupId: string): Promise<string> {
    return isTauriRuntime()
      ? tauriInvoke('export_group_preset', { groupId })
      : loopback.exportGroupPreset(groupId);
  },

  async importGroupPreset(json: string): Promise<WorkspaceGraph> {
    return isTauriRuntime()
      ? tauriInvoke('import_group_preset', { json })
      : loopback.importGroupPreset(json);
  },

  async exportMappingPreset(
    name: string,
    description: string | null,
    modifiers: Modifier[]
  ): Promise<string> {
    return isTauriRuntime()
      ? tauriInvoke('export_mapping_preset', { name, description, modifiers })
      : loopback.exportMappingPreset(name, description, modifiers);
  },

  async importMappingPreset(json: string): Promise<MappingPreset> {
    return isTauriRuntime()
      ? tauriInvoke('import_mapping_preset', { json })
      : loopback.importMappingPreset(json);
  },

  async setCalmMode(enabled: boolean): Promise<WorkspaceSettings> {
    if (isTauriRuntime()) return tauriInvoke('set_calm_mode', { enabled });
    const cur = loopback.getWorkspace().settings;
    return loopback.updateSettings({ ...cur, calm_mode: enabled });
  },

  async subscribeTelemetry(
    onBatch: (batch: TelemetryBatch) => void,
    subscribedPorts?: string[] | null
  ): Promise<() => void> {
    if (isTauriRuntime()) {
      const { invoke, Channel } = await import('@tauri-apps/api/core');
      const channel = new Channel<TelemetryBatch>();
      channel.onmessage = onBatch;
      await invoke('subscribe_telemetry', {
        onBatch: channel,
        subscribedPorts: subscribedPorts ?? null,
      });
      return () => {
        channel.onmessage = () => {};
      };
    }
    return loopback.startTelemetryStream(onBatch);
  },
};
