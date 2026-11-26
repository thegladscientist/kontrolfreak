export type SignalKind =
  | 'control'
  | 'trigger'
  | 'midi'
  | 'video'
  | 'sensor'
  | 'byte_stream';

export type PixelFormat =
  | 'rgba8'
  | 'bgra8'
  | 'uyvy'
  | 'nv12'
  | 'depth16'
  | 'gray8';

export interface FrameRef {
  frame_id: number;
  width: number;
  height: number;
  format: PixelFormat;
  source_id: string;
  timestamp_ns: number;
}

export type SignalValue =
  | { type: 'Float'; value: number }
  | { type: 'Int'; value: number }
  | { type: 'Bool'; value: boolean }
  | { type: 'String'; value: string }
  | { type: 'Trigger' }
  | { type: 'Vec2'; value: [number, number] }
  | { type: 'Vec3'; value: [number, number, number] }
  | { type: 'Bytes'; value: number[] }
  | { type: 'FrameRef'; value: FrameRef };

export type CurveKind =
  | { kind: 'linear' }
  | { kind: 'exponential'; exponent: number }
  | { kind: 'logarithmic'; base: number }
  | { kind: 's_curve'; steepness: number }
  | { kind: 'bezier'; p1: [number, number]; p2: [number, number] };

export type Modifier =
  | { type: 'Invert' }
  | {
      type: 'RangeMap';
      in_min: number;
      in_max: number;
      out_min: number;
      out_max: number;
      clamp: boolean;
    }
  | { type: 'Clamp'; min: number; max: number }
  | { type: 'Deadzone'; center: number; radius: number; rescale: boolean }
  | { type: 'Curve'; kind: CurveKind }
  | { type: 'SmoothOneEuro'; min_cutoff: number; beta: number; d_cutoff: number }
  | {
      type: 'Hysteresis';
      low_threshold: number;
      high_threshold: number;
      low_value: number;
      high_value: number;
    }
  | {
      type: 'Threshold';
      threshold: number;
      rising_only: boolean;
      emit_trigger: boolean;
    }
  | { type: 'Expression'; expr: string };

export type DeviceStatus =
  | { state: 'online' }
  | { state: 'offline_ghost'; reason?: string | null; last_seen_ms?: number | null }
  | { state: 'learning' }
  | { state: 'error'; message: string };

export interface DeviceFingerprint {
  vid?: number | null;
  pid?: number | null;
  serial?: string | null;
  usage_page?: number | null;
  usage?: number | null;
  interface_number?: number | null;
  port_name?: string | null;
  endpoint?: string | null;
}

export type PortDirection = 'in' | 'out' | 'bidir';

export interface Port {
  id: string;
  logical_key: string;
  alias: string;
  direction: PortDirection;
  signal_kind: SignalKind;
  default_value?: SignalValue | null;
  min_value?: number | null;
  max_value?: number | null;
  modifiers: Modifier[];
  echo_suppression_ms?: number | null;
}

export type MidiBackendKind =
  | 'hardware'
  | 'core_midi_virtual'
  | 'loop_midi_windows'
  | 'alsa_virtual'
  | 'loopback_mock';

export type WsMode = 'server' | 'client';
export type WsFraming = 'json' | 'text' | 'binary' | 'osc_over_ws';
export type UdpFormat = 'utf8' | 'json' | 'csv' | 'hex';

export type DeviceKind =
  | {
      type: 'OscEndpoint';
      bind_addr: string;
      target_addr?: string | null;
      auto_discover: boolean;
    }
  | {
      type: 'MidiPort';
      port_name: string;
      is_virtual: boolean;
      channel_filter?: number | null;
      backend: MidiBackendKind;
    }
  | {
      type: 'WebSocketEndpoint';
      mode: WsMode;
      url_or_bind: string;
      framing: WsFraming;
    }
  | {
      type: 'UdpEndpoint';
      bind_addr: string;
      target_addr?: string | null;
      multicast_group?: string | null;
      broadcast: boolean;
      format: UdpFormat;
    }
  | { type: 'VirtualControl' }
  // M2 seams
  | {
      type: 'RtpMidiSession';
      session_name: string;
      control_port: number;
      remote_host?: string | null;
      mdns_discover: boolean;
    }
  | {
      type: 'NdiVideoNode';
      source_name?: string | null;
      output_name?: string | null;
      out_width: number;
      out_height: number;
      out_fps: number;
    }
  // M3 seams
  | {
      type: 'HidDevice';
      device_preset_id?: string | null;
      vid?: number | null;
      pid?: number | null;
    }
  | {
      type: 'RgbCamera';
      camera_id: string;
      extractor_kind: string;
    }
  // M4 seams
  | {
      type: 'DepthSensor';
      vendor: string;
      sidecar_id?: string | null;
      roi_min_z: number;
      roi_max_z: number;
    };

export interface NodeUi {
  x: number;
  y: number;
  width?: number | null;
  height?: number | null;
  collapsed: boolean;
}

export interface Device {
  id: string;
  logical_key: string;
  display_name: string;
  kind: DeviceKind;
  fingerprint?: DeviceFingerprint | null;
  status: DeviceStatus;
  ports: Port[];
  ui: NodeUi;
  group_id?: string | null;
  manual_override: Record<string, SignalValue>;
}

export interface Wire {
  id: string;
  from_port_id: string;
  to_port_id: string;
  enabled: boolean;
  modifiers: Modifier[];
}

export interface MacroTarget {
  port_id: string;
  min: number;
  max: number;
  invert: boolean;
  curve: CurveKind;
}

export interface MacroBinding {
  id: string;
  alias: string;
  value: number;
  targets: MacroTarget[];
}

export interface RackGroup {
  id: string;
  display_name: string;
  collapsed: boolean;
  member_node_ids: string[];
  macros: MacroBinding[];
  ui: NodeUi;
}

export interface WorkspaceSettings {
  calm_mode: boolean;
  telemetry_hz: number;
  theme: string;
}

export interface WorkspaceGraph {
  project_id: string;
  name: string;
  devices: Device[];
  wires: Wire[];
  groups: RackGroup[];
  settings: WorkspaceSettings;
}

export interface TelemetryEntry {
  port_id: string;
  device_id: string;
  latest_value: SignalValue;
  numeric_value?: number | null;
  active: boolean;
  packets_per_sec: number;
  last_updated_ms: number;
}

export interface TrafficLogEntry {
  timestamp_ms: number;
  protocol: string;
  direction: string;
  summary: string;
  device_id?: string | null;
  port_id?: string | null;
}

export interface TelemetryBatch {
  timestamp_ms: number;
  values: TelemetryEntry[];
  recent_logs: TrafficLogEntry[];
  total_msg_per_sec: number;
}

export interface MidiPortInfo {
  name: string;
  direction: string;
  is_virtual: boolean;
  is_loopmidi: boolean;
  backend: MidiBackendKind;
}

export interface VirtualMidiGuidance {
  platform: string;
  native_supported: boolean;
  loopmidi_detected: boolean;
  recommended_tool?: string | null;
  download_url?: string | null;
  steps: string[];
}

export interface EndpointStatus {
  device_id: string;
  protocol: string;
  running: boolean;
  bound_address?: string | null;
  detail: string;
}

export interface ProjectFile {
  schema_version: number;
  graph: WorkspaceGraph;
}

export interface DevicePreset {
  schema_version: number;
  preset_id: string;
  name: string;
  device: Device;
}

export interface GroupPreset {
  schema_version: number;
  preset_id: string;
  name: string;
  group: RackGroup;
  member_devices: Device[];
  internal_wires: Wire[];
}

export interface MappingPreset {
  schema_version: number;
  preset_id: string;
  name: string;
  description?: string | null;
  modifiers: Modifier[];
}

export type SelectionTarget =
  | { kind: 'none' }
  | { kind: 'device'; id: string }
  | { kind: 'wire'; id: string }
  | { kind: 'rack'; id: string };

export interface ToastNotice {
  id: string;
  title: string;
  description?: string;
  accent?: 'amber' | 'cyan' | 'violet' | 'emerald' | 'rose';
  actionLabel?: string;
  onAction?: () => void;
}
