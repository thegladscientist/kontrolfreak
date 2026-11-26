import type { SignalKind } from '../types/kontrolfreak';

export interface SignalColorSpec {
  label: string;
  hex: string;
  bgClass: string;
  textClass: string;
  borderClass: string;
  ringClass: string;
}

export const SIGNAL_COLORS: Record<SignalKind, SignalColorSpec> = {
  control: {
    label: 'Control Float',
    hex: '#f59e0b',
    bgClass: 'bg-amber-500/15',
    textClass: 'text-amber-400',
    borderClass: 'border-amber-500/40',
    ringClass: 'ring-amber-400',
  },
  midi: {
    label: 'MIDI',
    hex: '#22d3ee',
    bgClass: 'bg-cyan-500/15',
    textClass: 'text-cyan-400',
    borderClass: 'border-cyan-500/40',
    ringClass: 'ring-cyan-400',
  },
  trigger: {
    label: 'Trigger',
    hex: '#22d3ee',
    bgClass: 'bg-cyan-500/15',
    textClass: 'text-cyan-300',
    borderClass: 'border-cyan-400/40',
    ringClass: 'ring-cyan-300',
  },
  video: {
    label: 'Video / NDI',
    hex: '#a78bfa',
    bgClass: 'bg-violet-500/15',
    textClass: 'text-violet-400',
    borderClass: 'border-violet-500/40',
    ringClass: 'ring-violet-400',
  },
  sensor: {
    label: 'Hardware / Sensor',
    hex: '#34d399',
    bgClass: 'bg-emerald-500/15',
    textClass: 'text-emerald-400',
    borderClass: 'border-emerald-500/40',
    ringClass: 'ring-emerald-400',
  },
  byte_stream: {
    label: 'ByteStream',
    hex: '#94a3b8',
    bgClass: 'bg-slate-500/15',
    textClass: 'text-slate-300',
    borderClass: 'border-slate-500/40',
    ringClass: 'ring-slate-400',
  },
};

export function areSignalKindsCompatible(source: SignalKind, target: SignalKind): boolean {
  if (source === target) return true;
  const pairs: Array<[SignalKind, SignalKind]> = [
    ['control', 'midi'],
    ['midi', 'control'],
    ['control', 'trigger'],
    ['trigger', 'control'],
    ['midi', 'trigger'],
    ['trigger', 'midi'],
    ['sensor', 'control'],
    ['sensor', 'midi'],
    ['sensor', 'trigger'],
    ['byte_stream', 'control'],
    ['control', 'byte_stream'],
  ];
  return pairs.some(([a, b]) => a === source && b === target);
}
