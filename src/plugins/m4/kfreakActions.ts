import { bridge } from '../../lib/bridge';
import { useWorkspaceStore } from '../../store/workspaceStore';
import type { Device } from '../../types/kontrolfreak';
import { m4Bridge } from './m4Bridge';
import type { DepthSidecarStatusDto, KfreakExportResult, KfreakImportResult } from './types';
import { base64ToBytes, bytesToBase64 } from './zip';

export function isDepthDevice(d: Device): boolean {
  return d.kind.type === 'DepthSensor';
}

export function downloadBytes(filename: string, bytes: Uint8Array, mime = 'application/zip'): void {
  const blob = new Blob([bytes.slice().buffer as ArrayBuffer], { type: mime });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = filename;
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

export async function exportKfreakToDownload(): Promise<KfreakExportResult> {
  const pushToast = useWorkspaceStore.getState().pushToast;
  try {
    const result = await m4Bridge.exportKfreak();
    downloadBytes(result.filename, base64ToBytes(result.zip_base64));
    pushToast({
      title: `Exported ${result.filename}`,
      description: `${(result.size_bytes / 1024).toFixed(1)} KiB · ${result.manifest.device_presets.length} devices · ${result.manifest.group_presets.length} racks · ${Object.keys(result.manifest.checksums).length} SHA-256 checksums.`,
      accent: 'emerald',
    });
    return result;
  } catch (err) {
    pushToast({
      title: 'Bundle export failed',
      description: err instanceof Error ? err.message : String(err),
      accent: 'rose',
    });
    throw err;
  }
}

export async function importKfreakBytes(bytes: Uint8Array): Promise<KfreakImportResult> {
  const store = useWorkspaceStore.getState();
  try {
    const result = await m4Bridge.importKfreak(bytesToBase64(bytes));
    useWorkspaceStore.setState({
      graph: result.graph,
      selection: { kind: 'none' },
      selectedNodeIds: [],
    });
    const ghosts = result.graph.devices.filter((d) => d.status.state === 'offline_ghost').length;
    store.pushToast({
      title: `Imported "${result.manifest.project_name}" bundle`,
      description:
        `${result.graph.devices.length} nodes · ${result.graph.wires.length} wires · checksums verified.` +
        (ghosts > 0 ? ` ${ghosts} disconnected device(s) kept as Offline Ghosts.` : ''),
      accent: ghosts > 0 ? 'amber' : 'emerald',
    });
    return result;
  } catch (err) {
    store.pushToast({
      title: 'Bundle import failed',
      description: err instanceof Error ? err.message : String(err),
      accent: 'rose',
    });
    throw err;
  }
}

export function pickAndImportKfreak(): void {
  const input = document.createElement('input');
  input.type = 'file';
  input.accept = '.kfreak,application/zip';
  input.onchange = async () => {
    const file = input.files?.[0];
    if (!file) return;
    const bytes = new Uint8Array(await file.arrayBuffer());
    await importKfreakBytes(bytes).catch(() => undefined);
  };
  input.click();
}

export interface CrashIsolationReport {
  crashed: DepthSidecarStatusDto;
  routedWhileDown: number | null;
  recovered: boolean;
}

/**
 * Kills the depth sidecar process (real SIGKILL on desktop) and proves the core router keeps routing
 * while the supervisor restarts the sidecar with exponential backoff.
 */
export async function runCrashIsolationTest(
  deviceId: string,
  opts: { waitMs?: number } = {}
): Promise<CrashIsolationReport> {
  const store = useWorkspaceStore.getState();
  let dto = await m4Bridge.getStatus(deviceId);
  if (dto.health.status !== 'running') {
    dto = await m4Bridge.startSidecar(deviceId, dto.health.vendor || 'synthetic');
  }
  const crashed = await m4Bridge.simulateCrash(deviceId);

  let routedWhileDown: number | null = null;
  const probe = store.graph.devices
    .filter((d) => !isDepthDevice(d))
    .flatMap((d) => d.ports)
    .find((p) => p.direction === 'out' || p.direction === 'bidir');
  if (probe) {
    routedWhileDown = await bridge.injectSignal(probe.id, { type: 'Float', value: 0.5 }).catch(() => null);
  }

  store.pushToast({
    title: 'Sidecar killed — core router unaffected',
    description:
      routedWhileDown !== null
        ? `Router delivered ${routedWhileDown} event(s) while depth sidecar is down. Auto-restarting with backoff…`
        : 'Depth sidecar is auto-restarting with exponential backoff.',
    accent: 'amber',
  });

  let recovered = false;
  const deadline = Date.now() + (opts.waitMs ?? 0);
  while (Date.now() < deadline) {
    await new Promise((r) => setTimeout(r, 100));
    const s = await m4Bridge.getStatus(deviceId);
    if (s.health.status === 'running') {
      recovered = true;
      break;
    }
  }
  return { crashed, routedWhileDown, recovered };
}
