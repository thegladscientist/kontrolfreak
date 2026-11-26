//! Pure-Rust mDNS-SD discovery and announcement for `_apple-midi._udp.local.`.
//! Gracefully falls back when multicast sockets are unavailable in headless/sandboxed environments.

use super::RtpMidiPeer;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

pub const APPLE_MIDI_MDNS_SERVICE: &str = "_apple-midi._udp.local.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MdnsStatus {
    pub active: bool,
    pub announcing: bool,
    pub service_type: String,
    pub fallback_reason: Option<String>,
}

/// Manages mDNS-SD advertising and peer browsing for an RTPMIDI session.
pub struct MdnsDiscoveryHandle {
    status: MdnsStatus,
    peers: Arc<Mutex<HashMap<String, RtpMidiPeer>>>,
    stop_flag: Arc<AtomicBool>,
    daemon: Option<mdns_sd::ServiceDaemon>,
    fullname: Option<String>,
    worker: Option<JoinHandle<()>>,
}

impl MdnsDiscoveryHandle {
    /// Starts mDNS-SD browsing (and optional announcement) for `_apple-midi._udp.local.`.
    /// Never fails hard: if mDNS multicast is unavailable on the host, enters graceful fallback mode.
    pub fn start(
        session_name: &str,
        control_port: u16,
        announce: bool,
        discover: bool,
    ) -> Self {
        let peers = Arc::new(Mutex::new(HashMap::new()));
        let stop_flag = Arc::new(AtomicBool::new(false));

        if !announce && !discover {
            return Self {
                status: MdnsStatus {
                    active: false,
                    announcing: false,
                    service_type: APPLE_MIDI_MDNS_SERVICE.to_string(),
                    fallback_reason: Some("mDNS disabled in session config".into()),
                },
                peers,
                stop_flag,
                daemon: None,
                fullname: None,
                worker: None,
            };
        }

        let daemon = match mdns_sd::ServiceDaemon::new() {
            Ok(d) => d,
            Err(err) => {
                tracing::warn!(
                    "mDNS-SD unavailable ({err}); continuing in direct-UDP fallback mode"
                );
                return Self {
                    status: MdnsStatus {
                        active: false,
                        announcing: false,
                        service_type: APPLE_MIDI_MDNS_SERVICE.to_string(),
                        fallback_reason: Some(format!("mDNS multicast unavailable: {err}")),
                    },
                    peers,
                    stop_flag,
                    daemon: None,
                    fullname: None,
                    worker: None,
                };
            }
        };

        let mut announcing = false;
        let mut fullname = None;
        if announce && control_port > 0 {
            let host_label = format!(
                "{}.local.",
                sanitize_mdns_label(session_name)
            );
            let props: HashMap<String, String> = HashMap::new();
            if let Ok(info) = mdns_sd::ServiceInfo::new(
                APPLE_MIDI_MDNS_SERVICE,
                session_name,
                &host_label,
                "",
                control_port,
                props,
            ) {
                let info = info.enable_addr_auto();
                let fn_name = info.get_fullname().to_string();
                match daemon.register(info) {
                    Ok(()) => {
                        announcing = true;
                        fullname = Some(fn_name);
                    }
                    Err(err) => {
                        tracing::warn!("mDNS-SD service registration failed: {err}");
                    }
                }
            }
        }

        let mut worker = None;
        let mut active = announcing;
        let mut fallback_reason = None;

        if discover {
            match daemon.browse(APPLE_MIDI_MDNS_SERVICE) {
                Ok(receiver) => {
                    active = true;
                    let peers_bg = Arc::clone(&peers);
                    let stop_bg = Arc::clone(&stop_flag);
                    let own_name = session_name.to_string();
                    let own_port = control_port;
                    worker = Some(std::thread::spawn(move || {
                        while !stop_bg.load(Ordering::Relaxed) {
                            match receiver.recv_timeout(std::time::Duration::from_millis(250)) {
                                Ok(mdns_sd::ServiceEvent::ServiceResolved(info)) => {
                                    let instance_name = info
                                        .get_fullname()
                                        .strip_suffix(&format!(".{APPLE_MIDI_MDNS_SERVICE}"))
                                        .unwrap_or(info.get_fullname())
                                        .to_string();
                                    let port = info.get_port();
                                    if instance_name == own_name && port == own_port {
                                        continue;
                                    }
                                    if let Some(ip) = info
                                        .get_addresses_v4()
                                        .into_iter()
                                        .next()
                                        .map(|ip| ip.to_string())
                                        .or_else(|| {
                                            info.get_addresses()
                                                .iter()
                                                .next()
                                                .map(|ip| ip.to_string())
                                        })
                                    {
                                        let peer = RtpMidiPeer {
                                            name: instance_name,
                                            host: ip,
                                            control_port: port,
                                            ssrc: None,
                                        };
                                        peers_bg
                                            .lock()
                                            .insert(info.get_fullname().to_string(), peer);
                                    }
                                }
                                Ok(mdns_sd::ServiceEvent::ServiceRemoved(_, full)) => {
                                    peers_bg.lock().remove(&full);
                                }
                                Ok(_) => {}
                                Err(flume_err) => {
                                    if flume_err.to_string().contains("disconnected") {
                                        break;
                                    }
                                }
                            }
                        }
                    }));
                }
                Err(err) => {
                    tracing::warn!("mDNS-SD browse unavailable ({err}); fallback active");
                    fallback_reason = Some(format!("mDNS browse unavailable: {err}"));
                }
            }
        }

        Self {
            status: MdnsStatus {
                active,
                announcing,
                service_type: APPLE_MIDI_MDNS_SERVICE.to_string(),
                fallback_reason,
            },
            peers,
            stop_flag,
            daemon: Some(daemon),
            fullname,
            worker,
        }
    }

    pub fn status(&self) -> MdnsStatus {
        self.status.clone()
    }

    pub fn discovered_peers(&self) -> Vec<RtpMidiPeer> {
        self.peers.lock().values().cloned().collect()
    }

    /// Manually registers a discovered peer (useful for loopback/fallback environments).
    pub fn insert_manual_peer(&self, peer: RtpMidiPeer) {
        let key = format!("{}:{}:{}", peer.name, peer.host, peer.control_port);
        self.peers.lock().insert(key, peer);
    }
}

impl Drop for MdnsDiscoveryHandle {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        if let Some(daemon) = self.daemon.take() {
            if let Some(fullname) = self.fullname.take() {
                let _ = daemon.unregister(&fullname);
            }
            let _ = daemon.shutdown();
        }
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}

fn sanitize_mdns_label(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('-');
    if trimmed.is_empty() {
        "kontrolfreak".to_string()
    } else {
        trimmed.chars().take(48).collect()
    }
}
