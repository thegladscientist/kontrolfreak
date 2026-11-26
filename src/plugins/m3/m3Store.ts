import { create } from 'zustand';
import { useWorkspaceStore } from '../../store/workspaceStore';
import { bridge } from '../../lib/bridge';
import { m3Bridge } from './m3Bridge';
import {
  DEFAULT_RGB_CV_CONFIG,
  type CameraCvStepOutput,
  type HidDeviceDefinition,
  type HidDeviceInfo,
  type HotplugNotification,
  type LearnWizardSnapshot,
  type RgbCameraInfo,
  type RgbCvConfig,
  type SimulatedHidAction,
  type SimulatedHidProfile,
} from './m3Types';
import type { Modifier } from '../../types/kontrolfreak';

export interface M3HardwareStoreState {
  hidDevices: HidDeviceInfo[];
  devicePresets: HidDeviceDefinition[];
  learnSnapshot: LearnWizardSnapshot;
  selectedHidPath: string | null;
  lastSavedPresetJson: string | null;
  rgbCameras: RgbCameraInfo[];
  cvConfigs: Record<string, RgbCvConfig>;
  cvLatest: Record<string, CameraCvStepOutput>;

  refreshDevices: () => Promise<void>;
  handleHotplugNotifications: (notes: HotplugNotification[]) => Promise<void>;
  simulatePlugIn: (profile?: SimulatedHidProfile, stableId?: string | null) => Promise<HidDeviceInfo>;
  simulateUnplug: (path: string) => Promise<void>;
  simulateActuate: (path: string, action: SimulatedHidAction) => Promise<void>;

  openLearnWizardFor: (devicePath: string) => Promise<void>;
  startBaseline: (devicePath: string) => Promise<void>;
  pollLearnSamples: (count?: number) => Promise<void>;
  startCapture: () => Promise<void>;
  classifyCaptured: (aliasHint?: string) => Promise<void>;
  commitControl: (
    alias: string,
    modifiers: Modifier[],
    sensitivity?: number,
    wrap?: boolean
  ) => Promise<void>;
  saveAndAttachDevice: (presetName: string) => Promise<void>;
  importPresetJson: (json: string) => Promise<void>;
  resetLearn: () => Promise<void>;

  updateRgbCvConfig: (cameraId: string, config: RgbCvConfig) => Promise<void>;
  stepRgbCamera: (cameraId: string, deviceId?: string | null) => Promise<CameraCvStepOutput>;
}

const EMPTY_LEARN: LearnWizardSnapshot = {
  step: 'idle',
  device_path: null,
  active_report_id: null,
  baseline_samples: 0,
  capture_samples: 0,
  resting_bytes: [],
  latest_bytes: [],
  noise_mask_bits: [],
  active_diff_bits: [],
  candidate: null,
  committed_controls: [],
};

export const useM3Store = create<M3HardwareStoreState>((set, get) => ({
  hidDevices: [],
  devicePresets: [],
  learnSnapshot: EMPTY_LEARN,
  selectedHidPath: null,
  lastSavedPresetJson: null,
  rgbCameras: [
    {
      id: 'sim://camera/studio-cv',
      name: 'Synthetic Studio RGB Camera',
      width: 160,
      height: 120,
      fps: 30,
      is_synthetic: true,
    },
  ],
  cvConfigs: {
    'sim://camera/studio-cv': { ...DEFAULT_RGB_CV_CONFIG },
  },
  cvLatest: {},

  refreshDevices: async () => {
    const [hidDevices, devicePresets, rgbCameras] = await Promise.all([
      m3Bridge.listHidDevices(),
      m3Bridge.listDevicePresets(),
      m3Bridge.listRgbCameras(),
    ]);
    set({ hidDevices, devicePresets, rgbCameras });
  },

  handleHotplugNotifications: async (notes) => {
    if (notes.length === 0) return;
    const ws = useWorkspaceStore.getState();

    // Sync workspace graph if any OfflineGhost devices were ghosted or auto-rebound
    const graphMutated = notes.some(
      (n) => n.auto_rebound_device_id || n.ghosted_device_ids.length > 0
    );
    if (graphMutated) {
      const freshGraph = await bridge.getWorkspace();
      useWorkspaceStore.setState({ graph: freshGraph });
    }

    for (const note of notes) {
      if (note.event.event === 'connected') {
        const info = note.event.info;
        const name =
          info.product_name ||
          `HID 0x${info.vid.toString(16).padStart(4, '0')}:0x${info.pid.toString(16).padStart(4, '0')}`;

        if (note.auto_rebound_device_id) {
          ws.pushToast({
            title: `HID Auto-Rebound: ${name}`,
            description: `Matched "${note.recognized_preset_name ?? 'Saved Preset'}" — Offline Ghost node is back Online.`,
            accent: 'emerald',
          });
        } else if (note.recognized_preset_name) {
          ws.pushToast({
            title: `Recognized HID: ${name}`,
            description: `Matched saved preset "${note.recognized_preset_name}".`,
            accent: 'emerald',
            actionLabel: 'Learn',
            onAction: () => {
              void get().openLearnWizardFor(info.path);
            },
          });
        } else {
          // Non-intrusive top-right hot-plug toast via ToastHost for unknown HID peripherals
          ws.pushToast({
            title: `New HID Device Detected: ${name}`,
            description: `VID 0x${info.vid.toString(16).padStart(4, '0')} · PID 0x${info.pid.toString(16).padStart(4, '0')} — Run 4-step Learn Wizard?`,
            accent: 'emerald',
            actionLabel: 'Learn',
            onAction: () => {
              void get().openLearnWizardFor(info.path);
            },
          });
        }
      } else if (note.event.event === 'disconnected') {
        ws.pushToast({
          title: 'HID Device Unplugged',
          description:
            note.ghosted_device_ids.length > 0
              ? 'Node entered Offline Ghost mode with wires preserved. Replug to auto-rebind.'
              : 'Disconnected HID peripheral removed from active bus.',
          accent: 'amber',
        });
      }
    }
  },

  simulatePlugIn: async (profile = 'unknown_mystery_dial', stableId = null) => {
    const res = await m3Bridge.simulateHidPlug(profile, stableId);
    const hidDevices = await m3Bridge.listHidDevices();
    set({ hidDevices, selectedHidPath: res.info.path });
    await get().handleHotplugNotifications(res.notifications);
    return res.info;
  },

  simulateUnplug: async (path) => {
    const notes = await m3Bridge.simulateHidUnplug(path);
    const hidDevices = await m3Bridge.listHidDevices();
    set((s) => ({
      hidDevices,
      selectedHidPath: s.selectedHidPath === path ? hidDevices[0]?.path ?? null : s.selectedHidPath,
    }));
    await get().handleHotplugNotifications(notes);
  },

  simulateActuate: async (path, action) => {
    await m3Bridge.simulateHidActuate(path, action);
    const snap = await m3Bridge.learnGetSnapshot();
    set({ learnSnapshot: snap });
  },

  openLearnWizardFor: async (devicePath) => {
    set({ selectedHidPath: devicePath });
    useWorkspaceStore.getState().setActiveDrawerTab('hid-learn');
    const snap = await m3Bridge.learnStartBaseline(devicePath);
    const sampled = await m3Bridge.learnPollSamples(24);
    set({ learnSnapshot: sampled ?? snap });
  },

  startBaseline: async (devicePath) => {
    await m3Bridge.learnStartBaseline(devicePath);
    const sampled = await m3Bridge.learnPollSamples(32);
    set({ selectedHidPath: devicePath, learnSnapshot: sampled });
  },

  pollLearnSamples: async (count = 10) => {
    const snap = await m3Bridge.learnPollSamples(count);
    set({ learnSnapshot: snap });
  },

  startCapture: async () => {
    const snap = await m3Bridge.learnStartCapture();
    set({ learnSnapshot: snap });
  },

  classifyCaptured: async (aliasHint) => {
    try {
      const snap = await m3Bridge.learnClassify(aliasHint ?? null);
      set({ learnSnapshot: snap });
    } catch (err) {
      useWorkspaceStore.getState().pushToast({
        title: 'No Control Movement Detected',
        description: err instanceof Error ? err.message : String(err),
        accent: 'rose',
      });
    }
  },

  commitControl: async (alias, modifiers, sensitivity, wrap) => {
    const snap = await m3Bridge.learnCommitControl(alias, modifiers, sensitivity, wrap);
    set({ learnSnapshot: snap });
    useWorkspaceStore.getState().pushToast({
      title: `Learned "${alias}"`,
      description: 'Control added to device map. Learn another control or save preset in Step 4.',
      accent: 'emerald',
    });
  },

  saveAndAttachDevice: async (presetName) => {
    const res = await m3Bridge.saveAndAttachLearnedDevice(presetName, 140, 220);
    const [devicePresets, freshGraph] = await Promise.all([
      m3Bridge.listDevicePresets(),
      bridge.getWorkspace(),
    ]);
    useWorkspaceStore.setState({
      graph: freshGraph,
      selection: { kind: 'device', id: res.device.id },
    });
    set({
      devicePresets,
      lastSavedPresetJson: res.preset_json,
    });
    useWorkspaceStore.getState().pushToast({
      title: `Saved "${res.definition.name}" (.kfdevice.json)`,
      description: `${res.definition.controls.length} learned control(s) attached live to canvas.`,
      accent: 'emerald',
    });
  },

  importPresetJson: async (json) => {
    const def = await m3Bridge.importHidDevicePreset(json);
    const devicePresets = await m3Bridge.listDevicePresets();
    set({ devicePresets });
    useWorkspaceStore.getState().pushToast({
      title: `Loaded HID Preset "${def.name}"`,
      description: `Auto-recognition armed for VID 0x${def.vendor_id.toString(16)} : PID 0x${def.product_id.toString(16)}.`,
      accent: 'emerald',
    });
  },

  resetLearn: async () => {
    const snap = await m3Bridge.learnReset();
    set({ learnSnapshot: snap });
  },

  updateRgbCvConfig: async (cameraId, config) => {
    const saved = await m3Bridge.updateRgbCvConfig(cameraId, config);
    set((s) => ({
      cvConfigs: { ...s.cvConfigs, [cameraId]: saved },
    }));
  },

  stepRgbCamera: async (cameraId, deviceId) => {
    const out = await m3Bridge.stepRgbCameraCv(cameraId, deviceId);
    set((s) => ({
      cvLatest: { ...s.cvLatest, [cameraId]: out },
    }));
    return out;
  },
}));
