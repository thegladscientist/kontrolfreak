pub mod compiler;
pub mod extensions;
pub mod graph;
pub mod model;
pub mod modifier;
pub mod persistence;
pub mod router;
pub mod signal;
pub mod telemetry;

pub use compiler::{CompiledGraph, CompiledMacroRoute, CompiledRoute};
pub use extensions::{
    AspectScaleMode, DepthRoiConfig, DepthSampleSummary, ExtensionCapability, HidControlClass,
    HidDeviceInfo, HidLearnStep, LearnedHidMapping, SidecarHealthStatus, VideoPreviewFrame,
    VideoResizeConfig, VideoSourceInfo,
};
pub use graph::{ControlPlane, GraphError};
pub use model::{
    Device, DeviceFingerprint, DeviceKind, DeviceStatus, MacroBinding, MacroTarget,
    MidiBackendKind, NodeUi, Port, PortDirection, RackGroup, UdpFormat, Wire, WorkspaceGraph,
    WorkspaceSettings, WsFraming, WsMode,
};
pub use modifier::{
    sample_transfer_curve, CompiledExpression, CompiledModifier, CompiledModifierChain, CurveKind,
    Modifier, ModifierChainState, ModifierError,
};
pub use persistence::{
    DevicePreset, GroupPreset, MappingPreset, PersistenceError, ProjectFile, CURRENT_SCHEMA_VERSION,
};
pub use router::{RoutedOutput, SignalRouter};
pub use signal::{FrameRef, PixelFormat, SignalEvent, SignalKind, SignalValue, MAX_SIGNAL_HOPS};
pub use telemetry::{TelemetryAggregator, TelemetryBatch, TelemetryEntry, TrafficLogEntry};

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use uuid::Uuid;

    #[test]
    fn modifier_chain_math_and_expressions() {
        let chain = CompiledModifierChain::compile(&[
            Modifier::Deadzone {
                center: 0.5,
                radius: 0.1,
                rescale: true,
            },
            Modifier::Curve {
                kind: CurveKind::Exponential { exponent: 2.0 },
            },
            Modifier::RangeMap {
                in_min: 0.0,
                in_max: 1.0,
                out_min: 0.0,
                out_max: 127.0,
                clamp: true,
            },
            Modifier::Expression {
                expr: "clamp(round(x), 0, 127)".into(),
            },
        ])
        .expect("chain compiles");

        let mut state = ModifierChainState::default();
        // Inside deadzone around 0.5 -> stays 0.5 -> 0.5^2 = 0.25 -> 31.75 -> 32.0
        let out = chain
            .apply(SignalValue::Float(0.55), 1_000_000, &mut state)
            .unwrap();
        assert_eq!(out, SignalValue::Float(32.0));

        // Full scale 1.0 -> 1.0 -> 1.0 -> 127.0
        let out_max = chain
            .apply(SignalValue::Float(1.0), 2_000_000, &mut state)
            .unwrap();
        assert_eq!(out_max, SignalValue::Float(127.0));

        // Invert + Clamp + Hysteresis + Threshold + OneEuro
        let stateful = CompiledModifierChain::compile(&[
            Modifier::Invert,
            Modifier::SmoothOneEuro {
                min_cutoff: 1.0,
                beta: 0.05,
                d_cutoff: 1.0,
            },
            Modifier::Hysteresis {
                low_threshold: 0.3,
                high_threshold: 0.7,
                low_value: 0.0,
                high_value: 1.0,
            },
        ])
        .unwrap();

        let mut st = ModifierChainState::default();
        let a = stateful.apply(SignalValue::Float(0.1), 10_000_000, &mut st).unwrap(); // invert -> 0.9 -> high
        assert_eq!(a, SignalValue::Float(1.0));
        // Small dip to 0.5 stays latched high because > low_threshold (0.3)
        let b = stateful.apply(SignalValue::Float(0.5), 26_000_000, &mut st).unwrap();
        assert_eq!(b, SignalValue::Float(1.0));

        // Transfer curve sampling
        let pts = sample_transfer_curve(
            &[Modifier::Invert, Modifier::Clamp { min: 0.1, max: 0.9 }],
            11,
        )
        .unwrap();
        assert_eq!(pts.len(), 11);
        assert!((pts[0][1] - 0.9).abs() < 1e-6);
        assert!((pts[10][1] - 0.1).abs() < 1e-6);
    }

    #[test]
    fn renaming_nodes_and_ports_never_breaks_wires() {
        let cp = ControlPlane::new(WorkspaceGraph::new("Rename Test")).unwrap();
        let telemetry = Arc::new(TelemetryAggregator::default());
        let router = SignalRouter::new(cp.compiled_handle(), telemetry);

        let src_port = Port::new("/fader/1", "Fader 1", PortDirection::Out, SignalKind::Control);
        let dst_port = Port::new("cc.1.7", "CC 7", PortDirection::In, SignalKind::Midi);
        let src_port_id = src_port.id;
        let dst_port_id = dst_port.id;

        let src_dev = cp
            .add_device(
                Device::new(
                    "osc.ipad",
                    "iPad TouchOSC",
                    DeviceKind::OscEndpoint {
                        bind_addr: "127.0.0.1:0".into(),
                        target_addr: None,
                        auto_discover: true,
                    },
                )
                .with_port(src_port),
            )
            .unwrap();
        let dst_dev = cp
            .add_device(
                Device::new(
                    "midi.ableton",
                    "Ableton In",
                    DeviceKind::MidiPort {
                        port_name: "Loopback".into(),
                        is_virtual: true,
                        channel_filter: None,
                        backend: MidiBackendKind::LoopbackMock,
                    },
                )
                .with_port(dst_port),
            )
            .unwrap();

        let _wire = cp.add_wire(src_port_id, dst_port_id).unwrap();

        // Rename both devices and both ports repeatedly
        cp.rename_device(src_dev.id, "Stage iPad Pro").unwrap();
        cp.rename_device(dst_dev.id, "Main Synth Bus").unwrap();
        cp.rename_port(src_dev.id, src_port_id, "Master Dimmer").unwrap();
        cp.rename_port(dst_dev.id, dst_port_id, "Volume CC").unwrap();

        // Routing continues uninterrupted
        let outputs = router.route(SignalEvent::new(src_port_id, SignalValue::Float(0.75), 100));
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].target_device_id, dst_dev.id);
        assert_eq!(outputs[0].target_port_id, dst_port_id);
        assert_eq!(outputs[0].value, SignalValue::Float(0.75));

        let snap = cp.snapshot();
        assert_eq!(snap.devices[0].display_name, "Stage iPad Pro");
        assert_eq!(snap.devices[0].ports[0].alias, "Master Dimmer");
        assert_eq!(snap.wires.len(), 1);
    }

    #[test]
    fn collapsible_rack_and_macro_propagation() {
        let cp = ControlPlane::new(WorkspaceGraph::new("Rack Test")).unwrap();
        let telemetry = Arc::new(TelemetryAggregator::default());
        let router = SignalRouter::new(cp.compiled_handle(), telemetry);

        let p_internal = Port::new("internal.level", "Internal Level", PortDirection::Bidir, SignalKind::Control);
        let p_internal_id = p_internal.id;
        let p_out = Port::new("cc.1.11", "Expression", PortDirection::In, SignalKind::Midi);
        let p_out_id = p_out.id;

        let d1 = cp
            .add_device(Device::new("ctrl.1", "Knob Bank", DeviceKind::VirtualControl).with_port(p_internal))
            .unwrap();
        let d2 = cp
            .add_device(
                Device::new(
                    "midi.out",
                    "Synth",
                    DeviceKind::MidiPort {
                        port_name: "Loop".into(),
                        is_virtual: true,
                        channel_filter: None,
                        backend: MidiBackendKind::LoopbackMock,
                    },
                )
                .with_port(p_out),
            )
            .unwrap();
        cp.add_wire(p_internal_id, p_out_id).unwrap();

        let rack = cp.create_rack_group("Stage Rack", vec![d1.id, d2.id]).unwrap();
        let m = cp
            .expose_macro(
                rack.id,
                "Master Macro",
                vec![MacroTarget {
                    port_id: p_internal_id,
                    min: 0.2,
                    max: 0.8,
                    invert: false,
                    curve: CurveKind::Linear,
                }],
            )
            .unwrap();

        // Collapse rack — routing & macro propagation must still work cleanly
        cp.set_rack_collapsed(rack.id, true).unwrap();
        cp.set_macro_value(rack.id, m.id, 0.5).unwrap();

        let outputs = router.route_macro(m.id, 0.5, 1000);
        // 1 direct macro hit on p_internal + 1 cascaded wire hit on p_out
        assert_eq!(outputs.len(), 2);
        assert_eq!(outputs[0].target_port_id, p_internal_id);
        assert_eq!(outputs[0].value, SignalValue::Float(0.5));
        assert_eq!(outputs[1].target_port_id, p_out_id);
        assert_eq!(outputs[1].value, SignalValue::Float(0.5));
    }

    #[test]
    fn project_save_reload_with_offline_ghost_and_auto_rebind() {
        let cp = ControlPlane::new(WorkspaceGraph::new("Show Rig")).unwrap();
        let telemetry = Arc::new(TelemetryAggregator::default());
        let router = SignalRouter::new(cp.compiled_handle(), telemetry);

        let osc_p = Port::new("/cutoff", "Cutoff", PortDirection::Out, SignalKind::Control);
        let midi_p = Port::new("cc.1.74", "Filter", PortDirection::In, SignalKind::Midi);
        let osc_pid = osc_p.id;
        let midi_pid = midi_p.id;

        let fp = DeviceFingerprint {
            vid: Some(0x1234),
            pid: Some(0x5678),
            serial: Some("SN-999".into()),
            port_name: Some("USB Synth".into()),
            ..Default::default()
        };

        let _osc = cp
            .add_device(
                Device::new(
                    "osc.in",
                    "TouchOSC",
                    DeviceKind::OscEndpoint {
                        bind_addr: "127.0.0.1:9000".into(),
                        target_addr: None,
                        auto_discover: true,
                    },
                )
                .with_port(osc_p),
            )
            .unwrap();
        let midi = cp
            .add_device(
                Device::new(
                    "midi.synth",
                    "Hardware Synth",
                    DeviceKind::MidiPort {
                        port_name: "USB Synth".into(),
                        is_virtual: false,
                        channel_filter: None,
                        backend: MidiBackendKind::Hardware,
                    },
                )
                .with_fingerprint(fp.clone())
                .with_port(midi_p),
            )
            .unwrap();
        let wire = cp.add_wire(osc_pid, midi_pid).unwrap();

        // Save to JSON
        let saved_json = ProjectFile::from_graph(cp.snapshot()).to_json_pretty().unwrap();

        // Reload with NO hardware plugged in -> MIDI device becomes OfflineGhost, wire intact!
        let mut loaded = ProjectFile::from_json(&saved_json).unwrap();
        loaded.reconcile_hardware(&[]);

        let reloaded_midi = loaded.graph.devices.iter().find(|d| d.id == midi.id).unwrap();
        assert!(reloaded_midi.status.is_offline_ghost());
        assert_eq!(loaded.graph.wires.len(), 1);
        assert_eq!(loaded.graph.wires[0].id, wire.id);

        cp.replace_graph(loaded.graph).unwrap();
        // While ghosted, outbound hardware emission is muted
        let muted = router.route(SignalEvent::new(osc_pid, SignalValue::Float(0.6), 500));
        assert!(muted.is_empty());

        // Hardware plugs back in -> auto-rebind wakes it live immediately
        let rebound_id = cp.auto_rebind_discovered(&fp, None).unwrap();
        assert_eq!(rebound_id, Some(midi.id));

        let live = router.route(SignalEvent::new(osc_pid, SignalValue::Float(0.6), 600));
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].target_port_id, midi_pid);
        assert_eq!(live[0].value, SignalValue::Float(0.6));
    }

    #[test]
    fn feedback_loop_guard_and_echo_suppression() {
        let cp = ControlPlane::new(WorkspaceGraph::new("Loop Guard")).unwrap();
        let telemetry = Arc::new(TelemetryAggregator::default());
        let router = SignalRouter::new(cp.compiled_handle(), Arc::clone(&telemetry));

        let mut pa = Port::new("a", "A", PortDirection::Bidir, SignalKind::Control);
        pa.echo_suppression_ms = Some(15);
        let mut pb = Port::new("b", "B", PortDirection::Bidir, SignalKind::Control);
        pb.echo_suppression_ms = Some(15);
        let pa_id = pa.id;
        let pb_id = pb.id;

        cp.add_device(Device::new("d.a", "Node A", DeviceKind::VirtualControl).with_port(pa))
            .unwrap();
        cp.add_device(Device::new("d.b", "Node B", DeviceKind::VirtualControl).with_port(pb))
            .unwrap();

        cp.add_wire(pa_id, pb_id).unwrap();
        cp.add_wire(pb_id, pa_id).unwrap();

        // Routing A -> B must NOT infinite-loop back to A!
        let outs = router.route(SignalEvent::new(pa_id, SignalValue::Float(0.8), 100_000_000));
        assert_eq!(outs.len(), 1);
        assert_eq!(outs[0].target_port_id, pb_id);

        // Immediate echo arriving back at B within 15ms window is suppressed
        let echo = router.route(SignalEvent::new(pb_id, SignalValue::Float(0.8), 105_000_000));
        assert!(echo.is_empty());

        // After 20ms echo suppression expires, B -> A routes cleanly again
        let after = router.route(SignalEvent::new(pb_id, SignalValue::Float(0.4), 120_000_000));
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].target_port_id, pa_id);
    }

    #[test]
    fn presets_roundtrip_and_telemetry_subscription_scoping() {
        let dev = Device::new("vc.1", "Test Control", DeviceKind::VirtualControl).with_port(
            Port::new("knob.1", "Knob 1", PortDirection::Out, SignalKind::Control),
        );
        let pid = dev.ports[0].id;
        let did = dev.id;

        let dp = DevicePreset::from_device("My Device", dev.clone());
        let dp_back = DevicePreset::from_json(&dp.to_json_pretty().unwrap()).unwrap();
        assert_eq!(dp_back.name, "My Device");
        assert_eq!(dp_back.schema_version, CURRENT_SCHEMA_VERSION);

        let mp = MappingPreset::new(
            "Smooth Dimmer",
            Some("Log curve + clamp".into()),
            vec![
                Modifier::Curve {
                    kind: CurveKind::Logarithmic { base: 10.0 },
                },
                Modifier::Clamp { min: 0.0, max: 1.0 },
            ],
        );
        let mp_back = MappingPreset::from_json(&mp.to_json_pretty().unwrap()).unwrap();
        assert_eq!(mp_back.modifiers.len(), 2);

        let mut g = WorkspaceGraph::new("Preset Group");
        let rack = RackGroup::new("FX Rack", vec![did]);
        let rack_id = rack.id;
        let mut dev_in_rack = dev;
        dev_in_rack.group_id = Some(rack_id);
        g.devices.push(dev_in_rack);
        g.groups.push(rack);
        let gp = GroupPreset::extract(&g, rack_id).unwrap();
        let gp_back = GroupPreset::from_json(&gp.to_json_pretty().unwrap()).unwrap();
        let (fresh_group, fresh_devs, _) = gp_back.instantiate_fresh(25.0, 25.0);
        assert_ne!(fresh_group.id, rack_id);
        assert_ne!(fresh_devs[0].id, did);

        // Telemetry aggregator subscription scoping
        let agg = TelemetryAggregator::default();
        let other_pid = Uuid::new_v4();
        agg.record_signal(did, pid, &SignalValue::Float(0.42));
        agg.record_signal(did, other_pid, &SignalValue::Float(0.99));

        let mut filter = std::collections::HashSet::new();
        filter.insert(pid);
        let batch = agg.flush_batch(Some(&filter));
        assert_eq!(batch.values.len(), 1);
        assert_eq!(batch.values[0].port_id, pid);
        assert_eq!(batch.values[0].numeric_value, Some(0.42));
    }
}
