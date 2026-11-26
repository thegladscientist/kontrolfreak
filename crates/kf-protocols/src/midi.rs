use kf_core::{MidiBackendKind, SignalValue};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MidiError {
    #[error("MIDI initialization failed: {0}")]
    Init(String),
    #[error("MIDI port '{0}' not found")]
    PortNotFound(String),
    #[error("Virtual MIDI port creation unsupported on this platform: {0}")]
    VirtualUnsupported(String),
    #[error("MIDI send failed: {0}")]
    Send(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MidiPortInfo {
    pub name: String,
    pub direction: String,
    pub is_virtual: bool,
    pub is_loopmidi: bool,
    pub backend: MidiBackendKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VirtualMidiGuidance {
    pub platform: String,
    pub native_supported: bool,
    pub loopmidi_detected: bool,
    pub recommended_tool: Option<String>,
    pub download_url: Option<String>,
    pub steps: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MidiMessage {
    ControlChange { ch: u8, cc: u8, val: u8 },
    NoteOn { ch: u8, note: u8, vel: u8 },
    NoteOff { ch: u8, note: u8, vel: u8 },
    PitchBend { ch: u8, value: u16 },
    ProgramChange { ch: u8, program: u8 },
    Clock,
    Raw(Vec<u8>),
}

impl MidiMessage {
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.is_empty() {
            return None;
        }
        let status = bytes[0];
        if status == 0xF8 {
            return Some(MidiMessage::Clock);
        }
        let kind = status & 0xF0;
        let ch = (status & 0x0F) + 1;
        match kind {
            0xB0 if bytes.len() >= 3 => Some(MidiMessage::ControlChange {
                ch,
                cc: bytes[1] & 0x7F,
                val: bytes[2] & 0x7F,
            }),
            0x90 if bytes.len() >= 3 => {
                let vel = bytes[2] & 0x7F;
                if vel == 0 {
                    Some(MidiMessage::NoteOff {
                        ch,
                        note: bytes[1] & 0x7F,
                        vel: 0,
                    })
                } else {
                    Some(MidiMessage::NoteOn {
                        ch,
                        note: bytes[1] & 0x7F,
                        vel,
                    })
                }
            }
            0x80 if bytes.len() >= 3 => Some(MidiMessage::NoteOff {
                ch,
                note: bytes[1] & 0x7F,
                vel: bytes[2] & 0x7F,
            }),
            0xE0 if bytes.len() >= 3 => {
                let lsb = (bytes[1] & 0x7F) as u16;
                let msb = (bytes[2] & 0x7F) as u16;
                Some(MidiMessage::PitchBend {
                    ch,
                    value: (msb << 7) | lsb,
                })
            }
            0xC0 if bytes.len() >= 2 => Some(MidiMessage::ProgramChange {
                ch,
                program: bytes[1] & 0x7F,
            }),
            _ => Some(MidiMessage::Raw(bytes.to_vec())),
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        match self {
            MidiMessage::ControlChange { ch, cc, val } => {
                vec![0xB0 | ((ch.saturating_sub(1)) & 0x0F), cc & 0x7F, val & 0x7F]
            }
            MidiMessage::NoteOn { ch, note, vel } => {
                vec![
                    0x90 | ((ch.saturating_sub(1)) & 0x0F),
                    note & 0x7F,
                    vel & 0x7F,
                ]
            }
            MidiMessage::NoteOff { ch, note, vel } => {
                vec![
                    0x80 | ((ch.saturating_sub(1)) & 0x0F),
                    note & 0x7F,
                    vel & 0x7F,
                ]
            }
            MidiMessage::PitchBend { ch, value } => {
                let v = (*value).min(16383);
                vec![
                    0xE0 | ((ch.saturating_sub(1)) & 0x0F),
                    (v & 0x7F) as u8,
                    ((v >> 7) & 0x7F) as u8,
                ]
            }
            MidiMessage::ProgramChange { ch, program } => {
                vec![0xC0 | ((ch.saturating_sub(1)) & 0x0F), program & 0x7F]
            }
            MidiMessage::Clock => vec![0xF8],
            MidiMessage::Raw(b) => b.clone(),
        }
    }

    pub fn to_logical_key_and_value(&self) -> (String, SignalValue) {
        match self {
            MidiMessage::ControlChange { ch, cc, val } => (
                format!("cc.{ch}.{cc}"),
                SignalValue::Float(*val as f64 / 127.0),
            ),
            MidiMessage::NoteOn { ch, note, vel } => (
                format!("note.{ch}.{note}"),
                SignalValue::Float(*vel as f64 / 127.0),
            ),
            MidiMessage::NoteOff { ch, note, .. } => {
                (format!("note.{ch}.{note}"), SignalValue::Float(0.0))
            }
            MidiMessage::PitchBend { ch, value } => (
                format!("pitch.{ch}"),
                SignalValue::Float(*value as f64 / 16383.0),
            ),
            MidiMessage::ProgramChange { ch, program } => (
                format!("pc.{ch}"),
                SignalValue::Int(*program as i64),
            ),
            MidiMessage::Clock => ("midi.clock".to_string(), SignalValue::Trigger),
            MidiMessage::Raw(b) => ("midi.raw".to_string(), SignalValue::bytes(b.clone())),
        }
    }

    /// Converts a logical key (`cc.1.7`, `note.1.60`, `pitch.1`, `pc.1`) + `SignalValue` into MIDI.
    pub fn from_logical_key_and_value(key: &str, value: &SignalValue) -> Self {
        let parts: Vec<&str> = key.split('.').collect();
        let norm = value.as_f64().unwrap_or(0.0);
        // Accept either normalized 0..1 or raw 0..127 if value > 1.0
        let midi7 = if norm > 1.0 {
            norm.round().clamp(0.0, 127.0) as u8
        } else {
            (norm.clamp(0.0, 1.0) * 127.0).round() as u8
        };

        match parts.as_slice() {
            ["cc", ch_s, cc_s] => {
                let ch = ch_s.parse::<u8>().unwrap_or(1).clamp(1, 16);
                let cc = cc_s.parse::<u8>().unwrap_or(1).min(127);
                MidiMessage::ControlChange { ch, cc, val: midi7 }
            }
            ["note", ch_s, note_s] => {
                let ch = ch_s.parse::<u8>().unwrap_or(1).clamp(1, 16);
                let note = note_s.parse::<u8>().unwrap_or(60).min(127);
                if midi7 == 0 {
                    MidiMessage::NoteOff { ch, note, vel: 0 }
                } else {
                    MidiMessage::NoteOn {
                        ch,
                        note,
                        vel: midi7,
                    }
                }
            }
            ["pitch", ch_s] => {
                let ch = ch_s.parse::<u8>().unwrap_or(1).clamp(1, 16);
                let bend = if norm > 1.0 {
                    norm.round().clamp(0.0, 16383.0) as u16
                } else {
                    (norm.clamp(0.0, 1.0) * 16383.0).round() as u16
                };
                MidiMessage::PitchBend { ch, value: bend }
            }
            ["pc", ch_s] => {
                let ch = ch_s.parse::<u8>().unwrap_or(1).clamp(1, 16);
                let program = norm.round().clamp(0.0, 127.0) as u8;
                MidiMessage::ProgramChange { ch, program }
            }
            _ => MidiMessage::ControlChange {
                ch: 1,
                cc: 1,
                val: midi7,
            },
        }
    }
}

/// Platform abstraction for Virtual MIDI ports:
/// - macOS: native CoreMIDI virtual input/output (`midir::os::unix`)
/// - Windows: `loopMIDI` port auto-detection + guided setup (future: Windows MIDI Services)
/// - Linux / Headless CI: ALSA virtual port or in-memory `LoopbackMidiBackend`
pub trait VirtualMidiProvider: Send + Sync {
    fn platform_name(&self) -> &'static str;
    fn supports_native_virtual_ports(&self) -> bool;
    fn list_virtual_ports(&self) -> Vec<MidiPortInfo>;
    fn setup_guidance(&self) -> VirtualMidiGuidance;
}

pub fn is_loopmidi_port_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("loopmidi") || lower.contains("kontrolfreak virtual")
}

/// Lists hardware MIDI ports via `midir`, returning an empty list cleanly if the OS audio/MIDI
/// subsystem is unavailable (e.g. headless Linux CI containers).
pub fn list_hardware_midi_ports() -> Vec<MidiPortInfo> {
    let mut ports = Vec::new();
    if let Ok(midi_in) = midir::MidiInput::new("kontrolfreak-enum-in") {
        for p in midi_in.ports() {
            if let Ok(name) = midi_in.port_name(&p) {
                let is_loop = is_loopmidi_port_name(&name);
                ports.push(MidiPortInfo {
                    name,
                    direction: "in".into(),
                    is_virtual: is_loop,
                    is_loopmidi: is_loop,
                    backend: if is_loop {
                        MidiBackendKind::LoopMidiWindows
                    } else {
                        MidiBackendKind::Hardware
                    },
                });
            }
        }
    }
    if let Ok(midi_out) = midir::MidiOutput::new("kontrolfreak-enum-out") {
        for p in midi_out.ports() {
            if let Ok(name) = midi_out.port_name(&p) {
                let is_loop = is_loopmidi_port_name(&name);
                ports.push(MidiPortInfo {
                    name,
                    direction: "out".into(),
                    is_virtual: is_loop,
                    is_loopmidi: is_loop,
                    backend: if is_loop {
                        MidiBackendKind::LoopMidiWindows
                    } else {
                        MidiBackendKind::Hardware
                    },
                });
            }
        }
    }
    ports
}

#[cfg(target_os = "macos")]
pub struct CoreMidiVirtualProvider;

#[cfg(target_os = "macos")]
impl CoreMidiVirtualProvider {
    pub fn create_virtual_input<F>(
        &self,
        port_name: &str,
        callback: F,
    ) -> Result<midir::MidiInputConnection<()>, MidiError>
    where
        F: FnMut(u64, &[u8], &mut ()) + Send + 'static,
    {
        use midir::os::unix::VirtualInput;
        let midi_in =
            midir::MidiInput::new("Kontrolfreak").map_err(|e| MidiError::Init(e.to_string()))?;
        midi_in
            .create_virtual(port_name, callback, ())
            .map_err(|e| MidiError::Init(e.to_string()))
    }

    pub fn create_virtual_output(
        &self,
        port_name: &str,
    ) -> Result<midir::MidiOutputConnection, MidiError> {
        use midir::os::unix::VirtualOutput;
        let midi_out =
            midir::MidiOutput::new("Kontrolfreak").map_err(|e| MidiError::Init(e.to_string()))?;
        midi_out
            .create_virtual(port_name)
            .map_err(|e| MidiError::Init(e.to_string()))
    }
}

#[cfg(target_os = "macos")]
impl VirtualMidiProvider for CoreMidiVirtualProvider {
    fn platform_name(&self) -> &'static str {
        "macos-coremidi"
    }

    fn supports_native_virtual_ports(&self) -> bool {
        true
    }

    fn list_virtual_ports(&self) -> Vec<MidiPortInfo> {
        list_hardware_midi_ports()
            .into_iter()
            .filter(|p| p.is_virtual)
            .collect()
    }

    fn setup_guidance(&self) -> VirtualMidiGuidance {
        VirtualMidiGuidance {
            platform: "macOS".into(),
            native_supported: true,
            loopmidi_detected: false,
            recommended_tool: None,
            download_url: None,
            steps: vec![
                "macOS CoreMIDI supports native virtual MIDI ports directly inside Kontrolfreak."
                    .into(),
                "Enable 'Virtual Port' on any MIDI node to publish it to Ableton, Resolume, or TouchDesigner."
                    .into(),
            ],
        }
    }
}

#[cfg(target_os = "windows")]
pub struct WindowsLoopMidiProvider;

#[cfg(target_os = "windows")]
impl VirtualMidiProvider for WindowsLoopMidiProvider {
    fn platform_name(&self) -> &'static str {
        "windows-loopmidi"
    }

    fn supports_native_virtual_ports(&self) -> bool {
        false
    }

    fn list_virtual_ports(&self) -> Vec<MidiPortInfo> {
        list_hardware_midi_ports()
            .into_iter()
            .filter(|p| p.is_loopmidi)
            .collect()
    }

    fn setup_guidance(&self) -> VirtualMidiGuidance {
        let detected = !self.list_virtual_ports().is_empty();
        VirtualMidiGuidance {
            platform: "Windows".into(),
            native_supported: false,
            loopmidi_detected: detected,
            recommended_tool: Some("loopMIDI by Tobias Erichsen".into()),
            download_url: Some("https://www.tobias-erichsen.de/software/loopmidi.html".into()),
            steps: if detected {
                vec![
                    "loopMIDI virtual port(s) detected and tagged as Virtual MIDI.".into(),
                    "Avoid wiring the same loopMIDI port back into itself to prevent loopback storms."
                        .into(),
                ]
            } else {
                vec![
                    "Install loopMIDI (free Windows virtual MIDI loopback driver).".into(),
                    "Create a port named 'Kontrolfreak Virtual 1' inside loopMIDI.".into(),
                    "Click Refresh Ports in the sidebar — Kontrolfreak will tag it automatically."
                        .into(),
                ]
            },
        }
    }
}

/// Cross-platform helper generating Windows loopMIDI setup guidance (also used by UI & tests
/// regardless of host OS so the guidance UI is always inspectable and testable).
pub fn windows_loopmidi_guidance(detected_ports: &[MidiPortInfo]) -> VirtualMidiGuidance {
    let detected = detected_ports.iter().any(|p| p.is_loopmidi);
    VirtualMidiGuidance {
        platform: "Windows".into(),
        native_supported: false,
        loopmidi_detected: detected,
        recommended_tool: Some("loopMIDI by Tobias Erichsen".into()),
        download_url: Some("https://www.tobias-erichsen.de/software/loopmidi.html".into()),
        steps: if detected {
            vec![
                "loopMIDI virtual port(s) detected and tagged as Virtual MIDI.".into(),
                "Kontrolfreak automatically guards paired ports against accidental self-loopback."
                    .into(),
            ]
        } else {
            vec![
                "Install loopMIDI by Tobias Erichsen for virtual MIDI loopback on Windows.".into(),
                "Add a port (e.g. 'Kontrolfreak Virtual 1') in loopMIDI.".into(),
                "Click Refresh Hardware in the sidebar to bind it instantly.".into(),
            ]
        },
    }
}

#[cfg(target_os = "linux")]
pub struct AlsaVirtualMidiProvider;

#[cfg(target_os = "linux")]
impl VirtualMidiProvider for AlsaVirtualMidiProvider {
    fn platform_name(&self) -> &'static str {
        "linux-alsa-virtual"
    }

    fn supports_native_virtual_ports(&self) -> bool {
        true
    }

    fn list_virtual_ports(&self) -> Vec<MidiPortInfo> {
        list_hardware_midi_ports()
            .into_iter()
            .filter(|p| p.is_virtual)
            .collect()
    }

    fn setup_guidance(&self) -> VirtualMidiGuidance {
        VirtualMidiGuidance {
            platform: "Linux".into(),
            native_supported: true,
            loopmidi_detected: false,
            recommended_tool: Some("ALSA Sequencer / In-Memory Loopback".into()),
            download_url: None,
            steps: vec![
                "Linux ALSA virtual ports are supported when /dev/snd/seq is available.".into(),
                "In headless environments, Kontrolfreak falls back cleanly to its in-memory Loopback MIDI backend."
                    .into(),
            ],
        }
    }
}

pub fn default_virtual_midi_provider() -> Box<dyn VirtualMidiProvider> {
    #[cfg(target_os = "macos")]
    {
        Box::new(CoreMidiVirtualProvider)
    }
    #[cfg(target_os = "windows")]
    {
        Box::new(WindowsLoopMidiProvider)
    }
    #[cfg(target_os = "linux")]
    {
        Box::new(AlsaVirtualMidiProvider)
    }
}

type MidiSubscriber = Arc<dyn Fn(&[u8]) + Send + Sync>;

/// In-memory zero-latency loopback/mock MIDI backend for headless CI, automated testing,
/// and local virtual patching without OS kernel drivers.
#[derive(Default)]
pub struct LoopbackMidiBackend {
    ports: Mutex<HashMap<String, Vec<MidiSubscriber>>>,
    captured: Mutex<HashMap<String, Vec<Vec<u8>>>>,
}

impl LoopbackMidiBackend {
    pub fn new() -> Arc<Self> {
        let backend = Arc::new(Self::default());
        backend.ensure_port("Kontrolfreak Loopback 1");
        backend
    }

    pub fn ensure_port(&self, name: &str) {
        let mut ports = self.ports.lock();
        ports.entry(name.to_string()).or_default();
    }

    pub fn list_ports(&self) -> Vec<MidiPortInfo> {
        let ports = self.ports.lock();
        ports
            .keys()
            .map(|name| MidiPortInfo {
                name: name.clone(),
                direction: "bidir".into(),
                is_virtual: true,
                is_loopmidi: false,
                backend: MidiBackendKind::LoopbackMock,
            })
            .collect()
    }

    pub fn subscribe(&self, port_name: &str, cb: MidiSubscriber) {
        let mut ports = self.ports.lock();
        ports.entry(port_name.to_string()).or_default().push(cb);
    }

    pub fn send(&self, port_name: &str, bytes: &[u8]) {
        {
            let mut cap = self.captured.lock();
            cap.entry(port_name.to_string())
                .or_default()
                .push(bytes.to_vec());
        }
        let subscribers: Vec<MidiSubscriber> = {
            let ports = self.ports.lock();
            ports.get(port_name).cloned().unwrap_or_default()
        };
        for cb in subscribers {
            cb(bytes);
        }
    }

    pub fn take_captured(&self, port_name: &str) -> Vec<Vec<u8>> {
        let mut cap = self.captured.lock();
        cap.remove(port_name).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn midi_message_roundtrip_and_logical_keys() {
        let msg = MidiMessage::ControlChange {
            ch: 2,
            cc: 74,
            val: 127,
        };
        let bytes = msg.encode();
        assert_eq!(bytes, vec![0xB1, 74, 127]);
        let decoded = MidiMessage::decode(&bytes).unwrap();
        assert_eq!(decoded, msg);
        let (key, val) = decoded.to_logical_key_and_value();
        assert_eq!(key, "cc.2.74");
        assert_eq!(val, SignalValue::Float(1.0));

        let rebuilt = MidiMessage::from_logical_key_and_value("cc.2.74", &SignalValue::Float(0.5));
        assert_eq!(
            rebuilt,
            MidiMessage::ControlChange {
                ch: 2,
                cc: 74,
                val: 64
            }
        );
    }

    #[test]
    fn windows_loopmidi_detection_and_guidance() {
        let none = windows_loopmidi_guidance(&[]);
        assert!(!none.loopmidi_detected);
        assert!(none.recommended_tool.as_deref().unwrap().contains("loopMIDI"));

        let with_loop = windows_loopmidi_guidance(&[MidiPortInfo {
            name: "loopMIDI Port 1".into(),
            direction: "out".into(),
            is_virtual: true,
            is_loopmidi: true,
            backend: MidiBackendKind::LoopMidiWindows,
        }]);
        assert!(with_loop.loopmidi_detected);
    }
}
