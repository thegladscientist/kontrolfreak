//! Milestone 2 seam module for pure-Rust RFC 6295 RTPMIDI + `mdns-sd` discovery.
//! M2 will implement dual-UDP session handshake (`IN`/`OK`/`NO`/`BY`), clock sync (`CK`),
//! receiver feedback (`RS`), and `_apple-midi._udp.local.` discovery here without modifying
//! OSC, MIDI, WebSocket, or UDP modules.

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const APPLE_MIDI_SIGNATURE: [u8; 2] = [0xFF, 0xFF];
pub const APPLE_MIDI_MDNS_SERVICE: &str = "_apple-midi._udp.local.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RtpMidiCommand {
    Invitation,      // "IN"
    InvitationAck,   // "OK"
    InvitationReject,// "NO"
    EndSession,      // "BY"
    Sync,            // "CK"
    ReceiverFeedback,// "RS"
}

impl RtpMidiCommand {
    pub fn tag_bytes(self) -> [u8; 2] {
        match self {
            RtpMidiCommand::Invitation => *b"IN",
            RtpMidiCommand::InvitationAck => *b"OK",
            RtpMidiCommand::InvitationReject => *b"NO",
            RtpMidiCommand::EndSession => *b"BY",
            RtpMidiCommand::Sync => *b"CK",
            RtpMidiCommand::ReceiverFeedback => *b"RS",
        }
    }
}

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
