import { isTauriRuntime } from '../../lib/bridge';
import { telemetryStore } from '../../lib/telemetryStore';
import type { Device } from '../../types/kontrolfreak';

// ---------- Shared M2 TypeScript DTOs (mirroring Rust `kf-video` & `kf-protocols`) ----------

export type VideoFitMode = 'contain' | 'cover' | 'stretch' | 'crop';
export type TestPatternKind = 'smpte_bars' | 'animated_gradient' | 'motion_grid';

export interface NdiRuntimeStatus {
  available: boolean;
  library_path: string | null;
  version: string | null;
  install_url: string;
}

export interface VideoSourceInfo {
  name: string;
  url_address: string | null;
  width: number | null;
  height: number | null;
  fps: number | null;
}

export interface NdiTransformConfig {
  fit_mode: VideoFitMode;
  crop_x: number;
  crop_y: number;
  crop_w: number;
  crop_h: number;
  zoom: number;
  out_width: number;
  out_height: number;
  out_fps: number;
  opacity: number;
  freeze: boolean;
  bypass: boolean;
}

export interface NdiNodeConfigPatch {
  source_name?: string;
  output_name?: string;
  test_pattern?: TestPatternKind;
  fit_mode?: VideoFitMode;
  crop_x?: number;
  crop_y?: number;
  crop_w?: number;
  crop_h?: number;
  zoom?: number;
  out_width?: number;
  out_height?: number;
  out_fps?: number;
  opacity?: number;
  freeze?: boolean;
  bypass?: boolean;
}

export interface NdiNodeState {
  device_id: string;
  source_name: string;
  output_name: string;
  using_synthetic_source: boolean;
  test_pattern: TestPatternKind;
  transform: NdiTransformConfig;
  active_width: number;
  active_height: number;
  pending_width: number | null;
  pending_height: number | null;
  reallocation_count: number;
  frames_rendered: number;
  scaler_backend: string;
}

export interface NdiPreviewPayload {
  device_id: string;
  thumb_width: number;
  thumb_height: number;
  output_width: number;
  output_height: number;
  out_fps: number;
  jpeg_base64: string;
  timestamp_ms: number;
  scaler_backend: string;
  using_synthetic_source: boolean;
  pending_resize: boolean;
}

export interface MdnsStatus {
  active: boolean;
  announcing: boolean;
  service_type: string;
  fallback_reason: string | null;
}

export interface RtpMidiPeer {
  name: string;
  host: string;
  control_port: number;
  ssrc: number | null;
}

export type PeerConnectionState = 'inviting_control' | 'inviting_data' | 'connected';

export interface ConnectedPeerInfo {
  name: string;
  ssrc: number;
  control_addr: string;
  data_addr: string;
  state: PeerConnectionState;
  rtt_ms: number | null;
  clock_offset_ms: number | null;
  sync_count: number;
  last_rx_seq: number | null;
}

export interface RtpMidiSessionStatus {
  session_name: string;
  local_ssrc: number;
  control_port: number;
  data_port: number;
  running: boolean;
  mdns: MdnsStatus;
  connected_peers: ConnectedPeerInfo[];
  discovered_peers: RtpMidiPeer[];
  tx_packets: number;
  rx_packets: number;
  recovered_messages: number;
}

// ---------- Built-in Synthetic Sources & Browser Loopback Mock Engine ----------

export const SYNTHETIC_NDI_SOURCES: VideoSourceInfo[] = [
  {
    name: 'Test Pattern: SMPTE Bars (1920x1080)',
    url_address: 'synthetic://smpte-bars',
    width: 1920,
    height: 1080,
    fps: 60,
  },
  {
    name: 'Test Pattern: Motion Grid (1920x1080)',
    url_address: 'synthetic://motion-grid',
    width: 1920,
    height: 1080,
    fps: 60,
  },
  {
    name: 'Test Pattern: Animated Gradient (1280x720)',
    url_address: 'synthetic://animated-gradient',
    width: 1280,
    height: 720,
    fps: 30,
  },
];

const MOCK_RTPMIDI_DISCOVERED_PEERS: RtpMidiPeer[] = [
  { name: 'iPad Pro (AudioMIDI)', host: '192.168.1.42', control_port: 5004, ssrc: 0x3a10_4b22 },
  { name: 'Ableton Mac Studio', host: '192.168.1.18', control_port: 5006, ssrc: 0x7c90_11ef },
  { name: 'Win11 rtpMIDI Rig', host: '192.168.1.77', control_port: 5004, ssrc: 0x5521_88aa },
];

export const RESIZE_DEBOUNCE_MS = 150;

interface MockNdiEntry {
  state: NdiNodeState;
  pendingTimer: ReturnType<typeof setTimeout> | null;
}

type StateListener = () => void;

class BrowserM2MockEngine {
  private ndiNodes = new Map<string, MockNdiEntry>();
  private rtpSessions = new Map<string, RtpMidiSessionStatus>();
  private listeners = new Set<StateListener>();

  subscribe(fn: StateListener): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  private emit(): void {
    for (const fn of this.listeners) fn();
  }

  getNdiRuntimeStatus(): NdiRuntimeStatus {
    return {
      available: false,
      library_path: null,
      version: null,
      install_url: 'https://ndi.video/tools/',
    };
  }

  discoverNdiSources(): VideoSourceInfo[] {
    return SYNTHETIC_NDI_SOURCES.map((s) => ({ ...s }));
  }

  private ensureNdi(deviceId: string, device?: Device): MockNdiEntry {
    let entry = this.ndiNodes.get(deviceId);
    if (!entry) {
      const kind = device?.kind.type === 'NdiVideoNode' ? device.kind : null;
      const outW = kind?.out_width ?? 1280;
      const outH = kind?.out_height ?? 720;
      const outFps = kind?.out_fps ?? 30;
      const sourceName = kind?.source_name ?? SYNTHETIC_NDI_SOURCES[0].name;
      entry = {
        state: {
          device_id: deviceId,
          source_name: sourceName,
          output_name: kind?.output_name ?? 'Kontrolfreak Out',
          using_synthetic_source: true,
          test_pattern: parsePatternFromSource(sourceName),
          transform: {
            fit_mode: 'contain',
            crop_x: 0,
            crop_y: 0,
            crop_w: 1,
            crop_h: 1,
            zoom: 1,
            out_width: outW,
            out_height: outH,
            out_fps: outFps,
            opacity: 1,
            freeze: false,
            bypass: false,
          },
          active_width: outW,
          active_height: outH,
          pending_width: null,
          pending_height: null,
          reallocation_count: 0,
          frames_rendered: 0,
          scaler_backend: 'canvas-2d (browser loopback)',
        },
        pendingTimer: null,
      };
      this.ndiNodes.set(deviceId, entry);
    }
    return entry;
  }

  getNdiNodeState(deviceId: string, device?: Device): NdiNodeState {
    const entry = this.ensureNdi(deviceId, device);
    return structuredCloneSafe(entry.state);
  }

  configureNdiNode(deviceId: string, patch: NdiNodeConfigPatch, device?: Device): NdiNodeState {
    const entry = this.ensureNdi(deviceId, device);
    const st = entry.state;

    if (patch.source_name !== undefined) {
      st.source_name = patch.source_name;
      st.test_pattern = parsePatternFromSource(patch.source_name);
      st.using_synthetic_source = patch.source_name.toLowerCase().includes('test pattern');
    }
    if (patch.output_name !== undefined) st.output_name = patch.output_name;
    if (patch.test_pattern !== undefined) st.test_pattern = patch.test_pattern;
    if (patch.fit_mode !== undefined) st.transform.fit_mode = patch.fit_mode;
    if (patch.crop_x !== undefined) st.transform.crop_x = clamp(patch.crop_x, 0, 1);
    if (patch.crop_y !== undefined) st.transform.crop_y = clamp(patch.crop_y, 0, 1);
    if (patch.crop_w !== undefined) st.transform.crop_w = clamp(patch.crop_w, 0.05, 1);
    if (patch.crop_h !== undefined) st.transform.crop_h = clamp(patch.crop_h, 0.05, 1);
    if (patch.zoom !== undefined) st.transform.zoom = clamp(patch.zoom, 1, 8);
    if (patch.out_fps !== undefined) st.transform.out_fps = clamp(patch.out_fps, 1, 120);
    if (patch.opacity !== undefined) st.transform.opacity = clamp(patch.opacity, 0, 1);
    if (patch.freeze !== undefined) st.transform.freeze = patch.freeze;
    if (patch.bypass !== undefined) st.transform.bypass = patch.bypass;

    if (patch.out_width !== undefined || patch.out_height !== undefined) {
      const nextW = Math.round(clamp(patch.out_width ?? st.transform.out_width, 16, 7680));
      const nextH = Math.round(clamp(patch.out_height ?? st.transform.out_height, 16, 4320));
      st.transform.out_width = nextW;
      st.transform.out_height = nextH;

      if (nextW !== st.active_width || nextH !== st.active_height) {
        st.pending_width = nextW;
        st.pending_height = nextH;
        if (entry.pendingTimer) clearTimeout(entry.pendingTimer);
        entry.pendingTimer = setTimeout(() => {
          st.active_width = nextW;
          st.active_height = nextH;
          st.pending_width = null;
          st.pending_height = null;
          st.reallocation_count += 1;
          entry.pendingTimer = null;
          this.emit();
        }, RESIZE_DEBOUNCE_MS);
      } else {
        st.pending_width = null;
        st.pending_height = null;
        if (entry.pendingTimer) {
          clearTimeout(entry.pendingTimer);
          entry.pendingTimer = null;
        }
      }
    }

    this.emit();
    return structuredCloneSafe(st);
  }

  /**
   * Computes live modulated transform by overlaying any active telemetry signals arriving on an
   * NDI device's input ports (`ndi.crop_x`, `ndi.crop_y`, `ndi.crop_w`, `ndi.crop_h`, `ndi.zoom`,
   * `ndi.opacity`, `ndi.out_width`, `ndi.out_height`).
   */
  getEffectiveTransform(device: Device): NdiTransformConfig {
    const base = this.ensureNdi(device.id, device).state;
    const t: NdiTransformConfig = {
      ...base.transform,
      out_width: base.active_width,
      out_height: base.active_height,
    };

    for (const port of device.ports) {
      const entry = telemetryStore.getPortLatest(port.id);
      if (!entry || typeof entry.numeric_value !== 'number') continue;
      const v = entry.numeric_value;
      const key = port.logical_key.toLowerCase().replace(/^ndi\./, '');
      switch (key) {
        case 'crop_x':
          t.crop_x = clamp(v, 0, 1);
          break;
        case 'crop_y':
          t.crop_y = clamp(v, 0, 1);
          break;
        case 'crop_w':
          t.crop_w = clamp(v, 0.05, 1);
          break;
        case 'crop_h':
          t.crop_h = clamp(v, 0.05, 1);
          break;
        case 'zoom':
          t.zoom = v <= 1 ? 1 + clamp(v, 0, 1) * 3 : clamp(v, 1, 8);
          break;
        case 'opacity':
          t.opacity = clamp(v, 0, 1);
          break;
        default:
          break;
      }
    }
    return t;
  }

  // ---------- RTPMIDI Browser Loopback Mock ----------

  private ensureRtp(deviceId: string, device?: Device): RtpMidiSessionStatus {
    let st = this.rtpSessions.get(deviceId);
    if (!st) {
      const kind = device?.kind.type === 'RtpMidiSession' ? device.kind : null;
      const controlPort = kind?.control_port ?? 5004;
      st = {
        session_name: kind?.session_name ?? 'Kontrolfreak LAN',
        local_ssrc: 0x4b46_0001 + (this.rtpSessions.size * 17),
        control_port: controlPort,
        data_port: controlPort + 1,
        running: true,
        mdns: {
          active: kind?.mdns_discover ?? true,
          announcing: kind?.mdns_discover ?? true,
          service_type: '_apple-midi._udp.local.',
          fallback_reason: null,
        },
        connected_peers: [
          {
            name: 'iPad Pro (AudioMIDI)',
            ssrc: 0x3a10_4b22,
            control_addr: '192.168.1.42:5004',
            data_addr: '192.168.1.42:5005',
            state: 'connected',
            rtt_ms: 1.4,
            clock_offset_ms: 0.2,
            sync_count: 3,
            last_rx_seq: 128,
          },
        ],
        discovered_peers: MOCK_RTPMIDI_DISCOVERED_PEERS.map((p) => ({ ...p })),
        tx_packets: 18,
        rx_packets: 42,
        recovered_messages: 0,
      };
      this.rtpSessions.set(deviceId, st);
    }
    return st;
  }

  getRtpMidiStatus(deviceId: string, device?: Device): RtpMidiSessionStatus {
    return structuredCloneSafe(this.ensureRtp(deviceId, device));
  }

  startRtpMidiSession(deviceId: string, device?: Device): RtpMidiSessionStatus {
    const st = this.ensureRtp(deviceId, device);
    const kind = device?.kind.type === 'RtpMidiSession' ? device.kind : null;
    if (kind) {
      st.session_name = kind.session_name;
      st.control_port = kind.control_port;
      st.data_port = kind.control_port + 1;
      st.mdns.active = kind.mdns_discover;
      st.mdns.announcing = kind.mdns_discover;
    }
    st.running = true;
    this.emit();
    return structuredCloneSafe(st);
  }

  stopRtpMidiSession(deviceId: string): void {
    const st = this.rtpSessions.get(deviceId);
    if (st) {
      st.running = false;
      st.connected_peers = [];
      this.emit();
    }
  }

  discoverRtpMidiPeers(): RtpMidiPeer[] {
    return MOCK_RTPMIDI_DISCOVERED_PEERS.map((p) => ({ ...p }));
  }

  connectRtpMidiPeer(deviceId: string, peerControlAddr: string, device?: Device): RtpMidiSessionStatus {
    const st = this.ensureRtp(deviceId, device);
    st.running = true;
    const [host, portStr] = peerControlAddr.split(':');
    const port = Number(portStr) || 5004;
    const matched = st.discovered_peers.find(
      (p) => p.host === host && p.control_port === port
    );
    const exists = st.connected_peers.some((p) => p.control_addr === `${host}:${port}`);
    if (!exists) {
      st.connected_peers.push({
        name: matched?.name ?? `Peer ${host}`,
        ssrc: matched?.ssrc ?? Math.floor(Math.random() * 0x7fff_ffff),
        control_addr: `${host}:${port}`,
        data_addr: `${host}:${port + 1}`,
        state: 'connected',
        rtt_ms: Number((0.9 + Math.random() * 1.8).toFixed(1)),
        clock_offset_ms: Number(((Math.random() - 0.5) * 0.6).toFixed(2)),
        sync_count: 1,
        last_rx_seq: 1,
      });
    }
    this.emit();
    return structuredCloneSafe(st);
  }

  syncRtpMidiClock(deviceId: string, device?: Device): RtpMidiSessionStatus {
    const st = this.ensureRtp(deviceId, device);
    for (const peer of st.connected_peers) {
      peer.sync_count += 1;
      peer.rtt_ms = Number((0.8 + Math.random() * 1.6).toFixed(1));
      peer.clock_offset_ms = Number(((Math.random() - 0.5) * 0.4).toFixed(2));
    }
    this.emit();
    return structuredCloneSafe(st);
  }

  sendTestMidi(deviceId: string, device?: Device): RtpMidiSessionStatus {
    const st = this.ensureRtp(deviceId, device);
    st.tx_packets += 1;
    this.emit();
    return structuredCloneSafe(st);
  }
}

export const browserM2Mock = new BrowserM2MockEngine();

async function tauriInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import('@tauri-apps/api/core');
  return invoke<T>(cmd, args);
}

/** Dual-runtime bridge for all Milestone 2 NDI & RTPMIDI backend commands. */
export const m2Bridge = {
  async getNdiRuntimeStatus(): Promise<NdiRuntimeStatus> {
    return isTauriRuntime()
      ? tauriInvoke('m2_get_ndi_runtime_status')
      : browserM2Mock.getNdiRuntimeStatus();
  },

  async discoverNdiSources(): Promise<VideoSourceInfo[]> {
    return isTauriRuntime()
      ? tauriInvoke('m2_discover_ndi_sources')
      : browserM2Mock.discoverNdiSources();
  },

  async getNdiNodeState(deviceId: string, device?: Device): Promise<NdiNodeState> {
    return isTauriRuntime()
      ? tauriInvoke('m2_get_ndi_node_state', { deviceId })
      : browserM2Mock.getNdiNodeState(deviceId, device);
  },

  async configureNdiNode(
    deviceId: string,
    patch: NdiNodeConfigPatch,
    device?: Device
  ): Promise<NdiNodeState> {
    return isTauriRuntime()
      ? tauriInvoke('m2_configure_ndi_node', { deviceId, patch })
      : browserM2Mock.configureNdiNode(deviceId, patch, device);
  },

  async getNdiPreviewFrame(
    deviceId: string,
    maxWidth = 240,
    maxHeight = 135
  ): Promise<NdiPreviewPayload | null> {
    if (!isTauriRuntime()) return null;
    return tauriInvoke('m2_get_ndi_preview_frame', {
      deviceId,
      maxWidth,
      maxHeight,
    });
  },

  async startRtpMidiSession(deviceId: string, device?: Device): Promise<void> {
    if (isTauriRuntime()) {
      await tauriInvoke('m2_start_rtpmidi_session', { deviceId });
      return;
    }
    browserM2Mock.startRtpMidiSession(deviceId, device);
  },

  async stopRtpMidiSession(deviceId: string): Promise<void> {
    if (isTauriRuntime()) {
      await tauriInvoke('m2_stop_rtpmidi_session', { deviceId });
      return;
    }
    browserM2Mock.stopRtpMidiSession(deviceId);
  },

  async getRtpMidiStatus(deviceId: string, device?: Device): Promise<RtpMidiSessionStatus | null> {
    return isTauriRuntime()
      ? tauriInvoke('m2_get_rtpmidi_status', { deviceId })
      : browserM2Mock.getRtpMidiStatus(deviceId, device);
  },

  async discoverRtpMidiPeers(deviceId?: string): Promise<RtpMidiPeer[]> {
    if (isTauriRuntime() && deviceId) {
      return tauriInvoke('m2_discover_rtpmidi_peers', { deviceId });
    }
    return browserM2Mock.discoverRtpMidiPeers();
  },

  async connectRtpMidiPeer(
    deviceId: string,
    peerControlAddr: string,
    device?: Device
  ): Promise<void> {
    if (isTauriRuntime()) {
      await tauriInvoke('m2_connect_rtpmidi_peer', { deviceId, peerControlAddr });
      return;
    }
    browserM2Mock.connectRtpMidiPeer(deviceId, peerControlAddr, device);
  },

  async syncRtpMidiClock(deviceId: string, device?: Device): Promise<void> {
    if (isTauriRuntime()) {
      await tauriInvoke('m2_sync_rtpmidi_clock', { deviceId });
      return;
    }
    browserM2Mock.syncRtpMidiClock(deviceId, device);
  },

  async sendRtpMidiTestMidi(
    deviceId: string,
    midiBytes: number[],
    device?: Device
  ): Promise<void> {
    if (isTauriRuntime()) {
      await tauriInvoke('m2_send_rtpmidi_test_midi', { deviceId, midiBytes });
      return;
    }
    browserM2Mock.sendTestMidi(deviceId, device);
  },
};

// ---------- Canvas Synthetic Test Pattern & Transform Renderer (Browser Loopback) ----------

export function computeFitGeometry(
  srcW: number,
  srcH: number,
  outW: number,
  outH: number,
  t: NdiTransformConfig
): { sx: number; sy: number; sw: number; sh: number; dx: number; dy: number; dw: number; dh: number } {
  const zoom = Math.max(1, t.zoom || 1);
  let nw = clamp(t.crop_w, 0.05, 1);
  let nh = clamp(t.crop_h, 0.05, 1);
  let nx = Math.min(clamp(t.crop_x, 0, 1), Math.max(0, 1 - nw));
  let ny = Math.min(clamp(t.crop_y, 0, 1), Math.max(0, 1 - nh));

  if (zoom > 1.0001) {
    const zw = nw / zoom;
    const zh = nh / zoom;
    const cx =
      nw >= 0.999
        ? t.crop_x <= 0.0001
          ? 0.5
          : zw * 0.5 + clamp(t.crop_x, 0, 1) * (1 - zw)
        : nx + nw * 0.5;
    const cy =
      nh >= 0.999
        ? t.crop_y <= 0.0001
          ? 0.5
          : zh * 0.5 + clamp(t.crop_y, 0, 1) * (1 - zh)
        : ny + nh * 0.5;
    nw = zw;
    nh = zh;
    nx = clamp(cx - nw * 0.5, 0, Math.max(0, 1 - nw));
    ny = clamp(cy - nh * 0.5, 0, Math.max(0, 1 - nh));
  }

  let sx = nx * srcW;
  let sy = ny * srcH;
  let sw = nw * srcW;
  let sh = nh * srcH;

  let dx = 0;
  let dy = 0;
  let dw = outW;
  let dh = outH;

  const srcAspect = sw / Math.max(1, sh);
  const dstAspect = outW / Math.max(1, outH);

  if (t.fit_mode === 'contain') {
    if (srcAspect > dstAspect) {
      dh = outW / srcAspect;
      dy = (outH - dh) * 0.5;
    } else {
      dw = outH * srcAspect;
      dx = (outW - dw) * 0.5;
    }
  } else if (t.fit_mode === 'cover') {
    if (srcAspect > dstAspect) {
      const nextSw = sh * dstAspect;
      sx += (sw - nextSw) * 0.5;
      sw = nextSw;
    } else {
      const nextSh = sw / dstAspect;
      sy += (sh - nextSh) * 0.5;
      sh = nextSh;
    }
  }

  return { sx, sy, sw, sh, dx, dy, dw, dh };
}

let cachedOffscreen: HTMLCanvasElement | null = null;

function getOffscreenSourceCanvas(w: number, h: number): HTMLCanvasElement | null {
  if (typeof document === 'undefined') return null;
  if (!cachedOffscreen) {
    cachedOffscreen = document.createElement('canvas');
  }
  if (cachedOffscreen.width !== w || cachedOffscreen.height !== h) {
    cachedOffscreen.width = w;
    cachedOffscreen.height = h;
  }
  return cachedOffscreen;
}

/**
 * Renders the animated synthetic Test Pattern (SMPTE bars / Motion Grid / Animated Gradient)
 * transformed with exact custom aspect ratio, fit mode, crop rect, zoom, and opacity onto `canvas`.
 */
export function renderSyntheticPatternToCanvas(
  canvas: HTMLCanvasElement,
  pattern: TestPatternKind,
  transform: NdiTransformConfig,
  phaseSecs: number
): void {
  const ctx = canvas.getContext('2d');
  if (!ctx) return;

  const cw = canvas.width;
  const ch = canvas.height;

  // Fill studio letterbox background
  ctx.fillStyle = '#05070c';
  ctx.fillRect(0, 0, cw, ch);

  // Compute inner viewport reflecting the configured output aspect ratio (e.g., 1280x360 ultra-wide!)
  const outAspect = transform.out_width / Math.max(1, transform.out_height);
  const canvasAspect = cw / Math.max(1, ch);
  let vpW = cw;
  let vpH = ch;
  let vpX = 0;
  let vpY = 0;
  if (outAspect > canvasAspect) {
    vpH = Math.max(16, Math.round(cw / outAspect));
    vpY = Math.round((ch - vpH) / 2);
  } else {
    vpW = Math.max(16, Math.round(ch * outAspect));
    vpX = Math.round((cw - vpW) / 2);
  }

  // Output frame border
  ctx.fillStyle = '#000000';
  ctx.fillRect(vpX, vpY, vpW, vpH);

  // Draw 16:9 synthetic source onto an offscreen canvas then transform-blit into viewport
  const srcW = 320;
  const srcH = 180;
  const off = getOffscreenSourceCanvas(srcW, srcH);
  const octx = off?.getContext('2d');
  if (off && octx) {
    drawSourcePattern(octx, srcW, srcH, pattern, phaseSecs);
    const g = computeFitGeometry(srcW, srcH, vpW, vpH, transform);
    ctx.save();
    ctx.globalAlpha = clamp(transform.opacity, 0, 1);
    ctx.drawImage(off, g.sx, g.sy, g.sw, g.sh, vpX + g.dx, vpY + g.dy, g.dw, g.dh);
    ctx.restore();
  }

  // Crisp violet framing around active output aspect bounds
  ctx.strokeStyle = 'rgba(139, 92, 246, 0.75)';
  ctx.lineWidth = 1;
  ctx.strokeRect(vpX + 0.5, vpY + 0.5, vpW - 1, vpH - 1);
}

function drawSourcePattern(
  ctx: CanvasRenderingContext2D,
  w: number,
  h: number,
  pattern: TestPatternKind,
  phase: number
): void {
  if (pattern === 'smpte_bars') {
    const topBars = ['#c0c0c0', '#c0c000', '#00c0c0', '#00c000', '#c000c0', '#c00000', '#0000c0'];
    const midBars = ['#0000c0', '#131313', '#c000c0', '#131313', '#00c0c0', '#131313', '#c0c0c0'];
    const barW = w / 7;
    const topH = h * 0.67;
    const midH = h * 0.09;
    for (let i = 0; i < 7; i++) {
      ctx.fillStyle = topBars[i];
      ctx.fillRect(i * barW, 0, barW + 1, topH);
      ctx.fillStyle = midBars[i];
      ctx.fillRect(i * barW, topH, barW + 1, midH);
    }
    const botY = topH + midH;
    const botH = h - botY;
    const grad = ctx.createLinearGradient(0, botY, w, botY);
    grad.addColorStop(0, '#00214c');
    grad.addColorStop(0.25, '#f8fafc');
    grad.addColorStop(0.45, '#32006a');
    grad.addColorStop(0.5, '#09090b');
    grad.addColorStop(1, '#ffffff');
    ctx.fillStyle = grad;
    ctx.fillRect(0, botY, w, botH);

    // Animated amber sweep bar
    const sweepX = ((phase * 0.35) % 1) * w;
    ctx.fillStyle = '#f59e0b';
    ctx.fillRect(sweepX - 4, botY, 8, botH);

    // Frame alignment border + center crosshair
    ctx.strokeStyle = '#f8fafc';
    ctx.lineWidth = 3;
    ctx.strokeRect(1.5, 1.5, w - 3, h - 3);
    ctx.strokeStyle = '#10b981';
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.moveTo(w / 2 - 18, h / 2);
    ctx.lineTo(w / 2 + 18, h / 2);
    ctx.moveTo(w / 2, h / 2 - 14);
    ctx.lineTo(w / 2, h / 2 + 14);
    ctx.stroke();
  } else if (pattern === 'motion_grid') {
    ctx.fillStyle = '#0f172a';
    ctx.fillRect(0, 0, w, h);
    const cell = 24;
    const shift = (phase * 28) % cell;
    ctx.strokeStyle = '#38bdf8';
    ctx.lineWidth = 1;
    for (let x = -cell + shift; x < w; x += cell) {
      ctx.beginPath();
      ctx.moveTo(x, 0);
      ctx.lineTo(x, h);
      ctx.stroke();
    }
    for (let y = -cell + shift; y < h; y += cell) {
      ctx.beginPath();
      ctx.moveTo(0, y);
      ctx.lineTo(w, y);
      ctx.stroke();
    }
    const bx = w * (0.5 + 0.36 * Math.sin(phase * 1.6));
    const by = h * (0.5 + 0.34 * Math.cos(phase * 1.2));
    ctx.fillStyle = '#8b5cf6';
    ctx.beginPath();
    ctx.arc(bx, by, 14, 0, Math.PI * 2);
    ctx.fill();
    ctx.strokeStyle = '#f59e0b';
    ctx.lineWidth = 3;
    ctx.strokeRect(1.5, 1.5, w - 3, h - 3);
  } else {
    const hue1 = Math.round((phase * 45) % 360);
    const hue2 = (hue1 + 140) % 360;
    const grad = ctx.createLinearGradient(0, 0, w, h);
    grad.addColorStop(0, `hsl(${hue1}, 82%, 48%)`);
    grad.addColorStop(1, `hsl(${hue2}, 82%, 42%)`);
    ctx.fillStyle = grad;
    ctx.fillRect(0, 0, w, h);
    const cx = w * (0.5 + 0.28 * Math.cos(phase * 1.1));
    const cy = h * (0.5 + 0.25 * Math.sin(phase * 0.85));
    ctx.fillStyle = 'rgba(255,255,255,0.9)';
    ctx.beginPath();
    ctx.arc(cx, cy, 12, 0, Math.PI * 2);
    ctx.fill();
  }
}

function parsePatternFromSource(name: string): TestPatternKind {
  const lower = name.toLowerCase();
  if (lower.includes('motion') || lower.includes('grid')) return 'motion_grid';
  if (lower.includes('gradient')) return 'animated_gradient';
  return 'smpte_bars';
}

function clamp(v: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, v));
}

function structuredCloneSafe<T>(v: T): T {
  return JSON.parse(JSON.stringify(v));
}
