use crate::extensions::LearnedHidMapping;
use crate::model::{
    Device, DeviceFingerprint, DeviceKind, DeviceStatus, RackGroup, Wire, WorkspaceGraph,
};
use crate::modifier::Modifier;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;
use uuid::Uuid;

pub const CURRENT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum PersistenceError {
    #[error("Unsupported schema version {found} (max supported: {CURRENT_SCHEMA_VERSION})")]
    UnsupportedSchemaVersion { found: u32 },
    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// `.kfdevice.json` — Portable Device Preset
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DevicePreset {
    pub schema_version: u32,
    pub preset_id: Uuid,
    pub name: String,
    pub device: Device,
    #[serde(default)]
    pub hid_mappings: Vec<LearnedHidMapping>,
}

impl DevicePreset {
    pub fn from_device(name: impl Into<String>, device: Device) -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            preset_id: Uuid::new_v4(),
            name: name.into(),
            device,
            hid_mappings: Vec::new(),
        }
    }

    pub fn to_json_pretty(&self) -> Result<String, PersistenceError> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    pub fn from_json(json: &str) -> Result<Self, PersistenceError> {
        let parsed: Self = serde_json::from_str(json)?;
        if parsed.schema_version > CURRENT_SCHEMA_VERSION {
            return Err(PersistenceError::UnsupportedSchemaVersion {
                found: parsed.schema_version,
            });
        }
        Ok(parsed)
    }
}

/// `.kfgroup.json` — Reusable Collapsible Rack / Group Preset
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroupPreset {
    pub schema_version: u32,
    pub preset_id: Uuid,
    pub name: String,
    pub group: RackGroup,
    pub member_devices: Vec<Device>,
    pub internal_wires: Vec<Wire>,
}

impl GroupPreset {
    pub fn extract(graph: &WorkspaceGraph, group_id: Uuid) -> Option<Self> {
        let group = graph.groups.iter().find(|g| g.id == group_id)?.clone();
        let member_devices: Vec<Device> = graph
            .devices
            .iter()
            .filter(|d| group.member_node_ids.contains(&d.id))
            .cloned()
            .collect();
        let member_ports: std::collections::HashSet<Uuid> = member_devices
            .iter()
            .flat_map(|d| d.ports.iter().map(|p| p.id))
            .collect();
        let internal_wires: Vec<Wire> = graph
            .wires
            .iter()
            .filter(|w| {
                member_ports.contains(&w.from_port_id) && member_ports.contains(&w.to_port_id)
            })
            .cloned()
            .collect();
        Some(Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            preset_id: Uuid::new_v4(),
            name: group.display_name.clone(),
            group,
            member_devices,
            internal_wires,
        })
    }

    /// Instantiates a fresh copy of this group preset with newly minted UUIDs so it can be
    /// dropped multiple times onto a canvas without ID collisions.
    pub fn instantiate_fresh(&self, offset_x: f64, offset_y: f64) -> (RackGroup, Vec<Device>, Vec<Wire>) {
        let mut device_map: HashMap<Uuid, Uuid> = HashMap::new();
        let mut port_map: HashMap<Uuid, Uuid> = HashMap::new();
        let new_group_id = Uuid::new_v4();

        let mut new_devices = Vec::with_capacity(self.member_devices.len());
        for d in &self.member_devices {
            let new_dev_id = Uuid::new_v4();
            device_map.insert(d.id, new_dev_id);
            let mut nd = d.clone();
            nd.id = new_dev_id;
            nd.group_id = Some(new_group_id);
            nd.ui.x += offset_x;
            nd.ui.y += offset_y;
            for p in &mut nd.ports {
                let new_pid = Uuid::new_v4();
                port_map.insert(p.id, new_pid);
                p.id = new_pid;
            }
            new_devices.push(nd);
        }

        let mut new_wires = Vec::with_capacity(self.internal_wires.len());
        for w in &self.internal_wires {
            if let (Some(&from_id), Some(&to_id)) =
                (port_map.get(&w.from_port_id), port_map.get(&w.to_port_id))
            {
                let mut nw = w.clone();
                nw.id = Uuid::new_v4();
                nw.from_port_id = from_id;
                nw.to_port_id = to_id;
                new_wires.push(nw);
            }
        }

        let mut new_group = self.group.clone();
        new_group.id = new_group_id;
        new_group.ui.x += offset_x;
        new_group.ui.y += offset_y;
        new_group.member_node_ids = new_group
            .member_node_ids
            .iter()
            .filter_map(|id| device_map.get(id).copied())
            .collect();
        for m in &mut new_group.macros {
            m.id = Uuid::new_v4();
            for t in &mut m.targets {
                if let Some(&new_pid) = port_map.get(&t.port_id) {
                    t.port_id = new_pid;
                }
            }
        }

        (new_group, new_devices, new_wires)
    }

    pub fn to_json_pretty(&self) -> Result<String, PersistenceError> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    pub fn from_json(json: &str) -> Result<Self, PersistenceError> {
        let parsed: Self = serde_json::from_str(json)?;
        if parsed.schema_version > CURRENT_SCHEMA_VERSION {
            return Err(PersistenceError::UnsupportedSchemaVersion {
                found: parsed.schema_version,
            });
        }
        Ok(parsed)
    }
}

/// `.kfmapping.json` — Reusable Modifier Chain / Parameter Mapping Preset
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MappingPreset {
    pub schema_version: u32,
    pub preset_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub modifiers: Vec<Modifier>,
}

impl MappingPreset {
    pub fn new(
        name: impl Into<String>,
        description: Option<String>,
        modifiers: Vec<Modifier>,
    ) -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            preset_id: Uuid::new_v4(),
            name: name.into(),
            description,
            modifiers,
        }
    }

    pub fn to_json_pretty(&self) -> Result<String, PersistenceError> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    pub fn from_json(json: &str) -> Result<Self, PersistenceError> {
        let parsed: Self = serde_json::from_str(json)?;
        if parsed.schema_version > CURRENT_SCHEMA_VERSION {
            return Err(PersistenceError::UnsupportedSchemaVersion {
                found: parsed.schema_version,
            });
        }
        Ok(parsed)
    }
}

/// `.kfproj.json` — Full Versioned Project File
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectFile {
    pub schema_version: u32,
    pub graph: WorkspaceGraph,
}

impl ProjectFile {
    pub fn from_graph(graph: WorkspaceGraph) -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            graph,
        }
    }

    pub fn to_json_pretty(&self) -> Result<String, PersistenceError> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    pub fn from_json(json: &str) -> Result<Self, PersistenceError> {
        let parsed: Self = serde_json::from_str(json)?;
        if parsed.schema_version > CURRENT_SCHEMA_VERSION {
            return Err(PersistenceError::UnsupportedSchemaVersion {
                found: parsed.schema_version,
            });
        }
        Ok(parsed)
    }

    pub fn save_to_path(&self, path: impl AsRef<Path>) -> Result<(), PersistenceError> {
        let json = self.to_json_pretty()?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self, PersistenceError> {
        let json = std::fs::read_to_string(path)?;
        Self::from_json(&json)
    }

    /// Reconciles hardware devices upon project load: any physical/MIDI/HID/camera/depth device
    /// whose fingerprint is not present in `available_fingerprints` enters `DeviceStatus::OfflineGhost`
    /// while preserving every UUID, alias, wire, modifier, and rack membership intact.
    pub fn reconcile_hardware(&mut self, available_fingerprints: &[DeviceFingerprint]) {
        for dev in &mut self.graph.devices {
            let needs_hardware = matches!(
                dev.kind,
                DeviceKind::MidiPort { .. }
                    | DeviceKind::HidDevice { .. }
                    | DeviceKind::RgbCamera { .. }
                    | DeviceKind::DepthSensor { .. }
            );
            if !needs_hardware {
                continue;
            }

            let matched = dev.fingerprint.as_ref().is_some_and(|fp| {
                available_fingerprints
                    .iter()
                    .any(|avail| fp.match_score(avail) >= 65)
            });

            if matched {
                dev.status = DeviceStatus::Online;
            } else {
                dev.status = DeviceStatus::OfflineGhost {
                    reason: Some("Hardware device not connected on load".into()),
                    last_seen_ms: None,
                };
            }
        }
    }
}
