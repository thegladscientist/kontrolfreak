use crate::compiler::CompiledGraph;
use crate::modifier::ModifierChainState;
use crate::signal::{SignalEvent, SignalValue, MAX_SIGNAL_HOPS};
use crate::telemetry::TelemetryAggregator;
use arc_swap::ArcSwap;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq)]
pub struct RoutedOutput {
    pub wire_id: Option<Uuid>,
    pub macro_id: Option<Uuid>,
    pub target_device_id: Uuid,
    pub target_port_id: Uuid,
    pub target_port_logical_key: String,
    pub value: SignalValue,
    pub timestamp_ns: u64,
}

/// Lock-free Data Plane signal router reading immutable `CompiledGraph` snapshots via `ArcSwap`.
pub struct SignalRouter {
    compiled: Arc<ArcSwap<CompiledGraph>>,
    wire_states: Mutex<HashMap<Uuid, (ModifierChainState, ModifierChainState)>>,
    echo_lockout_until_ns: Mutex<HashMap<Uuid, u64>>,
    telemetry: Arc<TelemetryAggregator>,
}

impl SignalRouter {
    pub fn new(compiled: Arc<ArcSwap<CompiledGraph>>, telemetry: Arc<TelemetryAggregator>) -> Self {
        Self {
            compiled,
            wire_states: Mutex::new(HashMap::new()),
            echo_lockout_until_ns: Mutex::new(HashMap::new()),
            telemetry,
        }
    }

    pub fn telemetry(&self) -> Arc<TelemetryAggregator> {
        Arc::clone(&self.telemetry)
    }

    pub fn compiled_version(&self) -> u64 {
        self.compiled.load().version
    }

    pub fn load_compiled(&self) -> arc_swap::Guard<Arc<CompiledGraph>> {
        self.compiled.load()
    }

    /// Routes an incoming `SignalEvent` through the current `CompiledGraph` snapshot.
    /// Supports multi-hop cascades while enforcing TTL (`MAX_SIGNAL_HOPS`), visited-node cycle
    /// guards, and optional per-port echo suppression.
    pub fn route(&self, event: SignalEvent) -> Vec<RoutedOutput> {
        let graph = self.compiled.load();
        let mut outputs = Vec::new();
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(event);

        while let Some(mut ev) = queue.pop_front() {
            if ev.hop_count >= MAX_SIGNAL_HOPS {
                continue;
            }

            if let Some(&src_dev_id) = graph.port_to_device.get(&ev.source_port_id) {
                self.telemetry
                    .record_signal(src_dev_id, ev.source_port_id, &ev.value);
                if !ev.visited_nodes.contains(&src_dev_id) {
                    ev.visited_nodes.push(src_dev_id);
                }
            }

            // Echo suppression check on incoming source port.
            {
                let lockouts = self.echo_lockout_until_ns.lock();
                if let Some(&until) = lockouts.get(&ev.source_port_id) {
                    if ev.timestamp_ns < until {
                        continue;
                    }
                }
            }

            let Some(routes) = graph.routes_by_source.get(&ev.source_port_id) else {
                continue;
            };

            for route in routes {
                if !route.enabled {
                    continue;
                }
                if ev.visited_nodes.contains(&route.to_device_id) {
                    continue;
                }

                let shaped = {
                    let mut states = self.wire_states.lock();
                    let (src_state, wire_state) = states.entry(route.wire_id).or_default();
                    let after_src = match route.source_modifiers.apply(
                        ev.value.clone(),
                        ev.timestamp_ns,
                        src_state,
                    ) {
                        Some(v) => v,
                        None => continue,
                    };
                    match route
                        .wire_modifiers
                        .apply(after_src, ev.timestamp_ns, wire_state)
                    {
                        Some(v) => v,
                        None => continue,
                    }
                };

                if route.echo_suppression_ms > 0 {
                    let mut lockouts = self.echo_lockout_until_ns.lock();
                    lockouts.insert(
                        route.to_port_id,
                        ev.timestamp_ns + route.echo_suppression_ms * 1_000_000,
                    );
                }

                self.telemetry
                    .record_signal(route.to_device_id, route.to_port_id, &shaped);

                // Only emit to outbound hardware/protocol if target device is online.
                if *graph.device_online.get(&route.to_device_id).unwrap_or(&true) {
                    outputs.push(RoutedOutput {
                        wire_id: Some(route.wire_id),
                        macro_id: None,
                        target_device_id: route.to_device_id,
                        target_port_id: route.to_port_id,
                        target_port_logical_key: route.to_port_logical_key.clone(),
                        value: shaped.clone(),
                        timestamp_ns: ev.timestamp_ns,
                    });
                }

                // Cascade if the target port itself has outgoing wires (e.g., inside Racks or Bidir).
                if graph.routes_by_source.contains_key(&route.to_port_id) {
                    let mut next_visited = ev.visited_nodes.clone();
                    next_visited.push(route.to_device_id);
                    queue.push_back(SignalEvent {
                        source_port_id: route.to_port_id,
                        value: shaped,
                        timestamp_ns: ev.timestamp_ns,
                        hop_count: ev.hop_count + 1,
                        visited_nodes: next_visited,
                    });
                }
            }
        }

        outputs
    }

    /// Fans out a Rack Macro value to all of its bound target ports and cascades downstream wires.
    pub fn route_macro(&self, macro_id: Uuid, normalized_value: f64, timestamp_ns: u64) -> Vec<RoutedOutput> {
        let graph = self.compiled.load();
        let Some(routes) = graph.macro_routes.get(&macro_id) else {
            return Vec::new();
        };

        let mut outputs = Vec::new();
        for route in routes {
            let computed = route.target.evaluate(normalized_value);
            let value = SignalValue::Float(computed);

            self.telemetry
                .record_signal(route.target_device_id, route.target_port_id, &value);

            if *graph
                .device_online
                .get(&route.target_device_id)
                .unwrap_or(&true)
            {
                outputs.push(RoutedOutput {
                    wire_id: None,
                    macro_id: Some(macro_id),
                    target_device_id: route.target_device_id,
                    target_port_id: route.target_port_id,
                    target_port_logical_key: route.target_port_logical_key.clone(),
                    value: value.clone(),
                    timestamp_ns,
                });
            }

            if graph.routes_by_source.contains_key(&route.target_port_id) {
                let cascaded = self.route(SignalEvent::new(
                    route.target_port_id,
                    value,
                    timestamp_ns,
                ));
                outputs.extend(cascaded);
            }
        }
        outputs
    }
}
