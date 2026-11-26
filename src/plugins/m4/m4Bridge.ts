import type { Device, Port, SignalValue, WorkspaceGraph } from '../../types/kontrolfreak';
import { bridge, isTauriRuntime } from '../../lib/bridge';
import {
  colorizeDepthToRgbaJs,
  createDefaultRoi,
  decodeBase64ToRgba,
  extractRoiJs,
  generateSyntheticDepthFrame,
} from './colormap';
import type {
  DepthColormap,
  DepthColormapConfig,
  DepthPreviewPacket,
  DepthSidecarStatusDto,
  DiagnosticsSnapshot,
  KfreakExportResult,
  KfreakImportResult,
  KfreakManifest,
  NamedDepthRoi,
  RoiExtractionResult,
  SidecarHealthStatus,
  SidecarSupervisorSnapshot,
  VendorAdapterInfo,
} from './types';
import {
  base64ToBytes,
  bytesToBase64,
  fromUtf8,
  readZipStore,
  sha256Hex,
  utf8,
  writeZipStore,
  type ZipEntry,
} from './zip';

export const KFREAK_BUNDLE_VERSION = 1;
export const KFREAK_PROJECT_ENTRY = 'project.kfproj.json';
export const KFREAK_MANIFEST_ENTRY = 'manifest.json';

export const DEPTH_PORT_KEYS = {
  closestZ: 'depth.closest_z',
  closestXyz: 'depth.closest_xyz',
  centroidXyz: 'depth.centroid_xyz',
  presence: 'depth.presence',
  fillRatio: 'depth.fill_ratio',
  ndiOut: 'depth.ndi_out',
} as const;

const VENDORS: VendorAdapterInfo[] = [
  {
    vendor_id: 'synthetic',
    display_name: 'Synthetic 3D Depth Simulator',
    sdk_compiled: true,
    hardware_detected: true,
    notes: 'Built-in animated 3D depth generator (30-60 Hz) for testing without physical cameras.',
  },
  {
    vendor_id: 'realsense',
    display_name: 'Intel RealSense D400',
    sdk_compiled: false,
    hardware_detected: false,
    notes: 'Isolated sidecar adapter for librealsense2 (falls back to synthetic stream if disconnected).',
  },
  {
    vendor_id: 'orbbec',
    display_name: 'Orbbec Femto / Astra',
    sdk_compiled: false,
    hardware_detected: false,
    notes: 'Isolated sidecar adapter for OrbbecSDK (falls back to synthetic stream if disconnected).',
  },
  {
    vendor_id: 'kinect',
    display_name: 'Azure Kinect DK',
    sdk_compiled: false,
    hardware_detected: false,
    notes: 'Isolated sidecar adapter for Azure Kinect k4a (falls back to synthetic stream if disconnected).',
  },
];

// ---------- `.kfreak` bundle (pure functions, shared by browser loopback + tests) ----------

function slug(s: string): string {
  const out = s
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '');
  return out || 'preset';
}

function uuid(): string {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
    return crypto.randomUUID();
  }
  return 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, (c) => {
    const r = (Math.random() * 16) | 0;
    return (c === 'x' ? r : (r & 0x3) | 0x8).toString(16);
  });
}

export function buildKfreakBundle(
  projectJson: string,
  now: () => number = Date.now
): { bytes: Uint8Array; manifest: KfreakManifest; filename: string } {
  const parsed = JSON.parse(projectJson) as { schema_version: number; graph: WorkspaceGraph };
  const graph = parsed.graph;
  const entries: ZipEntry[] = [];
  const checksums: Record<string, string> = {};
  const add = (name: string, text: string) => {
    const data = utf8(text);
    entries.push({ name, data });
    checksums[name] = sha256Hex(data);
  };

  add(KFREAK_PROJECT_ENTRY, JSON.stringify(parsed, null, 2));

  const devicePaths: string[] = [];
  graph.devices.forEach((d, i) => {
    const p = `presets/devices/${String(i + 1).padStart(2, '0')}-${slug(d.display_name)}.kfdevice.json`;
    add(
      p,
      JSON.stringify(
        { schema_version: 1, preset_id: uuid(), name: d.display_name, device: d, hid_mappings: [] },
        null,
        2
      )
    );
    devicePaths.push(p);
  });

  const groupPaths: string[] = [];
  graph.groups.forEach((g, i) => {
    const members = graph.devices.filter((d) => g.member_node_ids.includes(d.id));
    const portIds = new Set(members.flatMap((d) => d.ports.map((p) => p.id)));
    const internal = graph.wires.filter(
      (w) => portIds.has(w.from_port_id) && portIds.has(w.to_port_id)
    );
    const p = `presets/groups/${String(i + 1).padStart(2, '0')}-${slug(g.display_name)}.kfgroup.json`;
    add(
      p,
      JSON.stringify(
        {
          schema_version: 1,
          preset_id: uuid(),
          name: g.display_name,
          group: g,
          member_devices: members,
          internal_wires: internal,
        },
        null,
        2
      )
    );
    groupPaths.push(p);
  });

  const mappingPaths: string[] = [];
  graph.wires
    .filter((w) => w.modifiers.length > 0)
    .forEach((w, i) => {
      const name = `Wire Mapping ${i + 1}`;
      const p = `presets/mappings/${String(i + 1).padStart(2, '0')}-${slug(name)}.kfmapping.json`;
      add(
        p,
        JSON.stringify(
          {
            schema_version: 1,
            preset_id: uuid(),
            name,
            description: `Extracted from wire ${w.id}`,
            modifiers: w.modifiers,
          },
          null,
          2
        )
      );
      mappingPaths.push(p);
    });

  const manifest: KfreakManifest = {
    bundle_version: KFREAK_BUNDLE_VERSION,
    created_at_ms: now(),
    project_id: graph.project_id,
    project_name: graph.name,
    project_file: KFREAK_PROJECT_ENTRY,
    device_presets: devicePaths,
    group_presets: groupPaths,
    mapping_presets: mappingPaths,
    checksums,
  };
  entries.push({ name: KFREAK_MANIFEST_ENTRY, data: utf8(JSON.stringify(manifest, null, 2)) });

  return {
    bytes: writeZipStore(entries),
    manifest,
    filename: `${slug(graph.name)}.kfreak`,
  };
}

export interface ParsedKfreak {
  manifest: KfreakManifest;
  projectJson: string;
  graph: WorkspaceGraph;
  devicePresets: Array<{ device: Device }>;
  groupPresets: unknown[];
  mappingPresets: unknown[];
}

export function parseKfreakBundle(bytes: Uint8Array): ParsedKfreak {
  const entries = readZipStore(bytes);
  const byName = new Map(entries.map((e) => [e.name, e.data]));
  const mf = byName.get(KFREAK_MANIFEST_ENTRY);
  if (!mf) throw new Error(`Missing manifest file '${KFREAK_MANIFEST_ENTRY}' in .kfreak archive`);
  const manifest = JSON.parse(fromUtf8(mf)) as KfreakManifest;
  if (manifest.bundle_version > KFREAK_BUNDLE_VERSION) {
    throw new Error(`Unsupported .kfreak bundle version ${manifest.bundle_version}`);
  }

  for (const [path, expected] of Object.entries(manifest.checksums)) {
    const data = byName.get(path);
    if (!data) throw new Error(`Bundle entry '${path}' listed in manifest is missing from archive`);
    const actual = sha256Hex(data);
    if (actual !== expected) {
      throw new Error(
        `SHA-256 checksum mismatch for '${path}': expected ${expected}, got ${actual}`
      );
    }
  }

  const read = (path: string): string => {
    const d = byName.get(path);
    if (!d) throw new Error(`Bundle entry '${path}' listed in manifest is missing from archive`);
    return fromUtf8(d);
  };
  const projectJson = read(manifest.project_file);
  const project = JSON.parse(projectJson) as { graph: WorkspaceGraph };

  return {
    manifest,
    projectJson,
    graph: project.graph,
    devicePresets: manifest.device_presets.map((p) => JSON.parse(read(p))),
    groupPresets: manifest.group_presets.map((p) => JSON.parse(read(p))),
    mappingPresets: manifest.mapping_presets.map((p) => JSON.parse(read(p))),
  };
}

// ---------- Browser loopback depth sidecar engine ----------

interface LoopbackSidecar {
  deviceId: string;
  vendor: string;
  status: SidecarHealthStatus;
  pid: number | null;
  startedAtMs: number;
  restartAtMs: number | null;
  restartCount: number;
  consecutiveCrashes: number;
  lastCrashReason: string | null;
  framesBase: number;
  rois: NamedDepthRoi[];
  colormap: DepthColormapConfig;
  droppedFrames: number;
  frameClockMs: number;
}

const LOOPBACK_W = 160;
const LOOPBACK_H = 120;
const LOOPBACK_FPS = 30;
const BACKOFF_BASE_MS = 150;
const BACKOFF_MAX_MS = 4000;

/** Mirrors the Rust `SidecarSupervisor` state machine (crash -> backoff -> auto-restart) in-browser. */
export class M4LoopbackEngine {
  private sidecars = new Map<string, LoopbackSidecar>();
  private nextPid = 41000;
  private latenciesUs: number[] = [];
  private protoTotals = new Map<string, number>();
  private protoWindow = new Map<string, { count: number; atMs: number; rate: number }>();
  private lastLatencyFrame = new Map<string, number>();

  constructor(private now: () => number = Date.now) {}

  listVendors(): VendorAdapterInfo[] {
    return VENDORS.map((v) => ({ ...v }));
  }

  private tick(s: LoopbackSidecar): void {
    const t = this.now();
    if (s.status === 'crashed_restarting' && s.restartAtMs !== null && t >= s.restartAtMs) {
      s.status = 'running';
      s.pid = this.nextPid++;
      s.startedAtMs = t;
      s.restartAtMs = null;
      s.frameClockMs = t;
    }
  }

  private framesOf(s: LoopbackSidecar): number {
    if (s.status !== 'running') return s.framesBase;
    return s.framesBase + Math.floor(((this.now() - s.frameClockMs) * LOOPBACK_FPS) / 1000);
  }

  private snapshot(s: LoopbackSidecar): SidecarSupervisorSnapshot {
    this.tick(s);
    const frames = this.framesOf(s);
    const running = s.status === 'running';
    return {
      sidecar_id: s.deviceId,
      vendor: s.vendor,
      status: s.status,
      pid: s.pid,
      uptime_ms: running ? this.now() - s.startedAtMs : 0,
      restart_count: s.restartCount,
      consecutive_crashes: s.consecutiveCrashes,
      last_crash_reason: s.lastCrashReason,
      frames_received: frames,
      fps: running ? LOOPBACK_FPS : 0,
      width: LOOPBACK_W,
      height: LOOPBACK_H,
      ring_health: {
        slot_count: 4,
        write_seq: frames,
        read_seq: frames,
        dropped_frames: s.droppedFrames,
        utilization: running ? 0.25 : 0,
        producer_pid: s.pid ?? 0,
        producer_alive: running,
      },
    };
  }

  private dto(s: LoopbackSidecar): DepthSidecarStatusDto {
    const health = this.snapshot(s);
    return {
      device_id: s.deviceId,
      health,
      rois: s.rois.map((r) => ({ ...r })),
      colormap: { ...s.colormap },
      latest_results: [],
    };
  }

  startSidecar(deviceId: string, vendor = 'synthetic'): DepthSidecarStatusDto {
    const t = this.now();
    const prev = this.sidecars.get(deviceId);
    const s: LoopbackSidecar = {
      deviceId,
      vendor,
      status: 'running',
      pid: this.nextPid++,
      startedAtMs: t,
      restartAtMs: null,
      restartCount: prev?.restartCount ?? 0,
      consecutiveCrashes: 0,
      lastCrashReason: prev?.lastCrashReason ?? null,
      framesBase: prev ? this.framesOf(prev) : 0,
      rois: prev?.rois ?? [createDefaultRoi()],
      colormap: prev?.colormap ?? { palette: 'turbo', near_mm: 300, far_mm: 2500 },
      droppedFrames: prev?.droppedFrames ?? 0,
      frameClockMs: t,
    };
    this.sidecars.set(deviceId, s);
    return this.dto(s);
  }

  stopSidecar(deviceId: string): DepthSidecarStatusDto {
    const s = this.sidecars.get(deviceId);
    if (!s) return this.statusOrDefault(deviceId);
    s.framesBase = this.framesOf(s);
    s.status = 'stopped';
    s.pid = null;
    s.restartAtMs = null;
    return this.dto(s);
  }

  restartSidecar(deviceId: string): DepthSidecarStatusDto {
    const s = this.sidecars.get(deviceId);
    if (!s) return this.startSidecar(deviceId);
    this.killInternal(s, 'Manual sidecar restart requested', false);
    return this.dto(s);
  }

  simulateCrash(deviceId: string): DepthSidecarStatusDto {
    const s = this.sidecars.get(deviceId);
    if (!s || s.status !== 'running') {
      throw new Error(`No running depth sidecar for device ${deviceId}`);
    }
    this.killInternal(s, 'Sidecar process terminated unexpectedly (SIGKILL)', true);
    return this.dto(s);
  }

  private killInternal(s: LoopbackSidecar, reason: string, isCrash: boolean): void {
    s.framesBase = this.framesOf(s);
    s.status = 'crashed_restarting';
    s.pid = null;
    s.restartCount += 1;
    if (isCrash) s.consecutiveCrashes += 1;
    s.lastCrashReason = reason;
    const exp = Math.min(8, Math.max(0, s.consecutiveCrashes - 1));
    s.restartAtMs = this.now() + Math.min(BACKOFF_MAX_MS, BACKOFF_BASE_MS * 2 ** exp);
  }

  statusOrDefault(deviceId: string): DepthSidecarStatusDto {
    const s = this.sidecars.get(deviceId);
    if (s) return this.dto(s);
    return {
      device_id: deviceId,
      health: {
        sidecar_id: deviceId,
        vendor: 'synthetic',
        status: 'stopped',
        pid: null,
        uptime_ms: 0,
        restart_count: 0,
        consecutive_crashes: 0,
        last_crash_reason: null,
        frames_received: 0,
        fps: 0,
        width: LOOPBACK_W,
        height: LOOPBACK_H,
        ring_health: null,
      },
      rois: [createDefaultRoi()],
      colormap: { palette: 'turbo', near_mm: 300, far_mm: 2500 },
      latest_results: [],
    };
  }

  configureRois(deviceId: string, rois: NamedDepthRoi[]): NamedDepthRoi[] {
    const s = this.sidecars.get(deviceId);
    if (s) s.rois = rois.map((r) => ({ ...r }));
    return rois;
  }

  setColormap(deviceId: string, palette: DepthColormap, nearMm: number, farMm: number): DepthColormapConfig {
    const cfg: DepthColormapConfig = {
      palette,
      near_mm: Math.max(50, nearMm),
      far_mm: Math.max(Math.max(50, nearMm) + 50, farMm),
    };
    const s = this.sidecars.get(deviceId);
    if (s) s.colormap = cfg;
    return cfg;
  }

  pollPreview(deviceId: string): DepthPreviewPacket {
    const existing = this.sidecars.get(deviceId);
    const dto = this.statusOrDefault(deviceId);
    const health = dto.health;
    const sc = {
      colormap: existing?.colormap ?? dto.colormap,
      rois: existing?.rois ?? dto.rois,
    };
    const seq = health.frames_received;
    const depth = generateSyntheticDepthFrame(LOOPBACK_W, LOOPBACK_H, seq * 0.065);
    const rgba = colorizeDepthToRgbaJs(
      depth,
      LOOPBACK_W,
      LOOPBACK_H,
      sc.colormap.near_mm,
      sc.colormap.far_mm,
      sc.colormap.palette
    );
    const roiResults = sc.rois.map((r) => extractRoiJs(depth, LOOPBACK_W, LOOPBACK_H, r));

    if (health.status === 'running') {
      const last = this.lastLatencyFrame.get(deviceId) ?? seq;
      const fresh = Math.min(64, Math.max(0, seq - last));
      for (let i = 0; i < fresh; i++) {
        const n = last + i;
        this.latenciesUs.push(Math.round(1400 + 600 * Math.sin(n * 0.7) + (n % 17) * 45));
      }
      if (this.latenciesUs.length > 1024) this.latenciesUs.splice(0, this.latenciesUs.length - 1024);
      this.lastLatencyFrame.set(deviceId, seq);
      this.recordProtocol('DEPTH', fresh);
    }

    return {
      device_id: deviceId,
      frame_seq: seq,
      width: LOOPBACK_W,
      height: LOOPBACK_H,
      palette: sc.colormap.palette,
      near_mm: sc.colormap.near_mm,
      far_mm: sc.colormap.far_mm,
      rgba_pixels: rgba,
      frame_ref: {
        frame_id: seq,
        width: LOOPBACK_W,
        height: LOOPBACK_H,
        format: 'rgba8',
        source_id: deviceId,
        timestamp_ns: this.now() * 1_000_000,
      },
      roi_results: roiResults,
      health,
    };
  }

  recordProtocol(protocol: string, count: number): void {
    if (count <= 0) return;
    this.protoTotals.set(protocol, (this.protoTotals.get(protocol) ?? 0) + count);
    const w = this.protoWindow.get(protocol);
    const t = this.now();
    if (!w) {
      this.protoWindow.set(protocol, { count, atMs: t, rate: 0 });
    } else {
      w.count += count;
    }
  }

  diagnostics(extraRates: Array<{ protocol: string; rate: number }> = []): DiagnosticsSnapshot {
    const t = this.now();
    const rates = [] as DiagnosticsSnapshot['protocol_rates'];
    for (const [proto, w] of this.protoWindow) {
      const dt = (t - w.atMs) / 1000;
      if (dt >= 0.25) {
        const inst = w.count / dt;
        w.rate = w.rate === 0 ? inst : w.rate * 0.4 + inst * 0.6;
        w.count = 0;
        w.atMs = t;
      }
      rates.push({
        protocol: proto,
        msg_per_sec: Math.round(w.rate * 10) / 10,
        total_messages: this.protoTotals.get(proto) ?? 0,
      });
    }
    for (const e of extraRates) {
      rates.push({ protocol: e.protocol, msg_per_sec: Math.round(e.rate * 10) / 10, total_messages: 0 });
    }

    const sorted = [...this.latenciesUs].sort((a, b) => a - b);
    const pct = (p: number) =>
      sorted.length ? sorted[Math.min(sorted.length - 1, Math.round((p / 100) * (sorted.length - 1)))] / 1000 : 0;
    const sum = sorted.reduce((a, b) => a + b, 0);

    const sidecars = [...this.sidecars.values()].map((s) => {
      const snap = this.snapshot(s);
      return {
        sidecar_id: s.deviceId,
        vendor: s.vendor,
        status: snap.status,
        pid: snap.pid,
        uptime_ms: snap.uptime_ms,
        restart_count: snap.restart_count,
        last_crash_reason: snap.last_crash_reason,
        frames_received: snap.frames_received,
        fps: snap.fps,
        ring_buffer_utilization: snap.ring_health?.utilization ?? 0,
        ring_buffer_dropped: snap.ring_health?.dropped_frames ?? 0,
      };
    });

    return {
      timestamp_ms: t,
      uptime_ms: t,
      total_msg_per_sec: Math.round(rates.reduce((a, r) => a + r.msg_per_sec, 0) * 10) / 10,
      protocol_rates: rates,
      latency: {
        p50_ms: pct(50),
        p95_ms: pct(95),
        p99_ms: pct(99),
        mean_ms: sorted.length ? sum / sorted.length / 1000 : 0,
        max_ms: sorted.length ? sorted[sorted.length - 1] / 1000 : 0,
        sample_count: sorted.length,
      },
      dropped_frames: sidecars.reduce((a, s) => a + s.ring_buffer_dropped, 0),
      sidecars,
    };
  }
}

export const m4Loopback = new M4LoopbackEngine();

async function tauriInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import('@tauri-apps/api/core');
  return invoke<T>(cmd, args);
}

export function roiToSignalBindings(res: RoiExtractionResult): Array<[string, SignalValue]> {
  return [
    [DEPTH_PORT_KEYS.closestZ, { type: 'Float', value: res.closest_point_norm[2] }],
    [DEPTH_PORT_KEYS.closestXyz, { type: 'Vec3', value: res.closest_point_norm }],
    [DEPTH_PORT_KEYS.centroidXyz, { type: 'Vec3', value: res.centroid_norm }],
    [DEPTH_PORT_KEYS.presence, { type: 'Bool', value: res.presence }],
    [DEPTH_PORT_KEYS.fillRatio, { type: 'Float', value: res.fill_ratio }],
  ];
}

export function findDepthPort(device: Device, key: string): Port | undefined {
  return device.ports.find((p) => p.logical_key === key);
}

/** Unified dual-runtime M4 bridge: Tauri commands in desktop, `M4LoopbackEngine` in browser/tests. */
export const m4Bridge = {
  loopback: m4Loopback,

  async listVendors(): Promise<VendorAdapterInfo[]> {
    return isTauriRuntime() ? tauriInvoke('m4_list_depth_vendors') : m4Loopback.listVendors();
  },

  async startSidecar(deviceId: string, vendor = 'synthetic'): Promise<DepthSidecarStatusDto> {
    return isTauriRuntime()
      ? tauriInvoke('m4_start_depth_sidecar', { deviceId, vendor })
      : m4Loopback.startSidecar(deviceId, vendor);
  },

  async stopSidecar(deviceId: string): Promise<DepthSidecarStatusDto> {
    return isTauriRuntime()
      ? tauriInvoke('m4_stop_depth_sidecar', { deviceId })
      : m4Loopback.stopSidecar(deviceId);
  },

  async restartSidecar(deviceId: string): Promise<DepthSidecarStatusDto> {
    return isTauriRuntime()
      ? tauriInvoke('m4_restart_depth_sidecar', { deviceId })
      : m4Loopback.restartSidecar(deviceId);
  },

  async simulateCrash(deviceId: string): Promise<DepthSidecarStatusDto> {
    return isTauriRuntime()
      ? tauriInvoke('m4_simulate_sidecar_crash', { deviceId })
      : m4Loopback.simulateCrash(deviceId);
  },

  async getStatus(deviceId: string): Promise<DepthSidecarStatusDto> {
    return isTauriRuntime()
      ? tauriInvoke('m4_get_depth_sidecar_status', { deviceId })
      : m4Loopback.statusOrDefault(deviceId);
  },

  async configureRois(deviceId: string, rois: NamedDepthRoi[]): Promise<NamedDepthRoi[]> {
    return isTauriRuntime()
      ? tauriInvoke('m4_configure_depth_rois', { deviceId, rois })
      : m4Loopback.configureRois(deviceId, rois);
  },

  async setColormap(
    deviceId: string,
    palette: DepthColormap,
    nearMm: number,
    farMm: number
  ): Promise<DepthColormapConfig> {
    return isTauriRuntime()
      ? tauriInvoke('m4_set_depth_colormap', { deviceId, palette, nearMm, farMm })
      : m4Loopback.setColormap(deviceId, palette, nearMm, farMm);
  },

  async pollPreview(deviceId: string): Promise<DepthPreviewPacket> {
    if (isTauriRuntime()) {
      const pkt = await tauriInvoke<DepthPreviewPacket>('m4_poll_depth_preview', { deviceId });
      if (pkt.rgba_base64) pkt.rgba_pixels = decodeBase64ToRgba(pkt.rgba_base64);
      return pkt;
    }
    return m4Loopback.pollPreview(deviceId);
  },

  async exportKfreak(): Promise<KfreakExportResult> {
    if (isTauriRuntime()) return tauriInvoke('m4_export_kfreak_bundle');
    const projectJson = await bridge.exportProjectJson();
    const { bytes, manifest, filename } = buildKfreakBundle(projectJson);
    return {
      filename,
      size_bytes: bytes.length,
      manifest,
      zip_base64: bytesToBase64(bytes),
    };
  },

  async importKfreak(zipBase64: string): Promise<KfreakImportResult> {
    if (isTauriRuntime()) {
      return tauriInvoke('m4_import_kfreak_bundle', { zipBase64, reconcileHardware: true });
    }
    const parsed = parseKfreakBundle(base64ToBytes(zipBase64));
    const graph = await bridge.importProjectJson(parsed.projectJson, true);
    return {
      graph,
      manifest: parsed.manifest,
      device_preset_count: parsed.devicePresets.length,
      group_preset_count: parsed.groupPresets.length,
      mapping_preset_count: parsed.mappingPresets.length,
    };
  },

  async getDiagnostics(extraRates: Array<{ protocol: string; rate: number }> = []): Promise<DiagnosticsSnapshot> {
    return isTauriRuntime()
      ? tauriInvoke('m4_get_diagnostics_snapshot')
      : m4Loopback.diagnostics(extraRates);
  },
};
