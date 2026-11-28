import { isTauriRuntime } from '../lib/bridge';
import {
  SNAPSHOT_CHANNEL,
  SNAPSHOT_KEY,
  browserHost,
  browserOscStatus,
  browserSources,
  defaultSettings,
  emptyPreview,
  previewFromSource,
  sanitizeSettings,
  type DisplayInfo,
  type HostInfo,
  type OscListenStatus,
  type OutputSettings,
  type OutputWindowStatus,
  type SimplePoll,
  type SimplePreview,
  type SimpleSnapshot,
  type SimpleSource,
} from './model';

type Invoke = <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>;

async function tauriInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import('@tauri-apps/api/core');
  return (invoke as Invoke)<T>(cmd, args);
}

const browser = {
  port: 9000,
  selectedId: null as string | null,
  settings: defaultSettings(),
  message: '',
};

function publishBrowser(): SimpleSnapshot {
  const snapshot: SimpleSnapshot = {
    sources: browserSources(Date.now(), browser.port),
    osc: browserOscStatus(browser.port),
    selectedId: browser.selectedId,
    settings: browser.settings,
    host: browserHost(),
    ndiRuntime: false,
    message: browser.message,
  };
  try {
    localStorage.setItem(SNAPSHOT_KEY, JSON.stringify(snapshot));
  } catch {
    /* private mode */
  }
  try {
    const channel = new BroadcastChannel(SNAPSHOT_CHANNEL);
    channel.postMessage(snapshot);
    channel.close();
  } catch {
    /* BroadcastChannel missing */
  }
  return snapshot;
}

export function readBrowserSnapshot(): SimpleSnapshot | null {
  try {
    const raw = localStorage.getItem(SNAPSHOT_KEY);
    if (!raw) return null;
    return JSON.parse(raw) as SimpleSnapshot;
  } catch {
    return null;
  }
}

export async function pollSources(): Promise<SimplePoll> {
  if (isTauriRuntime()) {
    return tauriInvoke<SimplePoll>('simple_poll_sources');
  }
  const snapshot = publishBrowser();
  return {
    sources: snapshot.sources,
    osc: snapshot.osc,
    ndi_runtime: false,
    host: snapshot.host,
  };
}

export async function setOscPort(port: number): Promise<OscListenStatus> {
  const next = Math.round(port);
  if (isTauriRuntime()) {
    return tauriInvoke<OscListenStatus>('simple_set_osc_port', { port: next });
  }
  browser.port = Math.min(65535, Math.max(1, next));
  return publishBrowser().osc;
}

export async function selectSource(source: SimpleSource | null): Promise<void> {
  if (isTauriRuntime()) {
    await tauriInvoke('simple_select_source', { source });
    return;
  }
  browser.selectedId = source?.id ?? null;
  publishBrowser();
}

export async function getSettings(): Promise<OutputSettings> {
  if (isTauriRuntime()) return tauriInvoke<OutputSettings>('simple_get_settings');
  return browser.settings;
}

export async function setSettings(settings: OutputSettings): Promise<OutputSettings> {
  const clean = sanitizeSettings(settings);
  if (isTauriRuntime()) {
    return tauriInvoke<OutputSettings>('simple_set_settings', { settings: clean });
  }
  browser.settings = clean;
  publishBrowser();
  return clean;
}

export async function fetchPreview(maxWidth: number, maxHeight: number): Promise<SimplePreview> {
  if (isTauriRuntime()) {
    return tauriInvoke<SimplePreview>('simple_preview', {
      maxWidth,
      maxHeight,
    });
  }
  const snapshot = publishBrowser();
  const source = snapshot.sources.find((row) => row.id === snapshot.selectedId) ?? null;
  return previewFromSource(source);
}

export async function hostInfo(): Promise<HostInfo> {
  if (isTauriRuntime()) return tauriInvoke<HostInfo>('simple_host_info');
  return browserHost();
}

export async function listDisplays(): Promise<DisplayInfo[]> {
  if (isTauriRuntime()) return tauriInvoke<DisplayInfo[]>('simple_list_displays');
  const screens = await browserScreens();
  return screens;
}

export async function openOutputWindow(settings: OutputSettings): Promise<OutputWindowStatus> {
  if (isTauriRuntime()) return tauriInvoke<OutputWindowStatus>('simple_open_output');
  publishBrowser();
  const url = new URL(window.location.href);
  url.searchParams.set('view', 'output');
  url.hash = '';
  const displays = await browserScreens();
  const chosen =
    settings.display_index == null
      ? null
      : displays.find((display) => display.index === settings.display_index) ?? null;
  const features = [
    'popup=yes',
    `width=${Math.min(settings.width, window.screen.availWidth || settings.width)}`,
    `height=${Math.min(settings.height, window.screen.availHeight || settings.height)}`,
  ];
  if (chosen) {
    features.push(`left=${chosen.x}`, `top=${chosen.y}`);
  }
  const popup = window.open(url.toString(), 'kf-output', features.join(','));
  if (!popup) {
    const message = 'The browser blocked the output window. Allow popups, then try again.';
    browser.message = message;
    publishBrowser();
    return { open: false, wayland: false, positioned: false, message };
  }
  const message = chosen
    ? `Preview window opened on ${chosen.name}. Press F or double-click for fullscreen.`
    : 'Preview window opened. Press F or double-click for fullscreen. Esc leaves fullscreen.';
  browser.message = message;
  publishBrowser();
  return { open: true, wayland: false, positioned: Boolean(chosen), message };
}

export async function setOutputFullscreen(enabled: boolean): Promise<void> {
  if (!isTauriRuntime()) return;
  await tauriInvoke('simple_set_output_fullscreen', { enabled });
}

export function emptyFrame(): SimplePreview {
  return emptyPreview();
}

async function browserScreens(): Promise<DisplayInfo[]> {
  const nav = window as Window & {
    getScreenDetails?: () => Promise<{
      screens: Array<{
        label?: string;
        left: number;
        top: number;
        width: number;
        height: number;
        devicePixelRatio?: number;
      }>;
    }>;
  };
  try {
    if (typeof nav.getScreenDetails === 'function') {
      const details = await nav.getScreenDetails();
      if (details.screens.length > 0) {
        return details.screens.map((screen, index) => ({
          index,
          name: screen.label || `Display ${index + 1}`,
          x: screen.left,
          y: screen.top,
          width: screen.width,
          height: screen.height,
          scale: screen.devicePixelRatio ?? window.devicePixelRatio ?? 1,
        }));
      }
    }
  } catch {
    /* permission denied or unsupported */
  }
  return [
    {
      index: 0,
      name: 'This display',
      x: 0,
      y: 0,
      width: window.screen?.width ?? 1920,
      height: window.screen?.height ?? 1080,
      scale: window.devicePixelRatio || 1,
    },
  ];
}
