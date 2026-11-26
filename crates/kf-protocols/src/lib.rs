pub mod midi;
pub mod osc;
pub mod rtpmidi;
pub mod udp;
pub mod websocket;

use futures_util::{SinkExt, StreamExt};
use kf_core::{
    ControlPlane, DeviceKind, MidiBackendKind, Port, PortDirection, RoutedOutput, SignalEvent,
    SignalKind, SignalRouter, TelemetryAggregator, UdpFormat, WsFraming, WsMode,
};
use midi::{
    default_virtual_midi_provider, list_hardware_midi_ports, LoopbackMidiBackend, MidiMessage,
    MidiPortInfo, VirtualMidiGuidance,
};
use osc::{decode_osc_bytes, encode_osc_message, osc_address_matches, osc_args_to_signal};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use thiserror::Error;
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("Device not found in graph: {0}")]
    DeviceNotFound(Uuid),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Endpoint error: {0}")]
    Endpoint(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointStatus {
    pub device_id: Uuid,
    pub protocol: String,
    pub running: bool,
    pub bound_address: Option<String>,
    pub detail: String,
}

enum OutboundSink {
    Osc {
        socket: Arc<UdpSocket>,
        target: Option<SocketAddr>,
    },
    MidiLoopback {
        port_name: String,
    },
    WebSocketServer {
        tx: broadcast::Sender<Message>,
        framing: WsFraming,
    },
    WebSocketClient {
        tx: mpsc::UnboundedSender<Message>,
        framing: WsFraming,
    },
    Udp {
        socket: Arc<UdpSocket>,
        target: Option<SocketAddr>,
        format: UdpFormat,
    },
}

struct RunningEndpoint {
    status: EndpointStatus,
    sink: OutboundSink,
    tasks: Vec<JoinHandle<()>>,
}

/// Orchestrates live protocol endpoints (OSC, MIDI, WebSockets, Raw UDP), forwarding inbound
/// packets into `SignalRouter` and dispatching `RoutedOutput`s to outbound sinks.
pub struct ProtocolManager {
    control: Arc<ControlPlane>,
    router: Arc<SignalRouter>,
    telemetry: Arc<TelemetryAggregator>,
    loopback_midi: Arc<LoopbackMidiBackend>,
    endpoints: Arc<Mutex<HashMap<Uuid, RunningEndpoint>>>,
}

impl ProtocolManager {
    pub fn new(
        control: Arc<ControlPlane>,
        router: Arc<SignalRouter>,
        telemetry: Arc<TelemetryAggregator>,
    ) -> Arc<Self> {
        Arc::new(Self {
            control,
            router,
            telemetry,
            loopback_midi: LoopbackMidiBackend::new(),
            endpoints: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub fn loopback_midi(&self) -> Arc<LoopbackMidiBackend> {
        Arc::clone(&self.loopback_midi)
    }

    pub fn list_midi_ports(&self) -> Vec<MidiPortInfo> {
        let mut ports = list_hardware_midi_ports();
        ports.extend(self.loopback_midi.list_ports());
        ports
    }

    pub fn virtual_midi_guidance(&self) -> VirtualMidiGuidance {
        default_virtual_midi_provider().setup_guidance()
    }

    pub fn statuses(&self) -> Vec<EndpointStatus> {
        self.endpoints
            .lock()
            .values()
            .map(|e| e.status.clone())
            .collect()
    }

    pub fn stop_endpoint(&self, device_id: Uuid) {
        if let Some(ep) = self.endpoints.lock().remove(&device_id) {
            for handle in ep.tasks {
                handle.abort();
            }
        }
    }

    pub fn stop_all(&self) {
        let drained: Vec<RunningEndpoint> = {
            let mut guard = self.endpoints.lock();
            guard.drain().map(|(_, v)| v).collect()
        };
        for ep in drained {
            for h in ep.tasks {
                h.abort();
            }
        }
    }

    /// Dispatches a batch of `RoutedOutput`s to matching live outbound endpoints.
    pub async fn dispatch_outputs(self: &Arc<Self>, outputs: Vec<RoutedOutput>) {
        for out in outputs {
            self.dispatch_single(&out).await;
        }
    }

    async fn dispatch_single(&self, out: &RoutedOutput) {
        enum Action {
            OscSend(Arc<UdpSocket>, SocketAddr, Vec<u8>, String),
            MidiLoop(String, Vec<u8>),
            WsBroadcast(broadcast::Sender<Message>, Message, String),
            WsClientSend(mpsc::UnboundedSender<Message>, Message, String),
            UdpSend(Arc<UdpSocket>, SocketAddr, Vec<u8>, String),
            None,
        }

        let action = {
            let guard = self.endpoints.lock();
            match guard.get(&out.target_device_id).map(|ep| &ep.sink) {
                Some(OutboundSink::Osc {
                    socket,
                    target: Some(addr),
                }) => match encode_osc_message(&out.target_port_logical_key, &out.value) {
                    Ok(bytes) => Action::OscSend(
                        Arc::clone(socket),
                        *addr,
                        bytes,
                        format!("{} -> {}", out.target_port_logical_key, out.value.summary_display()),
                    ),
                    Err(_) => Action::None,
                },
                Some(OutboundSink::MidiLoopback { port_name }) => {
                    let msg = MidiMessage::from_logical_key_and_value(
                        &out.target_port_logical_key,
                        &out.value,
                    );
                    Action::MidiLoop(port_name.clone(), msg.encode())
                }
                Some(OutboundSink::WebSocketServer { tx, framing }) => {
                    if let Some(msg) =
                        websocket::encode_ws_message(&out.target_port_logical_key, &out.value, *framing)
                    {
                        Action::WsBroadcast(
                            tx.clone(),
                            msg,
                            format!("{} -> {}", out.target_port_logical_key, out.value.summary_display()),
                        )
                    } else {
                        Action::None
                    }
                }
                Some(OutboundSink::WebSocketClient { tx, framing }) => {
                    if let Some(msg) =
                        websocket::encode_ws_message(&out.target_port_logical_key, &out.value, *framing)
                    {
                        Action::WsClientSend(
                            tx.clone(),
                            msg,
                            format!("{} -> {}", out.target_port_logical_key, out.value.summary_display()),
                        )
                    } else {
                        Action::None
                    }
                }
                Some(OutboundSink::Udp {
                    socket,
                    target: Some(addr),
                    format,
                }) => {
                    let bytes =
                        udp::format_udp_payload(&out.target_port_logical_key, &out.value, *format);
                    Action::UdpSend(
                        Arc::clone(socket),
                        *addr,
                        bytes,
                        format!("{} -> {}", out.target_port_logical_key, out.value.summary_display()),
                    )
                }
                _ => Action::None,
            }
        };

        match action {
            Action::OscSend(socket, addr, bytes, summary) => {
                let _ = socket.send_to(&bytes, addr).await;
                self.telemetry.record_log(
                    "OSC",
                    "TX",
                    summary,
                    Some(out.target_device_id),
                    Some(out.target_port_id),
                );
            }
            Action::MidiLoop(port_name, bytes) => {
                self.loopback_midi.send(&port_name, &bytes);
                self.telemetry.record_log(
                    "MIDI",
                    "TX",
                    format!("{port_name} {:02X?}", bytes),
                    Some(out.target_device_id),
                    Some(out.target_port_id),
                );
            }
            Action::WsBroadcast(tx, msg, summary) => {
                let _ = tx.send(msg);
                self.telemetry.record_log(
                    "WS",
                    "TX",
                    summary,
                    Some(out.target_device_id),
                    Some(out.target_port_id),
                );
            }
            Action::WsClientSend(tx, msg, summary) => {
                let _ = tx.send(msg);
                self.telemetry.record_log(
                    "WS",
                    "TX",
                    summary,
                    Some(out.target_device_id),
                    Some(out.target_port_id),
                );
            }
            Action::UdpSend(socket, addr, bytes, summary) => {
                let _ = socket.send_to(&bytes, addr).await;
                self.telemetry.record_log(
                    "UDP",
                    "TX",
                    summary,
                    Some(out.target_device_id),
                    Some(out.target_port_id),
                );
            }
            Action::None => {}
        }
    }

    /// Starts (or restarts) a protocol endpoint for a `Device` in the `ControlPlane`.
    pub async fn start_endpoint(
        self: &Arc<Self>,
        device_id: Uuid,
    ) -> Result<EndpointStatus, ProtocolError> {
        self.stop_endpoint(device_id);

        let snap = self.control.snapshot();
        let dev = snap
            .devices
            .into_iter()
            .find(|d| d.id == device_id)
            .ok_or(ProtocolError::DeviceNotFound(device_id))?;

        match dev.kind {
            DeviceKind::OscEndpoint {
                bind_addr,
                target_addr,
                auto_discover,
            } => {
                self.start_osc(device_id, &bind_addr, target_addr.as_deref(), auto_discover)
                    .await
            }
            DeviceKind::MidiPort {
                port_name,
                backend,
                ..
            } => self.start_midi(device_id, &port_name, backend).await,
            DeviceKind::WebSocketEndpoint {
                mode,
                url_or_bind,
                framing,
            } => {
                self.start_websocket(device_id, mode, &url_or_bind, framing)
                    .await
            }
            DeviceKind::UdpEndpoint {
                bind_addr,
                target_addr,
                multicast_group,
                broadcast,
                format,
            } => {
                self.start_udp(
                    device_id,
                    &bind_addr,
                    target_addr.as_deref(),
                    multicast_group.as_deref(),
                    broadcast,
                    format,
                )
                .await
            }
            other => Err(ProtocolError::Endpoint(format!(
                "Endpoint kind {:?} does not run an M1 protocol transport",
                other
            ))),
        }
    }

    async fn start_osc(
        self: &Arc<Self>,
        device_id: Uuid,
        bind_addr: &str,
        target_addr: Option<&str>,
        auto_discover: bool,
    ) -> Result<EndpointStatus, ProtocolError> {
        let socket = Arc::new(UdpSocket::bind(bind_addr).await?);
        let local_addr = socket.local_addr()?.to_string();
        let parsed_target = match target_addr {
            Some(t) if !t.trim().is_empty() => Some(
                t.parse::<SocketAddr>()
                    .map_err(|e| ProtocolError::Endpoint(format!("invalid OSC target '{t}': {e}")))?,
            ),
            _ => None,
        };

        let mgr = Arc::clone(self);
        let rx_socket = Arc::clone(&socket);
        let task = tokio::spawn(async move {
            let mut buf = vec![0u8; 65536];
            loop {
                let Ok((len, _peer)) = rx_socket.recv_from(&mut buf).await else {
                    break;
                };
                let Ok(msgs) = decode_osc_bytes(&buf[..len]) else {
                    continue;
                };
                let ts = SignalEvent::now_ns();
                for msg in msgs {
                    let val = osc_args_to_signal(&msg.args);
                    mgr.telemetry.record_log(
                        "OSC",
                        "RX",
                        format!("{} = {}", msg.addr, val.summary_display()),
                        Some(device_id),
                        None,
                    );
                    let matched_ports = mgr.resolve_or_discover_osc_ports(
                        device_id,
                        &msg.addr,
                        auto_discover,
                    );
                    for pid in matched_ports {
                        let outputs = mgr.router.route(SignalEvent::new(pid, val.clone(), ts));
                        if !outputs.is_empty() {
                            mgr.dispatch_outputs(outputs).await;
                        }
                    }
                }
            }
        });

        let status = EndpointStatus {
            device_id,
            protocol: "OSC".into(),
            running: true,
            bound_address: Some(local_addr.clone()),
            detail: format!("Bound {local_addr}"),
        };

        self.endpoints.lock().insert(
            device_id,
            RunningEndpoint {
                status: status.clone(),
                sink: OutboundSink::Osc {
                    socket,
                    target: parsed_target,
                },
                tasks: vec![task],
            },
        );
        Ok(status)
    }

    async fn start_midi(
        self: &Arc<Self>,
        device_id: Uuid,
        port_name: &str,
        _backend: MidiBackendKind,
    ) -> Result<EndpointStatus, ProtocolError> {
        let name = if port_name.trim().is_empty() {
            "Kontrolfreak Loopback 1".to_string()
        } else {
            port_name.to_string()
        };
        self.loopback_midi.ensure_port(&name);

        let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
        self.loopback_midi.subscribe(
            &name,
            Arc::new(move |bytes| {
                let _ = tx.send(bytes.to_vec());
            }),
        );

        let mgr = Arc::clone(self);
        let pname_log = name.clone();
        let task = tokio::spawn(async move {
            while let Some(bytes) = rx.recv().await {
                if let Some(msg) = MidiMessage::decode(&bytes) {
                    let (key, val) = msg.to_logical_key_and_value();
                    mgr.telemetry.record_log(
                        "MIDI",
                        "RX",
                        format!("{pname_log} {key} = {}", val.summary_display()),
                        Some(device_id),
                        None,
                    );
                    if let Some(pid) =
                        mgr.resolve_or_discover_port(device_id, &key, SignalKind::Midi, true)
                    {
                        let outputs = mgr
                            .router
                            .route(SignalEvent::new(pid, val, SignalEvent::now_ns()));
                        if !outputs.is_empty() {
                            mgr.dispatch_outputs(outputs).await;
                        }
                    }
                }
            }
        });

        let status = EndpointStatus {
            device_id,
            protocol: "MIDI".into(),
            running: true,
            bound_address: Some(name.clone()),
            detail: format!("MIDI port '{name}' active"),
        };

        self.endpoints.lock().insert(
            device_id,
            RunningEndpoint {
                status: status.clone(),
                sink: OutboundSink::MidiLoopback { port_name: name },
                tasks: vec![task],
            },
        );
        Ok(status)
    }

    async fn start_websocket(
        self: &Arc<Self>,
        device_id: Uuid,
        mode: WsMode,
        url_or_bind: &str,
        framing: WsFraming,
    ) -> Result<EndpointStatus, ProtocolError> {
        match mode {
            WsMode::Server => {
                let listener = TcpListener::bind(url_or_bind).await?;
                let local_addr = listener.local_addr()?.to_string();
                let (bcast_tx, _) = broadcast::channel::<Message>(256);
                let bcast_tx_accept = bcast_tx.clone();
                let mgr = Arc::clone(self);

                let task = tokio::spawn(async move {
                    loop {
                        let Ok((stream, _peer)) = listener.accept().await else {
                            break;
                        };
                        let _ = stream.set_nodelay(true);
                        let mgr_c = Arc::clone(&mgr);
                        let mut rx_out = bcast_tx_accept.subscribe();
                        tokio::spawn(async move {
                            let Ok(ws_stream) = tokio_tungstenite::accept_async(stream).await else {
                                return;
                            };
                            let (mut write, mut read) = ws_stream.split();
                            let writer = tokio::spawn(async move {
                                while let Ok(msg) = rx_out.recv().await {
                                    if write.send(msg).await.is_err() {
                                        break;
                                    }
                                }
                            });
                            while let Some(Ok(msg)) = read.next().await {
                                let pairs = websocket::decode_ws_message(&msg, framing);
                                let ts = SignalEvent::now_ns();
                                for (key, val) in pairs {
                                    mgr_c.telemetry.record_log(
                                        "WS",
                                        "RX",
                                        format!("{key} = {}", val.summary_display()),
                                        Some(device_id),
                                        None,
                                    );
                                    if let Some(pid) = mgr_c.resolve_or_discover_port(
                                        device_id,
                                        &key,
                                        SignalKind::Control,
                                        true,
                                    ) {
                                        let outputs = mgr_c
                                            .router
                                            .route(SignalEvent::new(pid, val, ts));
                                        if !outputs.is_empty() {
                                            mgr_c.dispatch_outputs(outputs).await;
                                        }
                                    }
                                }
                            }
                            writer.abort();
                        });
                    }
                });

                let status = EndpointStatus {
                    device_id,
                    protocol: "WebSocket Server".into(),
                    running: true,
                    bound_address: Some(format!("ws://{local_addr}")),
                    detail: format!("Listening ws://{local_addr}"),
                };
                self.endpoints.lock().insert(
                    device_id,
                    RunningEndpoint {
                        status: status.clone(),
                        sink: OutboundSink::WebSocketServer {
                            tx: bcast_tx,
                            framing,
                        },
                        tasks: vec![task],
                    },
                );
                Ok(status)
            }
            WsMode::Client => {
                let (ws_stream, _) =
                    tokio_tungstenite::connect_async_with_config(url_or_bind, None, true)
                        .await
                        .map_err(|e| ProtocolError::Endpoint(format!("WS connect error: {e}")))?;
                let (mut write, mut read) = ws_stream.split();
                let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Message>();

                let writer = tokio::spawn(async move {
                    while let Some(msg) = out_rx.recv().await {
                        if write.send(msg).await.is_err() {
                            break;
                        }
                    }
                });

                let mgr = Arc::clone(self);
                let reader = tokio::spawn(async move {
                    while let Some(Ok(msg)) = read.next().await {
                        let pairs = websocket::decode_ws_message(&msg, framing);
                        let ts = SignalEvent::now_ns();
                        for (key, val) in pairs {
                            mgr.telemetry.record_log(
                                "WS",
                                "RX",
                                format!("{key} = {}", val.summary_display()),
                                Some(device_id),
                                None,
                            );
                            if let Some(pid) = mgr.resolve_or_discover_port(
                                device_id,
                                &key,
                                SignalKind::Control,
                                true,
                            ) {
                                let outputs = mgr.router.route(SignalEvent::new(pid, val, ts));
                                if !outputs.is_empty() {
                                    mgr.dispatch_outputs(outputs).await;
                                }
                            }
                        }
                    }
                });

                let status = EndpointStatus {
                    device_id,
                    protocol: "WebSocket Client".into(),
                    running: true,
                    bound_address: Some(url_or_bind.to_string()),
                    detail: format!("Connected {url_or_bind}"),
                };
                self.endpoints.lock().insert(
                    device_id,
                    RunningEndpoint {
                        status: status.clone(),
                        sink: OutboundSink::WebSocketClient {
                            tx: out_tx,
                            framing,
                        },
                        tasks: vec![writer, reader],
                    },
                );
                Ok(status)
            }
        }
    }

    async fn start_udp(
        self: &Arc<Self>,
        device_id: Uuid,
        bind_addr: &str,
        target_addr: Option<&str>,
        multicast_group: Option<&str>,
        broadcast: bool,
        format: UdpFormat,
    ) -> Result<EndpointStatus, ProtocolError> {
        let socket = Arc::new(UdpSocket::bind(bind_addr).await?);
        if broadcast {
            let _ = socket.set_broadcast(true);
        }
        if let Some(mcast) = multicast_group {
            if let Ok(group_ip) = mcast.parse::<Ipv4Addr>() {
                let _ = socket.join_multicast_v4(group_ip, Ipv4Addr::UNSPECIFIED);
            }
        }
        let local_addr = socket.local_addr()?.to_string();
        let parsed_target = match target_addr {
            Some(t) if !t.trim().is_empty() => Some(
                t.parse::<SocketAddr>()
                    .map_err(|e| ProtocolError::Endpoint(format!("invalid UDP target '{t}': {e}")))?,
            ),
            _ => None,
        };

        let mgr = Arc::clone(self);
        let rx_socket = Arc::clone(&socket);
        let task = tokio::spawn(async move {
            let mut buf = vec![0u8; 65536];
            loop {
                let Ok((len, _peer)) = rx_socket.recv_from(&mut buf).await else {
                    break;
                };
                let pairs = udp::parse_udp_payload(&buf[..len], format);
                let ts = SignalEvent::now_ns();
                for (key, val) in pairs {
                    mgr.telemetry.record_log(
                        "UDP",
                        "RX",
                        format!("{key} = {}", val.summary_display()),
                        Some(device_id),
                        None,
                    );
                    if let Some(pid) =
                        mgr.resolve_or_discover_port(device_id, &key, SignalKind::Control, true)
                    {
                        let outputs = mgr.router.route(SignalEvent::new(pid, val, ts));
                        if !outputs.is_empty() {
                            mgr.dispatch_outputs(outputs).await;
                        }
                    }
                }
            }
        });

        let status = EndpointStatus {
            device_id,
            protocol: "UDP".into(),
            running: true,
            bound_address: Some(local_addr.clone()),
            detail: format!("UDP {local_addr} ({format:?})"),
        };
        self.endpoints.lock().insert(
            device_id,
            RunningEndpoint {
                status: status.clone(),
                sink: OutboundSink::Udp {
                    socket,
                    target: parsed_target,
                    format,
                },
                tasks: vec![task],
            },
        );
        Ok(status)
    }

    fn resolve_or_discover_osc_ports(
        &self,
        device_id: Uuid,
        address: &str,
        auto_discover: bool,
    ) -> Vec<Uuid> {
        let compiled = self.router.load_compiled();
        let mut matched: Vec<Uuid> = compiled
            .emitting_ports_by_device
            .get(&device_id)
            .map(|ports| {
                ports
                    .iter()
                    .filter(|(key, _)| osc_address_matches(key, address))
                    .map(|(_, pid)| *pid)
                    .collect()
            })
            .unwrap_or_default();
        if matched.is_empty() && auto_discover {
            let alias = address.trim_start_matches('/').replace('/', " ");
            let new_port = Port::new(
                address,
                if alias.is_empty() { address } else { &alias },
                PortDirection::Out,
                SignalKind::Control,
            );
            if let Ok(added) = self.control.add_port(device_id, new_port) {
                matched.push(added.id);
            }
        }
        matched
    }

    fn resolve_or_discover_port(
        &self,
        device_id: Uuid,
        logical_key: &str,
        signal_kind: SignalKind,
        auto_discover: bool,
    ) -> Option<Uuid> {
        let compiled = self.router.load_compiled();
        if let Some(&pid) = compiled
            .emitting_port_by_key
            .get(&(device_id, logical_key.to_string()))
        {
            return Some(pid);
        }
        if auto_discover {
            let new_port = Port::new(logical_key, logical_key, PortDirection::Out, signal_kind);
            if let Ok(added) = self.control.add_port(device_id, new_port) {
                return Some(added.id);
            }
        }
        None
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use kf_core::{Device, Modifier, SignalValue, WorkspaceGraph};
    use std::time::{Duration, Instant};

    #[tokio::test]
    async fn osc_to_midi_roundtrip_under_2ms() {
        let cp = Arc::new(ControlPlane::new(WorkspaceGraph::new("OSC->MIDI")).unwrap());
        let telemetry = Arc::new(TelemetryAggregator::default());
        let router = Arc::new(SignalRouter::new(cp.compiled_handle(), Arc::clone(&telemetry)));
        let mgr = ProtocolManager::new(Arc::clone(&cp), Arc::clone(&router), Arc::clone(&telemetry));

        let osc_port = Port::new("/fader/*", "Any Fader", PortDirection::Out, SignalKind::Control);
        let midi_port = Port::new("cc.1.7", "Master Vol", PortDirection::In, SignalKind::Midi);
        let osc_pid = osc_port.id;
        let midi_pid = midi_port.id;

        let osc_dev = cp
            .add_device(
                Device::new(
                    "osc.rx",
                    "OSC In",
                    DeviceKind::OscEndpoint {
                        bind_addr: "127.0.0.1:0".into(),
                        target_addr: None,
                        auto_discover: true,
                    },
                )
                .with_port(osc_port),
            )
            .unwrap();
        let midi_dev = cp
            .add_device(
                Device::new(
                    "midi.tx",
                    "Virtual Synth",
                    DeviceKind::MidiPort {
                        port_name: "TestLoopOut".into(),
                        is_virtual: true,
                        channel_filter: None,
                        backend: MidiBackendKind::LoopbackMock,
                    },
                )
                .with_port(midi_port),
            )
            .unwrap();

        let wire = cp.add_wire(osc_pid, midi_pid).unwrap();
        cp.update_wire_modifiers(wire.id, vec![Modifier::Invert]).unwrap();

        let osc_status = mgr.start_endpoint(osc_dev.id).await.unwrap();
        let _midi_status = mgr.start_endpoint(midi_dev.id).await.unwrap();
        let osc_addr: SocketAddr = osc_status.bound_address.unwrap().parse().unwrap();

        let (done_tx, mut done_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        mgr.loopback_midi().subscribe(
            "TestLoopOut",
            Arc::new(move |bytes| {
                let _ = done_tx.send(bytes.to_vec());
            }),
        );

        let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let t0 = Instant::now();
        // Send OSC value 0.25 -> inverted to 0.75 -> MIDI CC 7 ~= 95
        osc::send_osc_udp(&sender, osc_addr, "/fader/3", &SignalValue::Float(0.25))
            .await
            .unwrap();

        let midi_bytes = tokio::time::timeout(Duration::from_millis(200), done_rx.recv())
            .await
            .expect("timed out waiting for MIDI output")
            .expect("channel closed");
        let elapsed = t0.elapsed();

        assert_eq!(midi_bytes[0], 0xB0);
        assert_eq!(midi_bytes[1], 7);
        assert_eq!(midi_bytes[2], 95); // round(0.75 * 127)
        assert!(
            elapsed < Duration::from_millis(20),
            "local OSC->MIDI took {elapsed:?}"
        );
        // Verify average latency across 20 warm packets is well under 2ms
        let warm_start = Instant::now();
        for _ in 0..20 {
            osc::send_osc_udp(&sender, osc_addr, "/fader/3", &SignalValue::Float(0.5))
                .await
                .unwrap();
            let _ = tokio::time::timeout(Duration::from_millis(100), done_rx.recv())
                .await
                .unwrap();
        }
        let avg = warm_start.elapsed() / 20;
        assert!(
            avg < Duration::from_millis(2),
            "expected < 2ms average OSC->MIDI round-trip, got {avg:?}"
        );

        // Verify dynamic OSC parameter discovery on an unknown address
        osc::send_osc_udp(&sender, osc_addr, "/brand/new/knob", &SignalValue::Float(0.9))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(25)).await;
        let updated = cp.snapshot();
        let dev_after = updated.devices.iter().find(|d| d.id == osc_dev.id).unwrap();
        assert!(
            dev_after
                .ports
                .iter()
                .any(|p| p.logical_key == "/brand/new/knob"),
            "OSC dynamic parameter discovery should register new port"
        );

        mgr.stop_all();
    }

    #[tokio::test]
    async fn websocket_to_osc_roundtrip_under_2ms() {
        let cp = Arc::new(ControlPlane::new(WorkspaceGraph::new("WS->OSC")).unwrap());
        let telemetry = Arc::new(TelemetryAggregator::default());
        let router = Arc::new(SignalRouter::new(cp.compiled_handle(), Arc::clone(&telemetry)));
        let mgr = ProtocolManager::new(Arc::clone(&cp), Arc::clone(&router), Arc::clone(&telemetry));

        // External UDP receiver simulating downstream OSC app
        let ext_osc_rx = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let ext_osc_addr = ext_osc_rx.local_addr().unwrap();

        let ws_port = Port::new("/brightness", "Brightness", PortDirection::Out, SignalKind::Control);
        let osc_out_port = Port::new("/light/dimmer", "Dimmer", PortDirection::In, SignalKind::Control);
        let ws_pid = ws_port.id;
        let osc_pid = osc_out_port.id;

        let ws_dev = cp
            .add_device(
                Device::new(
                    "ws.srv",
                    "Web UI WS",
                    DeviceKind::WebSocketEndpoint {
                        mode: WsMode::Server,
                        url_or_bind: "127.0.0.1:0".into(),
                        framing: WsFraming::Json,
                    },
                )
                .with_port(ws_port),
            )
            .unwrap();
        let osc_dev = cp
            .add_device(
                Device::new(
                    "osc.tx",
                    "Lighting Desk",
                    DeviceKind::OscEndpoint {
                        bind_addr: "127.0.0.1:0".into(),
                        target_addr: Some(ext_osc_addr.to_string()),
                        auto_discover: false,
                    },
                )
                .with_port(osc_out_port),
            )
            .unwrap();

        cp.add_wire(ws_pid, osc_pid).unwrap();

        let ws_status = mgr.start_endpoint(ws_dev.id).await.unwrap();
        let _osc_status = mgr.start_endpoint(osc_dev.id).await.unwrap();
        let ws_url = ws_status.bound_address.unwrap();

        let (mut client_ws, _) =
            tokio_tungstenite::connect_async_with_config(&ws_url, None, true)
                .await
                .unwrap();
        let mut buf = [0u8; 2048];

        // Warm up 1 packet then measure 20-packet average
        client_ws
            .send(Message::Text(r#"{"key":"/brightness","value":0.8}"#.into()))
            .await
            .unwrap();
        let (len, _) = tokio::time::timeout(Duration::from_millis(200), ext_osc_rx.recv_from(&mut buf))
            .await
            .unwrap()
            .unwrap();
        let msgs = decode_osc_bytes(&buf[..len]).unwrap();
        assert_eq!(msgs[0].addr, "/light/dimmer");
        let v = osc_args_to_signal(&msgs[0].args).as_f64().unwrap();
        assert!((v - 0.8).abs() < 1e-4);

        let t0 = Instant::now();
        for i in 0..20 {
            let val = i as f64 / 20.0;
            client_ws
                .send(Message::Text(format!(r#"{{"key":"/brightness","value":{val}}}"#)))
                .await
                .unwrap();
            let _ = tokio::time::timeout(Duration::from_millis(100), ext_osc_rx.recv_from(&mut buf))
                .await
                .unwrap()
                .unwrap();
        }
        let avg = t0.elapsed() / 20;
        assert!(
            avg < Duration::from_millis(2),
            "expected < 2ms average WS->OSC round-trip, got {avg:?}"
        );

        mgr.stop_all();
    }

    #[tokio::test]
    async fn raw_udp_routing_works() {
        let cp = Arc::new(ControlPlane::new(WorkspaceGraph::new("UDP")).unwrap());
        let telemetry = Arc::new(TelemetryAggregator::default());
        let router = Arc::new(SignalRouter::new(cp.compiled_handle(), Arc::clone(&telemetry)));
        let mgr = ProtocolManager::new(Arc::clone(&cp), Arc::clone(&router), Arc::clone(&telemetry));

        let ext_rx = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let ext_addr = ext_rx.local_addr().unwrap();

        let p_in = Port::new("col.1", "CSV Col 1", PortDirection::Out, SignalKind::Control);
        let p_out = Port::new("out.val", "JSON Out", PortDirection::In, SignalKind::Control);
        let p_in_id = p_in.id;
        let p_out_id = p_out.id;

        let d_in = cp
            .add_device(
                Device::new(
                    "udp.csv",
                    "Sensor CSV",
                    DeviceKind::UdpEndpoint {
                        bind_addr: "127.0.0.1:0".into(),
                        target_addr: None,
                        multicast_group: None,
                        broadcast: false,
                        format: UdpFormat::Csv,
                    },
                )
                .with_port(p_in),
            )
            .unwrap();
        let d_out = cp
            .add_device(
                Device::new(
                    "udp.json",
                    "Remote JSON",
                    DeviceKind::UdpEndpoint {
                        bind_addr: "127.0.0.1:0".into(),
                        target_addr: Some(ext_addr.to_string()),
                        multicast_group: None,
                        broadcast: false,
                        format: UdpFormat::Json,
                    },
                )
                .with_port(p_out),
            )
            .unwrap();

        cp.add_wire(p_in_id, p_out_id).unwrap();
        let st_in = mgr.start_endpoint(d_in.id).await.unwrap();
        let _st_out = mgr.start_endpoint(d_out.id).await.unwrap();

        let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let target: SocketAddr = st_in.bound_address.unwrap().parse().unwrap();
        sender.send_to(b"0.1,0.65,0.9", target).await.unwrap();

        let mut buf = [0u8; 512];
        let (len, _) = tokio::time::timeout(Duration::from_millis(200), ext_rx.recv_from(&mut buf))
            .await
            .unwrap()
            .unwrap();
        let body = std::str::from_utf8(&buf[..len]).unwrap();
        assert!(body.contains("0.65"), "unexpected UDP JSON output: {body}");

        mgr.stop_all();
    }
}
