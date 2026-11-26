import type { TelemetryBatch, TelemetryEntry, TrafficLogEntry } from '../types/kontrolfreak';

const MAX_HISTORY_SAMPLES = 120;
const MAX_TRAFFIC_LOGS = 250;

type TelemetryListener = (batch: TelemetryBatch) => void;

/**
 * Non-reactive high-frequency telemetry store keeping 30 Hz signal samples & logs completely
 * outside React / React Flow state so live meters and `<canvas>` scopes run at zero jank.
 */
class TelemetryStore {
  private latestByPort = new Map<string, TelemetryEntry>();
  private historyByPort = new Map<string, number[]>();
  private logs: TrafficLogEntry[] = [];
  private totalMsgPerSec = 0;
  private listeners = new Set<TelemetryListener>();
  private logPaused = false;

  ingestBatch(batch: TelemetryBatch): void {
    this.totalMsgPerSec = batch.total_msg_per_sec;
    for (const entry of batch.values) {
      this.latestByPort.set(entry.port_id, entry);
      if (typeof entry.numeric_value === 'number' && Number.isFinite(entry.numeric_value)) {
        let ring = this.historyByPort.get(entry.port_id);
        if (!ring) {
          ring = [];
          this.historyByPort.set(entry.port_id, ring);
        }
        ring.push(entry.numeric_value);
        if (ring.length > MAX_HISTORY_SAMPLES) {
          ring.shift();
        }
      }
    }

    if (!this.logPaused && batch.recent_logs.length > 0) {
      for (const l of batch.recent_logs) {
        this.logs.push(l);
      }
      if (this.logs.length > MAX_TRAFFIC_LOGS) {
        this.logs.splice(0, this.logs.length - MAX_TRAFFIC_LOGS);
      }
    }

    for (const fn of this.listeners) {
      fn(batch);
    }
  }

  subscribe(fn: TelemetryListener): () => void {
    this.listeners.add(fn);
    return () => {
      this.listeners.delete(fn);
    };
  }

  getPortLatest(portId: string): TelemetryEntry | undefined {
    return this.latestByPort.get(portId);
  }

  getPortHistory(portId: string): number[] {
    return this.historyByPort.get(portId) ?? [];
  }

  getLogs(): TrafficLogEntry[] {
    return this.logs;
  }

  clearLogs(): void {
    this.logs = [];
  }

  setLogPaused(paused: boolean): void {
    this.logPaused = paused;
  }

  isLogPaused(): boolean {
    return this.logPaused;
  }

  getTotalMsgPerSec(): number {
    return this.totalMsgPerSec;
  }
}

export const telemetryStore = new TelemetryStore();
