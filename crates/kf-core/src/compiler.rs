use crate::model::{MacroTarget, WorkspaceGraph};
use crate::modifier::{CompiledModifierChain, ModifierError};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct CompiledRoute {
    pub wire_id: Uuid,
    pub from_device_id: Uuid,
    pub from_port_id: Uuid,
    pub to_device_id: Uuid,
    pub to_port_id: Uuid,
    pub to_port_logical_key: String,
    pub enabled: bool,
    pub source_modifiers: CompiledModifierChain,
    pub wire_modifiers: CompiledModifierChain,
    pub echo_suppression_ms: u64,
}

#[derive(Debug, Clone)]
pub struct CompiledMacroRoute {
    pub group_id: Uuid,
    pub macro_id: Uuid,
    pub target_device_id: Uuid,
    pub target_port_id: Uuid,
    pub target_port_logical_key: String,
    pub target: MacroTarget,
}

/// Immutable, flat routing table swapped atomically into the Data Plane via `ArcSwap`.
#[derive(Debug, Clone, Default)]
pub struct CompiledGraph {
    pub version: u64,
    pub routes_by_source: HashMap<Uuid, Vec<CompiledRoute>>,
    pub macro_routes: HashMap<Uuid, Vec<CompiledMacroRoute>>,
    pub port_to_device: HashMap<Uuid, Uuid>,
    pub port_logical_keys: HashMap<Uuid, String>,
    pub device_online: HashMap<Uuid, bool>,
    /// Fast lock-free lookup `(device_id, logical_key) -> emitting port_id`.
    pub emitting_port_by_key: HashMap<(Uuid, String), Uuid>,
    /// Fast lock-free list of `(logical_key, port_id)` per device for OSC wildcard matching.
    pub emitting_ports_by_device: HashMap<Uuid, Vec<(String, Uuid)>>,
}

impl CompiledGraph {
    pub fn compile(graph: &WorkspaceGraph, version: u64) -> Result<Self, ModifierError> {
        let mut port_to_device = HashMap::new();
        let mut port_logical_keys = HashMap::new();
        let mut port_source_modifiers = HashMap::new();
        let mut port_echo_suppression = HashMap::new();
        let mut device_online = HashMap::new();
        let mut emitting_port_by_key = HashMap::new();
        let mut emitting_ports_by_device: HashMap<Uuid, Vec<(String, Uuid)>> = HashMap::new();

        for dev in &graph.devices {
            device_online.insert(dev.id, dev.status.is_online());
            for port in &dev.ports {
                port_to_device.insert(port.id, dev.id);
                port_logical_keys.insert(port.id, port.logical_key.clone());
                port_source_modifiers.insert(port.id, CompiledModifierChain::compile(&port.modifiers)?);
                port_echo_suppression.insert(port.id, port.echo_suppression_ms.unwrap_or(0));
                if port.can_emit() {
                    emitting_port_by_key.insert((dev.id, port.logical_key.clone()), port.id);
                    emitting_ports_by_device
                        .entry(dev.id)
                        .or_default()
                        .push((port.logical_key.clone(), port.id));
                }
            }
        }

        let mut routes_by_source: HashMap<Uuid, Vec<CompiledRoute>> = HashMap::new();
        for wire in &graph.wires {
            let Some(&from_device_id) = port_to_device.get(&wire.from_port_id) else {
                continue;
            };
            let Some(&to_device_id) = port_to_device.get(&wire.to_port_id) else {
                continue;
            };
            let to_port_logical_key = port_logical_keys
                .get(&wire.to_port_id)
                .cloned()
                .unwrap_or_default();

            let source_modifiers = port_source_modifiers
                .get(&wire.from_port_id)
                .cloned()
                .unwrap_or_default();
            let wire_modifiers = CompiledModifierChain::compile(&wire.modifiers)?;
            let echo_suppression_ms = port_echo_suppression
                .get(&wire.to_port_id)
                .copied()
                .unwrap_or(0);

            routes_by_source
                .entry(wire.from_port_id)
                .or_default()
                .push(CompiledRoute {
                    wire_id: wire.id,
                    from_device_id,
                    from_port_id: wire.from_port_id,
                    to_device_id,
                    to_port_id: wire.to_port_id,
                    to_port_logical_key,
                    enabled: wire.enabled,
                    source_modifiers,
                    wire_modifiers,
                    echo_suppression_ms,
                });
        }

        let mut macro_routes: HashMap<Uuid, Vec<CompiledMacroRoute>> = HashMap::new();
        for group in &graph.groups {
            for m in &group.macros {
                let mut targets = Vec::new();
                for target in &m.targets {
                    if let Some(&target_device_id) = port_to_device.get(&target.port_id) {
                        let target_port_logical_key = port_logical_keys
                            .get(&target.port_id)
                            .cloned()
                            .unwrap_or_default();
                        targets.push(CompiledMacroRoute {
                            group_id: group.id,
                            macro_id: m.id,
                            target_device_id,
                            target_port_id: target.port_id,
                            target_port_logical_key,
                            target: target.clone(),
                        });
                    }
                }
                macro_routes.insert(m.id, targets);
            }
        }

        Ok(Self {
            version,
            routes_by_source,
            macro_routes,
            port_to_device,
            port_logical_keys,
            device_online,
            emitting_port_by_key,
            emitting_ports_by_device,
        })
    }
}
