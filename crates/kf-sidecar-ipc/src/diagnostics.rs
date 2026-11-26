//! Real-time Diagnostics & Performance Profiler engine tracking protocol message rates,
//! P50/P95/P99 latency percentiles, dropped frames, sidecar uptime/restarts, and SHM health.

use crate::protocol::SidecarHealthStatus;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

const LATENCY_WINDOW: usize = 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProtocolRateEntry {
    pub protocol: String,
    pub msg_per_sec: f64,
    pub total_messages: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LatencyPercentilesMs {
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub mean_ms: f64,
    pub max_ms: f64,
    pub sample_count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SidecarDiagnosticEntry {
    pub sidecar_id: Uuid,
    pub vendor: String,
    pub status: SidecarHealthStatus,
    pub pid: Option<u32>,
    pub uptime_ms: u64,
    pub restart_count: u32,
    pub last_crash_reason: Option<String>,
    pub frames_received: u64,
    pub fps: f32,
    pub ring_buffer_utilization: f32,
    pub ring_buffer_dropped: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiagnosticsSnapshot {
    pub timestamp_ms: u64,
    pub uptime_ms: u64,
    pub total_msg_per_sec: f64,
    pub protocol_rates: Vec<ProtocolRateEntry>,
    pub latency: LatencyPercentilesMs,
    pub dropped_frames: u64,
    pub sidecars: Vec<SidecarDiagnosticEntry>,
}

struct ProtocolCounter {
    total: u64,
    window_count: u64,
    last_rate: f64,
}

pub struct DiagnosticsProfiler {
    started_at: Instant,
    last_rate_roll: Mutex<Instant>,
    protocols: Mutex<BTreeMap<String, ProtocolCounter>>,
    latencies_us: Mutex<VecDeque<u64>>,
    dropped_frames: AtomicU64,
    sidecars: Mutex<BTreeMap<Uuid, SidecarDiagnosticEntry>>,
}

impl Default for DiagnosticsProfiler {
    fn default() -> Self {
        Self::new()
    }
}

impl DiagnosticsProfiler {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            started_at: now,
            last_rate_roll: Mutex::new(now),
            protocols: Mutex::new(BTreeMap::new()),
            latencies_us: Mutex::new(VecDeque::with_capacity(LATENCY_WINDOW)),
            dropped_frames: AtomicU64::new(0),
            sidecars: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn record_protocol_messages(&self, protocol: &str, count: u64) {
        if count == 0 {
            return;
        }
        let mut map = self.protocols.lock();
        map.entry(protocol.to_string())
            .and_modify(|c| {
                c.total = c.total.saturating_add(count);
                c.window_count = c.window_count.saturating_add(count);
            })
            .or_insert(ProtocolCounter {
                total: count,
                window_count: count,
                last_rate: 0.0,
            });
    }

    pub fn record_latency_us(&self, latency_us: u64) {
        let mut ring = self.latencies_us.lock();
        if ring.len() >= LATENCY_WINDOW {
            ring.pop_front();
        }
        ring.push_back(latency_us);
    }

    pub fn record_dropped_frames(&self, count: u64) {
        if count > 0 {
            self.dropped_frames.fetch_add(count, Ordering::Relaxed);
        }
    }

    pub fn update_sidecar(&self, entry: SidecarDiagnosticEntry) {
        self.sidecars.lock().insert(entry.sidecar_id, entry);
    }

    pub fn remove_sidecar(&self, sidecar_id: Uuid) {
        self.sidecars.lock().remove(&sidecar_id);
    }

    pub fn snapshot(&self) -> DiagnosticsSnapshot {
        let now = Instant::now();
        let protocol_rates = {
            let mut last_roll = self.last_rate_roll.lock();
            let elapsed = now.duration_since(*last_roll).as_secs_f64();
            let should_roll = elapsed >= 0.25;
            let mut map = self.protocols.lock();

            let mut out = Vec::with_capacity(map.len());
            for (proto, counter) in map.iter_mut() {
                if should_roll {
                    let instant_rate = counter.window_count as f64 / elapsed.max(0.001);
                    counter.last_rate = if counter.last_rate == 0.0 {
                        instant_rate
                    } else {
                        counter.last_rate * 0.4 + instant_rate * 0.6
                    };
                    counter.window_count = 0;
                }
                out.push(ProtocolRateEntry {
                    protocol: proto.clone(),
                    msg_per_sec: (counter.last_rate * 10.0).round() / 10.0,
                    total_messages: counter.total,
                });
            }
            if should_roll {
                *last_roll = now;
            }
            out
        };

        let total_msg_per_sec: f64 = protocol_rates.iter().map(|p| p.msg_per_sec).sum();
        let latency = compute_percentiles(&self.latencies_us.lock());
        let sidecars: Vec<SidecarDiagnosticEntry> = self.sidecars.lock().values().cloned().collect();
        let sidecar_dropped: u64 = sidecars.iter().map(|s| s.ring_buffer_dropped).sum();
        let dropped_frames = self.dropped_frames.load(Ordering::Relaxed) + sidecar_dropped;

        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        DiagnosticsSnapshot {
            timestamp_ms,
            uptime_ms: self.started_at.elapsed().as_millis() as u64,
            total_msg_per_sec: (total_msg_per_sec * 10.0).round() / 10.0,
            protocol_rates,
            latency,
            dropped_frames,
            sidecars,
        }
    }
}

fn compute_percentiles(samples: &VecDeque<u64>) -> LatencyPercentilesMs {
    if samples.is_empty() {
        return LatencyPercentilesMs {
            p50_ms: 0.0,
            p95_ms: 0.0,
            p99_ms: 0.0,
            mean_ms: 0.0,
            max_ms: 0.0,
            sample_count: 0,
        };
    }

    let mut sorted: Vec<u64> = samples.iter().copied().collect();
    sorted.sort_unstable();
    let n = sorted.len();

    let pct = |p: f64| -> f64 {
        let idx = ((p / 100.0) * ((n - 1) as f64)).round() as usize;
        sorted[idx.min(n - 1)] as f64 / 1000.0
    };

    let sum: u64 = sorted.iter().sum();
    let mean_ms = (sum as f64 / n as f64) / 1000.0;
    let max_ms = sorted[n - 1] as f64 / 1000.0;

    LatencyPercentilesMs {
        p50_ms: pct(50.0),
        p95_ms: pct(95.0),
        p99_ms: pct(99.0),
        mean_ms,
        max_ms,
        sample_count: n,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiler_computes_accurate_percentiles_and_rates() {
        let prof = DiagnosticsProfiler::new();
        for us in 100..=1000u64 {
            if us % 10 == 0 {
                prof.record_latency_us(us);
            }
        }
        prof.record_protocol_messages("OSC", 120);
        prof.record_protocol_messages("DEPTH", 30);
        prof.record_dropped_frames(2);

        let snap = prof.snapshot();
        assert_eq!(snap.latency.sample_count, 91);
        assert!((snap.latency.p50_ms - 0.55).abs() < 0.05);
        assert!((snap.latency.p95_ms - 0.95).abs() < 0.06);
        assert_eq!(snap.dropped_frames, 2);
        assert_eq!(snap.protocol_rates.len(), 2);
    }
}
