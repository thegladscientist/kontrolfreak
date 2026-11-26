import { describe, expect, it, vi } from 'vitest';
import {
  getInspectorPanelsFor,
  getNodeDefinition,
  initBuiltinNodeRegistry,
  listCommandActions,
  listSidebarSections,
} from '../registries';
import {
  browserM2Mock,
  computeFitGeometry,
  m2Bridge,
  registerM2VideoRtpMidiPlugin,
  RESIZE_DEBOUNCE_MS,
} from '../plugins/m2VideoRtpMidi';
import { telemetryStore } from '../lib/telemetryStore';

describe('M2 Video & RTPMIDI Hybrid UI Plugin', () => {
  it('registers NdiVideoNode and RtpMidiSession across all four UI registries', () => {
    initBuiltinNodeRegistry();
    registerM2VideoRtpMidiPlugin();

    const ndiDef = getNodeDefinition('NdiVideoNode');
    const rtpDef = getNodeDefinition('RtpMidiSession');

    expect(ndiDef).toBeDefined();
    expect(ndiDef?.milestoneTag).toBe('M2');
    expect(ndiDef?.renderBody).toBeDefined();

    expect(rtpDef).toBeDefined();
    expect(rtpDef?.milestoneTag).toBe('M2');
    expect(rtpDef?.renderBody).toBeDefined();

    const ndiDevice = ndiDef!.createDevice({ x: 100, y: 100 });
    expect(ndiDevice.status.state).toBe('online');
    expect(ndiDevice.ports.some((p) => p.logical_key === 'ndi.crop_x')).toBe(true);
    expect(ndiDevice.ports.some((p) => p.logical_key === 'ndi.zoom')).toBe(true);
    expect(ndiDevice.ports.some((p) => p.logical_key === 'ndi.opacity')).toBe(true);

    const rtpDevice = rtpDef!.createDevice({ x: 200, y: 200 });
    expect(rtpDevice.status.state).toBe('online');
    expect(rtpDevice.ports.some((p) => p.logical_key === 'cc.1.1')).toBe(true);

    expect(listSidebarSections().some((s) => s.id === 'm2-media-net')).toBe(true);
    expect(getInspectorPanelsFor('NdiVideoNode').some((p) => p.id === 'm2-ndi-video-panel')).toBe(
      true
    );
    expect(
      getInspectorPanelsFor('RtpMidiSession').some((p) => p.id === 'm2-rtpmidi-session-panel')
    ).toBe(true);
    expect(listCommandActions().some((a) => a.id === 'm2.spawn-ndi-scaler')).toBe(true);
    expect(listCommandActions().some((a) => a.id === 'm2.spawn-rtpmidi-session')).toBe(true);
  });

  it('computes accurate Contain, Cover, and Crop+Zoom geometry for arbitrary resolutions', () => {
    // 1920x1080 -> 1280x360 Contain -> 640x360 centered horizontally at dx=320
    const contain = computeFitGeometry(1920, 1080, 1280, 360, {
      fit_mode: 'contain',
      crop_x: 0,
      crop_y: 0,
      crop_w: 1,
      crop_h: 1,
      zoom: 1,
      out_width: 1280,
      out_height: 360,
      out_fps: 30,
      opacity: 1,
      freeze: false,
      bypass: false,
    });
    expect(contain.dw).toBeCloseTo(640, 1);
    expect(contain.dh).toBeCloseTo(360, 1);
    expect(contain.dx).toBeCloseTo(320, 1);
    expect(contain.dy).toBeCloseTo(0, 1);

    // 1920x1080 -> 512x512 Cover -> crops 1080x1080 center square (sx=420)
    const cover = computeFitGeometry(1920, 1080, 512, 512, {
      fit_mode: 'cover',
      crop_x: 0,
      crop_y: 0,
      crop_w: 1,
      crop_h: 1,
      zoom: 1,
      out_width: 512,
      out_height: 512,
      out_fps: 30,
      opacity: 1,
      freeze: false,
      bypass: false,
    });
    expect(cover.sw).toBeCloseTo(1080, 1);
    expect(cover.sh).toBeCloseTo(1080, 1);
    expect(cover.sx).toBeCloseTo(420, 1);

    // Explicit Crop + 2x Zoom
    const cropZoom = computeFitGeometry(1920, 1080, 640, 360, {
      fit_mode: 'crop',
      crop_x: 0.25,
      crop_y: 0.25,
      crop_w: 0.5,
      crop_h: 0.5,
      zoom: 2,
      out_width: 640,
      out_height: 360,
      out_fps: 30,
      opacity: 0.8,
      freeze: false,
      bypass: false,
    });
    expect(cropZoom.sw).toBeCloseTo(480, 1);
    expect(cropZoom.sh).toBeCloseTo(270, 1);
    expect(cropZoom.sx).toBeCloseTo(720, 1);
    expect(cropZoom.sy).toBeCloseTo(405, 1);
  });

  it('debounces NDI resolution changes (~150ms) while applying crop/zoom and telemetry live', async () => {
    vi.useFakeTimers();
    try {
      const ndiDef = getNodeDefinition('NdiVideoNode')!;
      const dev = ndiDef.createDevice({ x: 0, y: 0 });

      const initial = await m2Bridge.getNdiNodeState(dev.id, dev);
      expect(initial.active_width).toBe(1280);
      expect(initial.active_height).toBe(720);

      // Scrub resolution twice rapidly -> still debouncing, crop_x is immediate
      await m2Bridge.configureNdiNode(dev.id, { out_width: 1024, out_height: 512 }, dev);
      vi.advanceTimersByTime(60);
      const mid = await m2Bridge.configureNdiNode(
        dev.id,
        { out_width: 1280, out_height: 360, crop_x: 0.4, zoom: 2.0 },
        dev
      );
      expect(mid.active_width).toBe(1280);
      expect(mid.active_height).toBe(720);
      expect(mid.pending_width).toBe(1280);
      expect(mid.pending_height).toBe(360);
      expect(mid.transform.crop_x).toBeCloseTo(0.4);
      expect(mid.transform.zoom).toBeCloseTo(2.0);

      // Advance past 150ms debounce -> single reallocation commits
      vi.advanceTimersByTime(RESIZE_DEBOUNCE_MS + 20);
      const settled = await m2Bridge.getNdiNodeState(dev.id, dev);
      expect(settled.active_width).toBe(1280);
      expect(settled.active_height).toBe(360);
      expect(settled.pending_width).toBeNull();
      expect(settled.reallocation_count).toBe(1);

      // Telemetry signal modulation drives crop_x live in browser loopback preview
      const cropXPort = dev.ports.find((p) => p.logical_key === 'ndi.crop_x')!;
      telemetryStore.ingestBatch({
        timestamp_ms: Date.now(),
        total_msg_per_sec: 10,
        recent_logs: [],
        values: [
          {
            port_id: cropXPort.id,
            device_id: dev.id,
            latest_value: { type: 'Float', value: 0.72 },
            numeric_value: 0.72,
            active: true,
            packets_per_sec: 10,
            last_updated_ms: Date.now(),
          },
        ],
      });
      const modulated = browserM2Mock.getEffectiveTransform(dev);
      expect(modulated.crop_x).toBeCloseTo(0.72);
    } finally {
      vi.useRealTimers();
    }
  });

  it('supports RTPMIDI session lifecycle, peer connect, and clock sync in browser loopback', async () => {
    const rtpDef = getNodeDefinition('RtpMidiSession')!;
    const dev = rtpDef.createDevice({ x: 0, y: 0 });

    await m2Bridge.startRtpMidiSession(dev.id, dev);
    await m2Bridge.connectRtpMidiPeer(dev.id, '192.168.1.99:5008', dev);
    await m2Bridge.syncRtpMidiClock(dev.id, dev);
    await m2Bridge.sendRtpMidiTestMidi(dev.id, [0x90, 60, 100], dev);

    const status = await m2Bridge.getRtpMidiStatus(dev.id, dev);
    expect(status?.running).toBe(true);
    expect(status?.connected_peers.some((p) => p.control_addr === '192.168.1.99:5008')).toBe(true);
    expect(status?.tx_packets).toBeGreaterThanOrEqual(1);

    await m2Bridge.stopRtpMidiSession(dev.id);
    const stopped = await m2Bridge.getRtpMidiStatus(dev.id, dev);
    expect(stopped?.running).toBe(false);
    expect(stopped?.connected_peers).toHaveLength(0);
  });
});
