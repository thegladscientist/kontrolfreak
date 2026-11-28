export type FitMode = 'contain' | 'cover' | 'stretch';
export type SourceKind = 'osc' | 'midi' | 'ndi' | 'test_pattern';
export type PatternKind = 'smpte' | 'grid' | 'gradient';

export interface CropRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface OutputSettings {
  width: number;
  height: number;
  fit: FitMode;
  crop_x: number;
  crop_y: number;
  crop_w: number;
  crop_h: number;
  display_index: number | null;
}

export interface SimpleSource {
  id: string;
  kind: SourceKind | string;
  label: string;
  detail: string;
  value_label: string;
  value_norm: number | null;
  live: boolean;
}

export interface OscListenStatus {
  requested_port: number;
  bound_address: string | null;
  listening: boolean;
  mdns_active: boolean;
  mdns_fallback: string | null;
  error: string | null;
}

export interface HostInfo {
  platform: string;
  session: string;
  wayland: boolean;
  position_reliable: boolean;
}

export interface DisplayInfo {
  index: number;
  name: string;
  x: number;
  y: number;
  width: number;
  height: number;
  scale: number;
}

export interface SimplePoll {
  sources: SimpleSource[];
  osc: OscListenStatus;
  ndi_runtime: boolean;
  host: HostInfo;
}

export interface SimplePreview {
  source_id: string | null;
  kind: string;
  label: string;
  detail: string;
  value_label: string;
  value_norm: number | null;
  jpeg_base64: string | null;
  frame_width: number;
  frame_height: number;
  awaiting_frame: boolean;
  using_synthetic: boolean;
}

export interface OutputWindowStatus {
  open: boolean;
  wayland: boolean;
  positioned: boolean;
  message: string;
}

export interface SimpleSnapshot {
  sources: SimpleSource[];
  osc: OscListenStatus;
  selectedId: string | null;
  settings: OutputSettings;
  host: HostInfo;
  ndiRuntime: boolean;
  message: string;
}

export const RESOLUTION_PRESETS: Array<{ id: string; width: number; height: number; label: string }> = [
  { id: '1280x720', width: 1280, height: 720, label: '1280×720' },
  { id: '1920x1080', width: 1920, height: 1080, label: '1920×1080' },
  { id: '3840x2160', width: 3840, height: 2160, label: '3840×2160' },
];

export const SNAPSHOT_KEY = 'kf-simple-vj';
export const SNAPSHOT_CHANNEL = 'kf-simple-vj';

export function defaultSettings(): OutputSettings {
  return {
    width: 1920,
    height: 1080,
    fit: 'contain',
    crop_x: 0,
    crop_y: 0,
    crop_w: 1,
    crop_h: 1,
    display_index: null,
  };
}

export function sanitizeSettings(input: OutputSettings): OutputSettings {
  const fit: FitMode = input.fit === 'cover' || input.fit === 'stretch' ? input.fit : 'contain';
  const cropW = clamp(input.crop_w, 0.05, 1);
  const cropH = clamp(input.crop_h, 0.05, 1);
  return {
    width: Math.round(clamp(input.width, 160, 7680)),
    height: Math.round(clamp(input.height, 90, 4320)),
    fit,
    crop_w: cropW,
    crop_h: cropH,
    crop_x: clamp(input.crop_x, 0, 1 - cropW),
    crop_y: clamp(input.crop_y, 0, 1 - cropH),
    display_index: input.display_index,
  };
}

export function cropOf(settings: OutputSettings): CropRect {
  return {
    x: settings.crop_x,
    y: settings.crop_y,
    w: settings.crop_w,
    h: settings.crop_h,
  };
}

export function isOutputLocation(href: string, flag?: string | null): boolean {
  if (flag === 'output') return true;
  try {
    const url = new URL(href, 'http://kontrolfreak.local');
    if (url.hash === '#output') return true;
    return url.searchParams.get('view') === 'output';
  } catch {
    return false;
  }
}

export function patternKindFromLabel(label: string): PatternKind {
  const name = label.toLowerCase();
  if (name.includes('grid') || name.includes('motion')) return 'grid';
  if (name.includes('gradient')) return 'gradient';
  return 'smpte';
}

export function groupSources(sources: SimpleSource[]): {
  osc: SimpleSource[];
  midi: SimpleSource[];
  video: SimpleSource[];
} {
  return {
    osc: sources.filter((source) => source.kind === 'osc'),
    midi: sources.filter((source) => source.kind === 'midi'),
    video: sources.filter((source) => source.kind === 'ndi' || source.kind === 'test_pattern'),
  };
}

export function browserHost(): HostInfo {
  return {
    platform: 'browser',
    session: 'loopback',
    wayland: false,
    position_reliable: false,
  };
}

export function browserOscStatus(port: number): OscListenStatus {
  return {
    requested_port: port,
    bound_address: `browser:${port}`,
    listening: true,
    mdns_active: false,
    mdns_fallback: 'Browser loopback has no multicast. A built-in sender stands in so you can try the flow.',
    error: null,
  };
}

/** Sources the browser loopback always offers, so `npm run dev` can run the whole VJ flow. */
export function browserSources(nowMs: number, port: number): SimpleSource[] {
  const t = nowMs / 1000;
  const dimmer = 0.5 + 0.5 * Math.sin(t * 0.7);
  const color = 0.5 + 0.5 * Math.sin(t * 0.35 + 1.2);
  const cc = 0.5 + 0.5 * Math.sin(t * 0.45);
  const address = Math.floor(t / 2) % 2 === 0 ? '/vj/dimmer' : '/vj/color';
  const oscValue = address === '/vj/dimmer' ? dimmer : color;
  return [
    {
      id: 'osc:udp:127.0.0.1',
      kind: 'osc',
      label: '127.0.0.1 loopback',
      detail: `127.0.0.1:${port} · packet · /vj/dimmer  /vj/color`,
      value_label: oscValue.toFixed(3),
      value_norm: oscValue,
      live: true,
    },
    {
      id: 'midi:Kontrolfreak Loopback 1',
      kind: 'midi',
      label: 'Kontrolfreak Loopback 1',
      detail: 'Virtual loopback · in/out',
      value_label: `cc.1.7 = ${cc.toFixed(3)}`,
      value_norm: cc,
      live: true,
    },
    {
      id: 'pattern:Test Pattern: SMPTE Bars (1920x1080)',
      kind: 'test_pattern',
      label: 'SMPTE Bars (1920x1080)',
      detail: 'Built-in · 1920×1080',
      value_label: '',
      value_norm: null,
      live: true,
    },
    {
      id: 'pattern:Test Pattern: Motion Grid (1920x1080)',
      kind: 'test_pattern',
      label: 'Motion Grid (1920x1080)',
      detail: 'Built-in · 1920×1080',
      value_label: '',
      value_norm: null,
      live: true,
    },
    {
      id: 'pattern:Test Pattern: Animated Gradient (1280x720)',
      kind: 'test_pattern',
      label: 'Animated Gradient (1280x720)',
      detail: 'Built-in · 1280×720',
      value_label: '',
      value_norm: null,
      live: true,
    },
  ];
}

export function emptyPreview(): SimplePreview {
  return {
    source_id: null,
    kind: 'empty',
    label: 'No source',
    detail: 'Pick a source from the list.',
    value_label: '',
    value_norm: null,
    jpeg_base64: null,
    frame_width: 0,
    frame_height: 0,
    awaiting_frame: false,
    using_synthetic: false,
  };
}

export function previewFromSource(source: SimpleSource | null): SimplePreview {
  if (!source) return emptyPreview();
  return {
    source_id: source.id,
    kind: source.kind,
    label: source.label,
    detail: source.detail,
    value_label: source.value_label,
    value_norm: source.value_norm,
    jpeg_base64: null,
    frame_width: 0,
    frame_height: 0,
    awaiting_frame: source.kind === 'midi' && !source.live,
    using_synthetic: source.kind === 'test_pattern',
  };
}

function clamp(value: number, min: number, max: number): number {
  if (Number.isNaN(value)) return min;
  return Math.min(max, Math.max(min, value));
}
