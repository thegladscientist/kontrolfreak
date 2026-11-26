use crate::signal::{SignalEvent, SignalValue};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TelemetryEntry {
    pub port_id: Uuid,
    pub device_id: Uuid,
    pub latest_value: SignalValue,
    pub numeric_value: Option<f64>,
    pub active: bool,
    pub packets_per_sec: f32,
    pub last_updated_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrafficLogEntry {
    pub timestamp_ms: u64,
    pub protocol: String,
    pub direction: String,
    pub summary: String,
    pub device_id: Option<Uuid>,
    pub port_id: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TelemetryBatch {
    pub timestamp_ms: u64,
    pub values: Vec<TelemetryEntry>,
    pub recent_logs: Vec<TrafficLogEntry>,
    pub total_msg_per_sec: f32,
}

struct PortTelemetrySlot {
    device_id: Uuid,
    latest_value: SignalValue,
    numeric_value: Option<f64>,
    count_since_flush: u64,
    last_updated_ms: u64,
}

/// 30 Hz telemetry batch aggregator keeping the real-time Data Plane decoupled from the UI.
pub struct TelemetryAggregator {
    slots: Mutex<HashMap<Uuid, PortTelemetrySlot>>,
    logs: Mutex<VecDeque<TrafficLogEntry>>,
    total_events_since_flush: AtomicU64,
    last_flush_ms: AtomicU64,
    max_logs: usize,
}

impl Default for TelemetryAggregator {
    fn default() -> Self {
        Self::new(200)
    }
}

impl TelemetryAggregator {
    pub fn new(max_logs: usize) -> Self {
        Self {
            slots: Mutex::new(HashMap::new()),
            logs: Mutex::new(VecDeque::with_capacity(max_logs)),
            total_events_since_flush: AtomicU64::new(0),
            last_flush_ms: AtomicU64::new(SignalEvent::now_ms()),
            max_logs,
        }
    }

    pub fn record_signal(&self, device_id: Uuid, port_id: Uuid, value: &SignalValue) {
        let now_ms = SignalEvent::now_ms();
        self.total_events_since_flush.fetch_add(1, Ordering::Relaxed);
        let numeric_value = value.as_f64();
        let mut slots = self.slots.lock();
        slots
            .entry(port_id)
            .and_modify(|slot| {
                slot.device_id = device_id;
                slot.latest_value = value.clone();
                slot.numeric_value = numeric_value;
                slot.count_since_flush += 1;
                slot.last_updated_ms = now_ms;
            })
            .or_insert_with(|| PortTelemetrySlot {
                device_id,
                latest_value: value.clone(),
                numeric_value,
                count_since_flush: 1,
                last_updated_ms: now_ms,
            });
    }

    pub fn record_log(
        &self,
        protocol: impl Into<String>,
        direction: impl Into<String>,
        summary: impl Into<String>,
        device_id: Option<Uuid>,
        port_id: Option<Uuid>,
    ) {
        let entry = TrafficLogEntry {
            timestamp_ms: SignalEvent::now_ms(),
            protocol: protocol.into(),
            direction: direction.into(),
            summary: summary.into(),
            device_id,
            port_id,
        };
        let mut logs = self.logs.lock();
        if logs.len() >= self.max_logs {
            logs.pop_front();
        }
        logs.push_back(entry);
    }

    /// Flushes a subscription-scoped batch suitable for pushing at 20–30 Hz over Tauri IPC.
    pub fn flush_batch(&self, subscribed_ports: Option<&HashSet<Uuid>>) -> TelemetryBatch {
        let now_ms = SignalEvent::now_ms();
        let prev_ms = self.last_flush_ms.swap(now_ms, Ordering::Relaxed);
        let elapsed_secs = ((now_ms.saturating_sub(prev_ms)).max(1)) as f32 / 1000.0;
        let total_events = self.total_events_since_flush.swap(0, Ordering::Relaxed);
        let total_msg_per_sec = total_events as f32 / elapsed_secs;

        let mut values = Vec::new();
        {
            let mut slots = self.slots.lock();
            for (&port_id, slot) in slots.iter_mut() {
                if let Some(filter) = subscribed_ports {
                    if !filter.contains(&port_id) {
                        slot.count_since_flush = 0;
                        continue;
                    }
                }
                let count = slot.count_since_flush;
                slot.count_since_flush = 0;
                let active = count > 0 || now_ms.saturating_sub(slot.last_updated_ms) < 180;
                values.push(TelemetryEntry {
                    port_id,
                    device_id: slot.device_id,
                    latest_value: slot.latest_value.clone(),
                    numeric_value: slot.numeric_value,
                    active,
                    packets_per_sec: count as f32 / elapsed_secs,
                    last_updated_ms: slot.last_updated_ms,
                });
            }
        }

        let recent_logs = {
            let mut logs = self.logs.lock();
            logs.drain(..).collect()
        };

        TelemetryBatch {
            timestamp_ms: now_ms,
            values,
            recent_logs,
            total_msg_per_sec,
        }
    }
}
