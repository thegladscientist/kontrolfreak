import { bridge, isTauriRuntime } from '../../lib/bridge';
import { applyModifierChain, type ModifierSlotState } from '../../lib/modifierMath';
import type { Device, DeviceFingerprint, Modifier, Port } from '../../types/kontrolfreak';
import {
  DEFAULT_RGB_CV_CONFIG,
  type CameraCvStepOutput,
  type HidControlType,
  type HidDeviceDefinition,
  type HidDeviceInfo,
  type HotplugNotification,
  type LearnedControlSpec,
  type LearnWizardSnapshot,
  type ParsedHidDescriptor,
  type RgbCameraInfo,
  type RgbCvConfig,
  type RgbCvResult,
  type SavedHidDeviceResponse,
  type SimulatedHidAction,
  type SimulatedHidProfile,
  type SimulatedPlugResponse,
} from './m3Types';

function uid(): string {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
    return crypto.randomUUID();
  }
  return `m3-${Date.now()}-${Math.random().toString(36).slice(2, 9)}`;
}

function clone<T>(v: T): T {
  return JSON.parse(JSON.stringify(v));
}

function popcount8(b: number): number {
  let x = b & 0xff;
  let c = 0;
  while (x) {
    c += x & 1;
    x >>= 1;
  }
  return c;
}

function trailingZeros8(b: number): number {
  const x = b & 0xff;
  if (x === 0) return 0;
  let n = 0;
  while (((x >> n) & 1) === 0) n++;
  return n;
}

function u8ToI8(b: number): number {
  return b > 127 ? b - 256 : b;
}

function read16(b0: number, b1: number, signed: boolean, littleEndian: boolean): number {
  const u = littleEndian ? (b0 & 0xff) | ((b1 & 0xff) << 8) : ((b0 & 0xff) << 8) | (b1 & 0xff);
  if (!signed) return u;
  return u >= 0x8000 ? u - 0x10000 : u;
}

function fingerprintFromInfo(info: HidDeviceInfo): DeviceFingerprint {
  return {
    vid: info.vid,
    pid: info.pid,
    serial: info.serial ?? null,
    usage_page: info.usage_page,
    usage: info.usage,
    interface_number: info.interface_number,
    port_name: info.product_name ?? null,
    endpoint: info.path,
  };
}

function fingerprintScore(a: DeviceFingerprint | null | undefined, b: DeviceFingerprint): number {
  if (!a) return 0;
  if (a.vid && a.pid && a.serial && b.vid && b.pid && b.serial) {
    if (a.vid === b.vid && a.pid === b.pid && a.serial === b.serial) return 100;
  }
  if (a.vid && a.pid && b.vid && b.pid && a.vid === b.vid && a.pid === b.pid) {
    if (a.usage_page != null && a.usage_page === b.usage_page) return 75;
    return 65;
  }
  return 0;
}

function presetMatchScore(def: HidDeviceDefinition, info: HidDeviceInfo): number {
  if (def.vendor_id !== info.vid || def.product_id !== info.pid) return 0;
  if (def.serial && info.serial && def.serial === info.serial) return 100;
  if (def.usage_page === info.usage_page && def.usage === info.usage) return 85;
  if (def.usage_page === info.usage_page) return 70;
  return 60;
}

interface SimDeviceState {
  profile: SimulatedHidProfile;
  info: HidDeviceInfo;
  faderU8: number;
  axisI16Le: number;
  dialU16Be: number;
  buttonA: boolean;
  buttonB: boolean;
  toggleC: boolean;
  hatSwitch: number;
  pendingEncoderDelta: number;
  noiseEnabled: boolean;
  tick: number;
}

interface ActiveBinding {
  deviceId: string;
  definition: HidDeviceDefinition;
  encoderAccum: Record<string, number>;
  modSlots: Record<string, ModifierSlotState[]>;
}

/**
 * Pure-TS RGB to HSV helper (`h` in 0..360, `s` and `v` in 0..1).
 */
export function rgbToHsv(r: number, g: number, b: number): [number, number, number] {
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const d = max - min;
  let h = 0;
  if (d > 1e-6) {
    if (max === r) h = 60 * (((g - b) / d + 6) % 6);
    else if (max === g) h = 60 * ((b - r) / d + 2);
    else h = 60 * ((r - g) / d + 4);
  }
  const s = max < 1e-6 ? 0 : d / max;
  return [h, s, max];
}

/**
 * Deterministic synthetic RGBA8 frame renderer matching Rust `render_synthetic_frame`.
 */
export function renderSyntheticCameraFrame(
  width: number,
  height: number,
  frameIndex: number,
  options?: {
    bgBrightness?: number;
    orbRgb?: [number, number, number];
    fixedOrbPos?: [number, number];
  }
): Uint8ClampedArray {
  const rgba = new Uint8ClampedArray(width * height * 4);
  const t = frameIndex * 0.14;
  const orbXNorm = options?.fixedOrbPos?.[0] ?? 0.5 + 0.34 * Math.sin(t * 0.9);
  const orbYNorm = options?.fixedOrbPos?.[1] ?? 0.5 + 0.3 * Math.cos(t * 1.35 + 0.6);

  const cx = Math.min(0.95, Math.max(0.05, orbXNorm)) * (width - 1);
  const cy = Math.min(0.95, Math.max(0.05, orbYNorm)) * (height - 1);
  const radius = Math.max(2, 0.14 * Math.min(width, height));
  const r2 = radius * radius;
  const bg = (options?.bgBrightness ?? 0.14) * 255;
  const [orbR, orbG, orbB] = options?.orbRgb ?? [30, 235, 165];

  for (let y = 0; y < height; y++) {
    const grad = 1 + 0.08 * (y / height);
    for (let x = 0; x < width; x++) {
      const idx = (y * width + x) * 4;
      let r = bg * grad;
      let g = bg * grad * 1.04;
      let b = bg * grad * 1.15;

      const dx = x - cx;
      const dy = y - cy;
      const dist2 = dx * dx + dy * dy;
      if (dist2 <= r2) {
        const falloff = Math.max(0, 1 - (dist2 / r2) * 0.25);
        r = orbR * falloff;
        g = orbG * falloff;
        b = orbB * falloff;
      }

      rgba[idx] = Math.min(255, Math.max(0, Math.round(r)));
      rgba[idx + 1] = Math.min(255, Math.max(0, Math.round(g)));
      rgba[idx + 2] = Math.min(255, Math.max(0, Math.round(b)));
      rgba[idx + 3] = 255;
    }
  }
  return rgba;
}

/**
 * Full browser-side M3 loopback engine mirroring Rust `kf-hardware::HardwareManager`,
 * so the 4-step HID Learn Wizard, noise masking, `.kfdevice.json` auto-rebind, and
 * RGB Camera CV extractors work identically in browser dev mode and Vitest.
 */
export class BrowserM3HardwareEngine {
  private simDevices = new Map<string, SimDeviceState>();
  private simCounter = 0;
  private knownPaths = new Map<string, HidDeviceInfo>();
  private presets: HidDeviceDefinition[] = [];
  private bindings = new Map<string, ActiveBinding>();

  // Learn wizard state
  private learnStep: LearnWizardSnapshot['step'] = 'idle';
  private learnPath: string | null = null;
  private baselineSamples: number[][] = [];
  private restingBytes: number[] = [];
  private noiseMaskBits: number[] = [];
  private captureSamples: number[][] = [];
  private latestBytes: number[] = [];
  private activeDiffBits: number[] = [];
  private candidate: LearnedControlSpec | null = null;
  private committed: LearnedControlSpec[] = [];

  // RGB Camera CV state
  private cvConfigs = new Map<string, RgbCvConfig>();
  private prevLuma = new Map<string, Float32Array>();
  private camFrames = new Map<string, number>();

  listHidDevices(): HidDeviceInfo[] {
    return Array.from(this.simDevices.values())
      .map((s) => clone(s.info))
      .sort((a, b) => a.path.localeCompare(b.path));
  }

  simulatePlugIn(profile: SimulatedHidProfile, stableId?: string | null): SimulatedPlugResponse {
    const suffix = stableId ?? String(++this.simCounter).padStart(2, '0');
    const info = this.makeProfileInfo(profile, suffix);
    this.simDevices.set(info.path, {
      profile,
      info,
      faderU8: 0,
      axisI16Le: 0,
      dialU16Be: 0,
      buttonA: false,
      buttonB: false,
      toggleC: false,
      hatSwitch: 8,
      pendingEncoderDelta: 0,
      noiseEnabled: true,
      tick: 0,
    });
    const notifications = this.pollHotplug();
    return { info: clone(info), notifications };
  }

  simulateUnplug(path: string): HotplugNotification[] {
    this.simDevices.delete(path);
    return this.pollHotplug();
  }

  pollHotplug(): HotplugNotification[] {
    const notifications: HotplugNotification[] = [];
    const currentPaths = new Set(this.simDevices.keys());

    // Disconnections
    for (const [path, oldInfo] of Array.from(this.knownPaths.entries())) {
      if (!currentPaths.has(path)) {
        this.knownPaths.delete(path);
        this.bindings.delete(path);
        const fp = fingerprintFromInfo(oldInfo);
        const ghosted: string[] = [];
        const ws = bridge.loopback.getWorkspace();
        for (const dev of ws.devices) {
          if (
            dev.kind.type === 'HidDevice' &&
            dev.status.state === 'online' &&
            fingerprintScore(dev.fingerprint, fp) >= 65
          ) {
            bridge.loopback.setDeviceStatus(dev.id, {
              state: 'offline_ghost',
              reason: 'HID device unplugged — waiting for replug',
              last_seen_ms: Date.now(),
            });
            ghosted.push(dev.id);
          }
        }
        notifications.push({
          event: { event: 'disconnected', path, fingerprint: fp },
          recognized_preset_name: null,
          auto_rebound_device_id: null,
          ghosted_device_ids: ghosted,
        });
      }
    }

    // Connections
    for (const [path, entry] of this.simDevices.entries()) {
      if (!this.knownPaths.has(path)) {
        this.knownPaths.set(path, clone(entry.info));
        const fp = fingerprintFromInfo(entry.info);

        const matchedPreset = this.presets
          .map((p) => ({ p, score: presetMatchScore(p, entry.info) }))
          .filter((x) => x.score >= 60)
          .sort((a, b) => b.score - a.score)[0]?.p;

        let autoReboundId: string | null = null;
        const ws = bridge.loopback.getWorkspace();
        for (const dev of ws.devices) {
          if (
            dev.kind.type === 'HidDevice' &&
            dev.status.state === 'offline_ghost' &&
            fingerprintScore(dev.fingerprint, fp) >= 65
          ) {
            bridge.loopback.rebindOfflineDevice(dev.id, fp, entry.info.product_name);
            autoReboundId = dev.id;
            break;
          }
        }

        if (matchedPreset && autoReboundId) {
          this.bindings.set(path, {
            deviceId: autoReboundId,
            definition: clone(matchedPreset),
            encoderAccum: {},
            modSlots: {},
          });
        }

        notifications.push({
          event: { event: 'connected', info: clone(entry.info), fingerprint: fp },
          recognized_preset_name: matchedPreset?.name ?? null,
          auto_rebound_device_id: autoReboundId,
          ghosted_device_ids: [],
        });
      }
    }

    return notifications;
  }

  simulateActuate(path: string, action: SimulatedHidAction): number {
    const entry = this.simDevices.get(path);
    if (!entry) throw new Error(`Simulated HID device not found: ${path}`);
    switch (action.control) {
      case 'set_fader_u8':
        entry.faderU8 = Math.max(0, Math.min(255, Math.round(action.value)));
        break;
      case 'set_axis_i16_le':
        entry.axisI16Le = Math.max(-32768, Math.min(32767, Math.round(action.value)));
        break;
      case 'set_dial_u16_be':
        entry.dialU16Be = Math.max(0, Math.min(65535, Math.round(action.value)));
        break;
      case 'set_button_a':
        entry.buttonA = action.pressed;
        break;
      case 'set_button_b':
        entry.buttonB = action.pressed;
        break;
      case 'toggle_switch_c':
        entry.toggleC = !entry.toggleC;
        break;
      case 'set_hat_switch':
        entry.hatSwitch = Math.max(0, Math.min(15, Math.round(action.direction)));
        break;
      case 'pulse_encoder':
        entry.pendingEncoderDelta = Math.max(-127, Math.min(127, Math.round(action.delta)));
        break;
      case 'set_noise_enabled':
        entry.noiseEnabled = action.enabled;
        break;
    }

    const pkt = this.nextPacket(entry);
    // If learn wizard is currently recording this device, feed sample automatically!
    if (this.learnPath === path && this.learnStep === 'recording_control') {
      this.ingestLearnPacket(pkt);
    }

    const binding = this.bindings.get(path);
    if (!binding) return 0;
    const ws = bridge.loopback.getWorkspace();
    const dev = ws.devices.find((d) => d.id === binding.deviceId);
    if (!dev || dev.status.state !== 'online') return 0;

    let routed = 0;
    const now = Date.now();
    for (const ctrl of binding.definition.controls) {
      const val = this.evaluateControl(ctrl, pkt, binding, now);
      if (val === null) continue;
      const port = dev.ports.find((p) => p.logical_key === ctrl.logical_key);
      if (port) {
        routed += bridge.loopback.injectSignal(port.id, { type: 'Float', value: val });
      }
    }
    return routed;
  }

  parseDescriptor(path: string): ParsedHidDescriptor | null {
    const entry = this.simDevices.get(path);
    if (!entry || entry.profile !== 'studio_fader_wing') return null;
    return {
      usage_page: 1,
      usage: 4,
      has_report_ids: true,
      fields: [
        { report_id: 1, usage_page: 1, usage: 0x30, usage_name: 'Axis X', bit_offset: 8, bit_size: 8, logical_min: 0, logical_max: 255, is_signed: false, is_relative: false, is_variable: true, is_constant: false },
        { report_id: 1, usage_page: 1, usage: 0x31, usage_name: 'Axis Y', bit_offset: 16, bit_size: 8, logical_min: 0, logical_max: 255, is_signed: false, is_relative: false, is_variable: true, is_constant: false },
        { report_id: 1, usage_page: 1, usage: 0x32, usage_name: 'Axis Z', bit_offset: 24, bit_size: 8, logical_min: 0, logical_max: 255, is_signed: false, is_relative: false, is_variable: true, is_constant: false },
        { report_id: 1, usage_page: 1, usage: 0x36, usage_name: 'Slider', bit_offset: 32, bit_size: 8, logical_min: 0, logical_max: 255, is_signed: false, is_relative: false, is_variable: true, is_constant: false },
        { report_id: 1, usage_page: 1, usage: 0x37, usage_name: 'Dial', bit_offset: 40, bit_size: 16, logical_min: -32768, logical_max: 32767, is_signed: true, is_relative: false, is_variable: true, is_constant: false },
        { report_id: 1, usage_page: 1, usage: 0x38, usage_name: 'Wheel', bit_offset: 56, bit_size: 8, logical_min: -127, logical_max: 127, is_signed: true, is_relative: true, is_variable: true, is_constant: false },
        { report_id: 1, usage_page: 9, usage: 1, usage_name: 'Button 1', bit_offset: 64, bit_size: 1, logical_min: 0, logical_max: 1, is_signed: false, is_relative: false, is_variable: true, is_constant: false },
        { report_id: 1, usage_page: 9, usage: 2, usage_name: 'Button 2', bit_offset: 65, bit_size: 1, logical_min: 0, logical_max: 1, is_signed: false, is_relative: false, is_variable: true, is_constant: false },
      ],
    };
  }

  // ---------- 4-Step Learn Engine ----------

  learnStartBaseline(devicePath: string): LearnWizardSnapshot {
    this.learnPath = devicePath;
    this.learnStep = 'capturing_baseline';
    this.baselineSamples = [];
    this.restingBytes = [];
    this.noiseMaskBits = [];
    this.captureSamples = [];
    this.activeDiffBits = [];
    this.candidate = null;
    this.committed = [];

    // Tier 1 pre-seed if descriptor exists
    const desc = this.parseDescriptor(devicePath);
    if (desc) {
      this.committed = desc.fields.map((f, idx) => {
        const byteOffset = Math.floor(f.bit_offset / 8);
        const bitInByte = f.bit_offset % 8;
        const bitMask = f.bit_size >= 8 ? 0xff : ((1 << f.bit_size) - 1) << bitInByte;
        const control_type: HidControlType = f.is_relative
          ? 'relative_encoder'
          : f.bit_size === 1
          ? 'button'
          : f.bit_size > 8
          ? 'axis16'
          : 'axis8';
        return {
          logical_key: `hid.r${f.report_id}.${f.usage_name.toLowerCase().replace(/[^a-z0-9]+/g, '_')}.${idx}`,
          alias: f.usage_name,
          report_id: f.report_id,
          byte_offset: byteOffset,
          bit_offset: f.bit_offset,
          bit_mask: bitMask,
          bit_width: f.bit_size,
          signed: f.is_signed,
          little_endian: true,
          control_type,
          observed_min: f.logical_min,
          observed_max: f.logical_max,
          sensitivity: 0.02,
          wrap: false,
          modifiers: [],
        };
      });
    }

    return this.learnGetSnapshot();
  }

  learnPollSamples(count = 12): LearnWizardSnapshot {
    if (!this.learnPath) throw new Error('No active Learn session device');
    const entry = this.simDevices.get(this.learnPath);
    if (!entry) throw new Error(`Device not connected: ${this.learnPath}`);
    for (let i = 0; i < count; i++) {
      this.ingestLearnPacket(this.nextPacket(entry));
    }
    return this.learnGetSnapshot();
  }

  learnStartCapture(): LearnWizardSnapshot {
    if (this.baselineSamples.length === 0 && this.restingBytes.length === 0) {
      throw new Error('Sample idle baseline first before starting capture.');
    }
    if (this.baselineSamples.length > 0) {
      this.computeBaseline();
    }
    this.captureSamples = [];
    this.activeDiffBits = new Array(this.restingBytes.length).fill(0);
    this.candidate = null;
    this.learnStep = 'recording_control';
    return this.learnGetSnapshot();
  }

  learnClassify(aliasHint?: string | null): LearnWizardSnapshot {
    const spec = this.classifyCaptured(aliasHint);
    this.candidate = spec;
    this.learnStep = 'review_and_tune';
    return this.learnGetSnapshot();
  }

  learnCommitControl(
    alias: string,
    modifiers: Modifier[],
    sensitivity?: number | null,
    wrap?: boolean | null
  ): LearnWizardSnapshot {
    const base = this.candidate ?? this.classifyCaptured(alias);
    const spec: LearnedControlSpec = clone(base);
    if (alias.trim()) {
      spec.alias = alias.trim();
      const slug = spec.alias.toLowerCase().replace(/[^a-z0-9]+/g, '_');
      spec.logical_key = `hid.r${spec.report_id}.b${spec.byte_offset}.${slug}`;
    }
    spec.modifiers = clone(modifiers);
    if (typeof sensitivity === 'number') spec.sensitivity = Math.max(0.001, Math.min(0.5, sensitivity));
    if (typeof wrap === 'boolean') spec.wrap = wrap;

    const existingIdx = this.committed.findIndex(
      (c) =>
        c.report_id === spec.report_id &&
        c.byte_offset === spec.byte_offset &&
        c.bit_mask === spec.bit_mask
    );
    if (existingIdx >= 0) this.committed[existingIdx] = spec;
    else this.committed.push(spec);

    this.candidate = spec;
    this.learnStep = 'completed';
    return this.learnGetSnapshot();
  }

  learnGetSnapshot(): LearnWizardSnapshot {
    if (this.learnStep === 'capturing_baseline' && this.baselineSamples.length > 0) {
      this.computeBaseline();
    }
    return {
      step: this.learnStep,
      device_path: this.learnPath,
      active_report_id: this.latestBytes[0] ?? 1,
      baseline_samples: this.baselineSamples.length,
      capture_samples: this.captureSamples.length,
      resting_bytes: [...this.restingBytes],
      latest_bytes: [...this.latestBytes],
      noise_mask_bits: [...this.noiseMaskBits],
      active_diff_bits: [...this.activeDiffBits],
      candidate: this.candidate ? clone(this.candidate) : null,
      committed_controls: clone(this.committed),
    };
  }

  learnReset(): LearnWizardSnapshot {
    this.learnStep = 'idle';
    this.learnPath = null;
    this.baselineSamples = [];
    this.restingBytes = [];
    this.noiseMaskBits = [];
    this.captureSamples = [];
    this.latestBytes = [];
    this.activeDiffBits = [];
    this.candidate = null;
    this.committed = [];
    return this.learnGetSnapshot();
  }

  saveAndAttachLearnedDevice(presetName: string, x = 140, y = 180): SavedHidDeviceResponse {
    if (!this.learnPath) throw new Error('No device in Learn session');
    const entry = this.simDevices.get(this.learnPath);
    if (!entry) throw new Error(`HID device not found: ${this.learnPath}`);

    const def: HidDeviceDefinition = {
      schema_version: 1,
      preset_id: uid(),
      name: presetName.trim() || entry.info.product_name || 'Learned HID Device',
      vendor_id: entry.info.vid,
      product_id: entry.info.pid,
      usage_page: entry.info.usage_page,
      usage: entry.info.usage,
      serial: entry.info.serial ?? null,
      noise_masks: { [this.latestBytes[0] ?? 1]: [...this.noiseMaskBits] },
      controls: clone(this.committed),
    };

    this.upsertPreset(def);

    const ports: Port[] = def.controls.map((c) => ({
      id: uid(),
      logical_key: c.logical_key,
      alias: c.alias,
      direction: 'out',
      signal_kind: c.control_type === 'button' || c.control_type === 'toggle' ? 'trigger' : 'sensor',
      min_value: 0,
      max_value: 1,
      modifiers: clone(c.modifiers),
    }));

    const device: Device = bridge.loopback.addDevice({
      id: uid(),
      logical_key: `hid.${def.vendor_id.toString(16).padStart(4, '0')}.${def.product_id.toString(16).padStart(4, '0')}`,
      display_name: def.name,
      kind: {
        type: 'HidDevice',
        device_preset_id: def.preset_id,
        vid: def.vendor_id,
        pid: def.product_id,
      },
      fingerprint: fingerprintFromInfo(entry.info),
      status: { state: 'online' },
      ports,
      ui: { x, y, collapsed: false },
      group_id: null,
      manual_override: {},
    });

    this.bindings.set(this.learnPath, {
      deviceId: device.id,
      definition: clone(def),
      encoderAccum: {},
      modSlots: {},
    });

    const presetJson = JSON.stringify(
      {
        schema_version: 1,
        preset_id: def.preset_id,
        name: def.name,
        device,
        hid_definition: def,
      },
      null,
      2
    );

    return { definition: clone(def), preset_json: presetJson, device };
  }

  importHidDevicePreset(json: string): HidDeviceDefinition {
    const parsed = JSON.parse(json) as { hid_definition?: HidDeviceDefinition } & Partial<HidDeviceDefinition>;
    const def = parsed.hid_definition ?? (parsed as HidDeviceDefinition);
    if (typeof def.vendor_id !== 'number' || typeof def.product_id !== 'number') {
      throw new Error('Invalid .kfdevice.json HID preset');
    }
    this.upsertPreset(def);
    return clone(def);
  }

  listDevicePresets(): HidDeviceDefinition[] {
    return clone(this.presets);
  }

  // ---------- RGB Camera CV Engine ----------

  listRgbCameras(): RgbCameraInfo[] {
    return [
      {
        id: 'sim://camera/studio-cv',
        name: 'Synthetic Studio RGB Camera',
        width: 160,
        height: 120,
        fps: 30,
        is_synthetic: true,
      },
    ];
  }

  getRgbCvConfig(cameraId: string): RgbCvConfig {
    return clone(this.cvConfigs.get(cameraId) ?? DEFAULT_RGB_CV_CONFIG);
  }

  updateRgbCvConfig(cameraId: string, config: RgbCvConfig): RgbCvConfig {
    this.cvConfigs.set(cameraId, clone(config));
    return clone(config);
  }

  stepRgbCameraCv(cameraId: string, targetDeviceId?: string | null): CameraCvStepOutput {
    const width = 160;
    const height = 120;
    const frameIndex = this.camFrames.get(cameraId) ?? 0;
    this.camFrames.set(cameraId, frameIndex + 1);
    const rgba = renderSyntheticCameraFrame(width, height, frameIndex);
    const config = this.cvConfigs.get(cameraId) ?? DEFAULT_RGB_CV_CONFIG;
    const cv = this.extractCvFromRgba(cameraId, rgba, width, height, config);

    let routed = 0;
    if (targetDeviceId) {
      const ws = bridge.loopback.getWorkspace();
      const dev = ws.devices.find((d) => d.id === targetDeviceId);
      if (dev && dev.status.state === 'online') {
        const values: Record<string, number> = {
          'cam.luma.global': cv.global_luma,
          'cam.luma.roi': cv.roi_luma,
          'cam.motion.energy': cv.motion_energy,
          'cam.motion.x': cv.motion_centroid[0],
          'cam.motion.y': cv.motion_centroid[1],
          'cam.color.r': cv.dominant_rgb[0],
          'cam.color.g': cv.dominant_rgb[1],
          'cam.color.b': cv.dominant_rgb[2],
          'cam.color.hue': cv.dominant_hsv[0],
          'cam.brightest.x': cv.brightest_point[0],
          'cam.brightest.y': cv.brightest_point[1],
          'cam.brightest.val': cv.brightest_luma,
          'cam.blob.x': cv.blob_centroid[0],
          'cam.blob.y': cv.blob_centroid[1],
          'cam.blob.area': cv.blob_area,
        };
        for (const z of cv.zones) {
          values[`cam.zone.${z.index}.energy`] = z.energy;
          values[`cam.zone.${z.index}.trigger`] = z.triggered ? 1 : 0;
        }
        for (const port of dev.ports) {
          if (port.logical_key in values) {
            routed += bridge.loopback.injectSignal(port.id, {
              type: 'Float',
              value: values[port.logical_key],
            });
          }
        }
      }
    }

    return {
      camera_id: cameraId,
      frame_index: frameIndex,
      width,
      height,
      cv,
      routed_count: routed,
    };
  }

  extractCvFromRgba(
    cameraId: string,
    rgba: Uint8ClampedArray,
    width: number,
    height: number,
    config: RgbCvConfig
  ): RgbCvResult {
    const totalPx = width * height;
    const prev = this.prevLuma.get(cameraId);
    const hasPrev = prev && prev.length === totalPx;
    const curr = new Float32Array(totalPx);

    const rx0 = Math.floor(Math.max(0, Math.min(1, config.roi.x)) * width);
    const ry0 = Math.floor(Math.max(0, Math.min(1, config.roi.y)) * height);
    const rx1 = Math.min(width, Math.max(rx0 + 1, Math.ceil((config.roi.x + config.roi.w) * width)));
    const ry1 = Math.min(height, Math.max(ry0 + 1, Math.ceil((config.roi.y + config.roi.h) * height)));

    const zoneCols = Math.max(1, Math.min(8, config.zone_cols));
    const zoneRows = Math.max(1, Math.min(8, config.zone_rows));
    const zCount = zoneCols * zoneRows;
    const zoneMotion = new Float64Array(zCount);
    const zonePixels = new Uint32Array(zCount);

    let gLuma = 0;
    let rLuma = 0;
    let rSum = 0;
    let gSum = 0;
    let bSum = 0;
    let roiCount = 0;

    let motionSum = 0;
    let motionWx = 0;
    let motionWy = 0;
    let motionW = 0;

    let bestLuma = -1;
    let bestX = 0;
    let bestY = 0;
    let bestCount = 0;

    let blobX = 0;
    let blobY = 0;
    let blobCount = 0;

    const inv255 = 1 / 255;
    const dw = Math.max(1, width - 1);
    const dh = Math.max(1, height - 1);

    for (let y = 0; y < height; y++) {
      const zRow = Math.min(zoneRows - 1, Math.floor((y * zoneRows) / height));
      const inRoiY = y >= ry0 && y < ry1;
      for (let x = 0; x < width; x++) {
        const pIdx = y * width + x;
        const bi = pIdx * 4;
        const r = rgba[bi] * inv255;
        const g = rgba[bi + 1] * inv255;
        const b = rgba[bi + 2] * inv255;
        const luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        curr[pIdx] = luma;
        gLuma += luma;

        if (luma > bestLuma + 1e-4) {
          bestLuma = luma;
          bestX = x;
          bestY = y;
          bestCount = 1;
        } else if (Math.abs(luma - bestLuma) <= 1e-4) {
          bestX += x;
          bestY += y;
          bestCount++;
        }

        const zCol = Math.min(zoneCols - 1, Math.floor((x * zoneCols) / width));
        const zIdx = zRow * zoneCols + zCol;
        zonePixels[zIdx]++;

        if (hasPrev) {
          const diff = Math.abs(luma - prev[pIdx]);
          if (diff >= config.motion_pixel_threshold) {
            motionSum += diff;
            motionWx += (x / dw) * diff;
            motionWy += (y / dh) * diff;
            motionW += diff;
            zoneMotion[zIdx] += diff;
          }
        }

        if (inRoiY && x >= rx0 && x < rx1) {
          roiCount++;
          rLuma += luma;
          rSum += r;
          gSum += g;
          bSum += b;
          const [hDeg, sat, val] = rgbToHsv(r, g, b);
          const hDiff = Math.abs(hDeg - config.target_hue_deg) % 360;
          const circDiff = Math.min(hDiff, 360 - hDiff);
          if (
            sat >= config.min_saturation &&
            val >= config.min_value &&
            circDiff <= config.hue_tolerance_deg
          ) {
            blobX += x / dw;
            blobY += y / dh;
            blobCount++;
          }
        }
      }
    }

    this.prevLuma.set(cameraId, curr);

    const roiDenom = Math.max(1, roiCount);
    const domR = rSum / roiDenom;
    const domG = gSum / roiDenom;
    const domB = bSum / roiDenom;
    const [domH, domS, domV] = rgbToHsv(domR, domG, domB);

    const zones = [];
    for (let r = 0; r < zoneRows; r++) {
      for (let c = 0; c < zoneCols; c++) {
        const idx = r * zoneCols + c;
        const energy = hasPrev
          ? Math.min(1, (zoneMotion[idx] / Math.max(1, zonePixels[idx])) * 4)
          : 0;
        zones.push({
          index: idx,
          col: c,
          row: r,
          energy,
          triggered: energy >= config.zone_trigger_threshold,
        });
      }
    }

    return {
      global_luma: Math.min(1, Math.max(0, gLuma / totalPx)),
      roi_luma: Math.min(1, Math.max(0, rLuma / roiDenom)),
      motion_energy: hasPrev ? Math.min(1, (motionSum / totalPx) * 4) : 0,
      motion_centroid: motionW > 1e-6 ? [motionWx / motionW, motionWy / motionW] : [0.5, 0.5],
      dominant_rgb: [domR, domG, domB],
      dominant_hsv: [domH / 360, domS, domV],
      brightest_point: bestCount > 0 ? [bestX / bestCount / dw, bestY / bestCount / dh] : [0.5, 0.5],
      brightest_luma: Math.max(0, Math.min(1, bestLuma)),
      blob_centroid: blobCount > 0 ? [blobX / blobCount, blobY / blobCount] : [0.5, 0.5],
      blob_area: Math.min(1, blobCount / roiDenom),
      zones,
    };
  }

  // ---------- Internal Helpers ----------

  private upsertPreset(def: HidDeviceDefinition): void {
    const idx = this.presets.findIndex(
      (p) =>
        p.preset_id === def.preset_id ||
        (p.vendor_id === def.vendor_id &&
          p.product_id === def.product_id &&
          p.usage_page === def.usage_page &&
          p.usage === def.usage)
    );
    if (idx >= 0) this.presets[idx] = clone(def);
    else this.presets.push(clone(def));
  }

  private makeProfileInfo(profile: SimulatedHidProfile, suffix: string): HidDeviceInfo {
    switch (profile) {
      case 'quirky_imu_gamepad':
        return {
          path: `sim://hid/quirky-imu-pad-${suffix}`,
          vid: 0x256f,
          pid: 0xc635,
          serial: `KF-QPAD-${suffix}`,
          product_name: 'KF QuirkyPad Pro (IMU + Encoders)',
          manufacturer: 'Kontrolfreak Virtual Hardware',
          usage_page: 0xff00,
          usage: 0x0001,
          interface_number: 0,
        };
      case 'studio_fader_wing':
        return {
          path: `sim://hid/studio-fader-wing-${suffix}`,
          vid: 0x16c0,
          pid: 0x0486,
          serial: `KF-FADER-${suffix}`,
          product_name: 'KF StudioFader 4 (HID Descriptor)',
          manufacturer: 'Kontrolfreak Virtual Hardware',
          usage_page: 0x0001,
          usage: 0x0004,
          interface_number: 0,
        };
      case 'unknown_mystery_dial':
        return {
          path: `sim://hid/unknown-mystery-dial-${suffix}`,
          vid: 0x1d50,
          pid: 0x614e,
          serial: `MYST-${suffix}`,
          product_name: 'Unknown USB Rotary HID (0x1D50:0x614E)',
          manufacturer: 'Generic Vendor HID',
          usage_page: 0xff01,
          usage: 0x0002,
          interface_number: 1,
        };
    }
  }

  private nextPacket(s: SimDeviceState): number[] {
    s.tick = (s.tick + 1) >>> 0;
    const lcg = (Math.imul(s.tick, 1664525) + 1013904223) >>> 0;
    const imuAx = s.noiseEnabled ? 120 + (((lcg >>> 16) & 0x0f) - 8) : 120;
    const imuAy = s.noiseEnabled ? -40 + (((lcg >>> 20) & 0x0f) - 8) : -40;

    if (s.profile === 'studio_fader_wing') {
      const le = s.axisI16Le & 0xffff;
      const enc = s.pendingEncoderDelta & 0xff;
      s.pendingEncoderDelta = 0;
      const btn = (s.buttonA ? 1 : 0) | (s.buttonB ? 2 : 0) | (s.toggleC ? 4 : 0);
      return [1, s.faderU8 & 0xff, (s.dialU16Be >>> 8) & 0xff, 128, 64, le & 0xff, (le >>> 8) & 0xff, enc, btn];
    }

    const ax = imuAx & 0xffff;
    const ay = imuAy & 0xffff;
    const le = s.axisI16Le & 0xffff;
    const be = s.dialU16Be & 0xffff;
    const btn =
      (s.buttonA ? 1 : 0) |
      (s.buttonB ? 2 : 0) |
      (s.toggleC ? 4 : 0) |
      ((s.hatSwitch & 0x0f) << 4);
    const enc = s.pendingEncoderDelta & 0xff;
    s.pendingEncoderDelta = 0;

    return [
      0x01,
      s.noiseEnabled ? s.tick & 0xff : 0,
      ax & 0xff,
      (ax >>> 8) & 0xff,
      ay & 0xff,
      (ay >>> 8) & 0xff,
      s.faderU8 & 0xff,
      le & 0xff,
      (le >>> 8) & 0xff,
      (be >>> 8) & 0xff,
      be & 0xff,
      btn,
      enc,
      s.noiseEnabled ? (s.tick * 7) & 0x03 : 0,
      0xaa,
      0x55,
    ];
  }

  private ingestLearnPacket(pkt: number[]): void {
    this.latestBytes = [...pkt];
    if (this.learnStep === 'capturing_baseline') {
      this.baselineSamples.push([...pkt]);
    } else if (this.learnStep === 'recording_control') {
      this.captureSamples.push([...pkt]);
      this.activeDiffBits = this.computeUnmaskedDiff();
    }
  }

  private computeBaseline(): void {
    const len = this.baselineSamples[0]?.length ?? 0;
    const rest = new Array(len).fill(0);
    const mask = new Array(len).fill(0);
    for (let i = 0; i < len; i++) {
      const first = this.baselineSamples[0][i] ?? 0;
      let flipped = 0;
      let transitions = 0;
      let prev = first;
      const distinct = new Set<number>();
      const freq = new Map<number, number>();
      for (const s of this.baselineSamples) {
        const b = s[i] ?? 0;
        flipped |= b ^ first;
        if (b !== prev) {
          transitions++;
          prev = b;
        }
        distinct.add(b);
        freq.set(b, (freq.get(b) ?? 0) + 1);
      }
      let mode = first;
      let bestCnt = 0;
      for (const [k, v] of freq.entries()) {
        if (v > bestCnt) {
          bestCnt = v;
          mode = k;
        }
      }
      rest[i] = mode;
      const ratio = this.baselineSamples.length > 1 ? transitions / (this.baselineSamples.length - 1) : 0;
      mask[i] = distinct.size >= 3 || ratio >= 0.2 || popcount8(flipped) >= 3 ? 0xff : flipped;
    }
    this.restingBytes = rest;
    this.noiseMaskBits = mask;
  }

  private computeUnmaskedDiff(): number[] {
    const len = this.restingBytes.length;
    const diff = new Array(len).fill(0);
    for (const s of this.captureSamples) {
      for (let i = 0; i < len; i++) {
        diff[i] |= ((s[i] ?? 0) ^ this.restingBytes[i]) & ~this.noiseMaskBits[i] & 0xff;
      }
    }
    return diff;
  }

  private classifyCaptured(aliasHint?: string | null): LearnedControlSpec {
    this.activeDiffBits = this.computeUnmaskedDiff();
    const active: Array<{ idx: number; bits: number; distinct: number }> = [];
    for (let i = 0; i < this.activeDiffBits.length; i++) {
      const bits = this.activeDiffBits[i];
      if (bits === 0) continue;
      const mask = ~this.noiseMaskBits[i] & 0xff;
      const distinct = new Set(this.captureSamples.map((s) => (s[i] ?? 0) & mask)).size;
      active.push({ idx: i, bits, distinct });
    }
    if (active.length === 0) {
      throw new Error('No non-noise bit activity detected. Move a control during Step 2 first.');
    }

    const rid = this.latestBytes[0] ?? 1;
    const controlIdx = this.committed.length;

    // Adjacent 2-byte 16-bit axis check
    for (let k = 0; k + 1 < active.length; k++) {
      const a = active[k];
      const b = active[k + 1];
      if (
        b.idx === a.idx + 1 &&
        popcount8(a.bits) + popcount8(b.bits) >= 3 &&
        a.distinct >= 2 &&
        b.distinct >= 2 &&
        this.noiseMaskBits[a.idx] === 0 &&
        this.noiseMaskBits[b.idx] === 0
      ) {
        const eval16 = (signed: boolean, le: boolean) => {
          const vals = this.captureSamples.map((s) => read16(s[a.idx], s[b.idx], signed, le));
          const min = Math.min(...vals);
          const max = Math.max(...vals);
          const span = Math.max(1, max - min);
          let tv = 0;
          let maxStep = 0;
          for (let j = 1; j < vals.length; j++) {
            const d = Math.abs(vals[j] - vals[j - 1]);
            tv += d;
            if (d > maxStep) maxStep = d;
          }
          return { signed, le, min, max, score: tv / span + (maxStep / span) * 3 + (signed ? 0.005 : 0) };
        };
        const best = [
          eval16(false, true),
          eval16(true, true),
          eval16(false, false),
          eval16(true, false),
        ].sort((x, y) => x.score - y.score)[0];

        return {
          logical_key: `hid.r${rid}.b${a.idx}.axis16_${controlIdx}`,
          alias:
            aliasHint?.trim() ||
            `Axis 16-bit (${best.signed ? 'i16' : 'u16'} ${best.le ? 'LE' : 'BE'}) #${controlIdx}`,
          report_id: rid,
          byte_offset: a.idx,
          bit_offset: a.idx * 8,
          bit_mask: 0xff,
          bit_width: 16,
          signed: best.signed,
          little_endian: best.le,
          control_type: 'axis16',
          observed_min: best.min,
          observed_max: best.max,
          sensitivity: 0.02,
          wrap: false,
          modifiers: [],
        };
      }
    }

    active.sort((x, y) => y.distinct - x.distinct || popcount8(y.bits) - popcount8(x.bits));
    const top = active[0];
    const rest = this.restingBytes[top.idx] ?? 0;
    const seq = this.captureSamples.map((s) => s[top.idx] ?? 0);

    // 1-bit Button / Toggle
    if (popcount8(top.bits) === 1) {
      const bitPos = trailingZeros8(top.bits);
      const restBit = (rest & top.bits) !== 0;
      const lastBit = ((seq[seq.length - 1] ?? rest) & top.bits) !== 0;
      let sawActive = false;
      let returned = false;
      for (const b of seq) {
        const cur = (b & top.bits) !== 0;
        if (cur !== restBit) sawActive = true;
        else if (sawActive) returned = true;
      }
      const control_type: HidControlType = !returned && lastBit !== restBit ? 'toggle' : 'button';
      return {
        logical_key: `hid.r${rid}.b${top.idx}.bit${bitPos}`,
        alias: aliasHint?.trim() || `${control_type === 'toggle' ? 'Toggle' : 'Button'} B${top.idx}.${bitPos}`,
        report_id: rid,
        byte_offset: top.idx,
        bit_offset: top.idx * 8 + bitPos,
        bit_mask: top.bits,
        bit_width: 1,
        signed: false,
        little_endian: true,
        control_type,
        observed_min: 0,
        observed_max: 1,
        sensitivity: 0.02,
        wrap: false,
        modifiers: [],
      };
    }

    // 4-bit Hat Switch
    for (const nibMask of [0x0f, 0xf0]) {
      if ((top.bits & ~nibMask) === 0) {
        const shift = trailingZeros8(nibMask);
        const restNib = (rest & nibMask) >> shift;
        const nibs = new Set(seq.map((b) => (b & nibMask) >> shift));
        if (restNib >= 8 && Array.from(nibs).some((v) => v <= 7) && nibs.size >= 2 && nibs.size <= 9) {
          return {
            logical_key: `hid.r${rid}.b${top.idx}.hat`,
            alias: aliasHint?.trim() || `Hat Switch B${top.idx}`,
            report_id: rid,
            byte_offset: top.idx,
            bit_offset: top.idx * 8 + shift,
            bit_mask: nibMask,
            bit_width: 4,
            signed: false,
            little_endian: true,
            control_type: 'hat_switch',
            observed_min: 0,
            observed_max: 7,
            sensitivity: 0.02,
            wrap: false,
            modifiers: [],
          };
        }
      }
    }

    // Relative Encoder
    if (rest === 0) {
      const signed = seq.map(u8ToI8);
      const nonZero = signed.filter((v) => v !== 0);
      const maxAbs = Math.max(0, ...nonZero.map((v) => Math.abs(v)));
      let returnsZero = 0;
      for (let j = 1; j < signed.length; j++) {
        if (signed[j - 1] !== 0 && signed[j] === 0) returnsZero++;
      }
      if (nonZero.length > 0 && maxAbs <= 16 && (returnsZero >= 1 || signed.filter((v) => v === 0).length >= 2)) {
        return {
          logical_key: `hid.r${rid}.b${top.idx}.encoder`,
          alias: aliasHint?.trim() || `Rotary Encoder B${top.idx}`,
          report_id: rid,
          byte_offset: top.idx,
          bit_offset: top.idx * 8,
          bit_mask: 0xff,
          bit_width: 8,
          signed: true,
          little_endian: true,
          control_type: 'relative_encoder',
          observed_min: -127,
          observed_max: 127,
          sensitivity: 0.03,
          wrap: false,
          modifiers: [],
        };
      }
    }

    // 8-bit Axis
    const min = Math.min(...seq);
    const max = Math.max(...seq);
    return {
      logical_key: `hid.r${rid}.b${top.idx}.axis8_${controlIdx}`,
      alias: aliasHint?.trim() || `Axis 8-bit (u8) #${controlIdx}`,
      report_id: rid,
      byte_offset: top.idx,
      bit_offset: top.idx * 8,
      bit_mask: 0xff,
      bit_width: 8,
      signed: false,
      little_endian: true,
      control_type: 'axis8',
      observed_min: min,
      observed_max: max,
      sensitivity: 0.02,
      wrap: false,
      modifiers: [],
    };
  }

  private evaluateControl(
    ctrl: LearnedControlSpec,
    pkt: number[],
    binding: ActiveBinding,
    nowMs: number
  ): number | null {
    let norm = 0;
    if (ctrl.control_type === 'axis16') {
      if (ctrl.byte_offset + 1 >= pkt.length) return null;
      const raw = read16(pkt[ctrl.byte_offset], pkt[ctrl.byte_offset + 1], ctrl.signed, ctrl.little_endian);
      const span = Math.max(1, ctrl.observed_max - ctrl.observed_min);
      norm = Math.max(0, Math.min(1, (raw - ctrl.observed_min) / span));
    } else if (ctrl.control_type === 'axis8') {
      const b = pkt[ctrl.byte_offset] ?? 0;
      const raw = ctrl.signed ? u8ToI8(b) : b;
      const span = Math.max(1, ctrl.observed_max - ctrl.observed_min);
      norm = Math.max(0, Math.min(1, (raw - ctrl.observed_min) / span));
    } else if (ctrl.control_type === 'button' || ctrl.control_type === 'toggle') {
      norm = ((pkt[ctrl.byte_offset] ?? 0) & ctrl.bit_mask) !== 0 ? 1 : 0;
    } else if (ctrl.control_type === 'hat_switch') {
      const shift = trailingZeros8(ctrl.bit_mask);
      const v = ((pkt[ctrl.byte_offset] ?? 0) & ctrl.bit_mask) >> shift;
      norm = v > 7 ? 0 : Math.min(1, v / 7);
    } else if (ctrl.control_type === 'relative_encoder') {
      const delta = u8ToI8(pkt[ctrl.byte_offset] ?? 0);
      const prev = binding.encoderAccum[ctrl.logical_key] ?? 0.5;
      const next = prev + delta * ctrl.sensitivity;
      norm = ctrl.wrap ? ((next % 1) + 1) % 1 : Math.max(0, Math.min(1, next));
      binding.encoderAccum[ctrl.logical_key] = norm;
    }

    if (ctrl.modifiers.length === 0) return norm;
    let slots = binding.modSlots[ctrl.logical_key];
    if (!slots) {
      slots = [];
      binding.modSlots[ctrl.logical_key] = slots;
    }
    const shaped = applyModifierChain(ctrl.modifiers, { type: 'Float', value: norm }, nowMs, slots);
    if (!shaped || shaped.type !== 'Float') return null;
    return shaped.value;
  }
}

const loopbackM3 = new BrowserM3HardwareEngine();

async function invokeM3<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import('@tauri-apps/api/core');
  return invoke<T>(cmd, args);
}

export const m3Bridge = {
  loopback: loopbackM3,

  async listHidDevices(): Promise<HidDeviceInfo[]> {
    return isTauriRuntime() ? invokeM3('m3_list_hid_devices') : loopbackM3.listHidDevices();
  },

  async pollHotplug(): Promise<HotplugNotification[]> {
    return isTauriRuntime() ? invokeM3('m3_poll_hid_hotplug') : loopbackM3.pollHotplug();
  },

  async simulateHidPlug(
    profile: SimulatedHidProfile,
    stableId?: string | null
  ): Promise<SimulatedPlugResponse> {
    return isTauriRuntime()
      ? invokeM3('m3_simulate_hid_plug', { profile, stableId: stableId ?? null })
      : loopbackM3.simulatePlugIn(profile, stableId);
  },

  async simulateHidUnplug(path: string): Promise<HotplugNotification[]> {
    return isTauriRuntime()
      ? invokeM3('m3_simulate_hid_unplug', { path })
      : loopbackM3.simulateUnplug(path);
  },

  async simulateHidActuate(path: string, action: SimulatedHidAction): Promise<number> {
    return isTauriRuntime()
      ? invokeM3('m3_simulate_hid_actuate', { path, action })
      : loopbackM3.simulateActuate(path, action);
  },

  async parseHidDescriptor(path: string): Promise<ParsedHidDescriptor | null> {
    return isTauriRuntime()
      ? invokeM3('m3_parse_hid_descriptor', { path })
      : loopbackM3.parseDescriptor(path);
  },

  async learnStartBaseline(devicePath: string): Promise<LearnWizardSnapshot> {
    return isTauriRuntime()
      ? invokeM3('m3_learn_start_baseline', { devicePath })
      : loopbackM3.learnStartBaseline(devicePath);
  },

  async learnPollSamples(sampleCount = 12): Promise<LearnWizardSnapshot> {
    return isTauriRuntime()
      ? invokeM3('m3_learn_poll_samples', { sampleCount })
      : loopbackM3.learnPollSamples(sampleCount);
  },

  async learnStartCapture(): Promise<LearnWizardSnapshot> {
    return isTauriRuntime() ? invokeM3('m3_learn_start_capture') : loopbackM3.learnStartCapture();
  },

  async learnClassify(aliasHint?: string | null): Promise<LearnWizardSnapshot> {
    return isTauriRuntime()
      ? invokeM3('m3_learn_classify', { aliasHint: aliasHint ?? null })
      : loopbackM3.learnClassify(aliasHint);
  },

  async learnCommitControl(
    alias: string,
    modifiers: Modifier[],
    sensitivity?: number | null,
    wrap?: boolean | null
  ): Promise<LearnWizardSnapshot> {
    return isTauriRuntime()
      ? invokeM3('m3_learn_commit_control', {
          alias,
          modifiers,
          sensitivity: sensitivity ?? null,
          wrap: wrap ?? null,
        })
      : loopbackM3.learnCommitControl(alias, modifiers, sensitivity, wrap);
  },

  async learnGetSnapshot(): Promise<LearnWizardSnapshot> {
    return isTauriRuntime() ? invokeM3('m3_learn_get_snapshot') : loopbackM3.learnGetSnapshot();
  },

  async learnReset(): Promise<LearnWizardSnapshot> {
    return isTauriRuntime() ? invokeM3('m3_learn_reset') : loopbackM3.learnReset();
  },

  async saveAndAttachLearnedDevice(
    presetName: string,
    x = 140,
    y = 180
  ): Promise<SavedHidDeviceResponse> {
    return isTauriRuntime()
      ? invokeM3('m3_save_and_attach_learned_device', { presetName, x, y })
      : loopbackM3.saveAndAttachLearnedDevice(presetName, x, y);
  },

  async importHidDevicePreset(json: string): Promise<HidDeviceDefinition> {
    return isTauriRuntime()
      ? invokeM3('m3_import_hid_device_preset', { json })
      : loopbackM3.importHidDevicePreset(json);
  },

  async listDevicePresets(): Promise<HidDeviceDefinition[]> {
    return isTauriRuntime() ? invokeM3('m3_list_device_presets') : loopbackM3.listDevicePresets();
  },

  async listRgbCameras(): Promise<RgbCameraInfo[]> {
    return isTauriRuntime() ? invokeM3('m3_list_rgb_cameras') : loopbackM3.listRgbCameras();
  },

  async getRgbCvConfig(cameraId: string): Promise<RgbCvConfig> {
    return isTauriRuntime()
      ? invokeM3('m3_get_rgb_cv_config', { cameraId })
      : loopbackM3.getRgbCvConfig(cameraId);
  },

  async updateRgbCvConfig(cameraId: string, config: RgbCvConfig): Promise<RgbCvConfig> {
    return isTauriRuntime()
      ? invokeM3('m3_update_rgb_cv_config', { cameraId, config })
      : loopbackM3.updateRgbCvConfig(cameraId, config);
  },

  async stepRgbCameraCv(
    cameraId: string,
    targetDeviceId?: string | null
  ): Promise<CameraCvStepOutput> {
    return isTauriRuntime()
      ? invokeM3('m3_step_rgb_camera_cv', { cameraId, targetDeviceId: targetDeviceId ?? null })
      : loopbackM3.stepRgbCameraCv(cameraId, targetDeviceId);
  },
};
