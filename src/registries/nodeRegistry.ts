import type React from 'react';
import type { Device, DeviceKind, SignalKind } from '../types/kontrolfreak';

export interface NodeTemplateDefinition {
  kindType: DeviceKind['type'];
  label: string;
  shortBadge: string;
  category: 'protocol' | 'video' | 'hardware' | 'sensor' | 'utility';
  primarySignal: SignalKind;
  description: string;
  milestoneTag?: 'M1' | 'M2' | 'M3' | 'M4';
  createDevice: (position: { x: number; y: number }) => Device;
  /** Optional extension hook: M2 registers NDI live preview tile, M3 RGB CV overlay, M4 Depth ROI. */
  renderBody?: React.FC<{ device: Device; calmMode: boolean }>;
}

function uid(): string {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
    return crypto.randomUUID();
  }
  return `id-${Date.now()}-${Math.random().toString(36).slice(2, 9)}`;
}

const registry = new Map<DeviceKind['type'], NodeTemplateDefinition>();

export function registerNodeType(def: NodeTemplateDefinition): void {
  registry.set(def.kindType, def);
}

export function getNodeDefinition(kindType: DeviceKind['type']): NodeTemplateDefinition | undefined {
  return registry.get(kindType);
}

export function listNodeDefinitions(): NodeTemplateDefinition[] {
  return Array.from(registry.values());
}

export function initBuiltinNodeRegistry(): void {
  registerNodeType({
    kindType: 'OscEndpoint',
    label: 'OSC Endpoint',
    shortBadge: 'OSC',
    category: 'protocol',
    primarySignal: 'control',
    description: 'UDP OSC 1.0/1.1 sender & receiver with bundles, wildcards & auto-discovery.',
    milestoneTag: 'M1',
    createDevice: ({ x, y }) => ({
      id: uid(),
      logical_key: `osc.${Date.now().toString(36)}`,
      display_name: 'OSC Endpoint',
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
          id: uid(),
          logical_key: '/fader/1',
          alias: 'Fader 1',
          direction: 'out',
          signal_kind: 'control',
          min_value: 0,
          max_value: 1,
          modifiers: [],
        },
        {
          id: uid(),
          logical_key: '/out/1',
          alias: 'OSC Out 1',
          direction: 'in',
          signal_kind: 'control',
          min_value: 0,
          max_value: 1,
          modifiers: [],
        },
      ],
      ui: { x, y, collapsed: false },
      group_id: null,
      manual_override: {},
    }),
  });

  registerNodeType({
    kindType: 'MidiPort',
    label: 'MIDI / Virtual MIDI',
    shortBadge: 'MIDI',
    category: 'protocol',
    primarySignal: 'midi',
    description: 'Hardware MIDI, macOS CoreMIDI virtual ports, Windows loopMIDI & loopback.',
    milestoneTag: 'M1',
    createDevice: ({ x, y }) => ({
      id: uid(),
      logical_key: `midi.${Date.now().toString(36)}`,
      display_name: 'MIDI Port',
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
          id: uid(),
          logical_key: 'cc.1.7',
          alias: 'CC 7 Volume',
          direction: 'bidir',
          signal_kind: 'midi',
          min_value: 0,
          max_value: 1,
          modifiers: [],
        },
        {
          id: uid(),
          logical_key: 'note.1.60',
          alias: 'Note C4',
          direction: 'bidir',
          signal_kind: 'trigger',
          min_value: 0,
          max_value: 1,
          modifiers: [],
        },
      ],
      ui: { x, y, collapsed: false },
      group_id: null,
      manual_override: {},
    }),
  });

  registerNodeType({
    kindType: 'WebSocketEndpoint',
    label: 'WebSocket Bridge',
    shortBadge: 'WS',
    category: 'protocol',
    primarySignal: 'control',
    description: 'Server or client WebSocket endpoint with JSON, Text, Binary & OSC-over-WS.',
    milestoneTag: 'M1',
    createDevice: ({ x, y }) => ({
      id: uid(),
      logical_key: `ws.${Date.now().toString(36)}`,
      display_name: 'WebSocket Bridge',
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
          id: uid(),
          logical_key: '/ws/value',
          alias: 'WS Value',
          direction: 'bidir',
          signal_kind: 'control',
          min_value: 0,
          max_value: 1,
          modifiers: [],
        },
      ],
      ui: { x, y, collapsed: false },
      group_id: null,
      manual_override: {},
    }),
  });

  registerNodeType({
    kindType: 'UdpEndpoint',
    label: 'Raw UDP Endpoint',
    shortBadge: 'UDP',
    category: 'protocol',
    primarySignal: 'byte_stream',
    description: 'Unicast, multicast & broadcast UDP with UTF-8, JSON, CSV & Hex framing.',
    milestoneTag: 'M1',
    createDevice: ({ x, y }) => ({
      id: uid(),
      logical_key: `udp.${Date.now().toString(36)}`,
      display_name: 'Raw UDP Node',
      kind: {
        type: 'UdpEndpoint',
        bind_addr: '0.0.0.0:7000',
        target_addr: '127.0.0.1:7001',
        multicast_group: null,
        broadcast: false,
        format: 'json',
      },
      fingerprint: { endpoint: '0.0.0.0:7000' },
      status: { state: 'online' },
      ports: [
        {
          id: uid(),
          logical_key: 'col.0',
          alias: 'Channel 0',
          direction: 'bidir',
          signal_kind: 'control',
          min_value: 0,
          max_value: 1,
          modifiers: [],
        },
      ],
      ui: { x, y, collapsed: false },
      group_id: null,
      manual_override: {},
    }),
  });

  registerNodeType({
    kindType: 'VirtualControl',
    label: 'Virtual Control Pad',
    shortBadge: 'CTRL',
    category: 'utility',
    primarySignal: 'control',
    description: 'Manual knobs & triggers for live testing, macro banks, and show overrides.',
    milestoneTag: 'M1',
    createDevice: ({ x, y }) => ({
      id: uid(),
      logical_key: `ctrl.${Date.now().toString(36)}`,
      display_name: 'Virtual Control',
      kind: { type: 'VirtualControl' },
      status: { state: 'online' },
      ports: [
        {
          id: uid(),
          logical_key: 'knob.a',
          alias: 'Knob A',
          direction: 'out',
          signal_kind: 'control',
          min_value: 0,
          max_value: 1,
          modifiers: [],
        },
        {
          id: uid(),
          logical_key: 'btn.a',
          alias: 'Trigger A',
          direction: 'out',
          signal_kind: 'trigger',
          min_value: 0,
          max_value: 1,
          modifiers: [],
        },
      ],
      ui: { x, y, collapsed: false },
      group_id: null,
      manual_override: {},
    }),
  });

  // --- Extension Seam Templates for M2 / M3 / M4 ---

  registerNodeType({
    kindType: 'RtpMidiSession',
    label: 'RTPMIDI Network',
    shortBadge: 'RTP',
    category: 'protocol',
    primarySignal: 'midi',
    description: 'M2 seam: Native RFC 6295 RTPMIDI with mDNS-SD peer discovery.',
    milestoneTag: 'M2',
    createDevice: ({ x, y }) => ({
      id: uid(),
      logical_key: `rtpmidi.${Date.now().toString(36)}`,
      display_name: 'RTPMIDI Session',
      kind: {
        type: 'RtpMidiSession',
        session_name: 'Kontrolfreak LAN',
        control_port: 5004,
        remote_host: null,
        mdns_discover: true,
      },
      status: { state: 'offline_ghost', reason: 'M2 RTPMIDI transport module pending' },
      ports: [
        {
          id: uid(),
          logical_key: 'cc.1.1',
          alias: 'Mod Wheel CC1',
          direction: 'bidir',
          signal_kind: 'midi',
          min_value: 0,
          max_value: 1,
          modifiers: [],
        },
      ],
      ui: { x, y, collapsed: false },
      group_id: null,
      manual_override: {},
    }),
  });

  registerNodeType({
    kindType: 'NdiVideoNode',
    label: 'NDI Video Scaler',
    shortBadge: 'NDI',
    category: 'video',
    primarySignal: 'video',
    description: 'M2 seam: NDI Rx -> GPU crop/scale/FPS -> NDI Tx with control-driven params.',
    milestoneTag: 'M2',
    createDevice: ({ x, y }) => ({
      id: uid(),
      logical_key: `ndi.${Date.now().toString(36)}`,
      display_name: 'NDI Scaler',
      kind: {
        type: 'NdiVideoNode',
        source_name: null,
        output_name: 'Kontrolfreak Out',
        out_width: 1280,
        out_height: 720,
        out_fps: 30,
      },
      status: { state: 'offline_ghost', reason: 'M2 NDI Runtime module pending' },
      ports: [
        {
          id: uid(),
          logical_key: 'ndi.crop_x',
          alias: 'Crop X',
          direction: 'in',
          signal_kind: 'control',
          min_value: 0,
          max_value: 1,
          modifiers: [],
        },
        {
          id: uid(),
          logical_key: 'ndi.video_out',
          alias: 'NDI Out',
          direction: 'out',
          signal_kind: 'video',
          modifiers: [],
        },
      ],
      ui: { x, y, collapsed: false },
      group_id: null,
      manual_override: {},
    }),
  });

  registerNodeType({
    kindType: 'HidDevice',
    label: 'HID Learn Device',
    shortBadge: 'HID',
    category: 'hardware',
    primarySignal: 'sensor',
    description: 'M3 seam: Hot-plug USB/BT HID peripheral learned via 2-tier diff engine.',
    milestoneTag: 'M3',
    createDevice: ({ x, y }) => ({
      id: uid(),
      logical_key: `hid.${Date.now().toString(36)}`,
      display_name: 'HID Peripheral',
      kind: { type: 'HidDevice', device_preset_id: null, vid: null, pid: null },
      status: { state: 'offline_ghost', reason: 'M3 HID Learn engine pending' },
      ports: [
        {
          id: uid(),
          logical_key: 'axis.0',
          alias: 'Axis 0',
          direction: 'out',
          signal_kind: 'sensor',
          min_value: 0,
          max_value: 1,
          modifiers: [],
        },
      ],
      ui: { x, y, collapsed: false },
      group_id: null,
      manual_override: {},
    }),
  });

  registerNodeType({
    kindType: 'DepthSensor',
    label: 'Depth Sensor Sidecar',
    shortBadge: 'DEPTH',
    category: 'sensor',
    primarySignal: 'sensor',
    description: 'M4 seam: Crash-isolated RealSense/Orbbec/Kinect depth ROI & centroid extractor.',
    milestoneTag: 'M4',
    createDevice: ({ x, y }) => ({
      id: uid(),
      logical_key: `depth.${Date.now().toString(36)}`,
      display_name: 'Depth Camera',
      kind: {
        type: 'DepthSensor',
        vendor: 'realsense',
        sidecar_id: null,
        roi_min_z: 0.3,
        roi_max_z: 3.0,
      },
      status: { state: 'offline_ghost', reason: 'M4 Sensor Sidecar pending' },
      ports: [
        {
          id: uid(),
          logical_key: 'depth.closest_z',
          alias: 'Closest Z',
          direction: 'out',
          signal_kind: 'sensor',
          min_value: 0,
          max_value: 1,
          modifiers: [],
        },
        {
          id: uid(),
          logical_key: 'depth.ndi_out',
          alias: 'Depth Matte',
          direction: 'out',
          signal_kind: 'video',
          modifiers: [],
        },
      ],
      ui: { x, y, collapsed: false },
      group_id: null,
      manual_override: {},
    }),
  });
}
