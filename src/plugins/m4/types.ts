import type { FrameRef, WorkspaceGraph } from '../../types/kontrolfreak';

export type DepthColormap = 'turbo' | 'viridis' | 'grayscale' | 'depth_slice';

export type SidecarHealthStatus =
  | 'stopped'
  | 'starting'
  | 'running'
  | 'degraded'
  | 'crashed_restarting';

export interface VendorAdapterInfo {
  vendor_id: string;
  display_name: string;
  sdk_compiled: boolean;
  hardware_detected: boolean;
  notes: string;
}

export interface NamedDepthRoi {
  id: string;
  name: string;
  x: number;
  y: number;
  w: number;
  h: number;
  near_mm: number;
  far_mm: number;
  occupancy_threshold: number;
}

export interface RoiExtractionResult {
  roi_id: string;
  roi_name: string;
  min_depth_mm: number;
  max_depth_mm: number;
  mean_depth_mm: number;
  presence: boolean;
  fill_ratio: number;
  valid_pixel_count: number;
  total_pixel_count: number;
  closest_point_norm: [number, number, number];
  closest_point_meters: [number, number, number];
  centroid_norm: [number, number, number];
  centroid_meters: [number, number, number];
  confidence: number;
}

export interface ShmRingHealth {
  slot_count: number;
  write_seq: number;
  read_seq: number;
  dropped_frames: number;
  utilization: number;
  producer_pid: number;
  producer_alive: boolean;
}

export interface SidecarSupervisorSnapshot {
  sidecar_id: string;
  vendor: string;
  status: SidecarHealthStatus;
  pid: number | null;
  uptime_ms: number;
  restart_count: number;
  consecutive_crashes: number;
  last_crash_reason: string | null;
  frames_received: number;
  fps: number;
  width: number;
  height: number;
  ring_health: ShmRingHealth | null;
}

export interface DepthColormapConfig {
  palette: DepthColormap;
  near_mm: number;
  far_mm: number;
}

export interface DepthSidecarStatusDto {
  device_id: string;
  health: SidecarSupervisorSnapshot;
  rois: NamedDepthRoi[];
  colormap: DepthColormapConfig;
  latest_results: RoiExtractionResult[];
}

export interface DepthPreviewPacket {
  device_id: string;
  frame_seq: number;
  width: number;
  height: number;
  palette: DepthColormap;
  near_mm: number;
  far_mm: number;
  rgba_base64?: string;
  /** Optional direct Uint8ClampedArray used in browser loopback mode to avoid base64 overhead. */
  rgba_pixels?: Uint8ClampedArray;
  frame_ref: FrameRef;
  roi_results: RoiExtractionResult[];
  health: SidecarSupervisorSnapshot;
}

export interface KfreakManifest {
  bundle_version: number;
  created_at_ms: number;
  project_id: string;
  project_name: string;
  project_file: string;
  device_presets: string[];
  group_presets: string[];
  mapping_presets: string[];
  checksums: Record<string, string>;
}

export interface KfreakExportResult {
  filename: string;
  size_bytes: number;
  manifest: KfreakManifest;
  zip_base64: string;
}

export interface KfreakImportResult {
  graph: WorkspaceGraph;
  manifest: KfreakManifest;
  device_preset_count: number;
  group_preset_count: number;
  mapping_preset_count: number;
}

export interface ProtocolRateEntry {
  protocol: string;
  msg_per_sec: number;
  total_messages: number;
}

export interface LatencyPercentilesMs {
  p50_ms: number;
  p95_ms: number;
  p99_ms: number;
  mean_ms: number;
  max_ms: number;
  sample_count: number;
}

export interface SidecarDiagnosticEntry {
  sidecar_id: string;
  vendor: string;
  status: SidecarHealthStatus;
  pid: number | null;
  uptime_ms: number;
  restart_count: number;
  last_crash_reason: string | null;
  frames_received: number;
  fps: number;
  ring_buffer_utilization: number;
  ring_buffer_dropped: number;
}

export interface DiagnosticsSnapshot {
  timestamp_ms: number;
  uptime_ms: number;
  total_msg_per_sec: number;
  protocol_rates: ProtocolRateEntry[];
  latency: LatencyPercentilesMs;
  dropped_frames: number;
  sidecars: SidecarDiagnosticEntry[];
}
