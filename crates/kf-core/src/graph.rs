use crate::compiler::CompiledGraph;
use crate::model::{
    Device, DeviceFingerprint, DeviceKind, DeviceStatus, MacroBinding, MacroTarget, Port,
    RackGroup, Wire, WorkspaceGraph, WorkspaceSettings,
};
use crate::modifier::{Modifier, ModifierError};
use crate::signal::SignalValue;
use arc_swap::ArcSwap;
use parking_lot::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum GraphError {
    #[error("Device not found: {0}")]
    DeviceNotFound(Uuid),
    #[error("Port not found: {0}")]
    PortNotFound(Uuid),
    #[error("Wire not found: {0}")]
    WireNotFound(Uuid),
    #[error("Rack group not found: {0}")]
    GroupNotFound(Uuid),
    #[error("Macro not found: {0}")]
    MacroNotFound(Uuid),
    #[error("Incompatible ports: {0}")]
    IncompatiblePorts(String),
    #[error("Duplicate wire between ports")]
    DuplicateWire,
    #[error(transparent)]
    Modifier(#[from] ModifierError),
}

/// Control Plane owning the canonical `WorkspaceGraph` and atomically publishing
/// `CompiledGraph` snapshots into the lock-free Data Plane.
pub struct ControlPlane {
    graph: RwLock<WorkspaceGraph>,
    compiled: Arc<ArcSwap<CompiledGraph>>,
    version: AtomicU64,
}

impl ControlPlane {
    pub fn new(initial: WorkspaceGraph) -> Result<Self, GraphError> {
        let compiled_graph = CompiledGraph::compile(&initial, 1)?;
        Ok(Self {
            graph: RwLock::new(initial),
            compiled: Arc::new(ArcSwap::from_pointee(compiled_graph)),
            version: AtomicU64::new(1),
        })
    }

    pub fn compiled_handle(&self) -> Arc<ArcSwap<CompiledGraph>> {
        Arc::clone(&self.compiled)
    }

    pub fn snapshot(&self) -> WorkspaceGraph {
        self.graph.read().clone()
    }

    pub fn replace_graph(&self, next: WorkspaceGraph) -> Result<(), GraphError> {
        let mut guard = self.graph.write();
        let next_ver = self.version.fetch_add(1, Ordering::SeqCst) + 1;
        let compiled = CompiledGraph::compile(&next, next_ver)?;
        *guard = next;
        self.compiled.store(Arc::new(compiled));
        Ok(())
    }

    fn recompile_locked(&self, graph: &WorkspaceGraph) -> Result<(), GraphError> {
        let next_ver = self.version.fetch_add(1, Ordering::SeqCst) + 1;
        let compiled = CompiledGraph::compile(graph, next_ver)?;
        self.compiled.store(Arc::new(compiled));
        Ok(())
    }

    pub fn add_device(&self, device: Device) -> Result<Device, GraphError> {
        let mut graph = self.graph.write();
        let cloned = device.clone();
        graph.devices.push(device);
        self.recompile_locked(&graph)?;
        Ok(cloned)
    }

    pub fn remove_device(&self, device_id: Uuid) -> Result<(), GraphError> {
        let mut graph = self.graph.write();
        let Some(pos) = graph.devices.iter().position(|d| d.id == device_id) else {
            return Err(GraphError::DeviceNotFound(device_id));
        };
        let removed_ports: Vec<Uuid> = graph.devices[pos].ports.iter().map(|p| p.id).collect();
        graph.devices.remove(pos);
        graph
            .wires
            .retain(|w| !removed_ports.contains(&w.from_port_id) && !removed_ports.contains(&w.to_port_id));
        for group in &mut graph.groups {
            group.member_node_ids.retain(|id| *id != device_id);
            for m in &mut group.macros {
                m.targets.retain(|t| !removed_ports.contains(&t.port_id));
            }
        }
        self.recompile_locked(&graph)?;
        Ok(())
    }

    /// Renames a device's display alias without touching its immutable UUID or any wires.
    pub fn rename_device(&self, device_id: Uuid, new_display_name: impl Into<String>) -> Result<Device, GraphError> {
        let mut graph = self.graph.write();
        let dev = graph
            .devices
            .iter_mut()
            .find(|d| d.id == device_id)
            .ok_or(GraphError::DeviceNotFound(device_id))?;
        dev.display_name = new_display_name.into();
        Ok(dev.clone())
    }

    /// Renames a port's display alias without touching its immutable UUID or logical_key.
    pub fn rename_port(
        &self,
        device_id: Uuid,
        port_id: Uuid,
        new_alias: impl Into<String>,
    ) -> Result<Port, GraphError> {
        let mut graph = self.graph.write();
        let dev = graph
            .devices
            .iter_mut()
            .find(|d| d.id == device_id)
            .ok_or(GraphError::DeviceNotFound(device_id))?;
        let port = dev
            .ports
            .iter_mut()
            .find(|p| p.id == port_id)
            .ok_or(GraphError::PortNotFound(port_id))?;
        port.alias = new_alias.into();
        Ok(port.clone())
    }

    pub fn update_device_position(&self, device_id: Uuid, x: f64, y: f64) -> Result<(), GraphError> {
        let mut graph = self.graph.write();
        let dev = graph
            .devices
            .iter_mut()
            .find(|d| d.id == device_id)
            .ok_or(GraphError::DeviceNotFound(device_id))?;
        dev.ui.x = x;
        dev.ui.y = y;
        Ok(())
    }

    pub fn update_device_kind(&self, device_id: Uuid, kind: DeviceKind) -> Result<Device, GraphError> {
        let mut graph = self.graph.write();
        let dev = graph
            .devices
            .iter_mut()
            .find(|d| d.id == device_id)
            .ok_or(GraphError::DeviceNotFound(device_id))?;
        dev.kind = kind;
        let out = dev.clone();
        self.recompile_locked(&graph)?;
        Ok(out)
    }

    pub fn add_port(&self, device_id: Uuid, port: Port) -> Result<Port, GraphError> {
        let mut graph = self.graph.write();
        let dev = graph
            .devices
            .iter_mut()
            .find(|d| d.id == device_id)
            .ok_or(GraphError::DeviceNotFound(device_id))?;
        if let Some(existing) = dev.ports.iter().find(|p| p.logical_key == port.logical_key) {
            return Ok(existing.clone());
        }
        let cloned = port.clone();
        dev.ports.push(port);
        self.recompile_locked(&graph)?;
        Ok(cloned)
    }

    pub fn remove_port(&self, device_id: Uuid, port_id: Uuid) -> Result<(), GraphError> {
        let mut graph = self.graph.write();
        let dev = graph
            .devices
            .iter_mut()
            .find(|d| d.id == device_id)
            .ok_or(GraphError::DeviceNotFound(device_id))?;
        let Some(pos) = dev.ports.iter().position(|p| p.id == port_id) else {
            return Err(GraphError::PortNotFound(port_id));
        };
        dev.ports.remove(pos);
        graph
            .wires
            .retain(|w| w.from_port_id != port_id && w.to_port_id != port_id);
        self.recompile_locked(&graph)?;
        Ok(())
    }

    pub fn set_device_status(&self, device_id: Uuid, status: DeviceStatus) -> Result<Device, GraphError> {
        let mut graph = self.graph.write();
        let dev = graph
            .devices
            .iter_mut()
            .find(|d| d.id == device_id)
            .ok_or(GraphError::DeviceNotFound(device_id))?;
        dev.status = status;
        let cloned = dev.clone();
        self.recompile_locked(&graph)?;
        Ok(cloned)
    }

    /// Manually rebinds an OfflineGhost device to a newly chosen physical device/fingerprint,
    /// preserving its UUID, display alias, wires, modifiers, and group membership.
    pub fn rebind_device(
        &self,
        device_id: Uuid,
        new_fingerprint: Option<DeviceFingerprint>,
        new_port_name: Option<String>,
    ) -> Result<Device, GraphError> {
        let mut graph = self.graph.write();
        let dev = graph
            .devices
            .iter_mut()
            .find(|d| d.id == device_id)
            .ok_or(GraphError::DeviceNotFound(device_id))?;
        if let Some(fp) = new_fingerprint {
            dev.fingerprint = Some(fp);
        }
        if let Some(pname) = new_port_name {
            if let DeviceKind::MidiPort { port_name, .. } = &mut dev.kind {
                *port_name = pname.clone();
            }
            if let Some(fp) = &mut dev.fingerprint {
                fp.port_name = Some(pname);
            }
        }
        dev.status = DeviceStatus::Online;
        let cloned = dev.clone();
        self.recompile_locked(&graph)?;
        Ok(cloned)
    }

    /// Attempts to auto-rebind any `OfflineGhost` device matching the discovered hardware
    /// fingerprint (or logical_key fallback). Returns the rebound device's UUID if matched.
    pub fn auto_rebind_discovered(
        &self,
        discovered: &DeviceFingerprint,
        logical_key_hint: Option<&str>,
    ) -> Result<Option<Uuid>, GraphError> {
        let mut graph = self.graph.write();
        let mut best: Option<(usize, u32)> = None;

        for (idx, dev) in graph.devices.iter().enumerate() {
            if !dev.status.is_offline_ghost() {
                continue;
            }
            let mut score = dev
                .fingerprint
                .as_ref()
                .map(|fp| fp.match_score(discovered))
                .unwrap_or(0);
            if score == 0 {
                if let Some(hint) = logical_key_hint {
                    if !hint.is_empty() && dev.logical_key == hint {
                        score = 50;
                    }
                }
            }
            if score > 0 && best.map(|(_, s)| score > s).unwrap_or(true) {
                best = Some((idx, score));
            }
        }

        let Some((idx, _)) = best else {
            return Ok(None);
        };

        let dev = &mut graph.devices[idx];
        dev.status = DeviceStatus::Online;
        if let Some(ref mut fp) = dev.fingerprint {
            if discovered.port_name.is_some() {
                fp.port_name = discovered.port_name.clone();
            }
            if discovered.endpoint.is_some() {
                fp.endpoint = discovered.endpoint.clone();
            }
        } else {
            dev.fingerprint = Some(discovered.clone());
        }
        if let (DeviceKind::MidiPort { port_name, .. }, Some(new_port)) =
            (&mut dev.kind, discovered.port_name.as_ref())
        {
            *port_name = new_port.clone();
        }
        let rebound_id = dev.id;
        self.recompile_locked(&graph)?;
        Ok(Some(rebound_id))
    }

    pub fn add_wire(&self, from_port_id: Uuid, to_port_id: Uuid) -> Result<Wire, GraphError> {
        let mut graph = self.graph.write();
        if from_port_id == to_port_id {
            return Err(GraphError::IncompatiblePorts(
                "cannot wire a port to itself".into(),
            ));
        }

        let find_port = |pid: Uuid| -> Option<(Uuid, Port)> {
            for d in &graph.devices {
                if let Some(p) = d.ports.iter().find(|p| p.id == pid) {
                    return Some((d.id, p.clone()));
                }
            }
            None
        };

        let (_, from_port) = find_port(from_port_id).ok_or(GraphError::PortNotFound(from_port_id))?;
        let (_, to_port) = find_port(to_port_id).ok_or(GraphError::PortNotFound(to_port_id))?;

        if !from_port.can_emit() {
            return Err(GraphError::IncompatiblePorts(
                "source port cannot emit signals".into(),
            ));
        }
        if !to_port.can_receive() {
            return Err(GraphError::IncompatiblePorts(
                "destination port cannot receive signals".into(),
            ));
        }
        if !from_port.signal_kind.is_compatible_with(to_port.signal_kind) {
            return Err(GraphError::IncompatiblePorts(format!(
                "{:?} is not compatible with {:?}",
                from_port.signal_kind, to_port.signal_kind
            )));
        }
        if graph
            .wires
            .iter()
            .any(|w| w.from_port_id == from_port_id && w.to_port_id == to_port_id)
        {
            return Err(GraphError::DuplicateWire);
        }

        let wire = Wire::new(from_port_id, to_port_id);
        let cloned = wire.clone();
        graph.wires.push(wire);
        self.recompile_locked(&graph)?;
        Ok(cloned)
    }

    pub fn remove_wire(&self, wire_id: Uuid) -> Result<(), GraphError> {
        let mut graph = self.graph.write();
        let Some(pos) = graph.wires.iter().position(|w| w.id == wire_id) else {
            return Err(GraphError::WireNotFound(wire_id));
        };
        graph.wires.remove(pos);
        self.recompile_locked(&graph)?;
        Ok(())
    }

    pub fn toggle_wire(&self, wire_id: Uuid, enabled: bool) -> Result<Wire, GraphError> {
        let mut graph = self.graph.write();
        let wire = graph
            .wires
            .iter_mut()
            .find(|w| w.id == wire_id)
            .ok_or(GraphError::WireNotFound(wire_id))?;
        wire.enabled = enabled;
        let cloned = wire.clone();
        self.recompile_locked(&graph)?;
        Ok(cloned)
    }

    pub fn update_wire_modifiers(
        &self,
        wire_id: Uuid,
        modifiers: Vec<Modifier>,
    ) -> Result<Wire, GraphError> {
        let mut graph = self.graph.write();
        // Validate modifiers before storing
        crate::modifier::CompiledModifierChain::compile(&modifiers)?;
        let wire = graph
            .wires
            .iter_mut()
            .find(|w| w.id == wire_id)
            .ok_or(GraphError::WireNotFound(wire_id))?;
        wire.modifiers = modifiers;
        let cloned = wire.clone();
        self.recompile_locked(&graph)?;
        Ok(cloned)
    }

    pub fn create_rack_group(
        &self,
        display_name: impl Into<String>,
        member_node_ids: Vec<Uuid>,
    ) -> Result<RackGroup, GraphError> {
        let mut graph = self.graph.write();
        let mut group = RackGroup::new(display_name, member_node_ids.clone());

        // Position rack around bounding box of members
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        for dev in &mut graph.devices {
            if member_node_ids.contains(&dev.id) {
                dev.group_id = Some(group.id);
                min_x = min_x.min(dev.ui.x);
                min_y = min_y.min(dev.ui.y);
            }
        }
        if min_x.is_finite() && min_y.is_finite() {
            group.ui.x = (min_x - 32.0).max(0.0);
            group.ui.y = (min_y - 56.0).max(0.0);
        }

        let cloned = group.clone();
        graph.groups.push(group);
        self.recompile_locked(&graph)?;
        Ok(cloned)
    }

    pub fn ungroup_rack(&self, group_id: Uuid) -> Result<(), GraphError> {
        let mut graph = self.graph.write();
        let Some(pos) = graph.groups.iter().position(|g| g.id == group_id) else {
            return Err(GraphError::GroupNotFound(group_id));
        };
        graph.groups.remove(pos);
        for dev in &mut graph.devices {
            if dev.group_id == Some(group_id) {
                dev.group_id = None;
            }
        }
        self.recompile_locked(&graph)?;
        Ok(())
    }

    pub fn rename_rack(
        &self,
        group_id: Uuid,
        new_name: impl Into<String>,
    ) -> Result<RackGroup, GraphError> {
        let mut graph = self.graph.write();
        let group = graph
            .groups
            .iter_mut()
            .find(|g| g.id == group_id)
            .ok_or(GraphError::GroupNotFound(group_id))?;
        group.display_name = new_name.into();
        Ok(group.clone())
    }

    pub fn set_rack_collapsed(&self, group_id: Uuid, collapsed: bool) -> Result<RackGroup, GraphError> {
        let mut graph = self.graph.write();
        let group = graph
            .groups
            .iter_mut()
            .find(|g| g.id == group_id)
            .ok_or(GraphError::GroupNotFound(group_id))?;
        group.collapsed = collapsed;
        Ok(group.clone())
    }

    pub fn expose_macro(
        &self,
        group_id: Uuid,
        alias: impl Into<String>,
        targets: Vec<MacroTarget>,
    ) -> Result<MacroBinding, GraphError> {
        let mut graph = self.graph.write();
        let group = graph
            .groups
            .iter_mut()
            .find(|g| g.id == group_id)
            .ok_or(GraphError::GroupNotFound(group_id))?;
        let binding = MacroBinding::new(alias, targets);
        let cloned = binding.clone();
        group.macros.push(binding);
        self.recompile_locked(&graph)?;
        Ok(cloned)
    }

    pub fn remove_macro(&self, group_id: Uuid, macro_id: Uuid) -> Result<(), GraphError> {
        let mut graph = self.graph.write();
        let group = graph
            .groups
            .iter_mut()
            .find(|g| g.id == group_id)
            .ok_or(GraphError::GroupNotFound(group_id))?;
        let Some(pos) = group.macros.iter().position(|m| m.id == macro_id) else {
            return Err(GraphError::MacroNotFound(macro_id));
        };
        group.macros.remove(pos);
        self.recompile_locked(&graph)?;
        Ok(())
    }

    pub fn set_macro_value(
        &self,
        group_id: Uuid,
        macro_id: Uuid,
        value: f64,
    ) -> Result<MacroBinding, GraphError> {
        let mut graph = self.graph.write();
        let group = graph
            .groups
            .iter_mut()
            .find(|g| g.id == group_id)
            .ok_or(GraphError::GroupNotFound(group_id))?;
        let m = group
            .macros
            .iter_mut()
            .find(|m| m.id == macro_id)
            .ok_or(GraphError::MacroNotFound(macro_id))?;
        m.value = value.clamp(0.0, 1.0);
        Ok(m.clone())
    }

    pub fn set_manual_override(
        &self,
        device_id: Uuid,
        port_id: Uuid,
        value: Option<SignalValue>,
    ) -> Result<(), GraphError> {
        let mut graph = self.graph.write();
        let dev = graph
            .devices
            .iter_mut()
            .find(|d| d.id == device_id)
            .ok_or(GraphError::DeviceNotFound(device_id))?;
        if let Some(v) = value {
            dev.manual_override.insert(port_id, v);
        } else {
            dev.manual_override.remove(&port_id);
        }
        Ok(())
    }

    pub fn update_settings(&self, settings: WorkspaceSettings) -> WorkspaceSettings {
        let mut graph = self.graph.write();
        graph.settings = settings.clone();
        settings
    }
}
