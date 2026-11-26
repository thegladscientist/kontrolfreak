import React, { useEffect, useMemo, useRef, useState } from 'react';
import {
  CheckCircle2,
  Download,
  Play,
  Radio,
  RotateCcw,
  Save,
  Sliders,
  Sparkles,
  Usb,
} from 'lucide-react';
import { useWorkspaceStore } from '../../store/workspaceStore';
import { useM3Store } from './m3Store';
import { applyModifierChain } from '../../lib/modifierMath';
import type { Modifier } from '../../types/kontrolfreak';
import type { HidControlType, LearnedControlSpec } from './m3Types';

const CONTROL_BADGES: Record<HidControlType, { label: string; color: string }> = {
  button: { label: 'Button (1-bit)', color: 'bg-cyan-500/20 text-cyan-300 border-cyan-500/40' },
  toggle: { label: 'Toggle Switch', color: 'bg-cyan-500/20 text-cyan-200 border-cyan-500/40' },
  axis8: { label: 'Axis 8-bit', color: 'bg-emerald-500/20 text-emerald-300 border-emerald-500/40' },
  axis16: { label: 'Axis 16-bit', color: 'bg-amber-500/20 text-amber-300 border-amber-500/40' },
  relative_encoder: {
    label: 'Relative Encoder',
    color: 'bg-violet-500/20 text-violet-300 border-violet-500/40',
  },
  hat_switch: {
    label: 'Hat Switch (4-bit)',
    color: 'bg-rose-500/20 text-rose-300 border-rose-500/40',
  },
};

/**
 * Live HTML5 <canvas> bit/byte activity strip for HID reports:
 * - Dimmed amber-slate cells = noise-masked bits (IMU drift, counters, timestamps)
 * - Bright emerald/cyan cells = unmasked active diff bits
 * - Subtle slate cells = steady resting bits
 */
const BitActivityStripCanvas: React.FC<{
  latestBytes: number[];
  noiseMaskBits: number[];
  activeDiffBits: number[];
  calmMode: boolean;
}> = ({ latestBytes, noiseMaskBits, activeDiffBits, calmMode }) => {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const w = canvas.width;
    const h = canvas.height;
    ctx.clearRect(0, 0, w, h);
    ctx.fillStyle = '#0b0e14';
    ctx.fillRect(0, 0, w, h);

    const byteCount = Math.max(latestBytes.length, noiseMaskBits.length, 16);
    const padX = 6;
    const gapX = 4;
    const cellW = Math.max(12, Math.floor((w - padX * 2 - gapX * (byteCount - 1)) / byteCount));
    const bitH = 5;
    const bitGap = 2;

    for (let i = 0; i < byteCount; i++) {
      const bx = padX + i * (cellW + gapX);
      const val = latestBytes[i] ?? 0;
      const noise = noiseMaskBits[i] ?? 0;
      const diff = activeDiffBits[i] ?? 0;

      // Byte index header
      ctx.fillStyle = diff !== 0 ? '#34d399' : noise === 0xff ? '#b45309' : '#64748b';
      ctx.font = '9px monospace';
      ctx.textAlign = 'center';
      ctx.fillText(`B${i}`, bx + cellW / 2, 10);

      // 8 bit LEDs stacked (MSB top -> LSB bottom)
      for (let bit = 7; bit >= 0; bit--) {
        const bitMask = 1 << bit;
        const row = 7 - bit;
        const by = 14 + row * (bitH + bitGap);
        const isNoise = (noise & bitMask) !== 0;
        const isDiff = (diff & bitMask) !== 0;
        const isHigh = (val & bitMask) !== 0;

        if (isDiff) {
          ctx.fillStyle = isHigh ? '#10b981' : '#06b6d4';
          ctx.globalAlpha = calmMode ? 0.72 : 1.0;
        } else if (isNoise) {
          ctx.fillStyle = isHigh ? '#78350f' : '#451a03';
          ctx.globalAlpha = 0.65;
        } else {
          ctx.fillStyle = isHigh ? '#334155' : '#111827';
          ctx.globalAlpha = 1.0;
        }
        ctx.fillRect(bx, by, cellW, bitH);
      }

      ctx.globalAlpha = 1.0;
      // Hex footer
      ctx.fillStyle = diff !== 0 ? '#a7f3d0' : noise !== 0 ? '#d97706' : '#94a3b8';
      ctx.font = '9px monospace';
      ctx.fillText(val.toString(16).toUpperCase().padStart(2, '0'), bx + cellW / 2, h - 4);
    }
  }, [latestBytes, noiseMaskBits, activeDiffBits, calmMode]);

  return (
    <canvas
      ref={canvasRef}
      width={560}
      height={84}
      className="w-full h-[84px] rounded-md border border-studio-border bg-studio-bg block"
      aria-label="Live HID bit and byte activity strip"
    />
  );
};

/**
 * Live <canvas> verification bargraph showing raw vs modifier-shaped normalized output.
 */
const LiveBargraphCanvas: React.FC<{
  rawNorm: number;
  shapedNorm: number;
  calmMode: boolean;
}> = ({ rawNorm, shapedNorm, calmMode }) => {
  const ref = useRef<HTMLCanvasElement | null>(null);

  useEffect(() => {
    const canvas = ref.current;
    if (!canvas) return;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const w = canvas.width;
    const h = canvas.height;
    ctx.clearRect(0, 0, w, h);
    ctx.fillStyle = '#0b0e14';
    ctx.fillRect(0, 0, w, h);

    // Raw bar (top thin)
    ctx.fillStyle = '#334155';
    ctx.fillRect(0, 4, w, 6);
    ctx.fillStyle = '#38bdf8';
    ctx.fillRect(0, 4, Math.round(w * Math.max(0, Math.min(1, rawNorm))), 6);

    // Shaped bar (bottom thick)
    ctx.fillStyle = '#1e293b';
    ctx.fillRect(0, 14, w, 12);
    ctx.fillStyle = '#10b981';
    ctx.globalAlpha = calmMode ? 0.72 : 1;
    ctx.fillRect(0, 14, Math.round(w * Math.max(0, Math.min(1, shapedNorm))), 12);
    ctx.globalAlpha = 1;
  }, [rawNorm, shapedNorm, calmMode]);

  return (
    <canvas
      ref={ref}
      width={220}
      height={30}
      className="w-full h-[30px] rounded border border-studio-border block"
      aria-label="Live learned control verification bargraph"
    />
  );
};

function computeCandidateNorm(spec: LearnedControlSpec | null | undefined, bytes: number[]): number {
  if (!spec || bytes.length === 0) return 0;
  if (spec.control_type === 'axis16' && spec.byte_offset + 1 < bytes.length) {
    const b0 = bytes[spec.byte_offset] ?? 0;
    const b1 = bytes[spec.byte_offset + 1] ?? 0;
    const u = spec.little_endian ? b0 | (b1 << 8) : (b0 << 8) | b1;
    const raw = spec.signed && u >= 0x8000 ? u - 0x10000 : u;
    const span = Math.max(1, spec.observed_max - spec.observed_min);
    return Math.max(0, Math.min(1, (raw - spec.observed_min) / span));
  }
  if (spec.control_type === 'axis8') {
    const b = bytes[spec.byte_offset] ?? 0;
    const raw = spec.signed && b > 127 ? b - 256 : b;
    const span = Math.max(1, spec.observed_max - spec.observed_min);
    return Math.max(0, Math.min(1, (raw - spec.observed_min) / span));
  }
  if (spec.control_type === 'button' || spec.control_type === 'toggle') {
    return ((bytes[spec.byte_offset] ?? 0) & spec.bit_mask) !== 0 ? 1 : 0;
  }
  return 0.5;
}

export const HidLearnWizardTab: React.FC = () => {
  const calmMode = useWorkspaceStore((s) => s.graph.settings.calm_mode);
  const hidDevices = useM3Store((s) => s.hidDevices);
  const learn = useM3Store((s) => s.learnSnapshot);
  const selectedHidPath = useM3Store((s) => s.selectedHidPath);
  const lastSavedPresetJson = useM3Store((s) => s.lastSavedPresetJson);

  const simulatePlugIn = useM3Store((s) => s.simulatePlugIn);
  const simulateActuate = useM3Store((s) => s.simulateActuate);
  const startBaseline = useM3Store((s) => s.startBaseline);
  const pollLearnSamples = useM3Store((s) => s.pollLearnSamples);
  const startCapture = useM3Store((s) => s.startCapture);
  const classifyCaptured = useM3Store((s) => s.classifyCaptured);
  const commitControl = useM3Store((s) => s.commitControl);
  const saveAndAttachDevice = useM3Store((s) => s.saveAndAttachDevice);
  const resetLearn = useM3Store((s) => s.resetLearn);

  const [aliasDraft, setAliasDraft] = useState('');
  const [presetNameDraft, setPresetNameDraft] = useState('Learned USB Controller');
  const [enableInvert, setEnableInvert] = useState(false);
  const [enableDeadzone, setEnableDeadzone] = useState(false);
  const [enableOneEuro, setEnableOneEuro] = useState(true);
  const [enableHysteresis, setEnableHysteresis] = useState(false);
  const [exprDraft, setExprDraft] = useState('');
  const [simFaderVal, setSimFaderVal] = useState(0);
  const [simJoyVal, setSimJoyVal] = useState(0);
  const [testPreviewVal, setTestPreviewVal] = useState(0.5);

  const activePath = learn.device_path ?? selectedHidPath ?? hidDevices[0]?.path ?? null;
  const activeDevice = hidDevices.find((d) => d.path === activePath);

  useEffect(() => {
    if (learn.candidate) {
      setAliasDraft(learn.candidate.alias);
    }
  }, [learn.candidate?.logical_key, learn.candidate?.alias]);

  useEffect(() => {
    if (activeDevice?.product_name) {
      setPresetNameDraft(activeDevice.product_name.replace(/\s*\(.*\)\s*$/, '').trim() || 'Learned HID');
    }
  }, [activeDevice?.product_name]);

  const draftModifiers = useMemo<Modifier[]>(() => {
    const mods: Modifier[] = [];
    if (enableDeadzone) {
      mods.push({ type: 'Deadzone', center: 0.5, radius: 0.05, rescale: true });
    }
    if (enableInvert) {
      mods.push({ type: 'Invert' });
    }
    if (enableOneEuro) {
      mods.push({ type: 'SmoothOneEuro', min_cutoff: 1.4, beta: 0.05, d_cutoff: 1.0 });
    }
    if (enableHysteresis) {
      mods.push({
        type: 'Hysteresis',
        low_threshold: 0.35,
        high_threshold: 0.65,
        low_value: 0,
        high_value: 1,
      });
    }
    if (exprDraft.trim()) {
      mods.push({ type: 'Expression', expr: exprDraft.trim() });
    }
    return mods;
  }, [enableDeadzone, enableInvert, enableOneEuro, enableHysteresis, exprDraft]);

  const rawNorm = learn.candidate
    ? computeCandidateNorm(learn.candidate, learn.latest_bytes)
    : testPreviewVal;
  const shapedOut = applyModifierChain(
    draftModifiers,
    { type: 'Float', value: learn.step === 'review_and_tune' ? testPreviewVal : rawNorm },
    Date.now(),
    []
  );
  const shapedNorm =
    shapedOut && shapedOut.type === 'Float'
      ? Math.max(0, Math.min(1, shapedOut.value))
      : rawNorm;

  const activeStepNum =
    learn.step === 'capturing_baseline'
      ? 1
      : learn.step === 'recording_control'
      ? 2
      : learn.step === 'review_and_tune'
      ? 3
      : learn.step === 'completed'
      ? 4
      : 0;

  const noisyByteCount = learn.noise_mask_bits.filter((m) => m !== 0).length;

  const handleDownloadPreset = () => {
    if (!lastSavedPresetJson) return;
    const blob = new Blob([lastSavedPresetJson], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `${presetNameDraft.toLowerCase().replace(/[^a-z0-9]+/g, '-')}.kfdevice.json`;
    a.click();
    URL.revokeObjectURL(url);
  };

  return (
    <div className="h-full flex gap-3 p-2.5 overflow-hidden text-xs">
      {/* Left column: 4-step progress + Live Bit/Byte Activity Strip */}
      <div className="flex-1 min-w-0 flex flex-col gap-1.5">
        <div className="flex items-center justify-between gap-2">
          {/* 4-Step Indicator */}
          <div className="flex items-center gap-1 text-[10px] font-mono">
            {[
              { n: 1, label: '1. Baseline' },
              { n: 2, label: '2. Move Control' },
              { n: 3, label: '3. Name & Tune' },
              { n: 4, label: '4. Save Preset' },
            ].map((s) => (
              <span
                key={s.n}
                className={`px-2 py-0.5 rounded border ${
                  activeStepNum === s.n
                    ? 'bg-emerald-500/20 text-emerald-300 border-emerald-500/50 font-semibold'
                    : activeStepNum > s.n
                    ? 'bg-studio-panel text-slate-300 border-studio-border'
                    : 'bg-studio-bg text-studio-muted border-studio-border/60'
                }`}
              >
                {s.label}
              </span>
            ))}
          </div>

          <div className="flex items-center gap-2 text-[10px] text-studio-muted">
            {noisyByteCount > 0 && (
              <span className="px-1.5 py-0.5 rounded bg-amber-950/60 border border-amber-500/40 text-amber-300 font-mono">
                Noise Mask: {noisyByteCount}B ignored
              </span>
            )}
            <span className="font-mono text-slate-300">
              Learned: {learn.committed_controls.length}
            </span>
            <button
              onClick={() => void resetLearn()}
              className="p-1 rounded hover:bg-studio-elevated text-slate-400 hover:text-slate-200"
              title="Reset Learn Wizard"
            >
              <RotateCcw className="w-3 h-3" />
            </button>
          </div>
        </div>

        {/* Live Bit/Byte Activity Strip Canvas */}
        <BitActivityStripCanvas
          latestBytes={learn.latest_bytes}
          noiseMaskBits={learn.noise_mask_bits}
          activeDiffBits={learn.active_diff_bits}
          calmMode={calmMode}
        />

        <div className="flex items-center justify-between text-[10px] text-studio-muted">
          <div className="flex items-center gap-3">
            <span className="flex items-center gap-1">
              <span className="w-2 h-2 rounded-sm bg-emerald-400 inline-block" />
              Active Diff
            </span>
            <span className="flex items-center gap-1">
              <span className="w-2 h-2 rounded-sm bg-amber-700 inline-block" />
              Masked Noise (IMU / Counter)
            </span>
            <span>
              Base: {learn.baseline_samples} pkts · Cap: {learn.capture_samples} pkts
            </span>
          </div>
          {learn.candidate && (
            <span
              className={`px-1.5 py-0.5 rounded border font-mono text-[10px] ${
                CONTROL_BADGES[learn.candidate.control_type].color
              }`}
            >
              {CONTROL_BADGES[learn.candidate.control_type].label} · B{learn.candidate.byte_offset}{' '}
              {learn.candidate.control_type === 'axis16'
                ? `${learn.candidate.signed ? 'i16' : 'u16'} ${learn.candidate.little_endian ? 'LE' : 'BE'}`
                : `mask 0x${learn.candidate.bit_mask.toString(16).toUpperCase().padStart(2, '0')}`}
            </span>
          )}
        </div>
      </div>

      {/* Right column: Interactive Step Controls */}
      <div className="w-80 shrink-0 rounded-lg border border-studio-border bg-studio-panel p-2 flex flex-col justify-between overflow-y-auto">
        {!activePath ? (
          <div className="h-full flex flex-col items-center justify-center text-center gap-2">
            <Usb className="w-5 h-5 text-emerald-400 opacity-80" />
            <div className="text-xs font-medium text-slate-200">No HID Peripheral Selected</div>
            <button
              onClick={() => void simulatePlugIn('unknown_mystery_dial')}
              className="px-3 py-1.5 rounded-md bg-emerald-500/20 hover:bg-emerald-500/30 border border-emerald-500/40 text-emerald-200 text-xs font-medium"
            >
              Simulate Plug-In Unknown HID
            </button>
          </div>
        ) : learn.step === 'idle' || learn.step === 'capturing_baseline' ? (
          <div className="space-y-2">
            <div className="flex items-center justify-between">
              <span className="font-semibold text-emerald-300">Step 1 · Baseline Noise Mask</span>
              <span className="text-[10px] font-mono text-studio-muted truncate max-w-[120px]">
                {activeDevice?.product_name ?? activePath}
              </span>
            </div>
            <p className="text-[11px] text-studio-muted leading-snug">
              Hands off! Sample idle reports to mask fluctuating IMU, counter, and timestamp bytes.
            </p>
            <div className="flex gap-1.5">
              <button
                onClick={() => void startBaseline(activePath)}
                className="flex-1 flex items-center justify-center gap-1 py-1.5 rounded bg-studio-elevated hover:bg-slate-700 border border-studio-border text-slate-100 text-[11px] font-medium"
              >
                <Radio className="w-3 h-3 text-amber-400" />
                Sample Idle (32 pkts)
              </button>
              <button
                onClick={() => void startCapture()}
                disabled={learn.baseline_samples === 0}
                className="flex-1 flex items-center justify-center gap-1 py-1.5 rounded bg-emerald-500/20 hover:bg-emerald-500/30 disabled:opacity-40 border border-emerald-500/40 text-emerald-200 text-[11px] font-medium"
              >
                <Play className="w-3 h-3" />
                Next: Move Control
              </button>
            </div>
          </div>
        ) : learn.step === 'recording_control' ? (
          <div className="space-y-1.5">
            <div className="flex items-center justify-between">
              <span className="font-semibold text-emerald-300">Step 2 · Move One Control</span>
              <button
                onClick={() => void classifyCaptured()}
                className="px-2 py-0.5 rounded bg-emerald-500/25 hover:bg-emerald-500/35 border border-emerald-400/50 text-emerald-100 text-[11px] font-semibold flex items-center gap-1"
              >
                <Sparkles className="w-3 h-3" />
                Classify
              </button>
            </div>

            {/* Simulated virtual peripheral test controls (for hardware-free Learn demos) */}
            {activePath.startsWith('sim://') && (
              <div className="space-y-1 bg-studio-bg rounded p-1.5 border border-studio-border">
                <div className="flex items-center gap-1.5 text-[10px]">
                  <span className="w-14 text-studio-muted">Fader 8b</span>
                  <input
                    type="range"
                    min={0}
                    max={255}
                    value={simFaderVal}
                    onChange={(e) => {
                      const v = Number(e.target.value);
                      setSimFaderVal(v);
                      void simulateActuate(activePath, { control: 'set_fader_u8', value: v });
                    }}
                    className="flex-1 accent-emerald-400 h-1"
                  />
                  <span className="w-6 font-mono text-right">{simFaderVal}</span>
                </div>
                <div className="flex items-center gap-1.5 text-[10px]">
                  <span className="w-14 text-studio-muted">Axis 16b</span>
                  <input
                    type="range"
                    min={-30000}
                    max={30000}
                    step={1500}
                    value={simJoyVal}
                    onChange={(e) => {
                      const v = Number(e.target.value);
                      setSimJoyVal(v);
                      void simulateActuate(activePath, { control: 'set_axis_i16_le', value: v });
                    }}
                    className="flex-1 accent-amber-400 h-1"
                  />
                </div>
                <div className="flex gap-1 pt-0.5">
                  <button
                    onClick={() => {
                      void (async () => {
                        await simulateActuate(activePath, { control: 'set_button_a', pressed: true });
                        await simulateActuate(activePath, { control: 'set_button_a', pressed: false });
                      })();
                    }}
                    className="flex-1 py-0.5 rounded bg-studio-panel hover:bg-studio-elevated border border-studio-border text-[10px] text-cyan-300"
                  >
                    Press Btn A
                  </button>
                  <button
                    onClick={() => {
                      void (async () => {
                        await simulateActuate(activePath, { control: 'pulse_encoder', delta: 2 });
                        await pollLearnSamples(1);
                        await simulateActuate(activePath, { control: 'pulse_encoder', delta: -1 });
                        await pollLearnSamples(1);
                      })();
                    }}
                    className="flex-1 py-0.5 rounded bg-studio-panel hover:bg-studio-elevated border border-studio-border text-[10px] text-violet-300"
                  >
                    Turn Encoder
                  </button>
                </div>
              </div>
            )}
          </div>
        ) : learn.step === 'review_and_tune' ? (
          <div className="space-y-1.5">
            <div className="flex items-center justify-between gap-1.5">
              <span className="font-semibold text-amber-300 flex items-center gap-1">
                <Sliders className="w-3 h-3" />
                Step 3 · Name &amp; Tune
              </span>
              <button
                onClick={() =>
                  void commitControl(aliasDraft || 'Control', draftModifiers, 0.03, false)
                }
                className="px-2 py-0.5 rounded bg-emerald-500/25 hover:bg-emerald-500/35 border border-emerald-400/50 text-emerald-100 text-[11px] font-semibold flex items-center gap-1"
              >
                <CheckCircle2 className="w-3 h-3" />
                Accept
              </button>
            </div>

            <div className="flex gap-1.5">
              <input
                value={aliasDraft}
                onChange={(e) => setAliasDraft(e.target.value)}
                placeholder="Control name..."
                className="flex-1 bg-studio-bg border border-studio-border rounded px-2 py-0.5 text-[11px] text-white outline-none"
              />
              <input
                value={exprDraft}
                onChange={(e) => setExprDraft(e.target.value)}
                placeholder="expr (e.g. x^2)"
                className="w-24 bg-studio-bg border border-studio-border rounded px-1.5 py-0.5 text-[10px] font-mono text-amber-200 outline-none"
              />
            </div>

            <div className="flex flex-wrap gap-2 text-[10px] text-slate-300">
              <label className="flex items-center gap-1 cursor-pointer">
                <input
                  type="checkbox"
                  checked={enableDeadzone}
                  onChange={(e) => setEnableDeadzone(e.target.checked)}
                  className="accent-amber-400"
                />
                Deadzone
              </label>
              <label className="flex items-center gap-1 cursor-pointer">
                <input
                  type="checkbox"
                  checked={enableInvert}
                  onChange={(e) => setEnableInvert(e.target.checked)}
                  className="accent-amber-400"
                />
                Invert
              </label>
              <label className="flex items-center gap-1 cursor-pointer">
                <input
                  type="checkbox"
                  checked={enableOneEuro}
                  onChange={(e) => setEnableOneEuro(e.target.checked)}
                  className="accent-emerald-400"
                />
                1-Euro
              </label>
              <label className="flex items-center gap-1 cursor-pointer">
                <input
                  type="checkbox"
                  checked={enableHysteresis}
                  onChange={(e) => setEnableHysteresis(e.target.checked)}
                  className="accent-cyan-400"
                />
                Hysteresis
              </label>
            </div>

            <div className="flex items-center gap-2">
              <input
                type="range"
                min={0}
                max={1}
                step={0.02}
                value={testPreviewVal}
                onChange={(e) => setTestPreviewVal(Number(e.target.value))}
                className="w-20 accent-cyan-400 h-1"
                title="Preview transfer through modifiers"
              />
              <div className="flex-1">
                <LiveBargraphCanvas
                  rawNorm={testPreviewVal}
                  shapedNorm={shapedNorm}
                  calmMode={calmMode}
                />
              </div>
            </div>
          </div>
        ) : (
          <div className="space-y-2">
            <div className="flex items-center justify-between">
              <span className="font-semibold text-emerald-300">Step 4 · Save .kfdevice.json</span>
              <button
                onClick={() => void startCapture()}
                className="text-[10px] text-cyan-300 hover:text-cyan-200 underline"
              >
                + Learn Another Control
              </button>
            </div>

            <input
              value={presetNameDraft}
              onChange={(e) => setPresetNameDraft(e.target.value)}
              placeholder="Device preset name..."
              className="w-full bg-studio-bg border border-studio-border rounded px-2 py-1 text-[11px] text-white outline-none"
            />

            <div className="flex gap-1.5">
              <button
                onClick={() => void saveAndAttachDevice(presetNameDraft)}
                className="flex-1 flex items-center justify-center gap-1 py-1.5 rounded bg-emerald-500/25 hover:bg-emerald-500/35 border border-emerald-400/50 text-emerald-100 text-[11px] font-semibold"
              >
                <Save className="w-3 h-3" />
                Save &amp; Add to Canvas
              </button>
              {lastSavedPresetJson && (
                <button
                  onClick={handleDownloadPreset}
                  className="px-2 py-1.5 rounded bg-studio-elevated hover:bg-slate-700 border border-studio-border text-slate-200"
                  title="Download .kfdevice.json"
                >
                  <Download className="w-3.5 h-3.5 text-emerald-400" />
                </button>
              )}
            </div>
          </div>
        )}
      </div>
    </div>
  );
};
