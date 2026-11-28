//! OSC source scout for the simple VJ screen.
//!
//! Raw OSC has no device directory, so discovery is two paths at once:
//! a passive UDP listener (default port 9000) that records sender IPs and the
//! address prefixes they have used, and mDNS-SD browse for `_osc._udp` and
//! `_oscjson._tcp` (OSCQuery). If multicast is unavailable the mDNS path
//! records a fallback reason and the UDP listener keeps working.

use crate::osc::{decode_osc_bytes, osc_args_to_signal};
use kf_core::SignalEvent;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::sync::watch;

pub const OSC_UDP_SERVICE: &str = "_osc._udp.local.";
pub const OSCQUERY_TCP_SERVICE: &str = "_oscjson._tcp.local.";
pub const DEFAULT_OSC_PORT: u16 = 9000;
/// How long a silent sender stays in the list. OSC devices often send only
/// while a finger is down, so this is long enough to stay selectable.
pub const UDP_REMEMBER_MS: u64 = 10 * 60 * 1000;
pub const UDP_LIVE_MS: u64 = 3_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OscDiscoveredSource {
    pub id: String,
    pub origin: String,
    pub label: String,
    pub host: String,
    pub port: Option<u16>,
    pub recent_prefixes: Vec<String>,
    pub last_address: String,
    pub last_value: String,
    pub last_norm: Option<f64>,
    pub last_seen_ms: u64,
    pub live: bool,
    pub service_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OscListenStatus {
    pub requested_port: u16,
    pub bound_address: Option<String>,
    pub listening: bool,
    pub mdns_active: bool,
    pub mdns_fallback: Option<String>,
    pub error: Option<String>,
}

impl Default for OscListenStatus {
    fn default() -> Self {
        Self {
            requested_port: DEFAULT_OSC_PORT,
            bound_address: None,
            listening: false,
            mdns_active: false,
            mdns_fallback: None,
            error: None,
        }
    }
}

#[derive(Debug, Clone)]
struct UdpSender {
    host: String,
    port: u16,
    prefixes: Vec<String>,
    last_address: String,
    last_value: String,
    last_norm: Option<f64>,
    last_seen_ms: u64,
}

#[derive(Debug, Clone)]
struct MdnsEntry {
    fullname: String,
    name: String,
    host: String,
    port: u16,
    service_type: String,
}

/// Shared catalog + listener control. The UDP task holds an `Arc` of this;
/// shutting it down does not require dropping the catalog.
pub struct OscScout {
    requested: AtomicU16,
    stop: Arc<AtomicBool>,
    wake: watch::Sender<u64>,
    generation: AtomicU64,
    status: Mutex<OscListenStatus>,
    udp: Mutex<HashMap<String, UdpSender>>,
    mdns: Arc<Mutex<HashMap<String, MdnsEntry>>>,
    mdns_started: AtomicBool,
    daemon: Mutex<Option<mdns_sd::ServiceDaemon>>,
    workers: Mutex<Vec<JoinHandle<()>>>,
}

impl OscScout {
    pub fn new() -> Arc<Self> {
        let (wake, _rx) = watch::channel(0u64);
        Arc::new(Self {
            requested: AtomicU16::new(DEFAULT_OSC_PORT),
            stop: Arc::new(AtomicBool::new(false)),
            wake,
            generation: AtomicU64::new(0),
            status: Mutex::new(OscListenStatus::default()),
            udp: Mutex::new(HashMap::new()),
            mdns: Arc::new(Mutex::new(HashMap::new())),
            mdns_started: AtomicBool::new(false),
            daemon: Mutex::new(None),
            workers: Mutex::new(Vec::new()),
        })
    }

    pub fn status(&self) -> OscListenStatus {
        self.status.lock().clone()
    }

    pub fn set_listen_port(&self, port: u16) {
        self.requested.store(port, Ordering::SeqCst);
        let gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        {
            let mut status = self.status.lock();
            status.requested_port = port;
            status.error = None;
            status.listening = false;
        }
        let _ = self.wake.send(gen);
    }

    /// Records one already-decoded OSC message. Safe to call from tests.
    pub fn note_message(&self, peer: SocketAddr, address: &str, value: &str, norm: Option<f64>) {
        let host = peer.ip().to_string();
        let prefix = osc_address_prefix(address);
        let now = SignalEvent::now_ms();
        let mut map = self.udp.lock();
        let entry = map.entry(host.clone()).or_insert_with(|| UdpSender {
            host,
            port: peer.port(),
            prefixes: Vec::new(),
            last_address: String::new(),
            last_value: String::new(),
            last_norm: None,
            last_seen_ms: now,
        });
        entry.port = peer.port();
        entry.last_seen_ms = now;
        entry.last_address = address.to_string();
        entry.last_value = value.to_string();
        entry.last_norm = norm;
        if !entry.prefixes.iter().any(|p| p == &prefix) {
            entry.prefixes.insert(0, prefix);
        } else if entry.prefixes.first().map(String::as_str) != Some(prefix.as_str()) {
            entry.prefixes.retain(|p| p != &prefix);
            entry.prefixes.insert(0, prefix);
        }
        entry.prefixes.truncate(6);
    }

    pub fn ingest_bytes(&self, peer: SocketAddr, buf: &[u8]) {
        let Ok(messages) = decode_osc_bytes(buf) else {
            return;
        };
        for msg in messages {
            let value = osc_args_to_signal(&msg.args);
            self.note_message(peer, &msg.addr, &value.summary_display(), value.as_f64());
        }
    }

    pub fn snapshot(&self) -> Vec<OscDiscoveredSource> {
        let now = SignalEvent::now_ms();
        let udp_rows = {
            let mut guard = self.udp.lock();
            guard.retain(|_, row| now.saturating_sub(row.last_seen_ms) <= UDP_REMEMBER_MS);
            guard.values().cloned().collect::<Vec<_>>()
        };
        let mdns_rows: Vec<MdnsEntry> = self.mdns.lock().values().cloned().collect();
        let mdns_hosts: HashSet<String> = mdns_rows.iter().map(|row| row.host.clone()).collect();
        let mut out = Vec::new();
        for row in &mdns_rows {
            let matched = udp_rows.iter().find(|udp| udp.host == row.host);
            out.push(OscDiscoveredSource {
                id: format!("osc:mdns:{}", row.fullname),
                origin: "mdns".into(),
                label: row.name.clone(),
                host: row.host.clone(),
                port: Some(row.port),
                recent_prefixes: matched
                    .map(|udp| udp.prefixes.clone())
                    .unwrap_or_default(),
                last_address: matched
                    .map(|udp| udp.last_address.clone())
                    .unwrap_or_default(),
                last_value: matched
                    .map(|udp| udp.last_value.clone())
                    .unwrap_or_default(),
                last_norm: matched.and_then(|udp| udp.last_norm),
                last_seen_ms: matched.map(|udp| udp.last_seen_ms).unwrap_or(0),
                live: matched
                    .map(|udp| now.saturating_sub(udp.last_seen_ms) <= UDP_LIVE_MS)
                    .unwrap_or(false),
                service_type: Some(row.service_type.clone()),
            });
        }
        for row in udp_rows {
            if mdns_hosts.contains(&row.host) {
                continue;
            }
            let live = now.saturating_sub(row.last_seen_ms) <= UDP_LIVE_MS;
            out.push(OscDiscoveredSource {
                id: format!("osc:udp:{}", row.host),
                origin: "udp".into(),
                label: row.host.clone(),
                host: row.host,
                port: Some(row.port),
                recent_prefixes: row.prefixes,
                last_address: row.last_address,
                last_value: row.last_value,
                last_norm: row.last_norm,
                last_seen_ms: row.last_seen_ms,
                live,
                service_type: None,
            });
        }
        out.sort_by(|a, b| a.label.to_lowercase().cmp(&b.label.to_lowercase()));
        out
    }

    /// Binds the UDP socket and browses mDNS until [`shutdown`](Self::shutdown).
    pub async fn serve(self: &Arc<Self>) {
        self.ensure_mdns();
        let mut wake = self.wake.subscribe();
        loop {
            if self.stop.load(Ordering::SeqCst) {
                break;
            }
            let port = self.requested.load(Ordering::SeqCst);
            let socket = match UdpSocket::bind(("0.0.0.0", port)).await {
                Ok(socket) => socket,
                Err(err) => {
                    self.mark_bind_error(port, err.to_string());
                    tokio::select! {
                        _ = wake.changed() => continue,
                        _ = tokio::time::sleep(Duration::from_secs(2)) => continue,
                    }
                }
            };
            let bound = socket.local_addr().ok().map(|addr| addr.to_string());
            self.mark_listening(port, bound);
            let mut buf = vec![0u8; 65536];
            loop {
                if self.stop.load(Ordering::SeqCst) {
                    return;
                }
                tokio::select! {
                    biased;
                    changed = wake.changed() => {
                        if changed.is_err() || self.stop.load(Ordering::SeqCst) {
                            return;
                        }
                        break;
                    }
                    rec = socket.recv_from(&mut buf) => {
                        match rec {
                            Ok((len, peer)) => self.ingest_bytes(peer, &buf[..len]),
                            Err(err) => {
                                self.mark_bind_error(port, err.to_string());
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::SeqCst);
        let gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = self.wake.send(gen);
        if let Some(daemon) = self.daemon.lock().take() {
            let _ = daemon.shutdown();
        }
        let workers: Vec<_> = std::mem::take(&mut *self.workers.lock());
        for worker in workers {
            let _ = worker.join();
        }
    }

    fn mark_listening(&self, port: u16, bound: Option<String>) {
        let mut status = self.status.lock();
        status.requested_port = port;
        status.bound_address = bound;
        status.listening = true;
        status.error = None;
    }

    fn mark_bind_error(&self, port: u16, error: String) {
        tracing::warn!("OSC listen on port {port} failed: {error}");
        let mut status = self.status.lock();
        status.requested_port = port;
        status.listening = false;
        status.bound_address = None;
        status.error = Some(format!("Port {port} is not available ({error})"));
    }

    fn ensure_mdns(&self) {
        if self.mdns_started.swap(true, Ordering::SeqCst) {
            return;
        }
        let daemon = match mdns_sd::ServiceDaemon::new() {
            Ok(daemon) => daemon,
            Err(err) => {
                tracing::warn!("OSC mDNS-SD unavailable ({err}); UDP listen still active");
                let mut status = self.status.lock();
                status.mdns_active = false;
                status.mdns_fallback = Some(format!("mDNS multicast unavailable: {err}"));
                return;
            }
        };

        let mut active = false;
        let mut fallback: Option<String> = None;
        for service in [OSC_UDP_SERVICE, OSCQUERY_TCP_SERVICE] {
            match daemon.browse(service) {
                Ok(receiver) => {
                    active = true;
                    let catalog = Arc::clone(&self.mdns);
                    let stop = Arc::clone(&self.stop);
                    let service_name = service.to_string();
                    let worker = std::thread::spawn(move || {
                        browse_loop(receiver, service_name, catalog, stop);
                    });
                    self.workers.lock().push(worker);
                }
                Err(err) => {
                    tracing::warn!("OSC mDNS browse for {service} failed: {err}");
                    fallback = Some(format!("mDNS browse unavailable: {err}"));
                }
            }
        }

        {
            let mut status = self.status.lock();
            status.mdns_active = active;
            if !active {
                status.mdns_fallback = fallback.or_else(|| {
                    Some("mDNS browse did not start; still listening for OSC packets".into())
                });
            } else if let Some(reason) = fallback {
                status.mdns_fallback = Some(reason);
            }
        }
        *self.daemon.lock() = Some(daemon);
    }
}

fn browse_loop(
    receiver: mdns_sd::Receiver<mdns_sd::ServiceEvent>,
    service: String,
    catalog: Arc<Mutex<HashMap<String, MdnsEntry>>>,
    stop: Arc<AtomicBool>,
) {
    while !stop.load(Ordering::Relaxed) {
        match receiver.recv_timeout(Duration::from_millis(250)) {
            Ok(mdns_sd::ServiceEvent::ServiceResolved(info)) => {
                let fullname = info.get_fullname().to_string();
                let name = instance_name(&fullname, &service);
                let port = info.get_port();
                let host = info
                    .get_addresses_v4()
                    .into_iter()
                    .next()
                    .map(|ip| ip.to_string())
                    .or_else(|| info.get_addresses().iter().next().map(|ip| ip.to_string()))
                    .unwrap_or_else(|| info.get_hostname().trim_end_matches('.').to_string());
                if host.is_empty() {
                    continue;
                }
                catalog.lock().insert(
                    fullname.clone(),
                    MdnsEntry {
                        fullname,
                        name,
                        host,
                        port,
                        service_type: service.clone(),
                    },
                );
            }
            Ok(mdns_sd::ServiceEvent::ServiceRemoved(_, fullname)) => {
                catalog.lock().remove(&fullname);
            }
            Ok(_) => {}
            Err(err) => {
                if err.to_string().contains("disconnected") || stop.load(Ordering::Relaxed) {
                    break;
                }
            }
        }
    }
}

pub fn osc_address_prefix(address: &str) -> String {
    let parts: Vec<&str> = address.split('/').filter(|part| !part.is_empty()).collect();
    match parts.as_slice() {
        [] => "/".to_string(),
        [only] => format!("/{only}"),
        [first, second, ..] => format!("/{first}/{second}"),
    }
}

fn instance_name(fullname: &str, service: &str) -> String {
    fullname
        .strip_suffix(&format!(".{service}"))
        .unwrap_or(fullname)
        .trim_end_matches('.')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::osc::encode_osc_message;
    use kf_core::SignalValue;
    use std::time::Duration;

    #[test]
    fn address_prefix_keeps_two_segments() {
        assert_eq!(osc_address_prefix("/composition/layers/1/opacity"), "/composition/layers");
        assert_eq!(osc_address_prefix("/fader/1"), "/fader/1");
        assert_eq!(osc_address_prefix("/go"), "/go");
        assert_eq!(osc_address_prefix("/"), "/");
    }

    #[test]
    fn catalog_groups_one_row_per_sender_ip() {
        let scout = OscScout::new();
        let peer: SocketAddr = "10.0.0.8:50000".parse().unwrap();
        scout.note_message(peer, "/composition/layers/1/opacity", "0.200", Some(0.2));
        scout.note_message(peer, "/fader/1", "0.900", Some(0.9));
        let rows = scout.snapshot();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].host, "10.0.0.8");
        assert_eq!(rows[0].origin, "udp");
        assert!(rows[0].recent_prefixes.iter().any(|p| p == "/composition/layers"));
        assert!(rows[0].recent_prefixes.iter().any(|p| p == "/fader/1"));
        assert_eq!(rows[0].last_address, "/fader/1");
        assert!(rows[0].live);
    }

    #[test]
    fn mdns_start_falls_back_without_panicking() {
        let scout = OscScout::new();
        scout.ensure_mdns();
        let status = scout.status();
        assert!(status.mdns_active || status.mdns_fallback.is_some());
        scout.shutdown();
    }

    #[tokio::test]
    async fn passive_listener_lists_sender_after_packets() {
        let scout = OscScout::new();
        scout.set_listen_port(0);
        let running = Arc::clone(&scout);
        let task = tokio::spawn(async move { running.serve().await });

        let bound = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Some(addr) = scout.status().bound_address {
                    if scout.status().listening {
                        return addr;
                    }
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("OSC listener bind");

        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let first = encode_osc_message(
            "/composition/layers/1/opacity",
            &SignalValue::Float(0.4),
        )
        .unwrap();
        socket.send_to(&first, &bound).await.unwrap();
        let second = encode_osc_message("/fader/1", &SignalValue::Float(0.8)).unwrap();
        socket.send_to(&second, &bound).await.unwrap();

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let rows = scout.snapshot();
                let hit = rows.iter().any(|row| {
                    row.origin == "udp"
                        && row.host == "127.0.0.1"
                        && row.recent_prefixes.iter().any(|p| p == "/composition/layers")
                        && row.recent_prefixes.iter().any(|p| p == "/fader/1")
                });
                if hit {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("sender discovered from UDP");

        scout.shutdown();
        let _ = tokio::time::timeout(Duration::from_secs(2), task).await;
    }
}
