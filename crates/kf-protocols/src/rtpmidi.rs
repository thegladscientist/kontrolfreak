//! Pure-Rust RFC 6295 / AppleMIDI RTPMIDI transport + `mdns-sd` discovery.
//!
//! Features:
//! - Dual UDP ports: Control (`N`) and Data (`N+1`)
//! - AppleMIDI session handshake (`IN`, `OK`, `NO`, `BY`) on both ports
//! - 3-way Clock Synchronization (`CK`) with RTT and clock-offset tracking
//! - Receiver Feedback (`RS`) and RFC 6295 Recovery Journal (Chapter C & Chapter N)
//! - RFC 6295 MIDI command section framing/parsing (single & multi-message packets, VLQ delta times, running status, SysEx)
//! - `_apple-midi._udp.local.` mDNS-SD service announcement and discovery with headless fallback

pub mod discovery;
pub mod journal;
pub mod protocol;
pub mod session;

pub use discovery::{MdnsDiscoveryHandle, MdnsStatus, APPLE_MIDI_MDNS_SERVICE};
pub use journal::{decode_recovery_journal, seq_is_newer, DecodedJournal, RecoveryJournal};
pub use protocol::{
    apple_midi_clock_10khz, decode_delta_time, decode_rtp_midi_packet, encode_delta_time,
    encode_rtp_midi_packet, midi_data_length, AppleMidiPacket, RtpMidiCommand, RtpMidiDataPacket,
    RtpMidiEvent, APPLE_MIDI_PROTOCOL_VERSION, APPLE_MIDI_SIGNATURE, RTP_MIDI_PAYLOAD_TYPE,
    RTP_VERSION,
};
pub use session::{
    default_bind_ip, ConnectedPeerInfo, InboundMidiCallback, PeerConnectionState, RtpMidiSession,
    RtpMidiSessionStatus,
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RtpMidiConfig {
    pub session_name: String,
    pub control_port: u16,
    pub remote_host: Option<String>,
    pub mdns_discover: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RtpMidiPeer {
    pub name: String,
    pub host: String,
    pub control_port: u16,
    pub ssrc: Option<u32>,
}

#[derive(Debug, Error)]
pub enum RtpMidiError {
    #[error("RTPMIDI transport not implemented yet (scheduled for Milestone 2)")]
    NotImplementedYet,
    #[error("RTPMIDI network error: {0}")]
    Network(String),
}

pub trait RtpMidiSessionTrait: Send + Sync {
    fn config(&self) -> &RtpMidiConfig;
    fn discovered_peers(&self) -> Vec<RtpMidiPeer>;
    fn send_midi_payload(&self, midi_bytes: &[u8]) -> Result<(), RtpMidiError>;
}

/// Legacy M1 stub preserved for compatibility with any caller testing stub behavior.
#[derive(Debug, Clone)]
pub struct RtpMidiStub {
    config: RtpMidiConfig,
}

impl RtpMidiStub {
    pub fn new(config: RtpMidiConfig) -> Self {
        Self { config }
    }
}

impl RtpMidiSessionTrait for RtpMidiStub {
    fn config(&self) -> &RtpMidiConfig {
        &self.config
    }

    fn discovered_peers(&self) -> Vec<RtpMidiPeer> {
        Vec::new()
    }

    fn send_midi_payload(&self, _midi_bytes: &[u8]) -> Result<(), RtpMidiError> {
        Err(RtpMidiError::NotImplementedYet)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::mpsc;

    #[test]
    fn apple_midi_control_packets_roundtrip() {
        let packets = vec![
            AppleMidiPacket::Invitation {
                protocol_version: APPLE_MIDI_PROTOCOL_VERSION,
                initiator_token: 0x1122_3344,
                ssrc: 0xAABB_CCDD,
                name: "Kontrolfreak Studio".into(),
            },
            AppleMidiPacket::InvitationAck {
                protocol_version: APPLE_MIDI_PROTOCOL_VERSION,
                initiator_token: 0x1122_3344,
                ssrc: 0x5566_7788,
                name: "iPad Pro".into(),
            },
            AppleMidiPacket::InvitationReject {
                protocol_version: APPLE_MIDI_PROTOCOL_VERSION,
                initiator_token: 0x9988_7766,
                ssrc: 0x1234_5678,
                name: "BusyPeer".into(),
            },
            AppleMidiPacket::EndSession {
                protocol_version: APPLE_MIDI_PROTOCOL_VERSION,
                initiator_token: 0x1122_3344,
                ssrc: 0xAABB_CCDD,
            },
            AppleMidiPacket::ClockSync {
                ssrc: 0xCAFE_BABE,
                count: 2,
                timestamp1: 100_000,
                timestamp2: 100_015,
                timestamp3: 100_024,
            },
            AppleMidiPacket::ReceiverFeedback {
                ssrc: 0xDEAD_BEEF,
                sequence_number: 4219,
            },
        ];

        for pkt in packets {
            let encoded = pkt.encode();
            assert!(AppleMidiPacket::is_apple_midi(&encoded));
            let decoded = AppleMidiPacket::decode(&encoded).expect("packet decodes");
            assert_eq!(decoded, pkt);
        }
    }

    #[test]
    fn rfc6295_multi_message_delta_times_and_journal_roundtrip() {
        let mut journal = RecoveryJournal::new(100);
        journal.record_midi(101, &[0xB0, 7, 99]); // Ch1 CC7 = 99
        journal.record_midi(102, &[0x90, 60, 110]); // Ch1 Note C4 On vel 110
        let jbytes = journal.encode().expect("journal encodes");

        let pkt = RtpMidiDataPacket {
            sequence_number: 103,
            timestamp: 987_654,
            ssrc: 0x1357_2468,
            marker: true,
            events: vec![
                RtpMidiEvent {
                    delta_time: 12,
                    midi_bytes: vec![0x90, 64, 100],
                },
                RtpMidiEvent {
                    delta_time: 240,
                    midi_bytes: vec![0xB0, 74, 88],
                },
                RtpMidiEvent {
                    delta_time: 16384,
                    midi_bytes: vec![0x80, 64, 0],
                },
                RtpMidiEvent {
                    delta_time: 5,
                    midi_bytes: vec![0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7],
                },
            ],
            journal_bytes: Some(jbytes),
        };

        let wire = encode_rtp_midi_packet(&pkt);
        let decoded = decode_rtp_midi_packet(&wire).expect("RTP-MIDI decodes");
        assert_eq!(decoded.sequence_number, 103);
        assert_eq!(decoded.timestamp, 987_654);
        assert_eq!(decoded.ssrc, 0x1357_2468);
        assert_eq!(decoded.events.len(), 4);
        assert_eq!(decoded.events[0], pkt.events[0]);
        assert_eq!(decoded.events[1], pkt.events[1]);
        assert_eq!(decoded.events[2], pkt.events[2]);
        assert_eq!(decoded.events[3], pkt.events[3]);

        let dj = decode_recovery_journal(decoded.journal_bytes.as_ref().unwrap())
            .expect("journal decodes");
        assert_eq!(dj.checkpoint_seq, 100);
        assert!(dj.recovered_midi.contains(&vec![0xB0, 7, 99]));
        assert!(dj.recovered_midi.contains(&vec![0x90, 60, 110]));

        // Receiver feedback prunes acknowledged entries
        journal.acknowledge(102);
        assert!(journal.is_empty());
    }

    #[tokio::test]
    async fn two_peer_rtpmidi_loopback_handshake_clock_sync_and_bidirectional_midi() {
        let (rx_a_tx, mut rx_a_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let (rx_b_tx, mut rx_b_rx) = mpsc::unbounded_channel::<Vec<u8>>();

        let peer_a = RtpMidiSession::start(
            RtpMidiConfig {
                session_name: "Kontrolfreak-A".into(),
                control_port: 0,
                remote_host: None,
                mdns_discover: false,
            },
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            Some(Arc::new(move |bytes, _ssrc| {
                let _ = rx_a_tx.send(bytes.to_vec());
            })),
        )
        .await
        .expect("Peer A starts");

        let peer_b = RtpMidiSession::start(
            RtpMidiConfig {
                session_name: "Kontrolfreak-B".into(),
                control_port: 0,
                remote_host: None,
                mdns_discover: false,
            },
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            Some(Arc::new(move |bytes, _ssrc| {
                let _ = rx_b_tx.send(bytes.to_vec());
            })),
        )
        .await
        .expect("Peer B starts");

        assert_eq!(peer_a.data_addr().port(), peer_a.control_addr().port() + 1);
        assert_eq!(peer_b.data_addr().port(), peer_b.control_addr().port() + 1);

        // Peer A invites Peer B over dual UDP (control N -> OK -> data N+1 -> OK -> CK 3-way)
        peer_a
            .connect_peer(peer_b.control_addr())
            .await
            .expect("invite sent");

        // Wait until both sides report Connected + completed initial 3-way CK clock sync
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            let a_peers = peer_a.connected_peers();
            let b_peers = peer_b.connected_peers();
            let a_ready = a_peers
                .iter()
                .any(|p| p.state == PeerConnectionState::Connected && p.sync_count >= 1);
            let b_ready = b_peers
                .iter()
                .any(|p| p.state == PeerConnectionState::Connected && p.sync_count >= 1);
            if a_ready && b_ready {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for handshake + 3-way CK sync: A={a_peers:?} B={b_peers:?}"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        // Verify RTT is measured and low on loopback
        let a_peer = &peer_a.connected_peers()[0];
        assert_eq!(a_peer.name, "Kontrolfreak-B");
        assert!(a_peer.rtt_ms.is_some());
        assert!(a_peer.rtt_ms.unwrap() < 100.0);

        // Bidirectional transfer: A -> B (multi-message batch with delta times)
        peer_a
            .send_midi_batch(vec![
                RtpMidiEvent {
                    delta_time: 0,
                    midi_bytes: vec![0xB0, 7, 108],
                },
                RtpMidiEvent {
                    delta_time: 10,
                    midi_bytes: vec![0x90, 60, 127],
                },
            ])
            .await
            .expect("A->B send");

        let msg1 = tokio::time::timeout(Duration::from_millis(500), rx_b_rx.recv())
            .await
            .expect("timeout B msg1")
            .unwrap();
        let msg2 = tokio::time::timeout(Duration::from_millis(500), rx_b_rx.recv())
            .await
            .expect("timeout B msg2")
            .unwrap();
        assert_eq!(msg1, vec![0xB0, 7, 108]);
        assert_eq!(msg2, vec![0x90, 60, 127]);

        // Bidirectional transfer: B -> A via RtpMidiSessionTrait::send_midi_payload
        peer_b
            .send_midi_payload(&[0xB1, 74, 64])
            .expect("B->A send");

        let msg_back = tokio::time::timeout(Duration::from_millis(500), rx_a_rx.recv())
            .await
            .expect("timeout A msg")
            .unwrap();
        assert_eq!(msg_back, vec![0xB1, 74, 64]);

        // Graceful EndSession (BY) clears peer state on remote peer
        peer_a.shutdown().await;
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert!(
            peer_b.connected_peers().is_empty(),
            "Peer B should clear Peer A upon receiving BY"
        );
        peer_b.shutdown().await;
    }

    #[test]
    fn mdns_discovery_gracefully_starts_or_falls_back() {
        let handle = MdnsDiscoveryHandle::start("Kontrolfreak-Test", 5004, false, true);
        let status = handle.status();
        assert_eq!(status.service_type, APPLE_MIDI_MDNS_SERVICE);
        // Should never panic regardless of whether multicast is allowed in CI container
        let _ = handle.discovered_peers();
    }
}
