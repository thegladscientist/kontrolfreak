use kf_core::{
    ControlPlane, Device, DeviceFingerprint, DeviceKind, MidiBackendKind, Port, PortDirection,
    SignalKind, SignalRouter, TelemetryAggregator, WorkspaceGraph, WsFraming, WsMode,
};
use kf_protocols::ProtocolManager;
use parking_lot::Mutex;
use std::collections::HashSet;
use std::sync::Arc;
use tauri::async_runtime::JoinHandle;
use uuid::Uuid;

pub struct AppState {
    pub control: Arc<ControlPlane>,
    pub router: Arc<SignalRouter>,
    pub telemetry: Arc<TelemetryAggregator>,
    pub protocols: Arc<ProtocolManager>,
    pub telemetry_pump: Mutex<Option<JoinHandle<()>>>,
    pub subscribed_ports: Arc<Mutex<Option<HashSet<Uuid>>>>,
}

impl AppState {
    pub fn new() -> Self {
        let initial = default_starter_workspace();
        let control = Arc::new(
            ControlPlane::new(initial).expect("default workspace compiles cleanly"),
        );
        let telemetry = Arc::new(TelemetryAggregator::default());
        let router = Arc::new(SignalRouter::new(
            control.compiled_handle(),
            Arc::clone(&telemetry),
        ));
        let protocols = ProtocolManager::new(
            Arc::clone(&control),
            Arc::clone(&router),
            Arc::clone(&telemetry),
        );

        Self {
            control,
            router,
            telemetry,
            protocols,
            telemetry_pump: Mutex::new(None),
            subscribed_ports: Arc::new(Mutex::new(None)),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

/// Seeds a ready-to-explore starter rig on first launch so users can test wiring immediately.
pub fn default_starter_workspace() -> WorkspaceGraph {
    let mut graph = WorkspaceGraph::new("Studio Starter Rig");

    let osc_fader = Port::new("/fader/1", "Main Fader", PortDirection::Out, SignalKind::Control);
    let osc_xy = Port::new("/xy/pad", "XY Pad", PortDirection::Out, SignalKind::Control);
    let osc_fader_id = osc_fader.id;

    let mut osc = Device::new(
        "osc.ipad",
        "TouchOSC iPad",
        DeviceKind::OscEndpoint {
            bind_addr: "127.0.0.1:9000".into(),
            target_addr: Some("127.0.0.1:9001".into()),
            auto_discover: true,
        },
    )
    .with_port(osc_fader)
    .with_port(osc_xy);
    osc.ui.x = 80.0;
    osc.ui.y = 120.0;

    let midi_cc = Port::new("cc.1.7", "Master Volume CC7", PortDirection::In, SignalKind::Midi);
    let midi_note = Port::new("note.1.60", "Trigger Note C4", PortDirection::In, SignalKind::Trigger);
    let midi_cc_id = midi_cc.id;

    let mut midi = Device::new(
        "midi.loopback",
        "Virtual Synth Bus",
        DeviceKind::MidiPort {
            port_name: "Kontrolfreak Loopback 1".into(),
            is_virtual: true,
            channel_filter: None,
            backend: MidiBackendKind::LoopbackMock,
        },
    )
    .with_fingerprint(DeviceFingerprint {
        port_name: Some("Kontrolfreak Loopback 1".into()),
        ..Default::default()
    })
    .with_port(midi_cc)
    .with_port(midi_note);
    midi.ui.x = 460.0;
    midi.ui.y = 110.0;

    let ws_in = Port::new("/hud/level", "HUD Level", PortDirection::Bidir, SignalKind::Control);
    let mut ws = Device::new(
        "ws.bridge",
        "WebSocket Bridge",
        DeviceKind::WebSocketEndpoint {
            mode: WsMode::Server,
            url_or_bind: "127.0.0.1:8765".into(),
            framing: WsFraming::Json,
        },
    )
    .with_port(ws_in);
    ws.ui.x = 460.0;
    ws.ui.y = 330.0;

    let wire = kf_core::Wire::new(osc_fader_id, midi_cc_id);

    graph.devices.push(osc);
    graph.devices.push(midi);
    graph.devices.push(ws);
    graph.wires.push(wire);
    graph
}
