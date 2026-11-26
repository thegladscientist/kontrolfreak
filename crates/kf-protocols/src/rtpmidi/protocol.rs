//! Wire-level RFC 6295 RTP-MIDI and AppleMIDI session protocol encoding/decoding.

use serde::{Deserialize, Serialize};

pub const APPLE_MIDI_SIGNATURE: [u8; 2] = [0xFF, 0xFF];
pub const APPLE_MIDI_PROTOCOL_VERSION: u32 = 2;
pub const RTP_VERSION: u8 = 2;
pub const RTP_MIDI_PAYLOAD_TYPE: u8 = 97;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RtpMidiCommand {
    Invitation,       // "IN"
    InvitationAck,    // "OK"
    InvitationReject, // "NO"
    EndSession,       // "BY"
    Sync,             // "CK"
    ReceiverFeedback, // "RS"
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

    pub fn from_tag(tag: [u8; 2]) -> Option<Self> {
        match &tag {
            b"IN" => Some(Self::Invitation),
            b"OK" => Some(Self::InvitationAck),
            b"NO" => Some(Self::InvitationReject),
            b"BY" => Some(Self::EndSession),
            b"CK" => Some(Self::Sync),
            b"RS" => Some(Self::ReceiverFeedback),
            _ => None,
        }
    }
}

/// Decoded AppleMIDI control/synchronization packet (starts with `0xFFFF`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppleMidiPacket {
    Invitation {
        protocol_version: u32,
        initiator_token: u32,
        ssrc: u32,
        name: String,
    },
    InvitationAck {
        protocol_version: u32,
        initiator_token: u32,
        ssrc: u32,
        name: String,
    },
    InvitationReject {
        protocol_version: u32,
        initiator_token: u32,
        ssrc: u32,
        name: String,
    },
    EndSession {
        protocol_version: u32,
        initiator_token: u32,
        ssrc: u32,
    },
    ClockSync {
        ssrc: u32,
        count: u8,
        timestamp1: u64,
        timestamp2: u64,
        timestamp3: u64,
    },
    ReceiverFeedback {
        ssrc: u32,
        sequence_number: u16,
    },
}

impl AppleMidiPacket {
    pub fn is_apple_midi(bytes: &[u8]) -> bool {
        bytes.len() >= 4 && bytes[0] == APPLE_MIDI_SIGNATURE[0] && bytes[1] == APPLE_MIDI_SIGNATURE[1]
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(48);
        out.extend_from_slice(&APPLE_MIDI_SIGNATURE);
        match self {
            AppleMidiPacket::Invitation {
                protocol_version,
                initiator_token,
                ssrc,
                name,
            } => {
                out.extend_from_slice(&RtpMidiCommand::Invitation.tag_bytes());
                out.extend_from_slice(&protocol_version.to_be_bytes());
                out.extend_from_slice(&initiator_token.to_be_bytes());
                out.extend_from_slice(&ssrc.to_be_bytes());
                out.extend_from_slice(name.as_bytes());
                out.push(0);
            }
            AppleMidiPacket::InvitationAck {
                protocol_version,
                initiator_token,
                ssrc,
                name,
            } => {
                out.extend_from_slice(&RtpMidiCommand::InvitationAck.tag_bytes());
                out.extend_from_slice(&protocol_version.to_be_bytes());
                out.extend_from_slice(&initiator_token.to_be_bytes());
                out.extend_from_slice(&ssrc.to_be_bytes());
                out.extend_from_slice(name.as_bytes());
                out.push(0);
            }
            AppleMidiPacket::InvitationReject {
                protocol_version,
                initiator_token,
                ssrc,
                name,
            } => {
                out.extend_from_slice(&RtpMidiCommand::InvitationReject.tag_bytes());
                out.extend_from_slice(&protocol_version.to_be_bytes());
                out.extend_from_slice(&initiator_token.to_be_bytes());
                out.extend_from_slice(&ssrc.to_be_bytes());
                out.extend_from_slice(name.as_bytes());
                out.push(0);
            }
            AppleMidiPacket::EndSession {
                protocol_version,
                initiator_token,
                ssrc,
            } => {
                out.extend_from_slice(&RtpMidiCommand::EndSession.tag_bytes());
                out.extend_from_slice(&protocol_version.to_be_bytes());
                out.extend_from_slice(&initiator_token.to_be_bytes());
                out.extend_from_slice(&ssrc.to_be_bytes());
            }
            AppleMidiPacket::ClockSync {
                ssrc,
                count,
                timestamp1,
                timestamp2,
                timestamp3,
            } => {
                out.extend_from_slice(&RtpMidiCommand::Sync.tag_bytes());
                out.extend_from_slice(&ssrc.to_be_bytes());
                out.push(*count);
                out.extend_from_slice(&[0u8; 3]); // 24-bit reserved padding
                out.extend_from_slice(&timestamp1.to_be_bytes());
                out.extend_from_slice(&timestamp2.to_be_bytes());
                out.extend_from_slice(&timestamp3.to_be_bytes());
            }
            AppleMidiPacket::ReceiverFeedback {
                ssrc,
                sequence_number,
            } => {
                out.extend_from_slice(&RtpMidiCommand::ReceiverFeedback.tag_bytes());
                out.extend_from_slice(&ssrc.to_be_bytes());
                out.extend_from_slice(&sequence_number.to_be_bytes());
                out.extend_from_slice(&[0u8; 2]);
            }
        }
        out
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if !Self::is_apple_midi(bytes) {
            return None;
        }
        let cmd = RtpMidiCommand::from_tag([bytes[2], bytes[3]])?;
        match cmd {
            RtpMidiCommand::Invitation
            | RtpMidiCommand::InvitationAck
            | RtpMidiCommand::InvitationReject => {
                if bytes.len() < 16 {
                    return None;
                }
                let protocol_version = u32::from_be_bytes(bytes[4..8].try_into().ok()?);
                let initiator_token = u32::from_be_bytes(bytes[8..12].try_into().ok()?);
                let ssrc = u32::from_be_bytes(bytes[12..16].try_into().ok()?);
                let raw_name = &bytes[16..];
                let nul_pos = raw_name.iter().position(|&b| b == 0).unwrap_or(raw_name.len());
                let name = String::from_utf8_lossy(&raw_name[..nul_pos]).to_string();
                match cmd {
                    RtpMidiCommand::Invitation => Some(Self::Invitation {
                        protocol_version,
                        initiator_token,
                        ssrc,
                        name,
                    }),
                    RtpMidiCommand::InvitationAck => Some(Self::InvitationAck {
                        protocol_version,
                        initiator_token,
                        ssrc,
                        name,
                    }),
                    RtpMidiCommand::InvitationReject => Some(Self::InvitationReject {
                        protocol_version,
                        initiator_token,
                        ssrc,
                        name,
                    }),
                    _ => unreachable!(),
                }
            }
            RtpMidiCommand::EndSession => {
                if bytes.len() < 16 {
                    return None;
                }
                let protocol_version = u32::from_be_bytes(bytes[4..8].try_into().ok()?);
                let initiator_token = u32::from_be_bytes(bytes[8..12].try_into().ok()?);
                let ssrc = u32::from_be_bytes(bytes[12..16].try_into().ok()?);
                Some(Self::EndSession {
                    protocol_version,
                    initiator_token,
                    ssrc,
                })
            }
            RtpMidiCommand::Sync => {
                if bytes.len() < 36 {
                    return None;
                }
                let ssrc = u32::from_be_bytes(bytes[4..8].try_into().ok()?);
                let count = bytes[8];
                let timestamp1 = u64::from_be_bytes(bytes[12..20].try_into().ok()?);
                let timestamp2 = u64::from_be_bytes(bytes[20..28].try_into().ok()?);
                let timestamp3 = u64::from_be_bytes(bytes[28..36].try_into().ok()?);
                Some(Self::ClockSync {
                    ssrc,
                    count,
                    timestamp1,
                    timestamp2,
                    timestamp3,
                })
            }
            RtpMidiCommand::ReceiverFeedback => {
                if bytes.len() < 12 {
                    return None;
                }
                let ssrc = u32::from_be_bytes(bytes[4..8].try_into().ok()?);
                let sequence_number = u16::from_be_bytes(bytes[8..10].try_into().ok()?);
                Some(Self::ReceiverFeedback {
                    ssrc,
                    sequence_number,
                })
            }
        }
    }
}

/// A single MIDI command paired with its RFC 6295 delta time (in RTP clock ticks, 10 kHz).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtpMidiEvent {
    pub delta_time: u32,
    pub midi_bytes: Vec<u8>,
}

/// Decoded RFC 6295 RTP-MIDI data packet (RTP header + MIDI command section + optional journal).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtpMidiDataPacket {
    pub sequence_number: u16,
    pub timestamp: u32,
    pub ssrc: u32,
    pub marker: bool,
    pub events: Vec<RtpMidiEvent>,
    pub journal_bytes: Option<Vec<u8>>,
}

/// Encodes an RFC 6295 variable-length delta time (1 to 4 octets, 7-bit big-endian chunks).
pub fn encode_delta_time(delta: u32, out: &mut Vec<u8>) {
    let val = delta & 0x0FFF_FFFF; // max 28 bits per RFC 6295 §3.1
    if val < 0x80 {
        out.push(val as u8);
    } else if val < 0x4000 {
        out.push(((val >> 7) as u8 & 0x7F) | 0x80);
        out.push((val as u8) & 0x7F);
    } else if val < 0x0020_0000 {
        out.push(((val >> 14) as u8 & 0x7F) | 0x80);
        out.push(((val >> 7) as u8 & 0x7F) | 0x80);
        out.push((val as u8) & 0x7F);
    } else {
        out.push(((val >> 21) as u8 & 0x7F) | 0x80);
        out.push(((val >> 14) as u8 & 0x7F) | 0x80);
        out.push(((val >> 7) as u8 & 0x7F) | 0x80);
        out.push((val as u8) & 0x7F);
    }
}

/// Decodes an RFC 6295 variable-length delta time, returning `(delta, bytes_consumed)`.
pub fn decode_delta_time(bytes: &[u8]) -> Option<(u32, usize)> {
    let mut value: u32 = 0;
    for (idx, &b) in bytes.iter().take(4).enumerate() {
        value = (value << 7) | (b & 0x7F) as u32;
        if (b & 0x80) == 0 {
            return Some((value, idx + 1));
        }
    }
    None
}

/// Returns expected MIDI data byte count (excluding status byte) for a standard MIDI status byte.
/// Returns `None` for SysEx (`0xF0`) or unknown/unframed status bytes.
pub fn midi_data_length(status: u8) -> Option<usize> {
    match status {
        0x80..=0xBF | 0xE0..=0xEF => Some(2),
        0xC0..=0xDF => Some(1),
        0xF1 | 0xF3 => Some(1),
        0xF2 => Some(2),
        0xF6 | 0xF8..=0xFF => Some(0),
        _ => None,
    }
}

/// Encodes a complete RFC 6295 RTP-MIDI packet (12-byte RTP header + command section + optional journal).
pub fn encode_rtp_midi_packet(packet: &RtpMidiDataPacket) -> Vec<u8> {
    let mut midi_list = Vec::new();
    let has_first_delta = packet
        .events
        .first()
        .map(|ev| ev.delta_time > 0)
        .unwrap_or(false);

    for (idx, ev) in packet.events.iter().enumerate() {
        if ev.midi_bytes.is_empty() {
            continue;
        }
        if idx == 0 {
            if has_first_delta {
                encode_delta_time(ev.delta_time, &mut midi_list);
            }
        } else {
            encode_delta_time(ev.delta_time, &mut midi_list);
        }
        midi_list.extend_from_slice(&ev.midi_bytes);
    }

    let list_len = midi_list.len().min(0x0FFF);
    let has_journal = packet
        .journal_bytes
        .as_ref()
        .map(|j| !j.is_empty())
        .unwrap_or(false);
    let long_header = list_len > 15;

    let mut out = Vec::with_capacity(
        12 + if long_header { 2 } else { 1 }
            + list_len
            + packet.journal_bytes.as_ref().map_or(0, Vec::len),
    );

    // 12-byte RTP Header (RFC 3550)
    out.push(RTP_VERSION << 6); // V=2, P=0, X=0, CC=0
    out.push((if packet.marker { 0x80 } else { 0x00 }) | (RTP_MIDI_PAYLOAD_TYPE & 0x7F));
    out.extend_from_slice(&packet.sequence_number.to_be_bytes());
    out.extend_from_slice(&packet.timestamp.to_be_bytes());
    out.extend_from_slice(&packet.ssrc.to_be_bytes());

    // RFC 6295 MIDI Command Section Header: B | J | Z | P | LEN
    let flags = (if long_header { 0x80 } else { 0x00 })
        | (if has_journal { 0x40 } else { 0x00 })
        | (if has_first_delta { 0x20 } else { 0x00 });

    if long_header {
        let len12 = list_len as u16;
        out.push(flags | ((len12 >> 8) as u8 & 0x0F));
        out.push((len12 & 0xFF) as u8);
    } else {
        out.push(flags | (list_len as u8 & 0x0F));
    }

    out.extend_from_slice(&midi_list[..list_len]);

    if let Some(journal) = &packet.journal_bytes {
        if !journal.is_empty() {
            out.extend_from_slice(journal);
        }
    }

    out
}

/// Decodes an RFC 6295 RTP-MIDI data packet (returns `None` for non-RTP or malformed packets).
pub fn decode_rtp_midi_packet(bytes: &[u8]) -> Option<RtpMidiDataPacket> {
    if bytes.len() < 13 {
        return None;
    }
    let version = bytes[0] >> 6;
    if version != RTP_VERSION {
        return None;
    }
    let csrc_count = (bytes[0] & 0x0F) as usize;
    let has_extension = (bytes[0] & 0x10) != 0;
    let has_padding = (bytes[0] & 0x20) != 0;
    let marker = (bytes[1] & 0x80) != 0;
    let payload_type = bytes[1] & 0x7F;
    if payload_type != RTP_MIDI_PAYLOAD_TYPE {
        return None;
    }

    let sequence_number = u16::from_be_bytes([bytes[2], bytes[3]]);
    let timestamp = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    let ssrc = u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);

    let mut offset = 12 + csrc_count * 4;
    if offset >= bytes.len() {
        return None;
    }
    if has_extension {
        if bytes.len() < offset + 4 {
            return None;
        }
        let ext_words = u16::from_be_bytes([bytes[offset + 2], bytes[offset + 3]]) as usize;
        offset += 4 + ext_words * 4;
    }

    let mut end = bytes.len();
    if has_padding {
        let pad_len = *bytes.last()? as usize;
        if pad_len == 0 || pad_len > end.saturating_sub(offset) {
            return None;
        }
        end -= pad_len;
    }

    if offset >= end {
        return None;
    }

    // MIDI Command Section Header
    let hdr0 = bytes[offset];
    let long_header = (hdr0 & 0x80) != 0;
    let has_journal = (hdr0 & 0x40) != 0;
    let has_first_delta = (hdr0 & 0x20) != 0;
    let _phantom_status = (hdr0 & 0x10) != 0;

    let (list_len, list_start) = if long_header {
        if offset + 2 > end {
            return None;
        }
        let len = (((hdr0 & 0x0F) as usize) << 8) | (bytes[offset + 1] as usize);
        (len, offset + 2)
    } else {
        ((hdr0 & 0x0F) as usize, offset + 1)
    };

    if list_start + list_len > end {
        return None;
    }

    let list_slice = &bytes[list_start..list_start + list_len];
    let events = parse_midi_command_list(list_slice, has_first_delta);

    let journal_start = list_start + list_len;
    let journal_bytes = if has_journal && journal_start < end {
        Some(bytes[journal_start..end].to_vec())
    } else {
        None
    };

    Some(RtpMidiDataPacket {
        sequence_number,
        timestamp,
        ssrc,
        marker,
        events,
        journal_bytes,
    })
}

fn parse_midi_command_list(slice: &[u8], has_first_delta: bool) -> Vec<RtpMidiEvent> {
    let mut events = Vec::new();
    let mut pos = 0;
    let mut running_status: Option<u8> = None;
    let mut first = true;

    while pos < slice.len() {
        let delta_time = if !first || has_first_delta {
            match decode_delta_time(&slice[pos..]) {
                Some((dt, consumed)) => {
                    pos += consumed;
                    dt
                }
                None => break,
            }
        } else {
            0
        };
        first = false;

        if pos >= slice.len() {
            break;
        }

        let b = slice[pos];
        if b == 0xF0 {
            // SysEx message until 0xF7 (or end of list)
            let start = pos;
            pos += 1;
            while pos < slice.len() && slice[pos] != 0xF7 {
                pos += 1;
            }
            if pos < slice.len() && slice[pos] == 0xF7 {
                pos += 1;
            }
            running_status = None;
            events.push(RtpMidiEvent {
                delta_time,
                midi_bytes: slice[start..pos].to_vec(),
            });
        } else if (b & 0x80) != 0 {
            let status = b;
            pos += 1;
            // Channel voice messages update running status; system common clears it; realtime preserves it.
            if (0x80..=0xEF).contains(&status) {
                running_status = Some(status);
            } else if (0xF0..=0xF7).contains(&status) {
                running_status = None;
            }

            let Some(data_len) = midi_data_length(status) else {
                break;
            };
            if pos + data_len > slice.len() {
                break;
            }
            let mut msg = Vec::with_capacity(1 + data_len);
            msg.push(status);
            msg.extend_from_slice(&slice[pos..pos + data_len]);
            pos += data_len;
            events.push(RtpMidiEvent {
                delta_time,
                midi_bytes: msg,
            });
        } else if let Some(rs) = running_status {
            let Some(data_len) = midi_data_length(rs) else {
                break;
            };
            if pos + data_len > slice.len() {
                break;
            }
            let mut msg = Vec::with_capacity(1 + data_len);
            msg.push(rs);
            msg.extend_from_slice(&slice[pos..pos + data_len]);
            pos += data_len;
            events.push(RtpMidiEvent {
                delta_time,
                midi_bytes: msg,
            });
        } else {
            // Stray data byte without running status — skip safely
            pos += 1;
        }
    }

    events
}

/// Returns current timestamp in 100-microsecond units (10 kHz AppleMIDI clock).
pub fn apple_midi_clock_10khz() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| (d.as_micros() / 100) as u64)
        .unwrap_or(0)
}
