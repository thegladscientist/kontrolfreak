import { bridge, isTauriRuntime } from '../../lib/bridge';
import { useWorkspaceStore } from '../../store/workspaceStore';
import { m4Bridge, roiToSignalBindings } from './m4Bridge';
import type { DepthPreviewPacket, DepthSidecarStatusDto, SidecarHealthStatus } from './types';

export interface DepthFeedState {
  packet: DepthPreviewPacket | null;
  status: DepthSidecarStatusDto | null;
  error: string | null;
}

type Listener = (state: DepthFeedState) => void;

interface Feed {
  listeners: Set<Listener>;
  timer: ReturnType<typeof setInterval> | null;
  state: DepthFeedState;
  busy: boolean;
  lastHealth: SidecarHealthStatus | null;
  pollCount: number;
}

const POLL_INTERVAL_MS = 66;
const feeds = new Map<string, Feed>();

async function syncDeviceStatus(deviceId: string, health: SidecarHealthStatus): Promise<void> {
  const store = useWorkspaceStore.getState();
  const dev = store.graph.devices.find((d) => d.id === deviceId);
  if (!dev) return;
  if (health === 'running' && dev.status.state !== 'online') {
    await store.setDeviceStatus(deviceId, { state: 'online' });
  } else if (
    (health === 'crashed_restarting' || health === 'stopped') &&
    dev.status.state === 'online'
  ) {
    await store.setDeviceStatus(deviceId, {
      state: 'offline_ghost',
      reason:
        health === 'stopped'
          ? 'Depth sidecar stopped'
          : 'Depth sidecar crashed — auto-restarting (core router unaffected)',
    });
  }
}

/** Browser loopback has no native router thread, so the poller injects ROI signals into the loopback router. */
async function injectLoopbackSignals(deviceId: string, packet: DepthPreviewPacket): Promise<void> {
  if (isTauriRuntime() || packet.health.status !== 'running' || packet.roi_results.length === 0) return;
  const dev = useWorkspaceStore.getState().graph.devices.find((d) => d.id === deviceId);
  if (!dev) return;
  for (const [key, value] of roiToSignalBindings(packet.roi_results[0])) {
    const port = dev.ports.find((p) => p.logical_key === key);
    if (port) await bridge.injectSignal(port.id, value);
  }
}

async function pollOnce(deviceId: string, feed: Feed): Promise<void> {
  if (feed.busy) return;
  feed.busy = true;
  try {
    let next: DepthFeedState;
    try {
      const packet = await m4Bridge.pollPreview(deviceId);
      next = {
        packet,
        status: {
          device_id: deviceId,
          health: packet.health,
          rois: feed.state.status?.rois ?? [],
          colormap: { palette: packet.palette, near_mm: packet.near_mm, far_mm: packet.far_mm },
          latest_results: packet.roi_results,
        },
        error: null,
      };
      if (!feed.state.status || feed.pollCount++ % 8 === 0) {
        const full = await m4Bridge.getStatus(deviceId);
        next.status = { ...full, health: packet.health, latest_results: packet.roi_results };
      }
      void injectLoopbackSignals(deviceId, packet).catch(() => undefined);
    } catch (err) {
      const status = await m4Bridge.getStatus(deviceId).catch(() => null);
      next = {
        packet: null,
        status,
        error: status && status.health.status === 'stopped' ? null : String(err),
      };
    }
    feed.state = next;
    const health = next.status?.health.status ?? null;
    if (health && health !== feed.lastHealth) {
      feed.lastHealth = health;
      void syncDeviceStatus(deviceId, health).catch(() => undefined);
    }
    feed.listeners.forEach((l) => l(next));
  } finally {
    feed.busy = false;
  }
}

/** Ref-counted shared poller: one timer per depth device, shared by node body + inspector + sidebar. */
export function subscribeDepthFeed(deviceId: string, listener: Listener): () => void {
  let feed = feeds.get(deviceId);
  if (!feed) {
    feed = {
      listeners: new Set(),
      timer: null,
      state: { packet: null, status: null, error: null },
      busy: false,
      lastHealth: null,
      pollCount: 0,
    };
    feeds.set(deviceId, feed);
  }
  feed.listeners.add(listener);
  listener(feed.state);
  if (!feed.timer) {
    const f = feed;
    f.timer = setInterval(() => void pollOnce(deviceId, f), POLL_INTERVAL_MS);
    void pollOnce(deviceId, f);
  }
  return () => {
    const cur = feeds.get(deviceId);
    if (!cur) return;
    cur.listeners.delete(listener);
    if (cur.listeners.size === 0) {
      if (cur.timer) clearInterval(cur.timer);
      feeds.delete(deviceId);
    }
  };
}

export function activeDepthFeedCount(): number {
  return feeds.size;
}
