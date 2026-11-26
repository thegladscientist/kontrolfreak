import { beforeEach, describe, expect, it } from 'vitest';
import { generateSyntheticDepthFrame, colorizeDepthToRgbaJs, createDefaultRoi, extractRoiJs } from '../plugins/m4/colormap';
import {
  M4LoopbackEngine,
  buildKfreakBundle,
  m4Bridge,
  parseKfreakBundle,
  roiToSignalBindings,
} from '../plugins/m4/m4Bridge';
import { crc32, readZipStore, sha256Hex, utf8, writeZipStore } from '../plugins/m4/zip';
import { bridge, createStarterGraph } from '../lib/bridge';
import { bytesToBase64 } from '../plugins/m4/zip';
import {
  M4_COMMAND_IDS,
  M4_DRAWER_TAB_ID,
  M4_INSPECTOR_ID,
  M4_SIDEBAR_ID,
  createDepthSensorDevice,
  registerM4DepthSidecarPlugin,
} from '../plugins/m4DepthSidecar';
import {
  getInspectorPanelsFor,
  getNodeDefinition,
  initBuiltinNodeRegistry,
  listCommandActions,
  listDrawerTabs,
  listSidebarSections,
} from '../registries';
import { registerBuiltinSidebarSections } from '../components/LeftSidebar';
import { runCrashIsolationTest } from '../plugins/m4/kfreakActions';
import { useWorkspaceStore } from '../store/workspaceStore';

describe('M4 zip + sha256 primitives', () => {
  it('computes known SHA-256 and CRC32 vectors', () => {
    expect(sha256Hex(utf8('abc'))).toBe('ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad');
    expect(sha256Hex(new Uint8Array(0))).toBe('e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855');
    expect(crc32(utf8('123456789'))).toBe(0xcbf43926);
  });

  it('round-trips STORE zip entries', () => {
    const zip = writeZipStore([
      { name: 'a.txt', data: utf8('hello') },
      { name: 'dir/b.json', data: utf8('{"x":1}') },
    ]);
    const out = readZipStore(zip);
    expect(out.map((e) => e.name)).toEqual(['a.txt', 'dir/b.json']);
    expect(new TextDecoder().decode(out[1].data)).toBe('{"x":1}');
  });
});

describe('M4 depth extraction + colormap (JS parity with Rust)', () => {
  it('extracts ROI min/max/mean depth and 3D coordinates accurately', () => {
    const w = 32;
    const h = 24;
    const depth = new Uint16Array(w * h).fill(2000);
    for (let y = 8; y < 12; y++) for (let x = 10; x < 14; x++) depth[y * w + x] = 1000;
    depth[9 * w + 11] = 800;
    const roi = { ...createDefaultRoi(), x: 0, y: 0, w: 1, h: 1, near_mm: 300, far_mm: 2500 };
    const r = extractRoiJs(depth, w, h, roi);
    expect(r.min_depth_mm).toBe(800);
    expect(r.max_depth_mm).toBe(2000);
    expect(r.presence).toBe(true);
    expect(r.closest_point_meters[2]).toBeCloseTo(0.8, 5);
    expect(r.closest_point_norm[2]).toBeCloseTo((800 - 300) / 2200, 5);
    expect(r.closest_point_norm[0]).toBeCloseTo(11 / (w - 1), 5);
  });

  it('reports empty ROI when nothing falls within the Z window', () => {
    const depth = new Uint16Array(16 * 16).fill(5000);
    const r = extractRoiJs(depth, 16, 16, { ...createDefaultRoi(), x: 0, y: 0, w: 1, h: 1 });
    expect(r.presence).toBe(false);
    expect(r.valid_pixel_count).toBe(0);
  });

  it('colorizes near as warm/bright and far as cool/dark across palettes', () => {
    const d = new Uint16Array([400, 2400]);
    const turbo = colorizeDepthToRgbaJs(d, 2, 1, 300, 2500, 'turbo');
    expect(turbo[3]).toBe(255);
    expect(turbo[0]).toBeGreaterThan(turbo[4]);
    const gray = colorizeDepthToRgbaJs(d, 2, 1, 300, 2500, 'grayscale');
    expect(gray[0]).toBeGreaterThan(gray[4]);
    const invalid = colorizeDepthToRgbaJs(new Uint16Array([0]), 1, 1, 300, 2500, 'viridis');
    expect([invalid[0], invalid[1], invalid[2]]).toEqual([11, 14, 20]);
  });

  it('maps ROI results to router signal bindings', () => {
    const depth = generateSyntheticDepthFrame(64, 48, 0.3);
    const r = extractRoiJs(depth, 64, 48, createDefaultRoi());
    const keys = roiToSignalBindings(r).map(([k]) => k);
    expect(keys).toEqual([
      'depth.closest_z',
      'depth.closest_xyz',
      'depth.centroid_xyz',
      'depth.presence',
      'depth.fill_ratio',
    ]);
  });
});

describe('M4 .kfreak bundle', () => {
  it('exports and re-imports a project with verified SHA-256 checksums', () => {
    const graph = createStarterGraph();
    const json = JSON.stringify({ schema_version: 1, graph });
    const { bytes, manifest, filename } = buildKfreakBundle(json, () => 1234);
    expect(filename.endsWith('.kfreak')).toBe(true);
    expect(manifest.created_at_ms).toBe(1234);
    expect(manifest.device_presets.length).toBe(graph.devices.length);
    expect(Object.keys(manifest.checksums)).toContain('project.kfproj.json');

    const parsed = parseKfreakBundle(bytes);
    expect(parsed.graph.devices.length).toBe(graph.devices.length);
    expect(parsed.graph.wires.length).toBe(graph.wires.length);
    expect(parsed.devicePresets.length).toBe(graph.devices.length);
  });

  it('rejects a tampered bundle entry', () => {
    const json = JSON.stringify({ schema_version: 1, graph: createStarterGraph() });
    const { bytes } = buildKfreakBundle(json);
    const entries = readZipStore(bytes).map((e) =>
      e.name === 'project.kfproj.json' ? { ...e, data: utf8(new TextDecoder().decode(e.data) + ' ') } : e
    );
    expect(() => parseKfreakBundle(writeZipStore(entries))).toThrow(/checksum mismatch/);
  });

  it('imports through the dual-runtime bridge in browser loopback mode', async () => {
    const exported = await m4Bridge.exportKfreak();
    expect(exported.size_bytes).toBeGreaterThan(100);
    const imported = await m4Bridge.importKfreak(exported.zip_base64);
    expect(imported.graph.devices.length).toBeGreaterThan(0);
    expect(imported.device_preset_count).toBe(imported.graph.devices.length);

    const tampered = readZipStore(Uint8Array.from(atob(exported.zip_base64), (c) => c.charCodeAt(0))).map((e) =>
      e.name.startsWith('presets/devices/') ? { ...e, data: utf8('{}') } : e
    );
    await expect(m4Bridge.importKfreak(bytesToBase64(writeZipStore(tampered)))).rejects.toThrow(/checksum/);
  });
});

describe('M4 loopback sidecar supervisor simulation', () => {
  let clock = 0;
  let engine: M4LoopbackEngine;
  beforeEach(() => {
    clock = 1_000_000;
    engine = new M4LoopbackEngine(() => clock);
  });

  it('streams frames while running and stops cleanly', () => {
    const dto = engine.startSidecar('dev-1');
    expect(dto.health.status).toBe('running');
    const pid = dto.health.pid;
    expect(pid).not.toBeNull();
    clock += 1000;
    const pkt = engine.pollPreview('dev-1');
    expect(pkt.health.frames_received).toBeGreaterThanOrEqual(29);
    expect(pkt.rgba_pixels?.length).toBe(pkt.width * pkt.height * 4);
    expect(pkt.roi_results[0].presence).toBe(true);
    expect(engine.stopSidecar('dev-1').health.status).toBe('stopped');
  });

  it('crashes, enters backoff, auto-restarts with a new pid and resumes frames', () => {
    const first = engine.startSidecar('dev-1').health.pid;
    clock += 500;
    const crashed = engine.simulateCrash('dev-1');
    expect(crashed.health.status).toBe('crashed_restarting');
    expect(crashed.health.pid).toBeNull();
    expect(crashed.health.restart_count).toBe(1);
    expect(crashed.health.last_crash_reason).toMatch(/SIGKILL/);

    clock += 50;
    expect(engine.statusOrDefault('dev-1').health.status).toBe('crashed_restarting');
    const framesDown = engine.statusOrDefault('dev-1').health.frames_received;

    clock += 400;
    const recovered = engine.statusOrDefault('dev-1');
    expect(recovered.health.status).toBe('running');
    expect(recovered.health.pid).not.toBe(first);
    expect(recovered.health.pid).not.toBeNull();

    clock += 500;
    expect(engine.statusOrDefault('dev-1').health.frames_received).toBeGreaterThan(framesDown);
  });

  it('applies exponential backoff on consecutive crashes', () => {
    engine.startSidecar('dev-1');
    engine.simulateCrash('dev-1');
    clock += 200;
    engine.statusOrDefault('dev-1');
    engine.simulateCrash('dev-1');
    clock += 200;
    expect(engine.statusOrDefault('dev-1').health.status).toBe('crashed_restarting');
    clock += 200;
    expect(engine.statusOrDefault('dev-1').health.status).toBe('running');
  });

  it('applies ROI and colormap configuration', () => {
    engine.startSidecar('dev-1');
    engine.setColormap('dev-1', 'viridis', 500, 3000);
    engine.configureRois('dev-1', [{ ...createDefaultRoi(), id: 'r2', name: 'Zone 2' }]);
    clock += 100;
    const pkt = engine.pollPreview('dev-1');
    expect(pkt.palette).toBe('viridis');
    expect(pkt.near_mm).toBe(500);
    expect(pkt.far_mm).toBe(3000);
    expect(pkt.roi_results[0].roi_id).toBe('r2');
  });

  it('produces diagnostics snapshots with rates, latency percentiles and sidecar entries', () => {
    engine.startSidecar('dev-1');
    for (let i = 0; i < 6; i++) {
      clock += 200;
      engine.pollPreview('dev-1');
      engine.diagnostics();
    }
    const snap = engine.diagnostics([{ protocol: 'ROUTER', rate: 42 }]);
    expect(snap.sidecars).toHaveLength(1);
    expect(snap.sidecars[0].status).toBe('running');
    expect(snap.protocol_rates.some((r) => r.protocol === 'DEPTH' && r.total_messages > 0)).toBe(true);
    expect(snap.protocol_rates.find((r) => r.protocol === 'ROUTER')?.msg_per_sec).toBe(42);
    expect(snap.latency.sample_count).toBeGreaterThan(0);
    expect(snap.latency.p50_ms).toBeLessThanOrEqual(snap.latency.p95_ms);
    expect(snap.latency.p95_ms).toBeLessThanOrEqual(snap.latency.p99_ms);
  });
});

describe('M4 plugin registration + crash isolation', () => {
  beforeEach(() => {
    initBuiltinNodeRegistry();
    registerBuiltinSidebarSections();
    registerM4DepthSidecarPlugin();
  });

  it('registers into node, sidebar, inspector, drawer and command registries', () => {
    const node = getNodeDefinition('DepthSensor');
    expect(node?.renderBody).toBeTypeOf('function');
    expect(node?.milestoneTag).toBe('M4');
    const dev = createDepthSensorDevice()!;
    expect(dev.ports.map((p) => p.logical_key)).toEqual([
      'depth.closest_z',
      'depth.closest_xyz',
      'depth.centroid_xyz',
      'depth.presence',
      'depth.fill_ratio',
      'depth.ndi_out',
    ]);
    expect(new Set(dev.ports.map((p) => p.id)).size).toBe(dev.ports.length);

    expect(listSidebarSections().some((s) => s.id === M4_SIDEBAR_ID && s.iconName === 'radar')).toBe(true);
    expect(listSidebarSections().some((s) => s.id === 'presets')).toBe(true);
    expect(getInspectorPanelsFor('DepthSensor').some((p) => p.id === M4_INSPECTOR_ID)).toBe(true);
    expect(listDrawerTabs().some((t) => t.id === M4_DRAWER_TAB_ID)).toBe(true);
    const ids = listCommandActions().map((a) => a.id);
    for (const id of Object.values(M4_COMMAND_IDS)) expect(ids).toContain(id);
  });

  it('keeps the core router routing while the depth sidecar is crashed', async () => {
    await useWorkspaceStore.getState().bootstrap();
    const dev = createDepthSensorDevice()!;
    const added = await useWorkspaceStore.getState().addDevice(dev);
    await m4Bridge.startSidecar(added.id, 'synthetic');

    const report = await runCrashIsolationTest(added.id);
    expect(report.crashed.health.status).toBe('crashed_restarting');
    expect(report.routedWhileDown).not.toBeNull();

    const ws = await bridge.getWorkspace();
    expect(ws.devices.length).toBeGreaterThan(1);
  });
});
