//! Cross-platform HID backend trait, realistic `SimulatedHidBackend`, hot-plug watcher,
//! and optional `hidapi`-backed native implementation (`native-hid` feature).

use crate::hid::descriptor::sample_studio_fader_descriptor;
use crate::{HardwareError, HardwareHotplugEvent, HardwareWatcher, HidReportSample};
use kf_core::{DeviceFingerprint, HidDeviceInfo, SignalEvent};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Converts a `HidDeviceInfo` into a `kf_core::DeviceFingerprint`.
pub fn fingerprint_from_hid_info(info: &HidDeviceInfo) -> DeviceFingerprint {
    DeviceFingerprint {
        vid: Some(info.vid),
        pid: Some(info.pid),
        serial: info.serial.clone(),
        usage_page: Some(info.usage_page),
        usage: Some(info.usage),
        interface_number: Some(info.interface_number),
        port_name: info.product_name.clone(),
        endpoint: Some(info.path.clone()),
    }
}

/// Cross-platform HID enumeration & report reading backend.
pub trait HidBackend: Send + Sync {
    fn enumerate(&self) -> Result<Vec<HidDeviceInfo>, HardwareError>;
    fn read_report(&self, path: &str) -> Result<Option<HidReportSample>, HardwareError>;
    fn report_descriptor(&self, path: &str) -> Result<Option<Vec<u8>>, HardwareError>;
    fn check_os_permissions(&self) -> Result<(), HardwareError>;
}

/// Simulated peripheral archetypes for deterministic testing and hardware-free demos.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SimulatedHidProfile {
    /// Quirky controller with noisy IMU bytes, rolling counter, 8-bit fader, 16-bit LE signed
    /// joystick, 16-bit BE unsigned dial, momentary/toggle buttons, hat switch & relative encoder.
    QuirkyImuGamepad,
    /// Standard descriptor-backed USB Fader Wing with 4 faders, 16-bit dial, wheel, and 4 buttons.
    StudioFaderWing,
    /// Unknown vendor-defined mystery rotary HID peripheral for hot-plug Learn Wizard demos.
    UnknownMysteryDial,
}

/// Control state of a simulated virtual HID peripheral.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulatedHidControlState {
    /// Byte 6 (Quirky/Unknown) or Byte 1 (FaderWing): 8-bit unsigned fader (0..=255)
    pub fader_u8: u8,
    /// Bytes 7..=8: 16-bit signed Little-Endian joystick X (-32768..=32767)
    pub axis_i16_le: i16,
    /// Bytes 9..=10: 16-bit unsigned Big-Endian precision dial (0..=65535)
    pub dial_u16_be: u16,
    /// Byte 11 bit 0: Momentary Button A
    pub button_a: bool,
    /// Byte 11 bit 1: Momentary Button B
    pub button_b: bool,
    /// Byte 11 bit 2: Toggle Switch C
    pub toggle_c: bool,
    /// Byte 11 bits 4..7: 4-bit Hat Switch (0..=7 directions, 8 = centered/null)
    pub hat_switch: u8,
    /// Byte 12: One-shot relative encoder delta (-127..=127), auto-resets to 0 after read
    pub pending_encoder_delta: i8,
    /// Enable/disable simulated IMU & counter noise on idle bytes (true by default)
    pub noise_enabled: bool,
    /// Internal deterministic packet counter / LCG seed
    pub tick: u32,
}

impl Default for SimulatedHidControlState {
    fn default() -> Self {
        Self {
            fader_u8: 0,
            axis_i16_le: 0,
            dial_u16_be: 0,
            button_a: false,
            button_b: false,
            toggle_c: false,
            hat_switch: 8, // centered
            pending_encoder_delta: 0,
            noise_enabled: true,
            tick: 0,
        }
    }
}

/// Action to actuate a virtual control on a simulated HID peripheral.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "control", rename_all = "snake_case")]
pub enum SimulatedHidAction {
    SetFaderU8 { value: u8 },
    SetAxisI16Le { value: i16 },
    SetDialU16Be { value: u16 },
    SetButtonA { pressed: bool },
    SetButtonB { pressed: bool },
    ToggleSwitchC,
    SetHatSwitch { direction: u8 },
    PulseEncoder { delta: i8 },
    SetNoiseEnabled { enabled: bool },
}

#[derive(Debug, Clone)]
struct SimulatedDeviceEntry {
    profile: SimulatedHidProfile,
    info: HidDeviceInfo,
    descriptor: Option<Vec<u8>>,
    state: SimulatedHidControlState,
}

impl SimulatedDeviceEntry {
    fn from_profile(profile: SimulatedHidProfile, path_suffix: &str) -> Self {
        match profile {
            SimulatedHidProfile::QuirkyImuGamepad => Self {
                profile,
                info: HidDeviceInfo {
                    path: format!("sim://hid/quirky-imu-pad-{path_suffix}"),
                    vid: 0x256F,
                    pid: 0xC635,
                    serial: Some(format!("KF-QPAD-{path_suffix}")),
                    product_name: Some("KF QuirkyPad Pro (IMU + Encoders)".into()),
                    manufacturer: Some("Kontrolfreak Virtual Hardware".into()),
                    usage_page: 0xFF00,
                    usage: 0x0001,
                    interface_number: 0,
                },
                descriptor: None, // Vendor-opaque: exercises Tier 2 Learn Engine!
                state: SimulatedHidControlState::default(),
            },
            SimulatedHidProfile::StudioFaderWing => Self {
                profile,
                info: HidDeviceInfo {
                    path: format!("sim://hid/studio-fader-wing-{path_suffix}"),
                    vid: 0x16C0,
                    pid: 0x0486,
                    serial: Some(format!("KF-FADER-{path_suffix}")),
                    product_name: Some("KF StudioFader 4 (HID Descriptor)".into()),
                    manufacturer: Some("Kontrolfreak Virtual Hardware".into()),
                    usage_page: 0x0001,
                    usage: 0x0004,
                    interface_number: 0,
                },
                descriptor: Some(sample_studio_fader_descriptor()),
                state: SimulatedHidControlState::default(),
            },
            SimulatedHidProfile::UnknownMysteryDial => Self {
                profile,
                info: HidDeviceInfo {
                    path: format!("sim://hid/unknown-mystery-dial-{path_suffix}"),
                    vid: 0x1D50,
                    pid: 0x614E,
                    serial: Some(format!("MYST-{path_suffix}")),
                    product_name: Some("Unknown USB Rotary HID (0x1D50:0x614E)".into()),
                    manufacturer: Some("Generic Vendor HID".into()),
                    usage_page: 0xFF01,
                    usage: 0x0002,
                    interface_number: 1,
                },
                descriptor: None,
                state: SimulatedHidControlState::default(),
            },
        }
    }

    fn next_report(&mut self) -> HidReportSample {
        self.state.tick = self.state.tick.wrapping_add(1);
        let t = self.state.tick;

        // Deterministic PRNG noise for realistic IMU gyro/accel jitter & timestamp bytes
        let lcg = t.wrapping_mul(1664525).wrapping_add(1013904223);
        let imu_ax: i16 = if self.state.noise_enabled {
            let jitter = ((lcg >> 16) & 0x0F) as i16 - 8;
            120 + jitter
        } else {
            120
        };
        let imu_ay: i16 = if self.state.noise_enabled {
            let jitter = ((lcg >> 20) & 0x0F) as i16 - 8;
            -40 + jitter
        } else {
            -40
        };

        let bytes = match self.profile {
            SimulatedHidProfile::QuirkyImuGamepad | SimulatedHidProfile::UnknownMysteryDial => {
                let mut pkt = vec![0u8; 16];
                pkt[0] = 0x01; // Report ID 1
                pkt[1] = if self.state.noise_enabled {
                    (t & 0xFF) as u8 // Rolling sequence counter (noise!)
                } else {
                    0
                };
                // Bytes 2..=5: Noisy 2-axis IMU accel/gyro bytes
                let ax_b = imu_ax.to_le_bytes();
                let ay_b = imu_ay.to_le_bytes();
                pkt[2] = ax_b[0];
                pkt[3] = ax_b[1];
                pkt[4] = ay_b[0];
                pkt[5] = ay_b[1];
                // Byte 6: 8-bit unsigned fader
                pkt[6] = self.state.fader_u8;
                // Bytes 7..=8: 16-bit signed LE joystick axis
                let le_bytes = self.state.axis_i16_le.to_le_bytes();
                pkt[7] = le_bytes[0];
                pkt[8] = le_bytes[1];
                // Bytes 9..=10: 16-bit unsigned BE precision dial
                let be_bytes = self.state.dial_u16_be.to_be_bytes();
                pkt[9] = be_bytes[0];
                pkt[10] = be_bytes[1];
                // Byte 11: Buttons (bits 0..2) + Hat Switch (bits 4..7)
                let mut btn_byte = 0u8;
                if self.state.button_a {
                    btn_byte |= 0x01;
                }
                if self.state.button_b {
                    btn_byte |= 0x02;
                }
                if self.state.toggle_c {
                    btn_byte |= 0x04;
                }
                btn_byte |= (self.state.hat_switch & 0x0F) << 4;
                pkt[11] = btn_byte;
                // Byte 12: Relative rotary encoder delta (consumes pending delta once)
                pkt[12] = self.state.pending_encoder_delta as u8;
                self.state.pending_encoder_delta = 0;
                // Bytes 13..=15: Noisy timestamp LSB / constant footer
                pkt[13] = if self.state.noise_enabled {
                    ((t * 7) & 0x03) as u8
                } else {
                    0
                };
                pkt[14] = 0xAA;
                pkt[15] = 0x55;
                pkt
            }
            SimulatedHidProfile::StudioFaderWing => {
                // Matches `sample_studio_fader_descriptor()` bit layout exactly:
                // Byte 0: Report ID (1)
                // Bytes 1..=4: 4 x 8-bit faders
                // Bytes 5..=6: 1 x 16-bit signed LE Dial
                // Byte 7: 1 x 8-bit relative Wheel
                // Byte 8: bits 0..3 = 4 buttons, bits 4..7 = constant 0
                let mut pkt = vec![0u8; 9];
                pkt[0] = 0x01;
                pkt[1] = self.state.fader_u8;
                pkt[2] = (self.state.dial_u16_be >> 8) as u8;
                pkt[3] = 128;
                pkt[4] = 64;
                let le_bytes = self.state.axis_i16_le.to_le_bytes();
                pkt[5] = le_bytes[0];
                pkt[6] = le_bytes[1];
                pkt[7] = self.state.pending_encoder_delta as u8;
                self.state.pending_encoder_delta = 0;
                let mut b = 0u8;
                if self.state.button_a {
                    b |= 0x01;
                }
                if self.state.button_b {
                    b |= 0x02;
                }
                if self.state.toggle_c {
                    b |= 0x04;
                }
                pkt[8] = b;
                pkt
            }
        };

        HidReportSample {
            report_id: bytes[0],
            bytes,
            timestamp_ns: SignalEvent::now_ns(),
        }
    }
}

/// Built-in deterministic virtual HID backend so the full plug-and-play Learn workflow,
/// noise-masking, and unplug/replug auto-rebind work without physical USB hardware.
#[derive(Debug, Default)]
pub struct SimulatedHidBackend {
    devices: RwLock<HashMap<String, SimulatedDeviceEntry>>,
    counter: RwLock<u32>,
}

impl SimulatedHidBackend {
    pub fn new() -> Self {
        Self::default()
    }

    /// Plugs in a simulated peripheral and returns its `HidDeviceInfo`.
    pub fn plug_in(&self, profile: SimulatedHidProfile) -> HidDeviceInfo {
        let mut count = self.counter.write();
        *count += 1;
        let suffix = format!("{:02}", *count);
        let entry = SimulatedDeviceEntry::from_profile(profile, &suffix);
        let info = entry.info.clone();
        self.devices.write().insert(info.path.clone(), entry);
        info
    }

    /// Plugs in a simulated peripheral with a deterministic serial/path (great for unplug/replug tests).
    pub fn plug_in_with_id(&self, profile: SimulatedHidProfile, stable_id: &str) -> HidDeviceInfo {
        let entry = SimulatedDeviceEntry::from_profile(profile, stable_id);
        let info = entry.info.clone();
        self.devices.write().insert(info.path.clone(), entry);
        info
    }

    /// Unplugs a simulated peripheral by path. Returns its `HidDeviceInfo` if it was connected.
    pub fn unplug(&self, path: &str) -> Option<HidDeviceInfo> {
        self.devices.write().remove(path).map(|e| e.info)
    }

    /// Actuates a virtual control on a connected simulated HID device.
    pub fn actuate(&self, path: &str, action: SimulatedHidAction) -> Result<(), HardwareError> {
        let mut map = self.devices.write();
        let entry = map
            .get_mut(path)
            .ok_or_else(|| HardwareError::Backend(format!("Simulated HID device not found: {path}")))?;
        match action {
            SimulatedHidAction::SetFaderU8 { value } => entry.state.fader_u8 = value,
            SimulatedHidAction::SetAxisI16Le { value } => entry.state.axis_i16_le = value,
            SimulatedHidAction::SetDialU16Be { value } => entry.state.dial_u16_be = value,
            SimulatedHidAction::SetButtonA { pressed } => entry.state.button_a = pressed,
            SimulatedHidAction::SetButtonB { pressed } => entry.state.button_b = pressed,
            SimulatedHidAction::ToggleSwitchC => entry.state.toggle_c = !entry.state.toggle_c,
            SimulatedHidAction::SetHatSwitch { direction } => {
                entry.state.hat_switch = direction.min(15)
            }
            SimulatedHidAction::PulseEncoder { delta } => {
                entry.state.pending_encoder_delta = delta;
            }
            SimulatedHidAction::SetNoiseEnabled { enabled } => {
                entry.state.noise_enabled = enabled;
            }
        }
        Ok(())
    }

    /// Reads the current control state of a simulated device (useful for UI verification).
    pub fn control_state(&self, path: &str) -> Option<SimulatedHidControlState> {
        self.devices.read().get(path).map(|e| e.state.clone())
    }
}

impl HidBackend for SimulatedHidBackend {
    fn enumerate(&self) -> Result<Vec<HidDeviceInfo>, HardwareError> {
        let mut list: Vec<HidDeviceInfo> = self
            .devices
            .read()
            .values()
            .map(|e| e.info.clone())
            .collect();
        list.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(list)
    }

    fn read_report(&self, path: &str) -> Result<Option<HidReportSample>, HardwareError> {
        let mut map = self.devices.write();
        let Some(entry) = map.get_mut(path) else {
            return Ok(None);
        };
        Ok(Some(entry.next_report()))
    }

    fn report_descriptor(&self, path: &str) -> Result<Option<Vec<u8>>, HardwareError> {
        let map = self.devices.read();
        Ok(map.get(path).and_then(|e| e.descriptor.clone()))
    }

    fn check_os_permissions(&self) -> Result<(), HardwareError> {
        Ok(())
    }
}

/// Optional real OS HID backend backed by `hidapi` (enabled via `native-hid` Cargo feature).
#[cfg(feature = "native-hid")]
pub struct HidApiBackend;

#[cfg(feature = "native-hid")]
impl HidBackend for HidApiBackend {
    fn enumerate(&self) -> Result<Vec<HidDeviceInfo>, HardwareError> {
        let api = hidapi::HidApi::new()
            .map_err(|e| HardwareError::Backend(format!("hidapi init failed: {e}")))?;
        let mut out = Vec::new();
        for d in api.device_list() {
            out.push(HidDeviceInfo {
                path: d.path().to_string_lossy().into_owned(),
                vid: d.vendor_id(),
                pid: d.product_id(),
                serial: d.serial_number().map(|s| s.to_string()),
                product_name: d.product_string().map(|s| s.to_string()),
                manufacturer: d.manufacturer_string().map(|s| s.to_string()),
                usage_page: d.usage_page(),
                usage: d.usage(),
                interface_number: d.interface_number(),
            });
        }
        Ok(out)
    }

    fn read_report(&self, path: &str) -> Result<Option<HidReportSample>, HardwareError> {
        let api = hidapi::HidApi::new()
            .map_err(|e| HardwareError::Backend(format!("hidapi init failed: {e}")))?;
        let c_path = std::ffi::CString::new(path)
            .map_err(|e| HardwareError::Backend(format!("invalid hid path: {e}")))?;
        let dev = api
            .open_path(c_path.as_c_str())
            .map_err(|e| HardwareError::Backend(format!("hid open failed: {e}")))?;
        let mut buf = [0u8; 64];
        let n = dev
            .read_timeout(&mut buf, 20)
            .map_err(|e| HardwareError::Backend(format!("hid read failed: {e}")))?;
        if n == 0 {
            return Ok(None);
        }
        Ok(Some(HidReportSample {
            report_id: buf[0],
            bytes: buf[..n].to_vec(),
            timestamp_ns: SignalEvent::now_ns(),
        }))
    }

    fn report_descriptor(&self, path: &str) -> Result<Option<Vec<u8>>, HardwareError> {
        let api = hidapi::HidApi::new()
            .map_err(|e| HardwareError::Backend(format!("hidapi init failed: {e}")))?;
        let c_path = std::ffi::CString::new(path)
            .map_err(|e| HardwareError::Backend(format!("invalid hid path: {e}")))?;
        let dev = api
            .open_path(c_path.as_c_str())
            .map_err(|e| HardwareError::Backend(format!("hid open failed: {e}")))?;
        let mut buf = [0u8; 4096];
        match dev.get_report_descriptor(&mut buf) {
            Ok(len) if len > 0 => Ok(Some(buf[..len].to_vec())),
            _ => Ok(None),
        }
    }

    fn check_os_permissions(&self) -> Result<(), HardwareError> {
        #[cfg(target_os = "macos")]
        {
            if hidapi::HidApi::new().is_err() {
                return Err(HardwareError::PermissionRequired(
                    "Grant Input Monitoring in macOS System Settings -> Privacy & Security -> Input Monitoring".into(),
                ));
            }
        }
        Ok(())
    }
}

/// Composite backend merging simulated HID devices with any optional native backend.
pub struct CompositeHidBackend {
    simulated: Arc<SimulatedHidBackend>,
    native: Option<Arc<dyn HidBackend>>,
}

impl CompositeHidBackend {
    pub fn new(simulated: Arc<SimulatedHidBackend>, native: Option<Arc<dyn HidBackend>>) -> Self {
        Self { simulated, native }
    }

    pub fn simulated(&self) -> Arc<SimulatedHidBackend> {
        Arc::clone(&self.simulated)
    }
}

impl HidBackend for CompositeHidBackend {
    fn enumerate(&self) -> Result<Vec<HidDeviceInfo>, HardwareError> {
        let mut list = self.simulated.enumerate()?;
        if let Some(native) = &self.native {
            if let Ok(native_list) = native.enumerate() {
                list.extend(native_list);
            }
        }
        Ok(list)
    }

    fn read_report(&self, path: &str) -> Result<Option<HidReportSample>, HardwareError> {
        if path.starts_with("sim://") {
            self.simulated.read_report(path)
        } else if let Some(native) = &self.native {
            native.read_report(path)
        } else {
            Ok(None)
        }
    }

    fn report_descriptor(&self, path: &str) -> Result<Option<Vec<u8>>, HardwareError> {
        if path.starts_with("sim://") {
            self.simulated.report_descriptor(path)
        } else if let Some(native) = &self.native {
            native.report_descriptor(path)
        } else {
            Ok(None)
        }
    }

    fn check_os_permissions(&self) -> Result<(), HardwareError> {
        if let Some(native) = &self.native {
            native.check_os_permissions()
        } else {
            self.simulated.check_os_permissions()
        }
    }
}

/// Diff-based hot-plug watcher tracking arrival and removal of HID devices.
pub struct HidHotplugWatcher {
    backend: Arc<dyn HidBackend>,
    known_connected: RwLock<HashMap<String, HidDeviceInfo>>,
}

impl HidHotplugWatcher {
    pub fn new(backend: Arc<dyn HidBackend>) -> Self {
        Self {
            backend,
            known_connected: RwLock::new(HashMap::new()),
        }
    }

    /// Polls the backend, diffs against previously connected paths, and emits
    /// `HardwareHotplugEvent::Connected` / `Disconnected`.
    pub fn poll_events(&self) -> Result<Vec<HardwareHotplugEvent>, HardwareError> {
        let current = self.backend.enumerate()?;
        let mut known = self.known_connected.write();

        let current_paths: HashSet<&str> = current.iter().map(|d| d.path.as_str()).collect();
        let mut events = Vec::new();

        // Detect removals first
        let removed_paths: Vec<String> = known
            .keys()
            .filter(|p| !current_paths.contains(p.as_str()))
            .cloned()
            .collect();
        for path in removed_paths {
            if let Some(old_info) = known.remove(&path) {
                let fp = fingerprint_from_hid_info(&old_info);
                events.push(HardwareHotplugEvent::Disconnected {
                    path,
                    fingerprint: fp,
                });
            }
        }

        // Detect arrivals
        for info in current {
            if !known.contains_key(&info.path) {
                let fp = fingerprint_from_hid_info(&info);
                known.insert(info.path.clone(), info.clone());
                events.push(HardwareHotplugEvent::Connected {
                    info,
                    fingerprint: fp,
                });
            }
        }

        Ok(events)
    }

    pub fn backend(&self) -> &Arc<dyn HidBackend> {
        &self.backend
    }
}

impl HardwareWatcher for HidHotplugWatcher {
    fn list_hid_devices(&self) -> Result<Vec<HidDeviceInfo>, HardwareError> {
        self.backend.enumerate()
    }

    fn check_os_permissions(&self) -> Result<(), HardwareError> {
        self.backend.check_os_permissions()
    }
}
