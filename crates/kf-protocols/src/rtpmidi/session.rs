//! Dual-UDP RFC 6295 / AppleMIDI asynchronous session manager.

use super::discovery::{MdnsDiscoveryHandle, MdnsStatus};
use super::journal::{decode_recovery_journal, seq_is_newer, RecoveryJournal};
use super::protocol::{
    apple_midi_clock_10khz, decode_rtp_midi_packet, encode_rtp_midi_packet, AppleMidiPacket,
    RtpMidiDataPacket, RtpMidiEvent, APPLE_MIDI_PROTOCOL_VERSION,
};
use super::{RtpMidiConfig, RtpMidiError, RtpMidiPeer, RtpMidiSessionTrait};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// Callback invoked whenever a MIDI message arrives over RTPMIDI (including multi-message packets
/// and recovered journal messages).
pub type InboundMidiCallback = Arc<dyn Fn(&[u8], u32) + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerConnectionState {
    InvitingControl,
    InvitingData,
    Connected,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectedPeerInfo {
    pub name: String,
    pub ssrc: u32,
    pub control_addr: String,
    pub data_addr: String,
    pub state: PeerConnectionState,
    pub rtt_ms: Option<f64>,
    pub clock_offset_ms: Option<f64>,
    pub sync_count: u64,
    pub last_rx_seq: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RtpMidiSessionStatus {
    pub session_name: String,
    pub local_ssrc: u32,
    pub control_port: u16,
    pub data_port: u16,
    pub running: bool,
    pub mdns: MdnsStatus,
    pub connected_peers: Vec<ConnectedPeerInfo>,
    pub discovered_peers: Vec<RtpMidiPeer>,
    pub tx_packets: u64,
    pub rx_packets: u64,
    pub recovered_messages: u64,
}

#[derive(Debug, Clone)]
struct PeerRecord {
    name: String,
    ssrc: u32,
    initiator_token: u32,
    control_addr: SocketAddr,
    data_addr: SocketAddr,
    state: PeerConnectionState,
    rtt_ms: Option<f64>,
    clock_offset_ms: Option<f64>,
    sync_count: u64,
    last_rx_seq: Option<u16>,
    last_activity: Instant,
}

impl PeerRecord {
    fn to_info(&self) -> ConnectedPeerInfo {
        ConnectedPeerInfo {
            name: self.name.clone(),
            ssrc: self.ssrc,
            control_addr: self.control_addr.to_string(),
            data_addr: self.data_addr.to_string(),
            state: self.state,
            rtt_ms: self.rtt_ms,
            clock_offset_ms: self.clock_offset_ms,
            sync_count: self.sync_count,
            last_rx_seq: self.last_rx_seq,
        }
    }
}

struct SessionInner {
    config: RtpMidiConfig,
    local_ssrc: u32,
    control_socket: Arc<UdpSocket>,
    data_socket: Arc<UdpSocket>,
    control_addr: SocketAddr,
    data_addr: SocketAddr,
    peers_by_token: Mutex<HashMap<u32, PeerRecord>>,
    journal: Mutex<RecoveryJournal>,
    seq_counter: AtomicU16,
    tx_packets: AtomicU64,
    rx_packets: AtomicU64,
    recovered_messages: AtomicU64,
    inbound_cb: Mutex<Option<InboundMidiCallback>>,
    mdns: Mutex<Option<MdnsDiscoveryHandle>>,
    outbound_tx: mpsc::UnboundedSender<Vec<RtpMidiEvent>>,
}

/// Live RFC 6295 / AppleMIDI dual-UDP session (Control port `N`, Data port `N+1`).
pub struct RtpMidiSession {
    inner: Arc<SessionInner>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
}

impl RtpMidiSession {
    /// Starts a dual-UDP RTPMIDI session bound to `bind_ip` on `(control_port, control_port + 1)`.
    /// If `config.control_port == 0`, automatically acquires an adjacent ephemeral `(N, N+1)` pair.
    pub async fn start(
        mut config: RtpMidiConfig,
        bind_ip: IpAddr,
        inbound_cb: Option<InboundMidiCallback>,
    ) -> Result<Arc<Self>, RtpMidiError> {
        let (control_socket, data_socket) = bind_dual_udp_pair(bind_ip, config.control_port).await?;
        let control_addr = control_socket
            .local_addr()
            .map_err(|e| RtpMidiError::Network(e.to_string()))?;
        let data_addr = data_socket
            .local_addr()
            .map_err(|e| RtpMidiError::Network(e.to_string()))?;
        config.control_port = control_addr.port();

        let local_ssrc: u32 = rand::random::<u32>().max(1);
        let initial_seq: u16 = rand::random();

        let mdns_handle = MdnsDiscoveryHandle::start(
            &config.session_name,
            control_addr.port(),
            config.mdns_discover,
            config.mdns_discover,
        );

        let (outbound_tx, mut outbound_rx) = mpsc::unbounded_channel::<Vec<RtpMidiEvent>>();

        let inner = Arc::new(SessionInner {
            config: config.clone(),
            local_ssrc,
            control_socket: Arc::new(control_socket),
            data_socket: Arc::new(data_socket),
            control_addr,
            data_addr,
            peers_by_token: Mutex::new(HashMap::new()),
            journal: Mutex::new(RecoveryJournal::new(initial_seq)),
            seq_counter: AtomicU16::new(initial_seq),
            tx_packets: AtomicU64::new(0),
            rx_packets: AtomicU64::new(0),
            recovered_messages: AtomicU64::new(0),
            inbound_cb: Mutex::new(inbound_cb),
            mdns: Mutex::new(Some(mdns_handle)),
            outbound_tx,
        });

        // Task 1: Control port receiver (IN, OK, NO, BY, RS)
        let inner_ctrl = Arc::clone(&inner);
        let ctrl_task = tokio::spawn(async move {
            let mut buf = vec![0u8; 4096];
            loop {
                let (len, src) = match inner_ctrl.control_socket.recv_from(&mut buf).await {
                    Ok(pair) => pair,
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::ConnectionReset
                                | std::io::ErrorKind::WouldBlock
                                | std::io::ErrorKind::Interrupted
                        ) =>
                    {
                        continue;
                    }
                    Err(_) => break,
                };
                if let Some(pkt) = AppleMidiPacket::decode(&buf[..len]) {
                    handle_control_packet(&inner_ctrl, pkt, src).await;
                }
            }
        });

        // Task 2: Data port receiver (IN, OK, NO, BY, CK, RTP-MIDI packets)
        let inner_data = Arc::clone(&inner);
        let data_task = tokio::spawn(async move {
            let mut buf = vec![0u8; 8192];
            loop {
                let (len, src) = match inner_data.data_socket.recv_from(&mut buf).await {
                    Ok(pair) => pair,
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::ConnectionReset
                                | std::io::ErrorKind::WouldBlock
                                | std::io::ErrorKind::Interrupted
                        ) =>
                    {
                        continue;
                    }
                    Err(_) => break,
                };
                if AppleMidiPacket::is_apple_midi(&buf[..len]) {
                    if let Some(pkt) = AppleMidiPacket::decode(&buf[..len]) {
                        handle_data_apple_packet(&inner_data, pkt, src).await;
                    }
                } else if let Some(rtp) = decode_rtp_midi_packet(&buf[..len]) {
                    handle_rtp_data_packet(&inner_data, rtp, src).await;
                }
            }
        });

        // Task 3: Outbound MIDI sender (enables sync `send_midi_payload` trait calls!)
        let inner_tx = Arc::clone(&inner);
        let tx_task = tokio::spawn(async move {
            while let Some(events) = outbound_rx.recv().await {
                let _ = transmit_midi_events(&inner_tx, events).await;
            }
        });

        // Task 4: Periodic clock sync (CK) & receiver feedback (RS) keepalive
        let inner_sync = Arc::clone(&inner);
        let sync_task = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(10));
            ticker.tick().await; // skip immediate tick
            loop {
                ticker.tick().await;
                let connected: Vec<(SocketAddr, SocketAddr, Option<u16>)> = {
                    let peers = inner_sync.peers_by_token.lock();
                    peers
                        .values()
                        .filter(|p| p.state == PeerConnectionState::Connected)
                        .map(|p| (p.control_addr, p.data_addr, p.last_rx_seq))
                        .collect()
                };
                for (ctrl_addr, data_addr, last_rx) in connected {
                    let ck = AppleMidiPacket::ClockSync {
                        ssrc: inner_sync.local_ssrc,
                        count: 0,
                        timestamp1: apple_midi_clock_10khz(),
                        timestamp2: 0,
                        timestamp3: 0,
                    };
                    let _ = inner_sync.data_socket.send_to(&ck.encode(), data_addr).await;

                    if let Some(seq) = last_rx {
                        let rs = AppleMidiPacket::ReceiverFeedback {
                            ssrc: inner_sync.local_ssrc,
                            sequence_number: seq,
                        };
                        let _ = inner_sync.control_socket.send_to(&rs.encode(), ctrl_addr).await;
                    }
                }
            }
        });

        let session = Arc::new(Self {
            inner,
            tasks: Mutex::new(vec![ctrl_task, data_task, tx_task, sync_task]),
        });

        // If a remote_host ("host:port") is preconfigured, initiate handshake immediately.
        if let Some(ref remote) = config.remote_host {
            if let Ok(addr) = remote.parse::<SocketAddr>() {
                let _ = session.connect_peer(addr).await;
            }
        }

        Ok(session)
    }

    pub fn control_addr(&self) -> SocketAddr {
        self.inner.control_addr
    }

    pub fn data_addr(&self) -> SocketAddr {
        self.inner.data_addr
    }

    pub fn local_ssrc(&self) -> u32 {
        self.inner.local_ssrc
    }

    pub fn set_inbound_callback(&self, cb: InboundMidiCallback) {
        *self.inner.inbound_cb.lock() = Some(cb);
    }

    /// Initiates a 2-port AppleMIDI invitation (`IN` on control port `N`, then `IN` on data port `N+1`).
    pub async fn connect_peer(&self, peer_control_addr: SocketAddr) -> Result<u32, RtpMidiError> {
        let token: u32 = rand::random::<u32>().max(1);
        let peer_data_addr = SocketAddr::new(
            peer_control_addr.ip(),
            peer_control_addr.port().saturating_add(1),
        );

        {
            let mut peers = self.inner.peers_by_token.lock();
            peers.insert(
                token,
                PeerRecord {
                    name: peer_control_addr.to_string(),
                    ssrc: 0,
                    initiator_token: token,
                    control_addr: peer_control_addr,
                    data_addr: peer_data_addr,
                    state: PeerConnectionState::InvitingControl,
                    rtt_ms: None,
                    clock_offset_ms: None,
                    sync_count: 0,
                    last_rx_seq: None,
                    last_activity: Instant::now(),
                },
            );
        }

        let inv = AppleMidiPacket::Invitation {
            protocol_version: APPLE_MIDI_PROTOCOL_VERSION,
            initiator_token: token,
            ssrc: self.inner.local_ssrc,
            name: self.inner.config.session_name.clone(),
        };
        self.inner
            .control_socket
            .send_to(&inv.encode(), peer_control_addr)
            .await
            .map_err(|e| RtpMidiError::Network(e.to_string()))?;

        Ok(token)
    }

    /// Triggers an immediate 3-way `CK` clock synchronization round with all connected peers.
    pub async fn trigger_clock_sync(&self) -> Result<usize, RtpMidiError> {
        let targets: Vec<SocketAddr> = {
            let peers = self.inner.peers_by_token.lock();
            peers
                .values()
                .filter(|p| p.state == PeerConnectionState::Connected)
                .map(|p| p.data_addr)
                .collect()
        };
        let now = apple_midi_clock_10khz();
        let ck = AppleMidiPacket::ClockSync {
            ssrc: self.inner.local_ssrc,
            count: 0,
            timestamp1: now,
            timestamp2: 0,
            timestamp3: 0,
        };
        let bytes = ck.encode();
        for addr in &targets {
            self.inner
                .data_socket
                .send_to(&bytes, addr)
                .await
                .map_err(|e| RtpMidiError::Network(e.to_string()))?;
        }
        Ok(targets.len())
    }

    /// Sends one or more MIDI events (with RFC 6295 delta times) immediately over the Data port.
    pub async fn send_midi_batch(&self, events: Vec<RtpMidiEvent>) -> Result<u16, RtpMidiError> {
        transmit_midi_events(&self.inner, events).await
    }

    /// Gracefully sends `BY` to all peers and stops all background UDP & mDNS tasks.
    pub async fn shutdown(&self) {
        let peers: Vec<(u32, SocketAddr, SocketAddr)> = {
            let mut guard = self.inner.peers_by_token.lock();
            guard
                .drain()
                .map(|(_, p)| (p.initiator_token, p.control_addr, p.data_addr))
                .collect()
        };
        for (token, ctrl_addr, data_addr) in peers {
            let bye = AppleMidiPacket::EndSession {
                protocol_version: APPLE_MIDI_PROTOCOL_VERSION,
                initiator_token: token,
                ssrc: self.inner.local_ssrc,
            }
            .encode();
            let _ = self.inner.control_socket.send_to(&bye, ctrl_addr).await;
            let _ = self.inner.data_socket.send_to(&bye, data_addr).await;
        }
        self.stop_tasks();
    }

    pub fn stop_tasks(&self) {
        for handle in self.tasks.lock().drain(..) {
            handle.abort();
        }
        self.inner.mdns.lock().take();
    }

    pub fn connected_peers(&self) -> Vec<ConnectedPeerInfo> {
        self.inner
            .peers_by_token
            .lock()
            .values()
            .map(PeerRecord::to_info)
            .collect()
    }

    pub fn status(&self) -> RtpMidiSessionStatus {
        let mdns = self
            .inner
            .mdns
            .lock()
            .as_ref()
            .map(MdnsDiscoveryHandle::status)
            .unwrap_or(MdnsStatus {
                active: false,
                announcing: false,
                service_type: super::APPLE_MIDI_MDNS_SERVICE.to_string(),
                fallback_reason: Some("Session stopped".into()),
            });
        let discovered_peers = self.discovered_peers();
        RtpMidiSessionStatus {
            session_name: self.inner.config.session_name.clone(),
            local_ssrc: self.inner.local_ssrc,
            control_port: self.inner.control_addr.port(),
            data_port: self.inner.data_addr.port(),
            running: !self.tasks.lock().is_empty(),
            mdns,
            connected_peers: self.connected_peers(),
            discovered_peers,
            tx_packets: self.inner.tx_packets.load(Ordering::Relaxed),
            rx_packets: self.inner.rx_packets.load(Ordering::Relaxed),
            recovered_messages: self.inner.recovered_messages.load(Ordering::Relaxed),
        }
    }
}

impl Drop for RtpMidiSession {
    fn drop(&mut self) {
        self.stop_tasks();
    }
}

impl RtpMidiSessionTrait for RtpMidiSession {
    fn config(&self) -> &RtpMidiConfig {
        &self.inner.config
    }

    fn discovered_peers(&self) -> Vec<RtpMidiPeer> {
        self.inner
            .mdns
            .lock()
            .as_ref()
            .map(MdnsDiscoveryHandle::discovered_peers)
            .unwrap_or_default()
    }

    fn send_midi_payload(&self, midi_bytes: &[u8]) -> Result<(), RtpMidiError> {
        if midi_bytes.is_empty() {
            return Ok(());
        }
        self.inner
            .outbound_tx
            .send(vec![RtpMidiEvent {
                delta_time: 0,
                midi_bytes: midi_bytes.to_vec(),
            }])
            .map_err(|e| RtpMidiError::Network(format!("RTPMIDI outbound channel closed: {e}")))
    }
}

async fn bind_dual_udp_pair(
    ip: IpAddr,
    requested_control_port: u16,
) -> Result<(UdpSocket, UdpSocket), RtpMidiError> {
    if requested_control_port != 0 {
        let data_port = requested_control_port
            .checked_add(1)
            .ok_or_else(|| RtpMidiError::Network("control_port 65535 has no N+1 data port".into()))?;
        let ctrl = UdpSocket::bind(SocketAddr::new(ip, requested_control_port))
            .await
            .map_err(|e| {
                RtpMidiError::Network(format!(
                    "failed binding RTPMIDI control port {requested_control_port}: {e}"
                ))
            })?;
        let data = UdpSocket::bind(SocketAddr::new(ip, data_port))
            .await
            .map_err(|e| {
                RtpMidiError::Network(format!(
                    "failed binding RTPMIDI data port {data_port}: {e}"
                ))
            })?;
        return Ok((ctrl, data));
    }

    // Ephemeral adjacent pair (N, N+1) for tests & dynamic allocation.
    for _ in 0..32 {
        let ctrl = UdpSocket::bind(SocketAddr::new(ip, 0))
            .await
            .map_err(|e| RtpMidiError::Network(e.to_string()))?;
        let ctrl_port = ctrl
            .local_addr()
            .map_err(|e| RtpMidiError::Network(e.to_string()))?
            .port();
        if let Some(data_port) = ctrl_port.checked_add(1) {
            if let Ok(data) = UdpSocket::bind(SocketAddr::new(ip, data_port)).await {
                return Ok((ctrl, data));
            }
        }
    }
    Err(RtpMidiError::Network(
        "could not allocate an adjacent UDP (N, N+1) port pair".into(),
    ))
}

async fn handle_control_packet(inner: &SessionInner, pkt: AppleMidiPacket, src: SocketAddr) {
    match pkt {
        AppleMidiPacket::Invitation {
            protocol_version,
            initiator_token,
            ssrc,
            name,
        } => {
            if protocol_version != APPLE_MIDI_PROTOCOL_VERSION {
                let rej = AppleMidiPacket::InvitationReject {
                    protocol_version: APPLE_MIDI_PROTOCOL_VERSION,
                    initiator_token,
                    ssrc: inner.local_ssrc,
                    name: inner.config.session_name.clone(),
                };
                let _ = inner.control_socket.send_to(&rej.encode(), src).await;
                return;
            }
            let data_addr = SocketAddr::new(src.ip(), src.port().saturating_add(1));
            {
                let mut peers = inner.peers_by_token.lock();
                peers.insert(
                    initiator_token,
                    PeerRecord {
                        name: name.clone(),
                        ssrc,
                        initiator_token,
                        control_addr: src,
                        data_addr,
                        state: PeerConnectionState::InvitingData,
                        rtt_ms: None,
                        clock_offset_ms: None,
                        sync_count: 0,
                        last_rx_seq: None,
                        last_activity: Instant::now(),
                    },
                );
            }
            let ack = AppleMidiPacket::InvitationAck {
                protocol_version: APPLE_MIDI_PROTOCOL_VERSION,
                initiator_token,
                ssrc: inner.local_ssrc,
                name: inner.config.session_name.clone(),
            };
            let _ = inner.control_socket.send_to(&ack.encode(), src).await;
        }
        AppleMidiPacket::InvitationAck {
            initiator_token,
            ssrc,
            name,
            ..
        } => {
            let data_target = {
                let mut peers = inner.peers_by_token.lock();
                if let Some(peer) = peers.get_mut(&initiator_token) {
                    peer.name = name;
                    peer.ssrc = ssrc;
                    peer.control_addr = src;
                    peer.data_addr = SocketAddr::new(src.ip(), src.port().saturating_add(1));
                    peer.state = PeerConnectionState::InvitingData;
                    peer.last_activity = Instant::now();
                    Some(peer.data_addr)
                } else {
                    None
                }
            };
            if let Some(data_addr) = data_target {
                let inv_data = AppleMidiPacket::Invitation {
                    protocol_version: APPLE_MIDI_PROTOCOL_VERSION,
                    initiator_token,
                    ssrc: inner.local_ssrc,
                    name: inner.config.session_name.clone(),
                };
                let _ = inner.data_socket.send_to(&inv_data.encode(), data_addr).await;
            }
        }
        AppleMidiPacket::InvitationReject {
            initiator_token, ..
        }
        | AppleMidiPacket::EndSession {
            initiator_token, ..
        } => {
            inner.peers_by_token.lock().remove(&initiator_token);
        }
        AppleMidiPacket::ReceiverFeedback {
            sequence_number, ..
        } => {
            inner.journal.lock().acknowledge(sequence_number);
        }
        AppleMidiPacket::ClockSync { .. } => {
            // Clock sync normally arrives on the Data port; ignore on Control port.
        }
    }
}

async fn handle_data_apple_packet(inner: &SessionInner, pkt: AppleMidiPacket, src: SocketAddr) {
    match pkt {
        AppleMidiPacket::Invitation {
            initiator_token,
            ssrc,
            name,
            ..
        } => {
            {
                let mut peers = inner.peers_by_token.lock();
                let entry = peers.entry(initiator_token).or_insert_with(|| PeerRecord {
                    name: name.clone(),
                    ssrc,
                    initiator_token,
                    control_addr: SocketAddr::new(src.ip(), src.port().saturating_sub(1)),
                    data_addr: src,
                    state: PeerConnectionState::Connected,
                    rtt_ms: None,
                    clock_offset_ms: None,
                    sync_count: 0,
                    last_rx_seq: None,
                    last_activity: Instant::now(),
                });
                entry.name = name;
                entry.ssrc = ssrc;
                entry.data_addr = src;
                entry.state = PeerConnectionState::Connected;
                entry.last_activity = Instant::now();
            }
            let ack = AppleMidiPacket::InvitationAck {
                protocol_version: APPLE_MIDI_PROTOCOL_VERSION,
                initiator_token,
                ssrc: inner.local_ssrc,
                name: inner.config.session_name.clone(),
            };
            let _ = inner.data_socket.send_to(&ack.encode(), src).await;
        }
        AppleMidiPacket::InvitationAck {
            initiator_token,
            ssrc,
            name,
            ..
        } => {
            let should_sync = {
                let mut peers = inner.peers_by_token.lock();
                if let Some(peer) = peers.get_mut(&initiator_token) {
                    peer.name = name;
                    peer.ssrc = ssrc;
                    peer.data_addr = src;
                    peer.state = PeerConnectionState::Connected;
                    peer.last_activity = Instant::now();
                    true
                } else {
                    false
                }
            };
            if should_sync {
                // Immediately run initial 3-way clock sync (CK count=0)
                let ck = AppleMidiPacket::ClockSync {
                    ssrc: inner.local_ssrc,
                    count: 0,
                    timestamp1: apple_midi_clock_10khz(),
                    timestamp2: 0,
                    timestamp3: 0,
                };
                let _ = inner.data_socket.send_to(&ck.encode(), src).await;
            }
        }
        AppleMidiPacket::InvitationReject {
            initiator_token, ..
        }
        | AppleMidiPacket::EndSession {
            initiator_token, ..
        } => {
            inner.peers_by_token.lock().remove(&initiator_token);
        }
        AppleMidiPacket::ClockSync {
            ssrc,
            count,
            timestamp1,
            timestamp2,
            timestamp3,
        } => {
            let now = apple_midi_clock_10khz();
            match count {
                0 => {
                    // Peer initiated sync -> respond with count=1 and our timestamp2
                    let reply = AppleMidiPacket::ClockSync {
                        ssrc: inner.local_ssrc,
                        count: 1,
                        timestamp1,
                        timestamp2: now,
                        timestamp3: 0,
                    };
                    let _ = inner.data_socket.send_to(&reply.encode(), src).await;
                }
                1 => {
                    // Peer replied to our count=0 -> compute RTT & clock offset, send count=2
                    let rtt_100us = now.saturating_sub(timestamp1);
                    let rtt_ms = rtt_100us as f64 / 10.0;
                    let midpoint = (timestamp1 as i128 + now as i128) / 2;
                    let offset_ms = (timestamp2 as i128 - midpoint) as f64 / 10.0;
                    update_peer_sync_stats(inner, ssrc, src, rtt_ms, offset_ms);

                    let reply = AppleMidiPacket::ClockSync {
                        ssrc: inner.local_ssrc,
                        count: 2,
                        timestamp1,
                        timestamp2,
                        timestamp3: now,
                    };
                    let _ = inner.data_socket.send_to(&reply.encode(), src).await;
                }
                2 => {
                    // Peer completed 3-way sync -> responder computes RTT & offset too
                    let rtt_100us = timestamp3.saturating_sub(timestamp1);
                    let rtt_ms = rtt_100us as f64 / 10.0;
                    let midpoint = (timestamp1 as i128 + timestamp3 as i128) / 2;
                    let offset_ms = (timestamp2 as i128 - midpoint) as f64 / 10.0;
                    update_peer_sync_stats(inner, ssrc, src, rtt_ms, offset_ms);
                }
                _ => {}
            }
        }
        AppleMidiPacket::ReceiverFeedback {
            sequence_number, ..
        } => {
            inner.journal.lock().acknowledge(sequence_number);
        }
    }
}

fn update_peer_sync_stats(
    inner: &SessionInner,
    ssrc: u32,
    src: SocketAddr,
    rtt_ms: f64,
    offset_ms: f64,
) {
    let mut peers = inner.peers_by_token.lock();
    if let Some(peer) = peers
        .values_mut()
        .find(|p| p.ssrc == ssrc || p.data_addr == src)
    {
        peer.rtt_ms = Some(rtt_ms);
        peer.clock_offset_ms = Some(offset_ms);
        peer.sync_count += 1;
        peer.last_activity = Instant::now();
    }
}

async fn handle_rtp_data_packet(inner: &SessionInner, rtp: RtpMidiDataPacket, src: SocketAddr) {
    inner.rx_packets.fetch_add(1, Ordering::Relaxed);

    let (gap_detected, feedback_target) = {
        let mut peers = inner.peers_by_token.lock();
        if let Some(peer) = peers
            .values_mut()
            .find(|p| p.ssrc == rtp.ssrc || p.data_addr == src)
        {
            let gap = match peer.last_rx_seq {
                Some(prev) => {
                    let expected = prev.wrapping_add(1);
                    seq_is_newer(rtp.sequence_number, expected)
                }
                None => false,
            };
            if peer
                .last_rx_seq
                .map(|prev| seq_is_newer(rtp.sequence_number, prev))
                .unwrap_or(true)
            {
                peer.last_rx_seq = Some(rtp.sequence_number);
            }
            peer.last_activity = Instant::now();
            (gap, Some(peer.control_addr))
        } else {
            (false, None)
        }
    };

    let cb = inner.inbound_cb.lock().clone();

    // If a sequence gap occurred and a recovery journal is present, apply recovered MIDI first.
    if gap_detected {
        if let Some(ref journal_bytes) = rtp.journal_bytes {
            if let Some(decoded) = decode_recovery_journal(journal_bytes) {
                for msg in decoded.recovered_midi {
                    inner.recovered_messages.fetch_add(1, Ordering::Relaxed);
                    if let Some(ref f) = cb {
                        f(&msg, rtp.ssrc);
                    }
                }
            }
        }
    }

    // Dispatch every MIDI command in the command section (single or multi-message).
    if let Some(ref f) = cb {
        for ev in &rtp.events {
            if !ev.midi_bytes.is_empty() {
                f(&ev.midi_bytes, rtp.ssrc);
            }
        }
    }

    // Send AppleMIDI Receiver Feedback (RS) on Control port so sender can prune its journal.
    if let Some(ctrl_addr) = feedback_target {
        let rs = AppleMidiPacket::ReceiverFeedback {
            ssrc: inner.local_ssrc,
            sequence_number: rtp.sequence_number,
        };
        let _ = inner.control_socket.send_to(&rs.encode(), ctrl_addr).await;
    }
}

async fn transmit_midi_events(
    inner: &SessionInner,
    events: Vec<RtpMidiEvent>,
) -> Result<u16, RtpMidiError> {
    let seq = inner.seq_counter.fetch_add(1, Ordering::Relaxed);
    let ts = (apple_midi_clock_10khz() & 0xFFFF_FFFF) as u32;

    let journal_bytes = {
        let mut journal = inner.journal.lock();
        let encoded = journal.encode();
        for ev in &events {
            journal.record_midi(seq, &ev.midi_bytes);
        }
        encoded
    };

    let packet = RtpMidiDataPacket {
        sequence_number: seq,
        timestamp: ts,
        ssrc: inner.local_ssrc,
        marker: !events.is_empty(),
        events,
        journal_bytes,
    };
    let wire = encode_rtp_midi_packet(&packet);

    let targets: Vec<SocketAddr> = {
        let peers = inner.peers_by_token.lock();
        peers
            .values()
            .filter(|p| p.state == PeerConnectionState::Connected)
            .map(|p| p.data_addr)
            .collect()
    };

    for addr in targets {
        inner
            .data_socket
            .send_to(&wire, addr)
            .await
            .map_err(|e| RtpMidiError::Network(e.to_string()))?;
        inner.tx_packets.fetch_add(1, Ordering::Relaxed);
    }

    Ok(seq)
}

/// Default IPv4 bind address helper (`0.0.0.0` for LAN sessions, `127.0.0.1` for local loopback).
pub fn default_bind_ip() -> IpAddr {
    IpAddr::V4(Ipv4Addr::UNSPECIFIED)
}
