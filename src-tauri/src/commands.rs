use crate::state::AppState;
use kf_core::{
    sample_transfer_curve, Device, DeviceFingerprint, DeviceKind, DevicePreset, DeviceStatus,
    GroupPreset, MacroBinding, MacroTarget, MappingPreset, Modifier, Port, ProjectFile, RackGroup,
    SignalEvent, SignalValue, TelemetryBatch, Wire, WorkspaceGraph, WorkspaceSettings,
};
use kf_protocols::midi::{MidiPortInfo, VirtualMidiGuidance};
use kf_protocols::EndpointStatus;
use kf_hardware::{HardwareStub, HardwareWatcher};
use kf_sidecar_ipc::{SidecarClient, SidecarClientStub, SidecarHealthStatus};
use kf_video::{NdiRuntimeLoader, NdiRuntimeStatus, NdiRuntimeStub};
use serde::Serialize;
use std::collections::HashSet;
use std::time::Duration;
use tauri::ipc::Channel;
use tauri::State;
use uuid::Uuid;

type CmdResult<T> = Result<T, String>;

fn err_str(e: impl std::fmt::Display) -> String {
    e.to_string()
}

// ---------- Graph CRUD ----------

#[tauri::command]
pub fn get_workspace(state: State<'_, AppState>) -> WorkspaceGraph {
    state.control.snapshot()
}

#[tauri::command]
pub fn add_device(state: State<'_, AppState>, device: Device) -> CmdResult<Device> {
    state.control.add_device(device).map_err(err_str)
}

#[tauri::command]
pub fn remove_device(state: State<'_, AppState>, device_id: Uuid) -> CmdResult<()> {
    state.protocols.stop_endpoint(device_id);
    state.control.remove_device(device_id).map_err(err_str)
}

#[tauri::command]
pub fn update_device_position(
    state: State<'_, AppState>,
    device_id: Uuid,
    x: f64,
    y: f64,
) -> CmdResult<()> {
    state
        .control
        .update_device_position(device_id, x, y)
        .map_err(err_str)
}

#[tauri::command]
pub fn rename_device(
    state: State<'_, AppState>,
    device_id: Uuid,
    display_name: String,
) -> CmdResult<Device> {
    state
        .control
        .rename_device(device_id, display_name)
        .map_err(err_str)
}

#[tauri::command]
pub fn rename_port(
    state: State<'_, AppState>,
    device_id: Uuid,
    port_id: Uuid,
    alias: String,
) -> CmdResult<Port> {
    state
        .control
        .rename_port(device_id, port_id, alias)
        .map_err(err_str)
}

#[tauri::command]
pub fn add_port(state: State<'_, AppState>, device_id: Uuid, port: Port) -> CmdResult<Port> {
    state.control.add_port(device_id, port).map_err(err_str)
}

#[tauri::command]
pub fn remove_port(state: State<'_, AppState>, device_id: Uuid, port_id: Uuid) -> CmdResult<()> {
    state
        .control
        .remove_port(device_id, port_id)
        .map_err(err_str)
}

#[tauri::command]
pub fn update_device_kind(
    state: State<'_, AppState>,
    device_id: Uuid,
    kind: DeviceKind,
) -> CmdResult<Device> {
    state
        .control
        .update_device_kind(device_id, kind)
        .map_err(err_str)
}

// ---------- Racks & Macros ----------

#[tauri::command]
pub fn create_rack(
    state: State<'_, AppState>,
    display_name: String,
    member_node_ids: Vec<Uuid>,
) -> CmdResult<RackGroup> {
    state
        .control
        .create_rack_group(display_name, member_node_ids)
        .map_err(err_str)
}

#[tauri::command]
pub fn ungroup_rack(state: State<'_, AppState>, group_id: Uuid) -> CmdResult<()> {
    state.control.ungroup_rack(group_id).map_err(err_str)
}

#[tauri::command]
pub fn rename_rack(
    state: State<'_, AppState>,
    group_id: Uuid,
    display_name: String,
) -> CmdResult<RackGroup> {
    state
        .control
        .rename_rack(group_id, display_name)
        .map_err(err_str)
}

#[tauri::command]
pub fn set_rack_collapsed(
    state: State<'_, AppState>,
    group_id: Uuid,
    collapsed: bool,
) -> CmdResult<RackGroup> {
    state
        .control
        .set_rack_collapsed(group_id, collapsed)
        .map_err(err_str)
}

#[tauri::command]
pub fn expose_macro(
    state: State<'_, AppState>,
    group_id: Uuid,
    alias: String,
    targets: Vec<MacroTarget>,
) -> CmdResult<MacroBinding> {
    state
        .control
        .expose_macro(group_id, alias, targets)
        .map_err(err_str)
}

#[tauri::command]
pub fn remove_macro(
    state: State<'_, AppState>,
    group_id: Uuid,
    macro_id: Uuid,
) -> CmdResult<()> {
    state
        .control
        .remove_macro(group_id, macro_id)
        .map_err(err_str)
}

#[tauri::command]
pub async fn set_macro_value(
    state: State<'_, AppState>,
    group_id: Uuid,
    macro_id: Uuid,
    value: f64,
) -> CmdResult<MacroBinding> {
    let binding = state
        .control
        .set_macro_value(group_id, macro_id, value)
        .map_err(err_str)?;
    let outputs = state
        .router
        .route_macro(macro_id, binding.value, SignalEvent::now_ns());
    if !outputs.is_empty() {
        state.protocols.dispatch_outputs(outputs).await;
    }
    Ok(binding)
}

// ---------- Wires & Modifiers ----------

#[tauri::command]
pub fn add_wire(
    state: State<'_, AppState>,
    from_port_id: Uuid,
    to_port_id: Uuid,
) -> CmdResult<Wire> {
    state
        .control
        .add_wire(from_port_id, to_port_id)
        .map_err(err_str)
}

#[tauri::command]
pub fn remove_wire(state: State<'_, AppState>, wire_id: Uuid) -> CmdResult<()> {
    state.control.remove_wire(wire_id).map_err(err_str)
}

#[tauri::command]
pub fn toggle_wire(
    state: State<'_, AppState>,
    wire_id: Uuid,
    enabled: bool,
) -> CmdResult<Wire> {
    state
        .control
        .toggle_wire(wire_id, enabled)
        .map_err(err_str)
}

#[tauri::command]
pub fn update_wire_modifiers(
    state: State<'_, AppState>,
    wire_id: Uuid,
    modifiers: Vec<Modifier>,
) -> CmdResult<Wire> {
    state
        .control
        .update_wire_modifiers(wire_id, modifiers)
        .map_err(err_str)
}

#[tauri::command]
pub fn compute_transfer_curve(
    modifiers: Vec<Modifier>,
    steps: Option<usize>,
) -> CmdResult<Vec<[f64; 2]>> {
    sample_transfer_curve(&modifiers, steps.unwrap_or(65)).map_err(err_str)
}

// ---------- Signal Injection & Overrides ----------

#[tauri::command]
pub async fn inject_signal(
    state: State<'_, AppState>,
    port_id: Uuid,
    value: SignalValue,
) -> CmdResult<usize> {
    let outputs = state
        .router
        .route(SignalEvent::new(port_id, value, SignalEvent::now_ns()));
    let count = outputs.len();
    if !outputs.is_empty() {
        state.protocols.dispatch_outputs(outputs).await;
    }
    Ok(count)
}

#[tauri::command]
pub async fn set_manual_override(
    state: State<'_, AppState>,
    device_id: Uuid,
    port_id: Uuid,
    value: Option<SignalValue>,
) -> CmdResult<()> {
    state
        .control
        .set_manual_override(device_id, port_id, value.clone())
        .map_err(err_str)?;
    if let Some(v) = value {
        let outputs = state
            .router
            .route(SignalEvent::new(port_id, v, SignalEvent::now_ns()));
        if !outputs.is_empty() {
            state.protocols.dispatch_outputs(outputs).await;
        }
    }
    Ok(())
}

// ---------- Protocols & Hardware ----------

#[tauri::command]
pub async fn start_protocol_endpoint(
    state: State<'_, AppState>,
    device_id: Uuid,
) -> CmdResult<EndpointStatus> {
    state
        .protocols
        .start_endpoint(device_id)
        .await
        .map_err(err_str)
}

#[tauri::command]
pub fn stop_protocol_endpoint(state: State<'_, AppState>, device_id: Uuid) {
    state.protocols.stop_endpoint(device_id);
}

#[tauri::command]
pub fn get_protocol_statuses(state: State<'_, AppState>) -> Vec<EndpointStatus> {
    state.protocols.statuses()
}

#[tauri::command]
pub fn list_midi_ports(state: State<'_, AppState>) -> Vec<MidiPortInfo> {
    state.protocols.list_midi_ports()
}

#[tauri::command]
pub fn get_virtual_midi_guidance(state: State<'_, AppState>) -> VirtualMidiGuidance {
    state.protocols.virtual_midi_guidance()
}

// ---------- Presets, Projects & Offline Ghost Rebinding ----------

#[tauri::command]
pub fn export_project_json(state: State<'_, AppState>) -> CmdResult<String> {
    ProjectFile::from_graph(state.control.snapshot())
        .to_json_pretty()
        .map_err(err_str)
}

#[tauri::command]
pub fn import_project_json(
    state: State<'_, AppState>,
    json: String,
    reconcile_hardware: Option<bool>,
) -> CmdResult<WorkspaceGraph> {
    state.protocols.stop_all();
    let mut proj = ProjectFile::from_json(&json).map_err(err_str)?;
    if reconcile_hardware.unwrap_or(true) {
        let avail: Vec<DeviceFingerprint> = state
            .protocols
            .list_midi_ports()
            .into_iter()
            .map(|p| DeviceFingerprint {
                port_name: Some(p.name),
                ..Default::default()
            })
            .collect();
        proj.reconcile_hardware(&avail);
    }
    state
        .control
        .replace_graph(proj.graph.clone())
        .map_err(err_str)?;
    Ok(proj.graph)
}

#[tauri::command]
pub fn save_project_file(state: State<'_, AppState>, path: String) -> CmdResult<()> {
    ProjectFile::from_graph(state.control.snapshot())
        .save_to_path(&path)
        .map_err(err_str)
}

#[tauri::command]
pub fn load_project_file(
    state: State<'_, AppState>,
    path: String,
) -> CmdResult<WorkspaceGraph> {
    let json = std::fs::read_to_string(&path).map_err(err_str)?;
    import_project_json(state, json, Some(true))
}

#[tauri::command]
pub fn export_device_preset(state: State<'_, AppState>, device_id: Uuid) -> CmdResult<String> {
    let snap = state.control.snapshot();
    let dev = snap
        .devices
        .into_iter()
        .find(|d| d.id == device_id)
        .ok_or_else(|| format!("Device {device_id} not found"))?;
    let preset = DevicePreset::from_device(dev.display_name.clone(), dev);
    preset.to_json_pretty().map_err(err_str)
}

#[tauri::command]
pub fn import_device_preset(state: State<'_, AppState>, json: String) -> CmdResult<Device> {
    let mut preset = DevicePreset::from_json(&json).map_err(err_str)?;
    preset.device.id = Uuid::new_v4();
    for p in &mut preset.device.ports {
        p.id = Uuid::new_v4();
    }
    state.control.add_device(preset.device).map_err(err_str)
}

#[tauri::command]
pub fn export_group_preset(state: State<'_, AppState>, group_id: Uuid) -> CmdResult<String> {
    let snap = state.control.snapshot();
    let preset = GroupPreset::extract(&snap, group_id)
        .ok_or_else(|| format!("Rack group {group_id} not found"))?;
    preset.to_json_pretty().map_err(err_str)
}

#[tauri::command]
pub fn import_group_preset(state: State<'_, AppState>, json: String) -> CmdResult<WorkspaceGraph> {
    let preset = GroupPreset::from_json(&json).map_err(err_str)?;
    let (group, devices, wires) = preset.instantiate_fresh(40.0, 40.0);
    let mut snap = state.control.snapshot();
    snap.devices.extend(devices);
    snap.wires.extend(wires);
    snap.groups.push(group);
    state.control.replace_graph(snap.clone()).map_err(err_str)?;
    Ok(snap)
}

#[tauri::command]
pub fn export_mapping_preset(
    name: String,
    description: Option<String>,
    modifiers: Vec<Modifier>,
) -> CmdResult<String> {
    MappingPreset::new(name, description, modifiers)
        .to_json_pretty()
        .map_err(err_str)
}

#[tauri::command]
pub fn import_mapping_preset(json: String) -> CmdResult<MappingPreset> {
    MappingPreset::from_json(&json).map_err(err_str)
}

#[tauri::command]
pub fn set_device_status(
    state: State<'_, AppState>,
    device_id: Uuid,
    status: DeviceStatus,
) -> CmdResult<Device> {
    state
        .control
        .set_device_status(device_id, status)
        .map_err(err_str)
}

#[tauri::command]
pub fn rebind_offline_device(
    state: State<'_, AppState>,
    device_id: Uuid,
    new_fingerprint: Option<DeviceFingerprint>,
    new_port_name: Option<String>,
) -> CmdResult<Device> {
    state
        .control
        .rebind_device(device_id, new_fingerprint, new_port_name)
        .map_err(err_str)
}

// ---------- Settings & 30 Hz Batched Telemetry Channel ----------

#[tauri::command]
pub fn update_settings(
    state: State<'_, AppState>,
    settings: WorkspaceSettings,
) -> WorkspaceSettings {
    state.control.update_settings(settings)
}

#[tauri::command]
pub fn set_calm_mode(state: State<'_, AppState>, enabled: bool) -> WorkspaceSettings {
    let mut s = state.control.snapshot().settings;
    s.calm_mode = enabled;
    state.control.update_settings(s)
}

#[tauri::command]
pub fn update_telemetry_subscription(
    state: State<'_, AppState>,
    subscribed_ports: Option<Vec<Uuid>>,
) {
    let mut guard = state.subscribed_ports.lock();
    *guard = subscribed_ports.map(|v| v.into_iter().collect());
}

#[tauri::command]
pub fn subscribe_telemetry(
    state: State<'_, AppState>,
    on_batch: Channel<TelemetryBatch>,
    subscribed_ports: Option<Vec<Uuid>>,
) {
    {
        let mut guard = state.subscribed_ports.lock();
        *guard = subscribed_ports.map(|v| v.into_iter().collect());
    }

    let telemetry = state.telemetry.clone();
    let sub_ports = state.subscribed_ports.clone();
    let hz = state.control.snapshot().settings.telemetry_hz.clamp(10, 60);
    let interval_ms = (1000 / hz as u64).max(16);

    let mut pump = state.telemetry_pump.lock();
    if let Some(prev) = pump.take() {
        prev.abort();
    }

    let handle = tauri::async_runtime::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_millis(interval_ms));
        loop {
            ticker.tick().await;
            let filter: Option<HashSet<Uuid>> = sub_ports.lock().clone();
            let batch = telemetry.flush_batch(filter.as_ref());
            if on_batch.send(batch).is_err() {
                break;
            }
        }
    });
    *pump = Some(handle);
}

#[derive(Debug, Clone, Serialize)]
pub struct ExtensionCapabilitiesStatus {
    pub ndi_runtime: NdiRuntimeStatus,
    pub hid_device_count: usize,
    pub depth_sidecar_status: SidecarHealthStatus,
}

#[tauri::command]
pub fn get_extension_capabilities() -> ExtensionCapabilitiesStatus {
    let ndi_runtime = NdiRuntimeStub.probe_runtime();
    let hid_device_count = HardwareStub
        .list_hid_devices()
        .map(|v| v.len())
        .unwrap_or(0);
    let depth_sidecar_status = SidecarClientStub.status();
    ExtensionCapabilitiesStatus {
        ndi_runtime,
        hid_device_count,
        depth_sidecar_status,
    }
}

