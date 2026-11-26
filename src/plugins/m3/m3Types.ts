import type { Device, DeviceFingerprint, Modifier } from '../../types/kontrolfreak';

export interface HidDeviceInfo {
  path: string;
  vid: number;
  pid: number;
  serial?: string | null;
  product_name?: string | null;
  manufacturer?: string | null;
  usage_page: number;
  usage: number;
  interface_number: number;
}

export type HidControlType =
  | 'button'
  | 'toggle'
  | 'axis8'
  | 'axis16'
  | 'relative_encoder'
  | 'hat_switch';

export type HidLearnStep =
  | 'idle'
  | 'capturing_baseline'
  | 'recording_control'
  | 'review_and_tune'
  | 'completed';

export type SimulatedHidProfile =
  | 'quirky_imu_gamepad'
  | 'studio_fader_wing'
  | 'unknown_mystery_dial';

export type SimulatedHidAction =
  | { control: 'set_fader_u8'; value: number }
  | { control: 'set_axis_i16_le'; value: number }
  | { control: 'set_dial_u16_be'; value: number }
  | { control: 'set_button_a'; pressed: boolean }
  | { control: 'set_button_b'; pressed: boolean }
  | { control: 'toggle_switch_c' }
  | { control: 'set_hat_switch'; direction: number }
  | { control: 'pulse_encoder'; delta: number }
  | { control: 'set_noise_enabled'; enabled: boolean };

export interface LearnedControlSpec {
  logical_key: string;
  alias: string;
  report_id: number;
  byte_offset: number;
  bit_offset: number;
  bit_mask: number;
  bit_width: number;
  signed: boolean;
  little_endian: boolean;
  control_type: HidControlType;
  observed_min: number;
  observed_max: number;
  sensitivity: number;
  wrap: boolean;
  modifiers: Modifier[];
}

export interface LearnWizardSnapshot {
  step: HidLearnStep;
  device_path?: string | null;
  active_report_id?: number | null;
  baseline_samples: number;
  capture_samples: number;
  resting_bytes: number[];
  latest_bytes: number[];
  noise_mask_bits: number[];
  active_diff_bits: number[];
  candidate?: LearnedControlSpec | null;
  committed_controls: LearnedControlSpec[];
}

export interface HidDeviceDefinition {
  schema_version: number;
  preset_id: string;
  name: string;
  vendor_id: number;
  product_id: number;
  usage_page: number;
  usage: number;
  serial?: string | null;
  noise_masks: Record<number, number[]>;
  controls: LearnedControlSpec[];
}

export type HardwareHotplugEvent =
  | {
      event: 'connected';
      info: HidDeviceInfo;
      fingerprint: DeviceFingerprint;
    }
  | {
      event: 'disconnected';
      path: string;
      fingerprint: DeviceFingerprint;
    };

export interface HotplugNotification {
  event: HardwareHotplugEvent;
  recognized_preset_name?: string | null;
  auto_rebound_device_id?: string | null;
  ghosted_device_ids: string[];
}

export interface SimulatedPlugResponse {
  info: HidDeviceInfo;
  notifications: HotplugNotification[];
}

export interface SavedHidDeviceResponse {
  definition: HidDeviceDefinition;
  preset_json: string;
  device: Device;
}

export interface HidDescriptorField {
  report_id: number;
  usage_page: number;
  usage: number;
  usage_name: string;
  bit_offset: number;
  bit_size: number;
  logical_min: number;
  logical_max: number;
  is_signed: boolean;
  is_relative: boolean;
  is_variable: boolean;
  is_constant: boolean;
}

export interface ParsedHidDescriptor {
  usage_page?: number | null;
  usage?: number | null;
  has_report_ids: boolean;
  fields: HidDescriptorField[];
}

// ---------- RGB Camera & CV Types ----------

export interface RgbCameraInfo {
  id: string;
  name: string;
  width: number;
  height: number;
  fps: number;
  is_synthetic: boolean;
}

export interface NormalizedRoi {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface RgbCvConfig {
  roi: NormalizedRoi;
  target_hue_deg: number;
  hue_tolerance_deg: number;
  min_saturation: number;
  min_value: number;
  motion_pixel_threshold: number;
  zone_cols: number;
  zone_rows: number;
  zone_trigger_threshold: number;
  parameter_modifiers: Record<string, Modifier[]>;
}

export interface MotionZoneReading {
  index: number;
  col: number;
  row: number;
  energy: number;
  triggered: boolean;
}

export interface RgbCvResult {
  global_luma: number;
  roi_luma: number;
  motion_energy: number;
  motion_centroid: [number, number];
  dominant_rgb: [number, number, number];
  dominant_hsv: [number, number, number];
  brightest_point: [number, number];
  brightest_luma: number;
  blob_centroid: [number, number];
  blob_area: number;
  zones: MotionZoneReading[];
}

export interface CameraCvStepOutput {
  camera_id: string;
  frame_index: number;
  width: number;
  height: number;
  cv: RgbCvResult;
  routed_count: number;
}

export const DEFAULT_RGB_CV_CONFIG: RgbCvConfig = {
  roi: { x: 0.08, y: 0.08, w: 0.84, h: 0.84 },
  target_hue_deg: 158,
  hue_tolerance_deg: 36,
  min_saturation: 0.35,
  min_value: 0.25,
  motion_pixel_threshold: 0.07,
  zone_cols: 2,
  zone_rows: 2,
  zone_trigger_threshold: 0.04,
  parameter_modifiers: {},
};
