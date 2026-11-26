import { describe, expect, it } from 'vitest';
import {
  getInspectorPanelsFor,
  getNodeDefinition,
  listCommandActions,
  listDrawerTabs,
  listSidebarSections,
} from '../registries';
import {
  createRgbCameraDevice,
  registerM3HardwareLearnPlugin,
} from '../plugins/m3HardwareLearn';
import { BrowserM3HardwareEngine, renderSyntheticCameraFrame } from '../plugins/m3/m3Bridge';
import { useM3Store } from '../plugins/m3/m3Store';
import { DEFAULT_RGB_CV_CONFIG } from '../plugins/m3/m3Types';
import { useWorkspaceStore } from '../store/workspaceStore';

describe('M3 Hardware Learn & RGB Camera Plugin Registrations', () => {
  it('registers HidDevice, RgbCamera, sidebar, inspector panels, drawer tab, and commands', () => {
    registerM3HardwareLearnPlugin();

    const hidDef = getNodeDefinition('HidDevice');
    const camDef = getNodeDefinition('RgbCamera');
    expect(hidDef).toBeDefined();
    expect(hidDef?.renderBody).toBeDefined();
    expect(camDef).toBeDefined();
    expect(camDef?.renderBody).toBeDefined();

    expect(listSidebarSections().some((s) => s.id === 'm3-hardware')).toBe(true);
    expect(getInspectorPanelsFor('HidDevice').some((p) => p.id === 'm3-hid-inspector')).toBe(true);
    expect(getInspectorPanelsFor('RgbCamera').some((p) => p.id === 'm3-rgb-camera-inspector')).toBe(
      true
    );
    expect(listDrawerTabs().some((t) => t.id === 'hid-learn')).toBe(true);
    expect(listCommandActions().some((a) => a.id === 'm3-sim-plug-unknown-hid')).toBe(true);
  });

  it('triggers non-intrusive ToastHost hot-plug notification when unknown HID connects', async () => {
    registerM3HardwareLearnPlugin();
    await useWorkspaceStore.getState().bootstrap();

    const info = await useM3Store.getState().simulatePlugIn('unknown_mystery_dial', 'toast01');
    expect(info.vid).toBe(0x1d50);

    const toasts = useWorkspaceStore.getState().toasts;
    const hit = toasts.find((t) => t.title.includes('New HID Device Detected'));
    expect(hit).toBeDefined();
    expect(hit?.actionLabel).toBe('Learn');

    // Clean up simulated plug
    await useM3Store.getState().simulateUnplug(info.path);
  });
});

describe('M3 Browser Loopback HID Learn Wizard & Auto-Rebind', () => {
  it('masks noisy IMU bytes, learns 8b/16b/button/encoder, saves .kfdevice.json, and auto-rebinds', () => {
    const eng = new BrowserM3HardwareEngine();
    const { info } = eng.simulatePlugIn('quirky_imu_gamepad', 'ui01');
    const path = info.path;

    // Step 1: Baseline idle sampling
    eng.learnStartBaseline(path);
    eng.learnPollSamples(32);
    const snapBase = eng.learnStartCapture();
    expect(snapBase.noise_mask_bits[1]).toBe(0xff); // rolling counter
    expect(snapBase.noise_mask_bits[2]).toBe(0xff); // noisy IMU
    expect(snapBase.noise_mask_bits[6]).toBe(0); // clean 8-bit fader

    // Step 2: Capture 8-bit fader
    for (let v = 0; v <= 255; v += 25) {
      eng.simulateActuate(path, { control: 'set_fader_u8', value: v });
    }
    eng.simulateActuate(path, { control: 'set_fader_u8', value: 0 });
    const c1 = eng.learnClassify('Master Fader');
    expect(c1.candidate?.control_type).toBe('axis8');
    expect(c1.candidate?.byte_offset).toBe(6);
    eng.learnCommitControl('Master Fader', [{ type: 'Invert' }]);

    // Capture 16-bit signed LE joystick axis
    eng.learnStartCapture();
    for (let step = -15; step <= 15; step++) {
      eng.simulateActuate(path, { control: 'set_axis_i16_le', value: step * 2000 });
    }
    eng.simulateActuate(path, { control: 'set_axis_i16_le', value: 0 });
    const c2 = eng.learnClassify('Joy X');
    expect(c2.candidate?.control_type).toBe('axis16');
    expect(c2.candidate?.signed).toBe(true);
    expect(c2.candidate?.little_endian).toBe(true);
    expect(c2.candidate?.byte_offset).toBe(7);
    eng.learnCommitControl('Joy X', []);

    // Capture Momentary Button A
    eng.learnStartCapture();
    eng.learnPollSamples(2);
    eng.simulateActuate(path, { control: 'set_button_a', pressed: true });
    eng.learnPollSamples(2);
    eng.simulateActuate(path, { control: 'set_button_a', pressed: false });
    eng.learnPollSamples(2);
    const c3 = eng.learnClassify('Btn A');
    expect(c3.candidate?.control_type).toBe('button');
    expect(c3.candidate?.byte_offset).toBe(11);
    expect(c3.candidate?.bit_mask).toBe(0x01);
    eng.learnCommitControl('Btn A', []);

    // Capture Relative Encoder
    eng.learnStartCapture();
    eng.learnPollSamples(2);
    for (const d of [1, 2, -1, -2]) {
      eng.simulateActuate(path, { control: 'pulse_encoder', delta: d });
      eng.learnPollSamples(1);
    }
    const c4 = eng.learnClassify('Dial Enc');
    expect(c4.candidate?.control_type).toBe('relative_encoder');
    expect(c4.candidate?.byte_offset).toBe(12);
    eng.learnCommitControl('Dial Enc', [], 0.05, false);

    // Step 4: Save preset & attach to workspace
    const saved = eng.saveAndAttachLearnedDevice('QuirkyPad UI Preset');
    expect(saved.definition.controls).toHaveLength(4);
    expect(saved.device.ports).toHaveLength(4);

    // Import preset JSON cleanly
    const reimported = eng.importHidDevicePreset(saved.preset_json);
    expect(reimported.vendor_id).toBe(0x256f);
    expect(reimported.controls).toHaveLength(4);

    // Unplug -> OfflineGhost
    const unplugNotes = eng.simulateUnplug(path);
    expect(unplugNotes[0].ghosted_device_ids).toContain(saved.device.id);

    // Replug -> Auto-rebind
    const replug = eng.simulatePlugIn('quirky_imu_gamepad', 'ui01');
    expect(replug.notifications[0].auto_rebound_device_id).toBe(saved.device.id);
    expect(replug.notifications[0].recognized_preset_name).toBe('QuirkyPad UI Preset');
  });
});

describe('M3 Browser Loopback RGB Camera CV Extractors', () => {
  it('tracks brightness, motion energy, target blob centroid, and zone triggers', () => {
    const eng = new BrowserM3HardwareEngine();
    const camDev = createRgbCameraDevice({ x: 100, y: 100 });
    expect(camDev.ports.some((p) => p.logical_key === 'cam.blob.x')).toBe(true);

    const f1 = renderSyntheticCameraFrame(120, 90, 0, {
      bgBrightness: 0.1,
      orbRgb: [20, 240, 160],
      fixedOrbPos: [0.25, 0.3],
    });
    const r1 = eng.extractCvFromRgba('test-cam', f1, 120, 90, DEFAULT_RGB_CV_CONFIG);
    expect(r1.motion_energy).toBe(0);
    expect(r1.blob_centroid[0]).toBeCloseTo(0.25, 1);
    expect(r1.blob_centroid[1]).toBeCloseTo(0.3, 1);
    expect(r1.blob_area).toBeGreaterThan(0.02);

    const f2 = renderSyntheticCameraFrame(120, 90, 1, {
      bgBrightness: 0.5,
      orbRgb: [20, 240, 160],
      fixedOrbPos: [0.75, 0.7],
    });
    const r2 = eng.extractCvFromRgba('test-cam', f2, 120, 90, DEFAULT_RGB_CV_CONFIG);
    expect(r2.global_luma).toBeGreaterThan(r1.global_luma + 0.25);
    expect(r2.motion_energy).toBeGreaterThan(0.4);
    expect(r2.blob_centroid[0]).toBeCloseTo(0.75, 1);
    expect(r2.blob_centroid[1]).toBeCloseTo(0.7, 1);
    expect(r2.zones.some((z) => z.triggered)).toBe(true);
  });
});
