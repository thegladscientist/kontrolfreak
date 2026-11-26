use crate::modifier::{CurveKind, Modifier};
use crate::signal::{SignalKind, SignalValue};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum DeviceStatus {
    Online,
    OfflineGhost {
        #[serde(default)]
        reason: Option<String>,
        #[serde(default)]
        last_seen_ms: Option<u64>,
    },
    Learning,
    Error {
        message: String,
    },
}

impl Default for DeviceStatus {
    fn default() -> Self {
        Self::Online
    }
}

impl DeviceStatus {
    pub fn is_online(&self) -> bool {
        matches!(self, DeviceStatus::Online)
    }

    pub fn is_offline_ghost(&self) -> bool {
        matches!(self, DeviceStatus::OfflineGhost { .. })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceFingerprint {
    #[serde(default)]
    pub vid: Option<u16>,
    #[serde(default)]
    pub pid: Option<u16>,
    #[serde(default)]
    pub serial: Option<String>,
    #[serde(default)]
    pub usage_page: Option<u16>,
    #[serde(default)]
    pub usage: Option<u16>,
    #[serde(default)]
    pub interface_number: Option<i32>,
    #[serde(default)]
    pub port_name: Option<String>,
    #[serde(default)]
    pub endpoint: Option<String>,
}

impl DeviceFingerprint {
    /// Strict matching score per architecture-plan §7:
    /// 1. Exact Match (vid+pid+serial or exact endpoint): 100
    /// 2. Model Match (vid+pid+usage_page or exact port_name): 70
    /// 3. Partial hardware/port match: 40
    /// 0 = no match
    pub fn match_score(&self, candidate: &DeviceFingerprint) -> u32 {
        if let (Some(sv), Some(sp), Some(ss), Some(cv), Some(cp), Some(cs)) = (
            self.vid,
            self.pid,
            self.serial.as_deref(),
            candidate.vid,
            candidate.pid,
            candidate.serial.as_deref(),
        ) {
            if sv == cv && sp == cp && ss == cs {
                return 100;
            }
        }

        if let (Some(se), Some(ce)) = (self.endpoint.as_deref(), candidate.endpoint.as_deref()) {
            if !se.is_empty() && se == ce {
                return 100;
            }
        }

        if let (Some(sv), Some(sp), Some(cv), Some(cp)) =
            (self.vid, self.pid, candidate.vid, candidate.pid)
        {
            if sv == cv && sp == cp {
                if self.usage_page.is_some() && self.usage_page == candidate.usage_page {
                    return 75;
                }
                return 65;
            }
        }

        if let (Some(sp), Some(cp)) = (self.port_name.as_deref(), candidate.port_name.as_deref()) {
            if !sp.is_empty() && sp.eq_ignore_ascii_case(cp) {
                return 70;
            }
        }

        0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortDirection {
    In,
    Out,
    Bidir,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Port {
    pub id: Uuid,
    pub logical_key: String,
    pub alias: String,
    pub direction: PortDirection,
    pub signal_kind: SignalKind,
    #[serde(default)]
    pub default_value: Option<SignalValue>,
    #[serde(default)]
    pub min_value: Option<f64>,
    #[serde(default)]
    pub max_value: Option<f64>,
    #[serde(default)]
    pub modifiers: Vec<Modifier>,
    #[serde(default)]
    pub echo_suppression_ms: Option<u64>,
}

impl Port {
    pub fn new(
        logical_key: impl Into<String>,
        alias: impl Into<String>,
        direction: PortDirection,
        signal_kind: SignalKind,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            logical_key: logical_key.into(),
            alias: alias.into(),
            direction,
            signal_kind,
            default_value: None,
            min_value: Some(0.0),
            max_value: Some(1.0),
            modifiers: Vec::new(),
            echo_suppression_ms: None,
        }
    }

    pub fn can_emit(&self) -> bool {
        matches!(self.direction, PortDirection::Out | PortDirection::Bidir)
    }

    pub fn can_receive(&self) -> bool {
        matches!(self.direction, PortDirection::In | PortDirection::Bidir)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MidiBackendKind {
    Hardware,
    CoreMidiVirtual,
    LoopMidiWindows,
    AlsaVirtual,
    LoopbackMock,
}

impl Default for MidiBackendKind {
    fn default() -> Self {
        Self::Hardware
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WsMode {
    Server,
    Client,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WsFraming {
    Json,
    Text,
    Binary,
    OscOverWs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UdpFormat {
    Utf8,
    Json,
    Csv,
    Hex,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DeviceKind {
    OscEndpoint {
        bind_addr: String,
        #[serde(default)]
        target_addr: Option<String>,
        #[serde(default = "default_true")]
        auto_discover: bool,
    },
    MidiPort {
        port_name: String,
        #[serde(default)]
        is_virtual: bool,
        #[serde(default)]
        channel_filter: Option<u8>,
        #[serde(default)]
        backend: MidiBackendKind,
    },
    WebSocketEndpoint {
        mode: WsMode,
        url_or_bind: String,
        framing: WsFraming,
    },
    UdpEndpoint {
        bind_addr: String,
        #[serde(default)]
        target_addr: Option<String>,
        #[serde(default)]
        multicast_group: Option<String>,
        #[serde(default)]
        broadcast: bool,
        format: UdpFormat,
    },
    VirtualControl,
    // --- M2 Seams ---
    RtpMidiSession {
        session_name: String,
        control_port: u16,
        #[serde(default)]
        remote_host: Option<String>,
        #[serde(default = "default_true")]
        mdns_discover: bool,
    },
    NdiVideoNode {
        #[serde(default)]
        source_name: Option<String>,
        #[serde(default)]
        output_name: Option<String>,
        #[serde(default = "default_ndi_w")]
        out_width: u32,
        #[serde(default = "default_ndi_h")]
        out_height: u32,
        #[serde(default = "default_ndi_fps")]
        out_fps: f32,
    },
    // --- M3 Seams ---
    HidDevice {
        #[serde(default)]
        device_preset_id: Option<String>,
        #[serde(default)]
        vid: Option<u16>,
        #[serde(default)]
        pid: Option<u16>,
    },
    RgbCamera {
        camera_id: String,
        extractor_kind: String,
    },
    // --- M4 Seams ---
    DepthSensor {
        vendor: String,
        #[serde(default)]
        sidecar_id: Option<String>,
        #[serde(default = "default_min_z")]
        roi_min_z: f32,
        #[serde(default = "default_max_z")]
        roi_max_z: f32,
    },
}

fn default_true() -> bool {
    true
}
fn default_ndi_w() -> u32 {
    1280
}
fn default_ndi_h() -> u32 {
    720
}
fn default_ndi_fps() -> f32 {
    30.0
}
fn default_min_z() -> f32 {
    0.3
}
fn default_max_z() -> f32 {
    3.0
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeUi {
    pub x: f64,
    pub y: f64,
    #[serde(default)]
    pub width: Option<f64>,
    #[serde(default)]
    pub height: Option<f64>,
    #[serde(default)]
    pub collapsed: bool,
}

impl Default for NodeUi {
    fn default() -> Self {
        Self {
            x: 120.0,
            y: 120.0,
            width: None,
            height: None,
            collapsed: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Device {
    pub id: Uuid,
    pub logical_key: String,
    pub display_name: String,
    pub kind: DeviceKind,
    #[serde(default)]
    pub fingerprint: Option<DeviceFingerprint>,
    #[serde(default)]
    pub status: DeviceStatus,
    #[serde(default)]
    pub ports: Vec<Port>,
    #[serde(default)]
    pub ui: NodeUi,
    #[serde(default)]
    pub group_id: Option<Uuid>,
    #[serde(default)]
    pub manual_override: HashMap<Uuid, SignalValue>,
}

impl Device {
    pub fn new(
        logical_key: impl Into<String>,
        display_name: impl Into<String>,
        kind: DeviceKind,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            logical_key: logical_key.into(),
            display_name: display_name.into(),
            kind,
            fingerprint: None,
            status: DeviceStatus::Online,
            ports: Vec::new(),
            ui: NodeUi::default(),
            group_id: None,
            manual_override: HashMap::new(),
        }
    }

    pub fn with_port(mut self, port: Port) -> Self {
        self.ports.push(port);
        self
    }

    pub fn with_fingerprint(mut self, fp: DeviceFingerprint) -> Self {
        self.fingerprint = Some(fp);
        self
    }

    pub fn find_port_by_key(&self, logical_key: &str) -> Option<&Port> {
        self.ports.iter().find(|p| p.logical_key == logical_key)
    }

    pub fn find_port_by_id(&self, port_id: Uuid) -> Option<&Port> {
        self.ports.iter().find(|p| p.id == port_id)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Wire {
    pub id: Uuid,
    pub from_port_id: Uuid,
    pub to_port_id: Uuid,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub modifiers: Vec<Modifier>,
}

impl Wire {
    pub fn new(from_port_id: Uuid, to_port_id: Uuid) -> Self {
        Self {
            id: Uuid::new_v4(),
            from_port_id,
            to_port_id,
            enabled: true,
            modifiers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MacroTarget {
    pub port_id: Uuid,
    pub min: f64,
    pub max: f64,
    #[serde(default)]
    pub invert: bool,
    #[serde(default)]
    pub curve: CurveKind,
}

impl MacroTarget {
    pub fn evaluate(&self, normalized: f64) -> f64 {
        let mut t = normalized.clamp(0.0, 1.0);
        if self.invert {
            t = 1.0 - t;
        }
        t = self.curve.apply(t);
        self.min + t * (self.max - self.min)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MacroBinding {
    pub id: Uuid,
    pub alias: String,
    #[serde(default)]
    pub value: f64,
    #[serde(default)]
    pub targets: Vec<MacroTarget>,
}

impl MacroBinding {
    pub fn new(alias: impl Into<String>, targets: Vec<MacroTarget>) -> Self {
        Self {
            id: Uuid::new_v4(),
            alias: alias.into(),
            value: 0.0,
            targets,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RackGroup {
    pub id: Uuid,
    pub display_name: String,
    #[serde(default)]
    pub collapsed: bool,
    #[serde(default)]
    pub member_node_ids: Vec<Uuid>,
    #[serde(default)]
    pub macros: Vec<MacroBinding>,
    #[serde(default)]
    pub ui: NodeUi,
}

impl RackGroup {
    pub fn new(display_name: impl Into<String>, member_node_ids: Vec<Uuid>) -> Self {
        Self {
            id: Uuid::new_v4(),
            display_name: display_name.into(),
            collapsed: false,
            member_node_ids,
            macros: Vec::new(),
            ui: NodeUi::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceSettings {
    #[serde(default)]
    pub calm_mode: bool,
    #[serde(default = "default_telemetry_hz")]
    pub telemetry_hz: u32,
    #[serde(default = "default_theme")]
    pub theme: String,
}

fn default_telemetry_hz() -> u32 {
    30
}

fn default_theme() -> String {
    "dark-studio".to_string()
}

impl Default for WorkspaceSettings {
    fn default() -> Self {
        Self {
            calm_mode: false,
            telemetry_hz: 30,
            theme: "dark-studio".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceGraph {
    pub project_id: Uuid,
    pub name: String,
    #[serde(default)]
    pub devices: Vec<Device>,
    #[serde(default)]
    pub wires: Vec<Wire>,
    #[serde(default)]
    pub groups: Vec<RackGroup>,
    #[serde(default)]
    pub settings: WorkspaceSettings,
}

impl WorkspaceGraph {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            project_id: Uuid::new_v4(),
            name: name.into(),
            devices: Vec::new(),
            wires: Vec::new(),
            groups: Vec::new(),
            settings: WorkspaceSettings::default(),
        }
    }
}
