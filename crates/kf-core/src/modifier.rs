use crate::signal::SignalValue;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ModifierError {
    #[error("Invalid math expression '{expr}': {reason}")]
    InvalidExpression { expr: String, reason: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CurveKind {
    Linear,
    Exponential { exponent: f64 },
    Logarithmic { base: f64 },
    SCurve { steepness: f64 },
    Bezier { p1: [f64; 2], p2: [f64; 2] },
}

impl Default for CurveKind {
    fn default() -> Self {
        Self::Linear
    }
}

impl CurveKind {
    pub fn apply(&self, input: f64) -> f64 {
        let x = input.clamp(0.0, 1.0);
        match self {
            CurveKind::Linear => x,
            CurveKind::Exponential { exponent } => {
                let exp = exponent.max(0.01);
                x.powf(exp)
            }
            CurveKind::Logarithmic { base } => {
                let b = base.max(1.01);
                ((b - 1.0) * x + 1.0).log(b)
            }
            CurveKind::SCurve { steepness } => {
                let k = steepness.max(0.1);
                let s = |v: f64| 1.0 / (1.0 + (-k * (v - 0.5)).exp());
                let lo = s(0.0);
                let hi = s(1.0);
                let denom = (hi - lo).max(1e-9);
                ((s(x) - lo) / denom).clamp(0.0, 1.0)
            }
            CurveKind::Bezier { p1, p2 } => eval_cubic_bezier(x, *p1, *p2),
        }
    }
}

fn eval_cubic_bezier(x: f64, p1: [f64; 2], p2: [f64; 2]) -> f64 {
    // Solve B_x(t) = x using Newton-Raphson with binary search fallback, then return B_y(t).
    let cx = 3.0 * p1[0];
    let bx = 3.0 * (p2[0] - p1[0]) - cx;
    let ax = 1.0 - cx - bx;

    let cy = 3.0 * p1[1];
    let by = 3.0 * (p2[1] - p1[1]) - cy;
    let ay = 1.0 - cy - by;

    let sample_x = |t: f64| ((ax * t + bx) * t + cx) * t;
    let sample_dx = |t: f64| (3.0 * ax * t + 2.0 * bx) * t + cx;
    let sample_y = |t: f64| ((ay * t + by) * t + cy) * t;

    let mut t = x;
    for _ in 0..8 {
        let err = sample_x(t) - x;
        if err.abs() < 1e-6 {
            return sample_y(t).clamp(0.0, 1.0);
        }
        let d = sample_dx(t);
        if d.abs() < 1e-6 {
            break;
        }
        t = (t - err / d).clamp(0.0, 1.0);
    }

    let mut lo = 0.0;
    let mut hi = 1.0;
    t = x;
    for _ in 0..16 {
        let sx = sample_x(t);
        if (sx - x).abs() < 1e-6 {
            break;
        }
        if x > sx {
            lo = t;
        } else {
            hi = t;
        }
        t = 0.5 * (lo + hi);
    }
    sample_y(t).clamp(0.0, 1.0)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Modifier {
    Invert,
    RangeMap {
        in_min: f64,
        in_max: f64,
        out_min: f64,
        out_max: f64,
        #[serde(default = "default_true")]
        clamp: bool,
    },
    Clamp {
        min: f64,
        max: f64,
    },
    Deadzone {
        center: f64,
        radius: f64,
        #[serde(default = "default_true")]
        rescale: bool,
    },
    Curve {
        kind: CurveKind,
    },
    SmoothOneEuro {
        min_cutoff: f64,
        beta: f64,
        #[serde(default = "default_d_cutoff")]
        d_cutoff: f64,
    },
    Hysteresis {
        low_threshold: f64,
        high_threshold: f64,
        #[serde(default)]
        low_value: f64,
        #[serde(default = "default_one")]
        high_value: f64,
    },
    Threshold {
        threshold: f64,
        #[serde(default)]
        rising_only: bool,
        #[serde(default)]
        emit_trigger: bool,
    },
    Expression {
        expr: String,
    },
}

fn default_true() -> bool {
    true
}
fn default_one() -> f64 {
    1.0
}
fn default_d_cutoff() -> f64 {
    1.0
}

#[derive(Debug, Clone, Default)]
pub struct OneEuroState {
    pub prev_x: Option<f64>,
    pub prev_dx: f64,
    pub prev_ts_ns: Option<u64>,
}

impl OneEuroState {
    fn alpha(cutoff: f64, dt: f64) -> f64 {
        let tau = 1.0 / (2.0 * std::f64::consts::PI * cutoff.max(1e-4));
        1.0 / (1.0 + tau / dt.max(1e-5))
    }

    pub fn filter(
        &mut self,
        x: f64,
        ts_ns: u64,
        min_cutoff: f64,
        beta: f64,
        d_cutoff: f64,
    ) -> f64 {
        let Some(prev_x) = self.prev_x else {
            self.prev_x = Some(x);
            self.prev_dx = 0.0;
            self.prev_ts_ns = Some(ts_ns);
            return x;
        };

        let dt = match self.prev_ts_ns {
            Some(prev_ts) if ts_ns > prev_ts => (ts_ns - prev_ts) as f64 / 1_000_000_000.0,
            _ => 1.0 / 60.0,
        }
        .clamp(1e-4, 1.0);

        let dx = (x - prev_x) / dt;
        let a_d = Self::alpha(d_cutoff, dt);
        let edx = a_d * dx + (1.0 - a_d) * self.prev_dx;

        let cutoff = min_cutoff.max(1e-4) + beta.max(0.0) * edx.abs();
        let a = Self::alpha(cutoff, dt);
        let filtered = a * x + (1.0 - a) * prev_x;

        self.prev_x = Some(filtered);
        self.prev_dx = edx;
        self.prev_ts_ns = Some(ts_ns);
        filtered
    }
}

#[derive(Debug, Clone, Default)]
pub struct ModifierSlotState {
    pub one_euro: OneEuroState,
    pub hysteresis_high: bool,
    pub threshold_prev_high: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ModifierChainState {
    pub slots: Vec<ModifierSlotState>,
}

impl ModifierChainState {
    fn ensure_len(&mut self, len: usize) {
        if self.slots.len() != len {
            self.slots.resize_with(len, ModifierSlotState::default);
        }
    }
}

/// Zero-allocation stack bytecode VM for pre-compiled math expressions.
#[derive(Debug, Clone)]
pub struct CompiledExpression {
    ops: Vec<ExprOp>,
}

#[derive(Debug, Clone)]
enum ExprOp {
    PushConst(f64),
    PushX,
    Neg,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Pow,
    Func1(fn(f64) -> f64),
    Func2(fn(f64, f64) -> f64),
    Func3(fn(f64, f64, f64) -> f64),
}

impl CompiledExpression {
    pub fn compile(expr: &str) -> Result<Self, ModifierError> {
        let tokens = tokenize(expr)?;
        let mut parser = ExprParser {
            tokens: &tokens,
            pos: 0,
            ops: Vec::new(),
            src: expr,
        };
        parser.parse_expr()?;
        if parser.pos != tokens.len() {
            return Err(ModifierError::InvalidExpression {
                expr: expr.to_string(),
                reason: format!("unexpected token at position {}", parser.pos),
            });
        }
        Ok(Self { ops: parser.ops })
    }

    /// Evaluates the pre-compiled expression using a fixed stack buffer (zero heap allocations).
    pub fn eval(&self, x: f64) -> f64 {
        let mut stack = [0.0_f64; 64];
        let mut sp: usize = 0;

        for op in &self.ops {
            match *op {
                ExprOp::PushConst(v) => {
                    if sp < stack.len() {
                        stack[sp] = v;
                        sp += 1;
                    }
                }
                ExprOp::PushX => {
                    if sp < stack.len() {
                        stack[sp] = x;
                        sp += 1;
                    }
                }
                ExprOp::Neg => {
                    if sp >= 1 {
                        stack[sp - 1] = -stack[sp - 1];
                    }
                }
                ExprOp::Add => {
                    if sp >= 2 {
                        sp -= 1;
                        stack[sp - 1] += stack[sp];
                    }
                }
                ExprOp::Sub => {
                    if sp >= 2 {
                        sp -= 1;
                        stack[sp - 1] -= stack[sp];
                    }
                }
                ExprOp::Mul => {
                    if sp >= 2 {
                        sp -= 1;
                        stack[sp - 1] *= stack[sp];
                    }
                }
                ExprOp::Div => {
                    if sp >= 2 {
                        sp -= 1;
                        let denom = stack[sp];
                        stack[sp - 1] = if denom.abs() < 1e-12 {
                            0.0
                        } else {
                            stack[sp - 1] / denom
                        };
                    }
                }
                ExprOp::Rem => {
                    if sp >= 2 {
                        sp -= 1;
                        let denom = stack[sp];
                        stack[sp - 1] = if denom.abs() < 1e-12 {
                            0.0
                        } else {
                            stack[sp - 1] % denom
                        };
                    }
                }
                ExprOp::Pow => {
                    if sp >= 2 {
                        sp -= 1;
                        stack[sp - 1] = stack[sp - 1].powf(stack[sp]);
                    }
                }
                ExprOp::Func1(f) => {
                    if sp >= 1 {
                        stack[sp - 1] = f(stack[sp - 1]);
                    }
                }
                ExprOp::Func2(f) => {
                    if sp >= 2 {
                        sp -= 1;
                        stack[sp - 1] = f(stack[sp - 1], stack[sp]);
                    }
                }
                ExprOp::Func3(f) => {
                    if sp >= 3 {
                        sp -= 2;
                        stack[sp - 1] = f(stack[sp - 1], stack[sp], stack[sp + 1]);
                    }
                }
            }
        }
        let result = if sp > 0 { stack[sp - 1] } else { x };
        if result.is_finite() {
            result
        } else {
            0.0
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Num(f64),
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Caret,
    LParen,
    RParen,
    Comma,
}

fn tokenize(src: &str) -> Result<Vec<Token>, ModifierError> {
    let mut out = Vec::new();
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if c.is_ascii_digit() || c == '.' {
            let start = i;
            while i < bytes.len() && ((bytes[i] as char).is_ascii_digit() || bytes[i] == b'.') {
                i += 1;
            }
            let num: f64 = src[start..i].parse().map_err(|_| {
                ModifierError::InvalidExpression {
                    expr: src.to_string(),
                    reason: format!("invalid number '{}'", &src[start..i]),
                }
            })?;
            out.push(Token::Num(num));
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < bytes.len()
                && ((bytes[i] as char).is_ascii_alphanumeric() || bytes[i] == b'_')
            {
                i += 1;
            }
            out.push(Token::Ident(src[start..i].to_ascii_lowercase()));
            continue;
        }
        match c {
            '+' => out.push(Token::Plus),
            '-' => out.push(Token::Minus),
            '*' => out.push(Token::Star),
            '/' => out.push(Token::Slash),
            '%' => out.push(Token::Percent),
            '^' => out.push(Token::Caret),
            '(' => out.push(Token::LParen),
            ')' => out.push(Token::RParen),
            ',' => out.push(Token::Comma),
            _ => {
                return Err(ModifierError::InvalidExpression {
                    expr: src.to_string(),
                    reason: format!("unexpected character '{c}'"),
                })
            }
        }
        i += 1;
    }
    Ok(out)
}

struct ExprParser<'a> {
    tokens: &'a [Token],
    pos: usize,
    ops: Vec<ExprOp>,
    src: &'a str,
}

impl<'a> ExprParser<'a> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn err(&self, reason: impl Into<String>) -> ModifierError {
        ModifierError::InvalidExpression {
            expr: self.src.to_string(),
            reason: reason.into(),
        }
    }

    fn parse_expr(&mut self) -> Result<(), ModifierError> {
        self.parse_term()?;
        while let Some(tok) = self.peek() {
            match tok {
                Token::Plus => {
                    self.pos += 1;
                    self.parse_term()?;
                    self.ops.push(ExprOp::Add);
                }
                Token::Minus => {
                    self.pos += 1;
                    self.parse_term()?;
                    self.ops.push(ExprOp::Sub);
                }
                _ => break,
            }
        }
        Ok(())
    }

    fn parse_term(&mut self) -> Result<(), ModifierError> {
        self.parse_power()?;
        while let Some(tok) = self.peek() {
            match tok {
                Token::Star => {
                    self.pos += 1;
                    self.parse_power()?;
                    self.ops.push(ExprOp::Mul);
                }
                Token::Slash => {
                    self.pos += 1;
                    self.parse_power()?;
                    self.ops.push(ExprOp::Div);
                }
                Token::Percent => {
                    self.pos += 1;
                    self.parse_power()?;
                    self.ops.push(ExprOp::Rem);
                }
                _ => break,
            }
        }
        Ok(())
    }

    fn parse_power(&mut self) -> Result<(), ModifierError> {
        self.parse_unary()?;
        if matches!(self.peek(), Some(Token::Caret)) {
            self.pos += 1;
            self.parse_power()?;
            self.ops.push(ExprOp::Pow);
        }
        Ok(())
    }

    fn parse_unary(&mut self) -> Result<(), ModifierError> {
        match self.peek() {
            Some(Token::Minus) => {
                self.pos += 1;
                self.parse_unary()?;
                self.ops.push(ExprOp::Neg);
                Ok(())
            }
            Some(Token::Plus) => {
                self.pos += 1;
                self.parse_unary()
            }
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> Result<(), ModifierError> {
        let tok = self.peek().cloned().ok_or_else(|| self.err("unexpected end of expression"))?;
        match tok {
            Token::Num(v) => {
                self.pos += 1;
                self.ops.push(ExprOp::PushConst(v));
                Ok(())
            }
            Token::LParen => {
                self.pos += 1;
                self.parse_expr()?;
                if !matches!(self.peek(), Some(Token::RParen)) {
                    return Err(self.err("expected ')'"));
                }
                self.pos += 1;
                Ok(())
            }
            Token::Ident(name) => {
                self.pos += 1;
                if matches!(self.peek(), Some(Token::LParen)) {
                    self.pos += 1;
                    let mut argc = 0usize;
                    if !matches!(self.peek(), Some(Token::RParen)) {
                        loop {
                            self.parse_expr()?;
                            argc += 1;
                            if matches!(self.peek(), Some(Token::Comma)) {
                                self.pos += 1;
                            } else {
                                break;
                            }
                        }
                    }
                    if !matches!(self.peek(), Some(Token::RParen)) {
                        return Err(self.err(format!("expected ')' after {name}(...)")));
                    }
                    self.pos += 1;
                    self.emit_func(&name, argc)?;
                    Ok(())
                } else {
                    match name.as_str() {
                        "x" | "v" | "val" => self.ops.push(ExprOp::PushX),
                        "pi" => self.ops.push(ExprOp::PushConst(std::f64::consts::PI)),
                        "e" => self.ops.push(ExprOp::PushConst(std::f64::consts::E)),
                        _ => return Err(self.err(format!("unknown identifier '{name}'"))),
                    }
                    Ok(())
                }
            }
            _ => Err(self.err(format!("unexpected token {tok:?}"))),
        }
    }

    fn emit_func(&mut self, name: &str, argc: usize) -> Result<(), ModifierError> {
        match (name, argc) {
            ("abs", 1) => self.ops.push(ExprOp::Func1(f64::abs)),
            ("sin", 1) => self.ops.push(ExprOp::Func1(f64::sin)),
            ("cos", 1) => self.ops.push(ExprOp::Func1(f64::cos)),
            ("tan", 1) => self.ops.push(ExprOp::Func1(f64::tan)),
            ("sqrt", 1) => self.ops.push(ExprOp::Func1(|v| v.max(0.0).sqrt())),
            ("floor", 1) => self.ops.push(ExprOp::Func1(f64::floor)),
            ("ceil", 1) => self.ops.push(ExprOp::Func1(f64::ceil)),
            ("round", 1) => self.ops.push(ExprOp::Func1(f64::round)),
            ("sign", 1) => self.ops.push(ExprOp::Func1(|v| {
                if v > 0.0 {
                    1.0
                } else if v < 0.0 {
                    -1.0
                } else {
                    0.0
                }
            })),
            ("min", 2) => self.ops.push(ExprOp::Func2(f64::min)),
            ("max", 2) => self.ops.push(ExprOp::Func2(f64::max)),
            ("pow", 2) => self.ops.push(ExprOp::Func2(f64::powf)),
            ("step", 2) => self.ops.push(ExprOp::Func2(|edge, v| if v < edge { 0.0 } else { 1.0 })),
            ("clamp", 3) => self.ops.push(ExprOp::Func3(|v, lo, hi| v.clamp(lo.min(hi), lo.max(hi)))),
            ("lerp", 3) => self.ops.push(ExprOp::Func3(|a, b, t| a + (b - a) * t)),
            ("smoothstep", 3) => self.ops.push(ExprOp::Func3(|e0, e1, v| {
                let t = ((v - e0) / (e1 - e0).max(1e-9)).clamp(0.0, 1.0);
                t * t * (3.0 - 2.0 * t)
            })),
            _ => {
                return Err(self.err(format!(
                    "unknown function '{name}' with {argc} argument(s)"
                )))
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum CompiledModifier {
    Invert,
    RangeMap {
        in_min: f64,
        in_max: f64,
        out_min: f64,
        out_max: f64,
        clamp: bool,
    },
    Clamp {
        min: f64,
        max: f64,
    },
    Deadzone {
        center: f64,
        radius: f64,
        rescale: bool,
    },
    Curve {
        kind: CurveKind,
    },
    SmoothOneEuro {
        min_cutoff: f64,
        beta: f64,
        d_cutoff: f64,
    },
    Hysteresis {
        low_threshold: f64,
        high_threshold: f64,
        low_value: f64,
        high_value: f64,
    },
    Threshold {
        threshold: f64,
        rising_only: bool,
        emit_trigger: bool,
    },
    Expression {
        compiled: CompiledExpression,
    },
}

#[derive(Debug, Clone, Default)]
pub struct CompiledModifierChain {
    modifiers: Vec<CompiledModifier>,
}

impl CompiledModifierChain {
    pub fn compile(modifiers: &[Modifier]) -> Result<Self, ModifierError> {
        let mut compiled = Vec::with_capacity(modifiers.len());
        for m in modifiers {
            compiled.push(match m {
                Modifier::Invert => CompiledModifier::Invert,
                Modifier::RangeMap {
                    in_min,
                    in_max,
                    out_min,
                    out_max,
                    clamp,
                } => CompiledModifier::RangeMap {
                    in_min: *in_min,
                    in_max: *in_max,
                    out_min: *out_min,
                    out_max: *out_max,
                    clamp: *clamp,
                },
                Modifier::Clamp { min, max } => CompiledModifier::Clamp {
                    min: *min,
                    max: *max,
                },
                Modifier::Deadzone {
                    center,
                    radius,
                    rescale,
                } => CompiledModifier::Deadzone {
                    center: *center,
                    radius: *radius,
                    rescale: *rescale,
                },
                Modifier::Curve { kind } => CompiledModifier::Curve { kind: kind.clone() },
                Modifier::SmoothOneEuro {
                    min_cutoff,
                    beta,
                    d_cutoff,
                } => CompiledModifier::SmoothOneEuro {
                    min_cutoff: *min_cutoff,
                    beta: *beta,
                    d_cutoff: *d_cutoff,
                },
                Modifier::Hysteresis {
                    low_threshold,
                    high_threshold,
                    low_value,
                    high_value,
                } => CompiledModifier::Hysteresis {
                    low_threshold: *low_threshold,
                    high_threshold: *high_threshold,
                    low_value: *low_value,
                    high_value: *high_value,
                },
                Modifier::Threshold {
                    threshold,
                    rising_only,
                    emit_trigger,
                } => CompiledModifier::Threshold {
                    threshold: *threshold,
                    rising_only: *rising_only,
                    emit_trigger: *emit_trigger,
                },
                Modifier::Expression { expr } => CompiledModifier::Expression {
                    compiled: CompiledExpression::compile(expr)?,
                },
            });
        }
        Ok(Self {
            modifiers: compiled,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.modifiers.is_empty()
    }

    pub fn len(&self) -> usize {
        self.modifiers.len()
    }

    /// Evaluates the modifier chain on a SignalValue. Returns `None` if a rising-only threshold
    /// suppressed the event.
    pub fn apply(
        &self,
        input: SignalValue,
        timestamp_ns: u64,
        state: &mut ModifierChainState,
    ) -> Option<SignalValue> {
        if self.modifiers.is_empty() {
            return Some(input);
        }
        let Some(mut x) = input.as_f64() else {
            return Some(input);
        };

        state.ensure_len(self.modifiers.len());
        let mut emit_as_trigger = false;

        for (idx, modifier) in self.modifiers.iter().enumerate() {
            let slot = &mut state.slots[idx];
            match modifier {
                CompiledModifier::Invert => {
                    x = 1.0 - x;
                }
                CompiledModifier::RangeMap {
                    in_min,
                    in_max,
                    out_min,
                    out_max,
                    clamp,
                } => {
                    let span = in_max - in_min;
                    let mut norm = if span.abs() < 1e-12 {
                        0.0
                    } else {
                        (x - in_min) / span
                    };
                    if *clamp {
                        norm = norm.clamp(0.0, 1.0);
                    }
                    x = out_min + norm * (out_max - out_min);
                }
                CompiledModifier::Clamp { min, max } => {
                    let lo = min.min(*max);
                    let hi = min.max(*max);
                    x = x.clamp(lo, hi);
                }
                CompiledModifier::Deadzone {
                    center,
                    radius,
                    rescale,
                } => {
                    let r = radius.abs();
                    let delta = x - center;
                    if delta.abs() <= r {
                        x = *center;
                    } else if *rescale {
                        if delta > 0.0 {
                            let max_span = (1.0 - center - r).max(1e-9);
                            x = center + ((delta - r) / max_span) * (1.0 - center);
                        } else {
                            let min_span = (center - r).max(1e-9);
                            x = center - ((-delta - r) / min_span) * center;
                        }
                    }
                }
                CompiledModifier::Curve { kind } => {
                    x = kind.apply(x);
                }
                CompiledModifier::SmoothOneEuro {
                    min_cutoff,
                    beta,
                    d_cutoff,
                } => {
                    x = slot
                        .one_euro
                        .filter(x, timestamp_ns, *min_cutoff, *beta, *d_cutoff);
                }
                CompiledModifier::Hysteresis {
                    low_threshold,
                    high_threshold,
                    low_value,
                    high_value,
                } => {
                    if slot.hysteresis_high {
                        if x <= *low_threshold {
                            slot.hysteresis_high = false;
                        }
                    } else if x >= *high_threshold {
                        slot.hysteresis_high = true;
                    }
                    x = if slot.hysteresis_high {
                        *high_value
                    } else {
                        *low_value
                    };
                }
                CompiledModifier::Threshold {
                    threshold,
                    rising_only,
                    emit_trigger,
                } => {
                    let is_high = x >= *threshold;
                    let rising_edge = is_high && !slot.threshold_prev_high;
                    slot.threshold_prev_high = is_high;
                    if *rising_only {
                        if !rising_edge {
                            return None;
                        }
                        if *emit_trigger {
                            emit_as_trigger = true;
                        } else {
                            x = 1.0;
                        }
                    } else if *emit_trigger && is_high {
                        emit_as_trigger = true;
                    } else {
                        x = if is_high { 1.0 } else { 0.0 };
                    }
                }
                CompiledModifier::Expression { compiled } => {
                    x = compiled.eval(x);
                }
            }
        }

        if emit_as_trigger {
            Some(SignalValue::Trigger)
        } else {
            Some(SignalValue::Float(x))
        }
    }
}

/// Samples a static transfer curve over `0.0..=1.0` for UI canvas visualization.
/// Statelessly evaluates deterministic modifiers (1-Euro smoothing is treated as pass-through on
/// static transfer curve previews so the curve represents steady-state mapping).
pub fn sample_transfer_curve(modifiers: &[Modifier], steps: usize) -> Result<Vec<[f64; 2]>, ModifierError> {
    let steps = steps.clamp(2, 512);
    let stateless_mods: Vec<Modifier> = modifiers
        .iter()
        .filter(|m| !matches!(m, Modifier::SmoothOneEuro { .. }))
        .cloned()
        .collect();
    let chain = CompiledModifierChain::compile(&stateless_mods)?;
    let mut points = Vec::with_capacity(steps);
    for i in 0..steps {
        let x = i as f64 / (steps - 1) as f64;
        let mut state = ModifierChainState::default();
        let y = chain
            .apply(SignalValue::Float(x), 0, &mut state)
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        points.push([x, y]);
    }
    Ok(points)
}
