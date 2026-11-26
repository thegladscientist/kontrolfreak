//! Basic RFC 6295 Recovery Journal encoding, decoding, and checkpoint pruning.
//!
//! Tracks recent Control Change (Chapter C) and Note On/Off (Chapter N) state so that if a UDP
//! RTP-MIDI packet is lost in transit, the receiver can reconstruct missed state from the journal
//! section attached to the next packet, and prune acknowledged entries upon receiving AppleMIDI
//! `RS` (Receiver Feedback) packets.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
struct CcEntry {
    seq: u16,
    value: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NoteEntry {
    seq: u16,
    velocity: u8,
    on: bool,
}

#[derive(Debug, Default, Clone)]
struct ChannelJournalState {
    cc: BTreeMap<u8, CcEntry>,
    notes: BTreeMap<u8, NoteEntry>,
}

/// Maintains outbound RFC 6295 recovery journal state per MIDI channel (0..15).
#[derive(Debug, Default, Clone)]
pub struct RecoveryJournal {
    checkpoint_seq: u16,
    channels: BTreeMap<u8, ChannelJournalState>,
}

impl RecoveryJournal {
    pub fn new(initial_checkpoint: u16) -> Self {
        Self {
            checkpoint_seq: initial_checkpoint,
            channels: BTreeMap::new(),
        }
    }

    pub fn checkpoint_seq(&self) -> u16 {
        self.checkpoint_seq
    }

    pub fn is_empty(&self) -> bool {
        self.channels
            .values()
            .all(|ch| ch.cc.is_empty() && ch.notes.is_empty())
    }

    /// Records a sent MIDI message at RTP sequence number `seq`.
    pub fn record_midi(&mut self, seq: u16, midi_bytes: &[u8]) {
        if midi_bytes.len() < 3 {
            return;
        }
        let status = midi_bytes[0];
        let kind = status & 0xF0;
        let chan = status & 0x0F;
        match kind {
            0xB0 => {
                let cc = midi_bytes[1] & 0x7F;
                let value = midi_bytes[2] & 0x7F;
                let ch = self.channels.entry(chan).or_default();
                ch.cc.insert(cc, CcEntry { seq, value });
            }
            0x90 => {
                let note = midi_bytes[1] & 0x7F;
                let vel = midi_bytes[2] & 0x7F;
                let ch = self.channels.entry(chan).or_default();
                ch.notes.insert(
                    note,
                    NoteEntry {
                        seq,
                        velocity: vel,
                        on: vel > 0,
                    },
                );
            }
            0x80 => {
                let note = midi_bytes[1] & 0x7F;
                let ch = self.channels.entry(chan).or_default();
                ch.notes.insert(
                    note,
                    NoteEntry {
                        seq,
                        velocity: 0,
                        on: false,
                    },
                );
            }
            _ => {}
        }
    }

    /// Prunes journal entries acknowledged by a peer's `RS` receiver feedback sequence number.
    pub fn acknowledge(&mut self, ack_seq: u16) {
        self.checkpoint_seq = ack_seq;
        for ch in self.channels.values_mut() {
            ch.cc.retain(|_, e| seq_is_newer(e.seq, ack_seq));
            ch.notes.retain(|_, e| seq_is_newer(e.seq, ack_seq));
        }
        self.channels
            .retain(|_, ch| !ch.cc.is_empty() || !ch.notes.is_empty());
    }

    /// Encodes the recovery journal in RFC 6295 format (Journal Header + Channel Journals with
    /// Chapter C and Chapter N). Returns `None` when the journal is empty.
    pub fn encode(&self) -> Option<Vec<u8>> {
        let active_channels: Vec<(u8, &ChannelJournalState)> = self
            .channels
            .iter()
            .filter(|(_, st)| !st.cc.is_empty() || !st.notes.is_empty())
            .map(|(&c, st)| (c, st))
            .collect();

        if active_channels.is_empty() {
            return None;
        }

        let totchan = (active_channels.len() - 1).min(15) as u8;
        let mut out = Vec::new();

        // Journal Header (3 bytes): S=0, Y=0, A=1 (channel journal present), H=0, TOTCHAN(4)
        out.push(0x20 | (totchan & 0x0F));
        out.extend_from_slice(&self.checkpoint_seq.to_be_bytes());

        for (chan, st) in active_channels.into_iter().take(16) {
            let mut chapters = Vec::new();
            let mut chapter_flags: u8 = 0;

            // Chapter C (bit 6 = 0x40): Control Change logs
            if !st.cc.is_empty() {
                chapter_flags |= 0x40;
                let count = st.cc.len().min(128);
                // S=0 | LENGTH(7) where LENGTH = count - 1
                chapters.push(((count - 1) as u8) & 0x7F);
                for (&cc_num, entry) in st.cc.iter().take(count) {
                    chapters.push(cc_num & 0x7F); // S=0 | NUMBER(7)
                    chapters.push(entry.value & 0x7F); // A=0 | VALUE(7) (absolute value)
                }
            }

            // Chapter N (bit 3 = 0x08): Note On/Off logs
            if !st.notes.is_empty() {
                chapter_flags |= 0x08;
                let active_notes: Vec<(u8, &NoteEntry)> =
                    st.notes.iter().filter(|(_, n)| n.on).map(|(&k, v)| (k, v)).collect();
                let off_notes: Vec<u8> = st
                    .notes
                    .iter()
                    .filter(|(_, n)| !n.on)
                    .map(|(&k, _)| k)
                    .collect();

                let note_log_len = active_notes.len().min(15) as u8;
                let (low, high, bitfield) = build_note_off_bitfield(&off_notes);

                // Chapter N header (2 bytes): B=0 | LENGTH(4) | LOW(4) | HIGH(4)
                chapters.push(((note_log_len & 0x0F) << 3) | ((low >> 1) & 0x07));
                chapters.push(((low & 0x01) << 7) | ((high & 0x0F) << 3));
                // Actually RFC 6295 §4.2.4 exact layout:
                // byte 0: B(1) | LENGTH(7)
                // byte 1: LOW(4) | HIGH(4)
                // Let's overwrite those 2 bytes with exact RFC 6295 §4.2.4 layout!
                let hdr_idx = chapters.len() - 2;
                chapters[hdr_idx] = note_log_len & 0x7F;
                chapters[hdr_idx + 1] = ((low & 0x0F) << 4) | (high & 0x0F);

                for (note_num, entry) in active_notes.into_iter().take(note_log_len as usize) {
                    chapters.push(note_num & 0x7F); // S=0 | NOTENUM(7)
                    chapters.push(entry.velocity & 0x7F); // Y=0 | VELOCITY(7)
                }
                chapters.extend_from_slice(&bitfield);
            }

            // Channel Journal Header (3 bytes):
            // byte 0: S=0 | CHAN(4) | H=0 | LENGTH_HI(2)
            // byte 1: LENGTH_LO(8)
            // byte 2: P C M W N E T A
            let total_len = (3 + chapters.len()).min(0x03FF) as u16;
            out.push(((chan & 0x0F) << 3) | ((total_len >> 8) as u8 & 0x03));
            out.push((total_len & 0xFF) as u8);
            out.push(chapter_flags);
            out.extend_from_slice(&chapters);
        }

        Some(out)
    }
}

/// Decoded MIDI recovery messages reconstructed from an incoming RFC 6295 journal section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedJournal {
    pub checkpoint_seq: u16,
    pub recovered_midi: Vec<Vec<u8>>,
}

/// Decodes Chapter C (CC) and Chapter N (Note) recovery messages from a journal section.
pub fn decode_recovery_journal(bytes: &[u8]) -> Option<DecodedJournal> {
    if bytes.len() < 3 {
        return None;
    }
    let hdr = bytes[0];
    let has_system = (hdr & 0x40) != 0;
    let has_channel = (hdr & 0x20) != 0;
    let totchan = ((hdr & 0x0F) as usize) + 1;
    let checkpoint_seq = u16::from_be_bytes([bytes[1], bytes[2]]);

    let mut pos = 3;
    let mut recovered = Vec::new();

    if has_system {
        // Skip system journal if present (header is 2 bytes: S | D | V | Q | F | X | LENGTH(10))
        if pos + 2 > bytes.len() {
            return Some(DecodedJournal {
                checkpoint_seq,
                recovered_midi: recovered,
            });
        }
        let sys_len = (((bytes[pos] & 0x03) as usize) << 8) | (bytes[pos + 1] as usize);
        pos = (pos + sys_len).min(bytes.len());
    }

    if !has_channel {
        return Some(DecodedJournal {
            checkpoint_seq,
            recovered_midi: recovered,
        });
    }

    for _ in 0..totchan {
        if pos + 3 > bytes.len() {
            break;
        }
        let c0 = bytes[pos];
        let c1 = bytes[pos + 1];
        let c2 = bytes[pos + 2];
        let chan = (c0 >> 3) & 0x0F;
        let ch_len = (((c0 & 0x03) as usize) << 8) | (c1 as usize);
        let ch_end = (pos + ch_len).min(bytes.len());
        let mut cpos = pos + 3;

        // Chapter P (Program Change, 0x80): 3 bytes if present
        if (c2 & 0x80) != 0 {
            if cpos + 3 <= ch_end {
                let program = bytes[cpos] & 0x7F;
                recovered.push(vec![0xC0 | chan, program]);
                cpos += 3;
            } else {
                pos = ch_end;
                continue;
            }
        }

        // Chapter C (Control Change, 0x40)
        if (c2 & 0x40) != 0 && cpos < ch_end {
            let count = ((bytes[cpos] & 0x7F) as usize) + 1;
            cpos += 1;
            for _ in 0..count {
                if cpos + 2 > ch_end {
                    break;
                }
                let cc_num = bytes[cpos] & 0x7F;
                let cc_val_byte = bytes[cpos + 1];
                let is_toggle = (cc_val_byte & 0x80) != 0;
                let val = cc_val_byte & 0x7F;
                if !is_toggle {
                    recovered.push(vec![0xB0 | chan, cc_num, val]);
                }
                cpos += 2;
            }
        }

        // Chapter M (0x20) and Chapter W (0x10): skip Pitch Wheel (2 bytes) if W set
        if (c2 & 0x10) != 0 && cpos + 2 <= ch_end {
            let lsb = bytes[cpos] & 0x7F;
            let msb = bytes[cpos + 1] & 0x7F;
            recovered.push(vec![0xE0 | chan, lsb, msb]);
            cpos += 2;
        }

        // Chapter N (Note On/Off, 0x08)
        if (c2 & 0x08) != 0 && cpos + 2 <= ch_end {
            let note_log_len = (bytes[cpos] & 0x7F) as usize;
            let low = (bytes[cpos + 1] >> 4) & 0x0F;
            let high = bytes[cpos + 1] & 0x0F;
            cpos += 2;

            for _ in 0..note_log_len {
                if cpos + 2 > ch_end {
                    break;
                }
                let note_num = bytes[cpos] & 0x7F;
                let velocity = bytes[cpos + 1] & 0x7F;
                recovered.push(vec![0x90 | chan, note_num, velocity]);
                cpos += 2;
            }

            if high >= low && high < 16 {
                let bf_len = (high - low + 1) as usize;
                for octet_idx in 0..bf_len {
                    if cpos >= ch_end {
                        break;
                    }
                    let octet = bytes[cpos];
                    cpos += 1;
                    let base_note = ((low as usize) + octet_idx) * 8;
                    for bit in 0..8 {
                        if (octet & (0x80 >> bit)) != 0 {
                            let note_num = (base_note + bit) as u8;
                            if note_num < 128 {
                                recovered.push(vec![0x80 | chan, note_num, 0]);
                            }
                        }
                    }
                }
            }
        }

        pos = ch_end.max(pos + 3);
    }

    Some(DecodedJournal {
        checkpoint_seq,
        recovered_midi: recovered,
    })
}

fn build_note_off_bitfield(off_notes: &[u8]) -> (u8, u8, Vec<u8>) {
    if off_notes.is_empty() {
        return (15, 0, Vec::new()); // low > high indicates no off-bitfield per RFC 6295 §4.2.4
    }
    let min_octet = off_notes.iter().map(|&n| n / 8).min().unwrap_or(0).min(15);
    let max_octet = off_notes.iter().map(|&n| n / 8).max().unwrap_or(0).min(15);
    let mut bf = vec![0u8; (max_octet - min_octet + 1) as usize];
    for &n in off_notes {
        let oct = (n / 8).min(15);
        let bit = n % 8;
        bf[(oct - min_octet) as usize] |= 0x80 >> bit;
    }
    (min_octet, max_octet, bf)
}

/// Wrapping 16-bit RTP sequence number comparison (`a` strictly newer than `b`).
pub fn seq_is_newer(a: u16, b: u16) -> bool {
    let diff = a.wrapping_sub(b);
    diff != 0 && diff < 0x8000
}
