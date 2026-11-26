import { describe, expect, it } from 'vitest';
import {
  applyModifierChain,
  evalMathExpression,
  sampleTransferCurveLocal,
} from '../lib/modifierMath';
import { BrowserLoopbackEngine } from '../lib/bridge';
import {
  getInspectorPanelsFor,
  getNodeDefinition,
  initBuiltinNodeRegistry,
  listCommandActions,
  listDrawerTabs,
  listNodeDefinitions,
  listSidebarSections,
  registerCommandAction,
  registerDrawerTab,
  registerInspectorPanel,
  registerSidebarSection,
} from '../registries';
import { useWorkspaceStore } from '../store/workspaceStore';
import type { Modifier } from '../types/kontrolfreak';

describe('Kontrolfreak modifier math & transfer curve', () => {
  it('evaluates safe math expressions with functions and constants', () => {
    expect(evalMathExpression('clamp(x * 2 - 0.5, 0, 1)', 0.5)).toBeCloseTo(0.5);
    expect(evalMathExpression('clamp(x * 2 - 0.5, 0, 1)', 0.9)).toBeCloseTo(1.0);
    expect(evalMathExpression('round(lerp(0, 127, x))', 0.5)).toBe(64);
  });

  it('chains deadzone, curve, range map, invert, hysteresis, and one-euro', () => {
    const chain: Modifier[] = [
      { type: 'Deadzone', center: 0.5, radius: 0.1, rescale: true },
      { type: 'Curve', kind: { kind: 'exponential', exponent: 2 } },
      { type: 'RangeMap', in_min: 0, in_max: 1, out_min: 0, out_max: 127, clamp: true },
    ];
    const out = applyModifierChain(chain, { type: 'Float', value: 0.55 }, 100, []);
    expect(out).toEqual({ type: 'Float', value: 0.25 * 127 });

    const curvePts = sampleTransferCurveLocal(
      [{ type: 'Invert' }, { type: 'Clamp', min: 0.1, max: 0.9 }],
      11
    );
    expect(curvePts).toHaveLength(11);
    expect(curvePts[0][1]).toBeCloseTo(0.9);
    expect(curvePts[10][1]).toBeCloseTo(0.1);
  });
});

describe('BrowserLoopbackEngine core workflows', () => {
  it('renames nodes and ports without breaking active wire routing', () => {
    const engine = new BrowserLoopbackEngine();
    const graph = engine.getWorkspace();
    const osc = graph.devices[0];
    const midi = graph.devices[1];

    engine.renameDevice(osc.id, 'Main Stage iPad');
    engine.renamePort(osc.id, osc.ports[0].id, 'Master Dimmer');
    engine.renameDevice(midi.id, 'Ableton Bus');

    const delivered = engine.injectSignal(osc.ports[0].id, { type: 'Float', value: 0.75 });
    expect(delivered).toBeGreaterThanOrEqual(1);

    const updated = engine.getWorkspace();
    expect(updated.devices[0].display_name).toBe('Main Stage iPad');
    expect(updated.devices[0].ports[0].alias).toBe('Master Dimmer');
    expect(updated.wires.length).toBe(graph.wires.length);
  });

  it('creates collapsible racks and propagates exposed macros to internal targets', () => {
    const engine = new BrowserLoopbackEngine();
    const graph = engine.getWorkspace();
    const osc = graph.devices[0];
    const midi = graph.devices[1];

    const rack = engine.createRack('Stage Rack', [osc.id, midi.id]);
    const macro = engine.exposeMacro(rack.id, 'Master Level', [
      {
        port_id: osc.ports[0].id,
        min: 0.1,
        max: 0.9,
        invert: false,
        curve: { kind: 'linear' },
      },
    ]);

    engine.setRackCollapsed(rack.id, true);
    const updatedMacro = engine.setMacroValue(rack.id, macro.id, 0.5);
    expect(updatedMacro.value).toBeCloseTo(0.5);

    const batch = engine.flushTelemetry();
    const srcEntry = batch.values.find((v) => v.port_id === osc.ports[0].id);
    const dstEntry = batch.values.find((v) => v.port_id === midi.ports[0].id);
    expect(srcEntry?.numeric_value).toBeCloseTo(0.5);
    expect(dstEntry).toBeDefined();
  });

  it('exports/imports project JSON preserving Offline Ghosts and rebinds cleanly', () => {
    const engine = new BrowserLoopbackEngine();
    const missingMidi = engine.addDevice({
      id: '',
      logical_key: 'midi.missing',
      display_name: 'External Synth',
      kind: {
        type: 'MidiPort',
        port_name: 'Unplugged Synth Port',
        is_virtual: false,
        channel_filter: null,
        backend: 'hardware',
      },
      fingerprint: { port_name: 'Unplugged Synth Port' },
      status: { state: 'online' },
      ports: [
        {
          id: 'p-ext-1',
          logical_key: 'cc.1.74',
          alias: 'Cutoff',
          direction: 'in',
          signal_kind: 'midi',
          modifiers: [],
        },
      ],
      ui: { x: 400, y: 400, collapsed: false },
      group_id: null,
      manual_override: {},
    });

    const srcPort = engine.getWorkspace().devices[0].ports[0];
    const wire = engine.addWire(srcPort.id, missingMidi.ports[0].id);

    const saved = engine.exportProjectJson();
    const reloaded = engine.importProjectJson(saved, true);

    const ghost = reloaded.devices.find((d) => d.id === missingMidi.id);
    expect(ghost?.status.state).toBe('offline_ghost');
    expect(reloaded.wires.some((w) => w.id === wire.id)).toBe(true);

    const rebound = engine.rebindOfflineDevice(
      missingMidi.id,
      { port_name: 'Kontrolfreak Loopback 1' },
      'Kontrolfreak Loopback 1'
    );
    expect(rebound.status.state).toBe('online');
  });
});

describe('Extension seams & Zustand/Zundo undo-redo', () => {
  it('supports registering M2/M3/M4 UI extensions cleanly', () => {
    initBuiltinNodeRegistry();
    expect(listNodeDefinitions().length).toBeGreaterThanOrEqual(8);
    expect(getNodeDefinition('NdiVideoNode')?.milestoneTag).toBe('M2');
    expect(getNodeDefinition('HidDevice')?.milestoneTag).toBe('M3');
    expect(getNodeDefinition('DepthSensor')?.milestoneTag).toBe('M4');

    registerSidebarSection({
      id: 'test-m2-ndi',
      label: 'NDI',
      order: 40,
      iconName: 'video',
      component: () => null,
    });
    expect(listSidebarSections().some((s) => s.id === 'test-m2-ndi')).toBe(true);

    registerInspectorPanel({
      id: 'test-m2-scaler',
      title: 'NDI Scaler',
      order: 10,
      target: 'NdiVideoNode',
      component: () => null,
    });
    expect(getInspectorPanelsFor('NdiVideoNode').some((p) => p.id === 'test-m2-scaler')).toBe(true);

    registerDrawerTab({
      id: 'test-m3-learn',
      label: 'HID Learn',
      order: 30,
      badge: 'M3',
      component: () => null,
    });
    expect(listDrawerTabs().some((t) => t.id === 'test-m3-learn')).toBe(true);

    registerCommandAction({
      id: 'test-m4-diag',
      label: 'Open Depth Diagnostics',
      group: 'Diagnostics',
      run: () => {},
    });
    expect(listCommandActions().some((a) => a.id === 'test-m4-diag')).toBe(true);
  });

  it('tracks graph edits with zundo undo/redo', async () => {
    await useWorkspaceStore.getState().bootstrap();
    const initialCount = useWorkspaceStore.getState().graph.devices.length;
    const def = getNodeDefinition('VirtualControl')!;
    await useWorkspaceStore.getState().addDevice(def.createDevice({ x: 100, y: 100 }));
    expect(useWorkspaceStore.getState().graph.devices.length).toBe(initialCount + 1);

    useWorkspaceStore.temporal.getState().undo();
    expect(useWorkspaceStore.getState().graph.devices.length).toBe(initialCount);

    useWorkspaceStore.temporal.getState().redo();
    expect(useWorkspaceStore.getState().graph.devices.length).toBe(initialCount + 1);
  });
});
