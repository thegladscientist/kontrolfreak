//! Milestone 2 self-contained Hybrid UI plugin:
//! Registers `RtpMidiSession` and `NdiVideoNode` in `nodeRegistry`, `sidebarSections`,
//! `inspectorPanels`, and `commandActions`.

import {
  getNodeDefinition,
  registerCommandAction,
  registerInspectorPanel,
  registerNodeType,
  registerSidebarSection,
} from '../registries';
import { useWorkspaceStore } from '../store/workspaceStore';
import { M2SidebarSection } from './m2/M2SidebarSection';
import { NdiInspectorPanel } from './m2/NdiInspectorPanel';
import { NdiNodeBody } from './m2/NdiNodeBody';
import { m2Bridge, SYNTHETIC_NDI_SOURCES } from './m2/m2Bridge';
import { RtpMidiInspectorPanel } from './m2/RtpMidiInspectorPanel';
import { RtpMidiNodeBody } from './m2/RtpMidiNodeBody';

function uid(): string {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
    return crypto.randomUUID();
  }
  return `m2-${Date.now()}-${Math.random().toString(36).slice(2, 9)}`;
}

let m2Registered = false;

export function registerM2VideoRtpMidiPlugin(): void {
  if (m2Registered) return;
  m2Registered = true;

  // 1. Upgrade `RtpMidiSession` node template with live RFC 6295 tile body & ports
  registerNodeType({
    kindType: 'RtpMidiSession',
    label: 'RTPMIDI Network',
    shortBadge: 'RTP',
    category: 'protocol',
    primarySignal: 'midi',
    description:
      'Pure-Rust RFC 6295 RTPMIDI (dual UDP N/N+1), 3-way clock sync & mDNS-SD peer discovery.',
    milestoneTag: 'M2',
    renderBody: RtpMidiNodeBody,
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
      fingerprint: { endpoint: 'rtpmidi://0.0.0.0:5004' },
      status: { state: 'online' },
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
        {
          id: uid(),
          logical_key: 'cc.1.7',
          alias: 'Volume CC7',
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

  // 2. Upgrade `NdiVideoNode` template with live <canvas> preview tile & modulatable ports
  registerNodeType({
    kindType: 'NdiVideoNode',
    label: 'NDI Video Scaler',
    shortBadge: 'NDI',
    category: 'video',
    primarySignal: 'video',
    description:
      'Dynamic NDI v6 + Test Patterns: arbitrary WxH resize, crop/zoom, FPS pacing & live control modulation.',
    milestoneTag: 'M2',
    renderBody: NdiNodeBody,
    createDevice: ({ x, y }) => ({
      id: uid(),
      logical_key: `ndi.${Date.now().toString(36)}`,
      display_name: 'NDI Video Scaler',
      kind: {
        type: 'NdiVideoNode',
        source_name: SYNTHETIC_NDI_SOURCES[0].name,
        output_name: 'Kontrolfreak Out',
        out_width: 1280,
        out_height: 720,
        out_fps: 30,
      },
      fingerprint: { endpoint: 'ndi://Kontrolfreak Out' },
      status: { state: 'online' },
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
          logical_key: 'ndi.crop_y',
          alias: 'Crop Y',
          direction: 'in',
          signal_kind: 'control',
          min_value: 0,
          max_value: 1,
          modifiers: [],
        },
        {
          id: uid(),
          logical_key: 'ndi.zoom',
          alias: 'Zoom',
          direction: 'in',
          signal_kind: 'control',
          min_value: 0,
          max_value: 1,
          modifiers: [],
        },
        {
          id: uid(),
          logical_key: 'ndi.opacity',
          alias: 'Opacity',
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

  // 3. Register Sidebar Section for NDI & RTPMIDI discovery
  registerSidebarSection({
    id: 'm2-media-net',
    label: 'NDI/RTP',
    order: 22,
    iconName: 'video',
    component: M2SidebarSection,
  });

  // 4. Register Inspector Panels for NdiVideoNode & RtpMidiSession
  registerInspectorPanel({
    id: 'm2-ndi-video-panel',
    title: 'NDI Video Pipeline & Scaler',
    order: 10,
    target: 'NdiVideoNode',
    component: NdiInspectorPanel,
  });

  registerInspectorPanel({
    id: 'm2-rtpmidi-session-panel',
    title: 'RTPMIDI Session & Peers',
    order: 10,
    target: 'RtpMidiSession',
    component: RtpMidiInspectorPanel,
  });

  // 5. Register Command Palette Actions (Cmd/Ctrl+K)
  registerCommandAction({
    id: 'm2.spawn-ndi-scaler',
    label: 'Add NDI Video Scaler (Test Pattern Ready)',
    group: 'M2 · Video & Network MIDI',
    keywords: ['ndi', 'video', 'scaler', 'crop', 'resize', 'smpte'],
    run: async () => {
      const def = getNodeDefinition('NdiVideoNode');
      if (!def) return;
      const offset = Math.round(Math.random() * 140);
      await useWorkspaceStore.getState().addDevice(def.createDevice({ x: 240 + offset, y: 120 + offset }));
    },
  });

  registerCommandAction({
    id: 'm2.spawn-rtpmidi-session',
    label: 'Add RTPMIDI Network Session (RFC 6295)',
    group: 'M2 · Video & Network MIDI',
    keywords: ['rtpmidi', 'applemidi', 'mdns', 'bonjour', 'network', 'midi'],
    run: async () => {
      const def = getNodeDefinition('RtpMidiSession');
      if (!def) return;
      const offset = Math.round(Math.random() * 140);
      const added = await useWorkspaceStore
        .getState()
        .addDevice(def.createDevice({ x: 180 + offset, y: 200 + offset }));
      await m2Bridge.startRtpMidiSession(added.id, added);
    },
  });

  registerCommandAction({
    id: 'm2.open-ndi-sidebar',
    label: 'Discover NDI Sources & RTPMIDI Peers',
    group: 'M2 · Video & Network MIDI',
    keywords: ['discover', 'scan', 'ndi', 'rtpmidi', 'peers', 'mdns'],
    run: () => {
      useWorkspaceStore.getState().setActiveSidebarSection('m2-media-net');
    },
  });
}

export * from './m2/m2Bridge';
