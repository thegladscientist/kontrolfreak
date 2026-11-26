import type { CurveKind, Modifier, SignalValue } from '../types/kontrolfreak';

export function signalToNumber(val: SignalValue): number | null {
  switch (val.type) {
    case 'Float':
    case 'Int':
      return val.value;
    case 'Bool':
      return val.value ? 1 : 0;
    case 'Trigger':
      return 1;
    case 'Vec2':
      return val.value[0];
    case 'Vec3':
      return val.value[0];
    case 'String': {
      const n = Number(val.value);
      return Number.isFinite(n) ? n : null;
    }
    case 'Bytes':
      return val.value.length > 0 ? val.value[0] / 255 : null;
    case 'FrameRef':
      return null;
  }
}

export function formatSignalSummary(val: SignalValue): string {
  switch (val.type) {
    case 'Float':
      return val.value.toFixed(3);
    case 'Int':
      return String(val.value);
    case 'Bool':
      return val.value ? 'true' : 'false';
    case 'String':
      return `"${val.value}"`;
    case 'Trigger':
      return 'TRIG';
    case 'Vec2':
      return `[${val.value[0].toFixed(2)}, ${val.value[1].toFixed(2)}]`;
    case 'Vec3':
      return `[${val.value[0].toFixed(2)}, ${val.value[1].toFixed(2)}, ${val.value[2].toFixed(2)}]`;
    case 'Bytes':
      return `${val.value.length}B`;
    case 'FrameRef':
      return `${val.value.width}x${val.value.height}`;
  }
}

export function applyCurve(kind: CurveKind, input: number): number {
  const x = Math.min(1, Math.max(0, input));
  switch (kind.kind) {
    case 'linear':
      return x;
    case 'exponential':
      return Math.pow(x, Math.max(0.01, kind.exponent));
    case 'logarithmic': {
      const b = Math.max(1.01, kind.base);
      return Math.log((b - 1) * x + 1) / Math.log(b);
    }
    case 's_curve': {
      const k = Math.max(0.1, kind.steepness);
      const s = (v: number) => 1 / (1 + Math.exp(-k * (v - 0.5)));
      const lo = s(0);
      const hi = s(1);
      return Math.min(1, Math.max(0, (s(x) - lo) / Math.max(1e-9, hi - lo)));
    }
    case 'bezier':
      return evalCubicBezier(x, kind.p1, kind.p2);
  }
}

function evalCubicBezier(x: number, p1: [number, number], p2: [number, number]): number {
  const cx = 3 * p1[0];
  const bx = 3 * (p2[0] - p1[0]) - cx;
  const ax = 1 - cx - bx;

  const cy = 3 * p1[1];
  const by = 3 * (p2[1] - p1[1]) - cy;
  const ay = 1 - cy - by;

  const sx = (t: number) => ((ax * t + bx) * t + cx) * t;
  const sy = (t: number) => ((ay * t + by) * t + cy) * t;

  let lo = 0;
  let hi = 1;
  let t = x;
  for (let i = 0; i < 16; i++) {
    const cur = sx(t);
    if (Math.abs(cur - x) < 1e-5) break;
    if (x > cur) lo = t;
    else hi = t;
    t = 0.5 * (lo + hi);
  }
  return Math.min(1, Math.max(0, sy(t)));
}

/** Safe recursive-descent math expression evaluator mirroring kf-core's bytecode VM. */
export function evalMathExpression(expr: string, x: number): number {
  const trimmed = expr.trim();
  if (!trimmed) return x;
  const tokens: Array<{ t: string; v?: number | string }> = [];
  let i = 0;
  while (i < trimmed.length) {
    const c = trimmed[i];
    if (/\s/.test(c)) {
      i++;
      continue;
    }
    if (/[0-9.]/.test(c)) {
      let s = i;
      while (i < trimmed.length && /[0-9.]/.test(trimmed[i])) i++;
      tokens.push({ t: 'num', v: Number(trimmed.slice(s, i)) });
      continue;
    }
    if (/[a-zA-Z_]/.test(c)) {
      let s = i;
      while (i < trimmed.length && /[a-zA-Z0-9_]/.test(trimmed[i])) i++;
      tokens.push({ t: 'id', v: trimmed.slice(s, i).toLowerCase() });
      continue;
    }
    if ('+-*/%^(),'.includes(c)) {
      tokens.push({ t: c });
      i++;
      continue;
    }
    return x;
  }

  let pos = 0;
  const peek = () => tokens[pos];

  const parseExpr = (): number => {
    let left = parseTerm();
    while (peek()?.t === '+' || peek()?.t === '-') {
      const op = tokens[pos++].t;
      const right = parseTerm();
      left = op === '+' ? left + right : left - right;
    }
    return left;
  };

  const parseTerm = (): number => {
    let left = parsePow();
    while (peek()?.t === '*' || peek()?.t === '/' || peek()?.t === '%') {
      const op = tokens[pos++].t;
      const right = parsePow();
      if (op === '*') left *= right;
      else if (op === '/') left = Math.abs(right) < 1e-12 ? 0 : left / right;
      else left = Math.abs(right) < 1e-12 ? 0 : left % right;
    }
    return left;
  };

  const parsePow = (): number => {
    let base = parseUnary();
    if (peek()?.t === '^') {
      pos++;
      base = Math.pow(base, parsePow());
    }
    return base;
  };

  const parseUnary = (): number => {
    if (peek()?.t === '-') {
      pos++;
      return -parseUnary();
    }
    if (peek()?.t === '+') {
      pos++;
      return parseUnary();
    }
    return parsePrimary();
  };

  const parsePrimary = (): number => {
    const tok = peek();
    if (!tok) return 0;
    if (tok.t === 'num') {
      pos++;
      return Number(tok.v);
    }
    if (tok.t === '(') {
      pos++;
      const v = parseExpr();
      if (peek()?.t === ')') pos++;
      return v;
    }
    if (tok.t === 'id') {
      const name = String(tok.v);
      pos++;
      if (peek()?.t === '(') {
        pos++;
        const args: number[] = [];
        if (peek()?.t !== ')') {
          while (pos < tokens.length) {
            args.push(parseExpr());
            if (peek()?.t === ',') pos++;
            else break;
          }
        }
        if (peek()?.t === ')') pos++;
        switch (name) {
          case 'abs':
            return Math.abs(args[0] ?? 0);
          case 'sin':
            return Math.sin(args[0] ?? 0);
          case 'cos':
            return Math.cos(args[0] ?? 0);
          case 'tan':
            return Math.tan(args[0] ?? 0);
          case 'sqrt':
            return Math.sqrt(Math.max(0, args[0] ?? 0));
          case 'floor':
            return Math.floor(args[0] ?? 0);
          case 'ceil':
            return Math.ceil(args[0] ?? 0);
          case 'round':
            return Math.round(args[0] ?? 0);
          case 'sign':
            return Math.sign(args[0] ?? 0);
          case 'min':
            return Math.min(args[0] ?? 0, args[1] ?? 0);
          case 'max':
            return Math.max(args[0] ?? 0, args[1] ?? 0);
          case 'pow':
            return Math.pow(args[0] ?? 0, args[1] ?? 1);
          case 'step':
            return (args[1] ?? 0) < (args[0] ?? 0) ? 0 : 1;
          case 'clamp': {
            const lo = Math.min(args[1] ?? 0, args[2] ?? 1);
            const hi = Math.max(args[1] ?? 0, args[2] ?? 1);
            return Math.min(hi, Math.max(lo, args[0] ?? 0));
          }
          case 'lerp':
            return (args[0] ?? 0) + ((args[1] ?? 1) - (args[0] ?? 0)) * (args[2] ?? 0);
          case 'smoothstep': {
            const e0 = args[0] ?? 0;
            const e1 = args[1] ?? 1;
            const t = Math.min(1, Math.max(0, ((args[2] ?? 0) - e0) / Math.max(1e-9, e1 - e0)));
            return t * t * (3 - 2 * t);
          }
          default:
            return x;
        }
      }
      if (name === 'x' || name === 'v' || name === 'val') return x;
      if (name === 'pi') return Math.PI;
      if (name === 'e') return Math.E;
      return 0;
    }
    return 0;
  };

  try {
    const out = parseExpr();
    return Number.isFinite(out) ? out : 0;
  } catch {
    return x;
  }
}

export interface ModifierSlotState {
  prevX?: number;
  prevDx: number;
  prevTsMs?: number;
  hysteresisHigh: boolean;
  thresholdPrevHigh: boolean;
}

export function applyModifierChain(
  modifiers: Modifier[],
  input: SignalValue,
  timestampMs: number,
  slots: ModifierSlotState[]
): SignalValue | null {
  if (modifiers.length === 0) return input;
  const num = signalToNumber(input);
  if (num === null) return input;

  while (slots.length < modifiers.length) {
    slots.push({ prevDx: 0, hysteresisHigh: false, thresholdPrevHigh: false });
  }

  let x = num;
  let emitTrigger = false;

  for (let i = 0; i < modifiers.length; i++) {
    const m = modifiers[i];
    const slot = slots[i];
    switch (m.type) {
      case 'Invert':
        x = 1 - x;
        break;
      case 'RangeMap': {
        const span = m.in_max - m.in_min;
        let norm = Math.abs(span) < 1e-12 ? 0 : (x - m.in_min) / span;
        if (m.clamp) norm = Math.min(1, Math.max(0, norm));
        x = m.out_min + norm * (m.out_max - m.out_min);
        break;
      }
      case 'Clamp': {
        const lo = Math.min(m.min, m.max);
        const hi = Math.max(m.min, m.max);
        x = Math.min(hi, Math.max(lo, x));
        break;
      }
      case 'Deadzone': {
        const r = Math.abs(m.radius);
        const delta = x - m.center;
        if (Math.abs(delta) <= r) {
          x = m.center;
        } else if (m.rescale) {
          if (delta > 0) {
            const maxSpan = Math.max(1e-9, 1 - m.center - r);
            x = m.center + ((delta - r) / maxSpan) * (1 - m.center);
          } else {
            const minSpan = Math.max(1e-9, m.center - r);
            x = m.center - ((-delta - r) / minSpan) * m.center;
          }
        }
        break;
      }
      case 'Curve':
        x = applyCurve(m.kind, x);
        break;
      case 'SmoothOneEuro': {
        if (slot.prevX === undefined) {
          slot.prevX = x;
          slot.prevDx = 0;
          slot.prevTsMs = timestampMs;
        } else {
          const dt = Math.min(
            1,
            Math.max(1e-4, ((timestampMs - (slot.prevTsMs ?? timestampMs)) || 16.6) / 1000)
          );
          const alpha = (cutoff: number) => {
            const tau = 1 / (2 * Math.PI * Math.max(1e-4, cutoff));
            return 1 / (1 + tau / dt);
          };
          const dx = (x - slot.prevX) / dt;
          const aD = alpha(m.d_cutoff || 1);
          const edx = aD * dx + (1 - aD) * slot.prevDx;
          const cutoff = Math.max(1e-4, m.min_cutoff) + Math.max(0, m.beta) * Math.abs(edx);
          const a = alpha(cutoff);
          x = a * x + (1 - a) * slot.prevX;
          slot.prevX = x;
          slot.prevDx = edx;
          slot.prevTsMs = timestampMs;
        }
        break;
      }
      case 'Hysteresis': {
        if (slot.hysteresisHigh) {
          if (x <= m.low_threshold) slot.hysteresisHigh = false;
        } else if (x >= m.high_threshold) {
          slot.hysteresisHigh = true;
        }
        x = slot.hysteresisHigh ? m.high_value : m.low_value;
        break;
      }
      case 'Threshold': {
        const isHigh = x >= m.threshold;
        const rising = isHigh && !slot.thresholdPrevHigh;
        slot.thresholdPrevHigh = isHigh;
        if (m.rising_only) {
          if (!rising) return null;
          if (m.emit_trigger) emitTrigger = true;
          else x = 1;
        } else if (m.emit_trigger && isHigh) {
          emitTrigger = true;
        } else {
          x = isHigh ? 1 : 0;
        }
        break;
      }
      case 'Expression':
        x = evalMathExpression(m.expr, x);
        break;
    }
  }

  return emitTrigger ? { type: 'Trigger' } : { type: 'Float', value: x };
}

export function sampleTransferCurveLocal(
  modifiers: Modifier[],
  steps = 65
): Array<[number, number]> {
  const stateless = modifiers.filter((m) => m.type !== 'SmoothOneEuro');
  const points: Array<[number, number]> = [];
  for (let i = 0; i < steps; i++) {
    const x = i / (steps - 1);
    const out = applyModifierChain(stateless, { type: 'Float', value: x }, 0, []);
    const y = out ? (signalToNumber(out) ?? 0) : 0;
    points.push([x, y]);
  }
  return points;
}
