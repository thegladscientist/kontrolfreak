import React, { useState } from 'react';
import {
  Camera,
  Cpu,
  Download,
  PlugZap,
  RefreshCw,
  Sparkles,
  Unplug,
  Usb,
} from 'lucide-react';
import {
  registerCommandAction,
  registerDrawerTab,
  registerInspectorPanel,
  registerNodeType,
  registerSidebarSection,
} from '../registries';
import { useWorkspaceStore } from '../store/workspaceStore';
import type { Device } from '../types/kontrolfreak';
import { HidLearnWizardTab } from './m3/HidLearnWizardTab';
import { RgbCameraInspectorPanel, RgbCameraNodeBody } from './m3/RgbCameraWidgets';
import { useM3Store } from './m3/m3Store';
import type { SimulatedHidProfile } from './m3/m3Types';

function uid(): string {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
    return crypto.randomUUID();
  }
  return `m3-${Date.now()}-${Math.random().toString(36).slice(2, 9)}`;
}

/** Custom canvas node body for `HidDevice` nodes. */
const HidDeviceNodeBody: React.FC<{ device: Device; calmMode: boolean }> = ({ device }) => {
  const hidDevices = useM3Store((s) => s.hidDevices);
  const openLearnWizardFor = useM3Store((s) => s.openLearnWizardFor);
  const simulateActuate = useM3Store((s) => s.simulateActuate);
  const [simVal, setSimVal] = useState(128);

  const vid = device.kind.type === 'HidDevice' ? device.kind.vid : device.fingerprint?.vid;
  const pid = device.kind.type === 'HidDevice' ? device.kind.pid : device.fingerprint?.pid;
  const matchedBus = hidDevices.find(
    (d) =>
      (device.fingerprint?.endpoint && d.path === device.fingerprint.endpoint) ||
      (vid != null && pid != null && d.vid === vid && d.pid === pid)
  );

  return (
    <div className="space-y-1.5 text-[10px]">
      <div className="flex items-center justify-between gap-1">
        <span className="font-mono text-emerald-300">
          {vid != null && pid != null
            ? `0x${vid.toString(16).padStart(4, '0')}:0x${pid.toString(16).padStart(4, '0')}`
            : 'HID Learn'}
        </span>
        {matchedBus && (
          <button
            onClick={(e) => {
              e.stopPropagation();
              void openLearnWizardFor(matchedBus.path);
            }}
            className="nodrag px-1.5 py-0.5 rounded bg-emerald-500/20 hover:bg-emerald-500/30 border border-emerald-500/40 text-emerald-200 flex items-center gap-1"
          >
            <Sparkles className="w-2.5 h-2.5" />
            Learn
          </button>
        )}
      </div>
      {matchedBus && matchedBus.path.startsWith('sim://') && device.status.state === 'online' && (
        <div className="flex items-center gap-1.5">
          <span className="text-studio-muted">Sim</span>
          <input
            type="range"
            min={0}
            max={255}
            value={simVal}
            onMouseDown={(e) => e.stopPropagation()}
            onChange={(e) => {
              const v = Number(e.target.value);
              setSimVal(v);
              void simulateActuate(matchedBus.path, { control: 'set_fader_u8', value: v });
            }}
            className="nodrag flex-1 accent-emerald-400 h-1 cursor-pointer"
            aria-label={`Simulated HID fader for ${device.display_name}`}
          />
        </div>
      )}
    </div>
  );
};

/** Inspector panel registered for `HidDevice` nodes. */
const HidDeviceInspectorPanel: React.FC<{ device?: Device }> = ({ device }) => {
  const hidDevices = useM3Store((s) => s.hidDevices);
  const devicePresets = useM3Store((s) => s.devicePresets);
  const openLearnWizardFor = useM3Store((s) => s.openLearnWizardFor);
  const simulateUnplug = useM3Store((s) => s.simulateUnplug);
  const simulatePlugIn = useM3Store((s) => s.simulatePlugIn);

  if (!device || device.kind.type !== 'HidDevice') return null;
  const vid = device.kind.vid ?? device.fingerprint?.vid ?? null;
  const pid = device.kind.pid ?? device.fingerprint?.pid ?? null;
  const devicePresetId = device.kind.device_preset_id;

  const connected = hidDevices.find(
    (d) =>
      (device.fingerprint?.endpoint && d.path === device.fingerprint.endpoint) ||
      (vid != null && pid != null && d.vid === vid && d.pid === pid)
  );
  const preset = devicePresets.find(
    (p) =>
      p.preset_id === devicePresetId ||
      (vid != null && pid != null && p.vendor_id === vid && p.product_id === pid)
  );

  const stableSerialSuffix = device.fingerprint?.serial?.split('-').pop() ?? '01';
  const replugProfile: SimulatedHidProfile =
    vid === 0x256f
      ? 'quirky_imu_gamepad'
      : vid === 0x16c0
      ? 'studio_fader_wing'
      : 'unknown_mystery_dial';

  return (
    <div className="space-y-2.5 text-xs">
      <div className="p-2 rounded-lg bg-studio-bg border border-studio-border space-y-1 font-mono text-[10px]">
        <div className="flex justify-between">
          <span className="text-studio-muted">USB VID:PID</span>
          <span className="text-emerald-300">
            {vid != null ? `0x${vid.toString(16).padStart(4, '0')}` : '--'}:
            {pid != null ? `0x${pid.toString(16).padStart(4, '0')}` : '--'}
          </span>
        </div>
        <div className="flex justify-between">
          <span className="text-studio-muted">Preset</span>
          <span className="text-slate-200 truncate max-w-[140px]">
            {preset?.name ?? 'Custom / Unsaved'}
          </span>
        </div>
        <div className="flex justify-between">
          <span className="text-studio-muted">Learned Controls</span>
          <span className="text-amber-300">{device.ports.length}</span>
        </div>
      </div>

      <div className="flex gap-1.5">
        {connected ? (
          <>
            <button
              onClick={() => void openLearnWizardFor(connected.path)}
              className="flex-1 flex items-center justify-center gap-1 py-1.5 rounded bg-emerald-500/20 hover:bg-emerald-500/30 border border-emerald-500/40 text-emerald-200 text-[11px] font-medium"
            >
              <Sparkles className="w-3.5 h-3.5" />
              Learn More
            </button>
            <button
              onClick={() => void simulateUnplug(connected.path)}
              className="px-2.5 py-1.5 rounded bg-amber-950/50 hover:bg-amber-900/50 border border-amber-500/40 text-amber-200 text-[11px] flex items-center gap-1"
              title="Simulate unplugging this HID device (enters Offline Ghost)"
            >
              <Unplug className="w-3.5 h-3.5" />
              Unplug
            </button>
          </>
        ) : (
          <button
            onClick={() => void simulatePlugIn(replugProfile, stableSerialSuffix)}
            className="w-full flex items-center justify-center gap-1.5 py-1.5 rounded bg-emerald-500/25 hover:bg-emerald-500/35 border border-emerald-400/50 text-emerald-100 text-[11px] font-medium"
          >
            <PlugZap className="w-3.5 h-3.5" />
            Simulate Replug (Auto-Rebind)
          </button>
        )}
      </div>
    </div>
  );
};

/** Sidebar section for M3 HID Hot-Plug, Learn Wizard quick launch, Presets, and RGB Camera CV. */
const M3HardwareSidebarSection: React.FC = () => {
  const hidDevices = useM3Store((s) => s.hidDevices);
  const devicePresets = useM3Store((s) => s.devicePresets);
  const simulatePlugIn = useM3Store((s) => s.simulatePlugIn);
  const simulateUnplug = useM3Store((s) => s.simulateUnplug);
  const openLearnWizardFor = useM3Store((s) => s.openLearnWizardFor);
  const refreshDevices = useM3Store((s) => s.refreshDevices);
  const addDevice = useWorkspaceStore((s) => s.addDevice);

  const [profile, setProfile] = useState<SimulatedHidProfile>('unknown_mystery_dial');

  const handleSpawnCamera = () => {
    void addDevice(createRgbCameraDevice({ x: 160, y: 310 }));
  };

  return (
    <div className="space-y-3 text-xs">
      <div className="flex items-center justify-between">
        <span className="text-[11px] font-semibold uppercase tracking-wider text-studio-muted">
          HID Hot-Plug &amp; Learn
        </span>
        <button
          onClick={() => void refreshDevices()}
          className="text-[11px] text-emerald-300 hover:text-emerald-200 flex items-center gap-1"
        >
          <RefreshCw className="w-3 h-3" />
          Scan
        </button>
      </div>

      {/* Quick Simulate Plug-In Unknown HID */}
      <div className="p-2 rounded-lg bg-studio-panel border border-studio-border space-y-1.5">
        <select
          value={profile}
          onChange={(e) => setProfile(e.target.value as SimulatedHidProfile)}
          className="w-full bg-studio-bg border border-studio-border rounded px-2 py-1 text-[11px] text-slate-200 outline-none"
        >
          <option value="unknown_mystery_dial">Unknown USB Rotary HID (Noisy IMU)</option>
          <option value="quirky_imu_gamepad">KF QuirkyPad Pro (16b LE/BE + IMU)</option>
          <option value="studio_fader_wing">KF StudioFader 4 (HID Descriptor)</option>
        </select>
        <button
          onClick={() => void simulatePlugIn(profile)}
          className="w-full flex items-center justify-center gap-1.5 py-1.5 rounded-md bg-emerald-500/20 hover:bg-emerald-500/30 border border-emerald-500/40 text-emerald-200 text-[11px] font-medium"
        >
          <Usb className="w-3.5 h-3.5" />
          Simulate Plug-In Unknown HID
        </button>
      </div>

      {/* Connected HID Peripherals */}
      <div className="space-y-1.5">
        <div className="text-[11px] text-emerald-400 font-medium">
          ● Connected HID ({hidDevices.length})
        </div>
        {hidDevices.length === 0 ? (
          <div className="text-[11px] text-studio-muted px-1">
            No HID peripherals active. Click above to simulate hot-plugging an unknown USB device.
          </div>
        ) : (
          hidDevices.map((dev) => (
            <div
              key={dev.path}
              className="p-2 rounded-md bg-studio-panel border border-studio-border space-y-1.5"
            >
              <div className="flex items-center justify-between gap-1">
                <span className="text-[11px] font-medium text-slate-100 truncate">
                  {dev.product_name ?? dev.path}
                </span>
                <span className="text-[9px] font-mono text-studio-muted shrink-0">
                  {dev.vid.toString(16).padStart(4, '0')}:{dev.pid.toString(16).padStart(4, '0')}
                </span>
              </div>
              <div className="flex gap-1">
                <button
                  onClick={() => void openLearnWizardFor(dev.path)}
                  className="flex-1 py-1 rounded bg-emerald-500/20 hover:bg-emerald-500/30 border border-emerald-500/40 text-emerald-200 text-[10px] font-medium flex items-center justify-center gap-1"
                >
                  <Sparkles className="w-3 h-3" />
                  Learn Controls
                </button>
                <button
                  onClick={() => void simulateUnplug(dev.path)}
                  className="px-2 py-1 rounded bg-studio-bg hover:bg-studio-elevated border border-studio-border text-amber-300 text-[10px]"
                  title="Simulate Unplug"
                >
                  <Unplug className="w-3 h-3" />
                </button>
              </div>
            </div>
          ))
        )}
      </div>

      {/* Saved .kfdevice.json Library */}
      <div className="pt-2 border-t border-studio-border space-y-1.5">
        <div className="text-[11px] font-semibold uppercase tracking-wider text-studio-muted flex items-center gap-1">
          <Download className="w-3 h-3 text-emerald-400" />
          Saved HID Presets ({devicePresets.length})
        </div>
        {devicePresets.length === 0 ? (
          <div className="text-[10px] text-studio-muted">
            Finish Step 4 in the HID Learn Wizard to save reusable `.kfdevice.json` maps.
          </div>
        ) : (
          devicePresets.map((p) => (
            <div
              key={p.preset_id}
              className="px-2 py-1 rounded bg-studio-bg border border-studio-border flex items-center justify-between text-[10px]"
            >
              <span className="truncate text-slate-200">{p.name}</span>
              <span className="font-mono text-emerald-300 shrink-0">
                {p.controls.length} ctrls
              </span>
            </div>
          ))
        )}
      </div>

      {/* RGB Camera CV Quick Spawn */}
      <div className="pt-2 border-t border-studio-border space-y-1.5">
        <div className="text-[11px] font-semibold uppercase tracking-wider text-studio-muted flex items-center gap-1">
          <Cpu className="w-3 h-3 text-cyan-400" />
          RGB Vision-to-Control
        </div>
        <button
          onClick={handleSpawnCamera}
          className="w-full flex items-center justify-center gap-1.5 py-1.5 rounded-md bg-cyan-500/20 hover:bg-cyan-500/30 border border-cyan-500/40 text-cyan-200 text-[11px] font-medium"
        >
          <Camera className="w-3.5 h-3.5" />
          Add RGB Camera CV Node
        </button>
      </div>
    </div>
  );
};

export function createHidDeviceNode(position: { x: number; y: number }): Device {
  return {
    id: uid(),
    logical_key: `hid.${Date.now().toString(36)}`,
    display_name: 'HID Learn Device',
    kind: { type: 'HidDevice', device_preset_id: null, vid: null, pid: null },
    status: { state: 'online' },
    ports: [
      {
        id: uid(),
        logical_key: 'hid.axis.0',
        alias: 'Learned Axis 0',
        direction: 'out',
        signal_kind: 'sensor',
        min_value: 0,
        max_value: 1,
        modifiers: [],
      },
      {
        id: uid(),
        logical_key: 'hid.btn.0',
        alias: 'Learned Button 0',
        direction: 'out',
        signal_kind: 'trigger',
        min_value: 0,
        max_value: 1,
        modifiers: [],
      },
    ],
    ui: { x: position.x, y: position.y, collapsed: false },
    group_id: null,
    manual_override: {},
  };
}

export function createRgbCameraDevice(position: { x: number; y: number }): Device {
  return {
    id: uid(),
    logical_key: `cam.${Date.now().toString(36)}`,
    display_name: 'RGB Camera CV',
    kind: {
      type: 'RgbCamera',
      camera_id: 'sim://camera/studio-cv',
      extractor_kind: 'kf-rgb-cv-pipeline',
    },
    fingerprint: { endpoint: 'sim://camera/studio-cv', port_name: 'Synthetic Studio RGB Camera' },
    status: { state: 'online' },
    ports: [
      {
        id: uid(),
        logical_key: 'cam.luma.global',
        alias: 'Global Brightness',
        direction: 'out',
        signal_kind: 'sensor',
        min_value: 0,
        max_value: 1,
        modifiers: [],
      },
      {
        id: uid(),
        logical_key: 'cam.motion.energy',
        alias: 'Motion Energy',
        direction: 'out',
        signal_kind: 'sensor',
        min_value: 0,
        max_value: 1,
        modifiers: [],
      },
      {
        id: uid(),
        logical_key: 'cam.blob.x',
        alias: 'Blob Centroid X',
        direction: 'out',
        signal_kind: 'sensor',
        min_value: 0,
        max_value: 1,
        modifiers: [],
      },
      {
        id: uid(),
        logical_key: 'cam.blob.y',
        alias: 'Blob Centroid Y',
        direction: 'out',
        signal_kind: 'sensor',
        min_value: 0,
        max_value: 1,
        modifiers: [],
      },
      {
        id: uid(),
        logical_key: 'cam.blob.area',
        alias: 'Blob Area',
        direction: 'out',
        signal_kind: 'sensor',
        min_value: 0,
        max_value: 1,
        modifiers: [],
      },
      {
        id: uid(),
        logical_key: 'cam.zone.0.trigger',
        alias: 'Motion Zone 0',
        direction: 'out',
        signal_kind: 'trigger',
        min_value: 0,
        max_value: 1,
        modifiers: [],
      },
    ],
    ui: { x: position.x, y: position.y, collapsed: false },
    group_id: null,
    manual_override: {},
  };
}

let m3Registered = false;

/**
 * Registers all Milestone 3 UI extensions into Kontrolfreak's registries:
 * - `nodeRegistry`: `HidDevice` (upgraded M3) & `RgbCamera` (with live CV preview body)
 * - `sidebarSections`: `HID/Cam` hot-plug & preset panel
 * - `inspectorPanels`: `HidDevice` & `RgbCamera` inspector panels
 * - `drawerTabs`: 4-step `HID Learn` wizard tab
 * - `commandActions`: quick Simulate Plug-In, Open Learn Wizard, and Spawn RGB Camera commands
 */
export function registerM3HardwareLearnPlugin(): void {
  if (m3Registered) return;
  m3Registered = true;

  registerNodeType({
    kindType: 'HidDevice',
    label: 'HID Learn Device',
    shortBadge: 'HID',
    category: 'hardware',
    primarySignal: 'sensor',
    description:
      'Hot-plug USB/BT HID peripheral learned via 2-tier descriptor & noise-masked diff engine.',
    milestoneTag: 'M3',
    createDevice: createHidDeviceNode,
    renderBody: HidDeviceNodeBody,
  });

  registerNodeType({
    kindType: 'RgbCamera',
    label: 'RGB Camera CV',
    shortBadge: 'CAM',
    category: 'sensor',
    primarySignal: 'sensor',
    description:
      'Lightweight CPU CV extractors: luma, frame-diff motion energy, dominant color, blob centroid & zones.',
    milestoneTag: 'M3',
    createDevice: createRgbCameraDevice,
    renderBody: RgbCameraNodeBody,
  });

  registerSidebarSection({
    id: 'm3-hardware',
    label: 'HID',
    order: 24,
    iconName: 'cpu',
    component: M3HardwareSidebarSection,
  });

  registerInspectorPanel({
    id: 'm3-hid-inspector',
    title: 'HID Device & Auto-Rebind',
    order: 10,
    target: 'HidDevice',
    component: HidDeviceInspectorPanel,
  });

  registerInspectorPanel({
    id: 'm3-rgb-camera-inspector',
    title: 'RGB Camera CV Extractors',
    order: 10,
    target: 'RgbCamera',
    component: RgbCameraInspectorPanel,
  });

  registerDrawerTab({
    id: 'hid-learn',
    label: 'HID Learn',
    order: 30,
    badge: 'M3',
    component: HidLearnWizardTab,
  });

  registerCommandAction({
    id: 'm3-sim-plug-unknown-hid',
    label: 'Simulate Plug-In Unknown HID Device',
    group: 'Hardware (M3)',
    keywords: ['hid', 'usb', 'hotplug', 'learn', 'simulate'],
    run: async () => {
      await useM3Store.getState().simulatePlugIn('unknown_mystery_dial');
    },
  });

  registerCommandAction({
    id: 'm3-sim-plug-quirky-imu',
    label: 'Simulate Plug-In QuirkyPad Pro (Noisy IMU)',
    group: 'Hardware (M3)',
    keywords: ['hid', 'imu', 'gamepad', 'encoder', 'simulate'],
    run: async () => {
      await useM3Store.getState().simulatePlugIn('quirky_imu_gamepad');
    },
  });

  registerCommandAction({
    id: 'm3-open-hid-learn',
    label: 'Open HID Learn Wizard (4-Step)',
    group: 'Hardware (M3)',
    keywords: ['hid', 'learn', 'wizard', 'baseline'],
    run: () => {
      useWorkspaceStore.getState().setActiveDrawerTab('hid-learn');
    },
  });

  registerCommandAction({
    id: 'm3-spawn-rgb-camera',
    label: 'Add RGB Camera CV Node',
    group: 'Hardware (M3)',
    keywords: ['camera', 'rgb', 'cv', 'vision', 'motion', 'blob'],
    run: async () => {
      await useWorkspaceStore.getState().addDevice(createRgbCameraDevice({ x: 200, y: 260 }));
    },
  });
}
