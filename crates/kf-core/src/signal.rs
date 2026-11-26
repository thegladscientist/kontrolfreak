use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

pub const MAX_SIGNAL_HOPS: u8 = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalKind {
    Control,
    Trigger,
    Midi,
    Video,
    Sensor,
    ByteStream,
}

impl SignalKind {
    /// Returns true if a source port of `self` can wire to a destination port of `target`.
    pub fn is_compatible_with(self, target: SignalKind) -> bool {
        if self == target {
            return true;
        }
        matches!(
            (self, target),
            (SignalKind::Control, SignalKind::Midi)
                | (SignalKind::Midi, SignalKind::Control)
                | (SignalKind::Control, SignalKind::Trigger)
                | (SignalKind::Trigger, SignalKind::Control)
                | (SignalKind::Midi, SignalKind::Trigger)
                | (SignalKind::Trigger, SignalKind::Midi)
                | (SignalKind::Sensor, SignalKind::Control)
                | (SignalKind::Sensor, SignalKind::Midi)
                | (SignalKind::Sensor, SignalKind::Trigger)
                | (SignalKind::ByteStream, SignalKind::Control)
                | (SignalKind::Control, SignalKind::ByteStream)
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PixelFormat {
    Rgba8,
    Bgra8,
    Uyvy,
    Nv12,
    Depth16,
    Gray8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameRef {
    pub frame_id: u64,
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
    pub source_id: Uuid,
    pub timestamp_ns: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum SignalValue {
    Float(f64),
    Int(i64),
    Bool(bool),
    String(Arc<str>),
    Trigger,
    Vec2([f64; 2]),
    Vec3([f64; 3]),
    Bytes(Arc<[u8]>),
    FrameRef(FrameRef),
}

impl SignalValue {
    pub fn string(s: impl Into<String>) -> Self {
        Self::String(Arc::from(s.into().into_boxed_str()))
    }

    pub fn bytes(b: impl Into<Vec<u8>>) -> Self {
        Self::Bytes(Arc::from(b.into().into_boxed_slice()))
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            SignalValue::Float(v) => Some(*v),
            SignalValue::Int(v) => Some(*v as f64),
            SignalValue::Bool(v) => Some(if *v { 1.0 } else { 0.0 }),
            SignalValue::Trigger => Some(1.0),
            SignalValue::Vec2([x, _]) => Some(*x),
            SignalValue::Vec3([x, _, _]) => Some(*x),
            SignalValue::String(s) => s.parse::<f64>().ok(),
            SignalValue::Bytes(b) => b.first().map(|v| *v as f64 / 255.0),
            SignalValue::FrameRef(_) => None,
        }
    }

    pub fn as_bool(&self) -> bool {
        match self {
            SignalValue::Float(v) => *v >= 0.5,
            SignalValue::Int(v) => *v != 0,
            SignalValue::Bool(v) => *v,
            SignalValue::Trigger => true,
            SignalValue::Vec2([x, y]) => x.abs() > 0.0 || y.abs() > 0.0,
            SignalValue::Vec3([x, y, z]) => x.abs() > 0.0 || y.abs() > 0.0 || z.abs() > 0.0,
            SignalValue::String(s) => !s.is_empty() && s.as_ref() != "0" && s.as_ref() != "false",
            SignalValue::Bytes(b) => !b.is_empty(),
            SignalValue::FrameRef(_) => true,
        }
    }

    pub fn summary_display(&self) -> String {
        match self {
            SignalValue::Float(v) => format!("{v:.3}"),
            SignalValue::Int(v) => format!("{v}"),
            SignalValue::Bool(v) => format!("{v}"),
            SignalValue::String(s) => format!("\"{s}\""),
            SignalValue::Trigger => "TRIG".to_string(),
            SignalValue::Vec2([x, y]) => format!("[{x:.2}, {y:.2}]"),
            SignalValue::Vec3([x, y, z]) => format!("[{x:.2}, {y:.2}, {z:.2}]"),
            SignalValue::Bytes(b) => format!("{}B", b.len()),
            SignalValue::FrameRef(f) => format!("{}x{} #{:?}", f.width, f.height, f.format),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignalEvent {
    pub source_port_id: Uuid,
    pub value: SignalValue,
    pub timestamp_ns: u64,
    pub hop_count: u8,
    pub visited_nodes: Vec<Uuid>,
}

impl SignalEvent {
    pub fn new(source_port_id: Uuid, value: SignalValue, timestamp_ns: u64) -> Self {
        Self {
            source_port_id,
            value,
            timestamp_ns,
            hop_count: 0,
            visited_nodes: Vec::new(),
        }
    }

    pub fn now_ns() -> u64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0)
    }

    pub fn now_ms() -> u64 {
        Self::now_ns() / 1_000_000
    }
}
