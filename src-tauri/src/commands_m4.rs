//! Milestone 4 Tauri commands & runtime supervisor registry for crash-isolated depth
//! camera sidecars, 3D ROI extraction, Depth-to-RGBA/NDI previews, `.kfreak` ZIP bundles,
//! and live Diagnostics & Performance Profiler telemetry.

use crate::state::AppState;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use kf_core::{
    ControlPlane, DeviceFingerprint, DeviceStatus, FrameRef, ProjectFile, SignalEvent,
    SignalRouter, SignalValue, TelemetryAggregator, WorkspaceGraph,
};
use kf_sensor_depth::{
    build_depth_ndi_frame_ref, colorize_depth_to_rgba, extract_all_rois, list_vendor_adapters,
    roi_to_signal_bindings, CameraIntrinsics, DepthCameraBackend, SyntheticDepthBackend,
    VendorAdapterInfo,
};
use kf_sidecar_ipc::{
    export_kfreak_bytes, export_kfreak_file, import_kfreak_bytes, import_kfreak_file,
    locate_depth_sidecar_binary, DepthColormap, DiagnosticsProfiler, DiagnosticsSnapshot,
    KfreakManifest, NamedDepthRoi, RoiExtractionResult, SidecarHealthStatus, SidecarSupervisor,
    SidecarSupervisorSnapshot, SupervisorConfig,
};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tauri::State;
use uuid::Uuid;

type CmdResult<T> = Result<T, String>;

fn err_str(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DepthColormapConfig {
    pub palette: DepthColormap,
    pub near_mm: u16,
    pub far_mm: u16,
}

impl Default for DepthColormapConfig {
    fn default() -> Self {
        Self {
            palette: DepthColormap::Turbo,
            near_mm: 300,
            far_mm: 2500,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepthSidecarStatusDto {
    pub device_id: Uuid,
    pub health: SidecarSupervisorSnapshot,
    pub rois: Vec<NamedDepthRoi>,
    pub colormap: DepthColormapConfig,
    pub latest_results: Vec<RoiExtractionResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepthPreviewPacket {
    pub device_id: Uuid,
    pub frame_seq: u64,
    pub width: u32,
    pub height: u32,
    pub palette: DepthColormap,
    pub near_mm: u16,
    pub far_mm: u16,
    pub rgba_base64: String,
    pub frame_ref: FrameRef,
    pub roi_results: Vec<RoiExtractionResult>,
    pub health: SidecarSupervisorSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KfreakExportResult {
    pub filename: String,
    pub size_bytes: usize,
    pub manifest: KfreakManifest,
    pub zip_base64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KfreakImportResult {
    pub graph: WorkspaceGraph,
    pub manifest: KfreakManifest,
    pub device_preset_count: usize,
    pub group_preset_count: usize,
    pub mapping_preset_count: usize,
}

struct ManagedDepthSidecar {
    supervisor: Arc<SidecarSupervisor>,
    rois: Arc<Mutex<Vec<NamedDepthRoi>>>,
    colormap: Arc<Mutex<DepthColormapConfig>>,
    latest_results: Arc<Mutex<Vec<RoiExtractionResult>>>,
}

pub struct M4RuntimeState {
    control: Arc<ControlPlane>,
    router: Arc<SignalRouter>,
    telemetry: Arc<TelemetryAggregator>,
    pub profiler: Arc<DiagnosticsProfiler>,
    sidecars: Mutex<HashMap<Uuid, ManagedDepthSidecar>>,
    fallback_frame_idx: Mutex<u64>,
}

impl M4RuntimeState {
    pub fn new(
        control: Arc<ControlPlane>,
        router: Arc<SignalRouter>,
        telemetry: Arc<TelemetryAggregator>,
    ) -> Self {
        Self {
            control,
            router,
            telemetry,
            profiler: Arc::new(DiagnosticsProfiler::new()),
            sidecars: Mutex::new(HashMap::new()),
            fallback_frame_idx: Mutex::new(0),
        }
    }

    pub fn start_sidecar(
        &self,
        device_id: Uuid,
        vendor: String,
        width: u32,
        height: u32,
        fps: u32,
    ) -> Result<DepthSidecarStatusDto, String> {
        // Stop any previous supervisor for this device_id cleanly
        if let Some(prev) = self.sidecars.lock().remove(&device_id) {
            prev.supervisor.stop();
        }

        let bin = locate_depth_sidecar_binary().ok_or_else(|| {
            "kf-sensor-depth sidecar binary not found (build workspace with `cargo build -p kf-sensor-depth`)"
                .to_string()
        })?;

        let rois = Arc::new(Mutex::new(vec![NamedDepthRoi::default()]));
        let colormap = Arc::new(Mutex::new(DepthColormapConfig::default()));
        let latest_results = Arc::new(Mutex::new(Vec::<RoiExtractionResult>::new()));

        let mut cfg = SupervisorConfig::new(device_id, bin);
        cfg.vendor = vendor;
        cfg.width = width.clamp(32, 640);
        cfg.height = height.clamp(24, 480);
        cfg.fps = fps.clamp(5, 60);
        cfg.initial_rois = rois.lock().clone();

        let control_for_frame = Arc::clone(&self.control);
        let router_for_frame = Arc::clone(&self.router);
        let rois_for_frame = Arc::clone(&rois);
        let results_for_frame = Arc::clone(&latest_results);

        let on_frame = Arc::new(move |dev_id: Uuid, hdr: &kf_sidecar_ipc::ShmFrameHeader, pixels: &[u16]| {
            let active_rois = rois_for_frame.lock().clone();
            let extracted = extract_all_rois(
                pixels,
                hdr.width,
                hdr.height,
                &active_rois,
                CameraIntrinsics::default(),
            );
            *results_for_frame.lock() = extracted.clone();

            // Route primary ROI parameters + NDI FrameRef into kf-core's lock-free SignalRouter
            let snap = control_for_frame.snapshot();
            let Some(dev) = snap.devices.iter().find(|d| d.id == dev_id) else {
                return;
            };
            let ts = SignalEvent::now_ns();
            if let Some(primary) = extracted.first() {
                for (key, value) in roi_to_signal_bindings(primary) {
                    if let Some(port) = dev.find_port_by_key(key) {
                        let _ = router_for_frame.route(SignalEvent::new(port.id, value, ts));
                    }
                }
            }
            if let Some(ndi_port) = dev.find_port_by_key("depth.ndi_out") {
                let fref = build_depth_ndi_frame_ref(
                    dev_id,
                    hdr.frame_seq,
                    hdr.width,
                    hdr.height,
                    hdr.timestamp_ns,
                );
                let _ = router_for_frame.route(SignalEvent::new(
                    ndi_port.id,
                    SignalValue::FrameRef(fref),
                    ts,
                ));
            }
        });

        let control_for_status = Arc::clone(&self.control);
        let telemetry_for_status = Arc::clone(&self.telemetry);
        let on_status = Arc::new(
            move |dev_id: Uuid, status: SidecarHealthStatus, reason: Option<String>| {
                let dev_status = match status {
                    SidecarHealthStatus::Running => DeviceStatus::Online,
                    SidecarHealthStatus::CrashedRestarting => DeviceStatus::OfflineGhost {
                        reason: Some(
                            reason.unwrap_or_else(|| {
                                "Depth sidecar crashed — auto-restarting (core router unaffected)"
                                    .into()
                            }),
                        ),
                        last_seen_ms: Some(SignalEvent::now_ms()),
                    },
                    SidecarHealthStatus::Starting => DeviceStatus::OfflineGhost {
                        reason: Some("Starting isolated depth sidecar...".into()),
                        last_seen_ms: Some(SignalEvent::now_ms()),
                    },
                    SidecarHealthStatus::Degraded => DeviceStatus::Online,
                    SidecarHealthStatus::Stopped => DeviceStatus::OfflineGhost {
                        reason: Some("Depth sidecar stopped".into()),
                        last_seen_ms: Some(SignalEvent::now_ms()),
                    },
                };
                let _ = control_for_status.set_device_status(dev_id, dev_status);
                telemetry_for_status.record_log(
                    "DEPTH",
                    "SYS",
                    format!("Sidecar {dev_id} -> {status:?}"),
                    Some(dev_id),
                    None,
                );
            },
        );

        let supervisor = Arc::new(SidecarSupervisor::new(
            cfg,
            Some(Arc::clone(&self.profiler)),
            Some(on_frame),
            Some(on_status),
        ));
        supervisor.start().map_err(err_str)?;

        let managed = ManagedDepthSidecar {
            supervisor,
            rois,
            colormap,
            latest_results,
        };
        let dto = build_status_dto(device_id, &managed);
        self.sidecars.lock().insert(device_id, managed);
        Ok(dto)
    }

    pub fn status_or_default(&self, device_id: Uuid) -> DepthSidecarStatusDto {
        let guard = self.sidecars.lock();
        if let Some(m) = guard.get(&device_id) {
            return build_status_dto(device_id, m);
        }
        DepthSidecarStatusDto {
            device_id,
            health: SidecarSupervisorSnapshot {
                sidecar_id: device_id,
                vendor: "synthetic".into(),
                status: SidecarHealthStatus::Stopped,
                pid: None,
                uptime_ms: 0,
                restart_count: 0,
                consecutive_crashes: 0,
                last_crash_reason: None,
                frames_received: 0,
                fps: 0.0,
                width: 160,
                height: 120,
                ring_health: None,
            },
            rois: vec![NamedDepthRoi::default()],
            colormap: DepthColormapConfig::default(),
            latest_results: Vec::new(),
        }
    }
}

fn build_status_dto(device_id: Uuid, m: &ManagedDepthSidecar) -> DepthSidecarStatusDto {
    DepthSidecarStatusDto {
        device_id,
        health: m.supervisor.health_snapshot(),
        rois: m.rois.lock().clone(),
        colormap: m.colormap.lock().clone(),
        latest_results: m.latest_results.lock().clone(),
    }
}

// ---------- Tauri Commands ----------

#[tauri::command]
pub fn m4_list_depth_vendors() -> Vec<VendorAdapterInfo> {
    list_vendor_adapters()
}

#[tauri::command]
pub fn m4_start_depth_sidecar(
    state: State<'_, AppState>,
    device_id: Uuid,
    vendor: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    fps: Option<u32>,
) -> CmdResult<DepthSidecarStatusDto> {
    state.m4.start_sidecar(
        device_id,
        vendor.unwrap_or_else(|| "synthetic".into()),
        width.unwrap_or(160),
        height.unwrap_or(120),
        fps.unwrap_or(30),
    )
}

#[tauri::command]
pub fn m4_stop_depth_sidecar(
    state: State<'_, AppState>,
    device_id: Uuid,
) -> DepthSidecarStatusDto {
    if let Some(managed) = state.m4.sidecars.lock().get(&device_id) {
        managed.supervisor.stop();
    }
    state.m4.status_or_default(device_id)
}

#[tauri::command]
pub fn m4_restart_depth_sidecar(
    state: State<'_, AppState>,
    device_id: Uuid,
) -> CmdResult<DepthSidecarStatusDto> {
    if let Some(managed) = state.m4.sidecars.lock().get(&device_id) {
        managed.supervisor.restart();
        return Ok(build_status_dto(device_id, managed));
    }
    state.m4.start_sidecar(device_id, "synthetic".into(), 160, 120, 30)
}

#[tauri::command]
pub fn m4_simulate_sidecar_crash(
    state: State<'_, AppState>,
    device_id: Uuid,
) -> CmdResult<DepthSidecarStatusDto> {
    let guard = state.m4.sidecars.lock();
    let managed = guard
        .get(&device_id)
        .ok_or_else(|| format!("No running depth sidecar for device {device_id}"))?;
    managed.supervisor.simulate_crash().map_err(err_str)?;
    Ok(build_status_dto(device_id, managed))
}

#[tauri::command]
pub fn m4_get_depth_sidecar_status(
    state: State<'_, AppState>,
    device_id: Uuid,
) -> DepthSidecarStatusDto {
    state.m4.status_or_default(device_id)
}

#[tauri::command]
pub fn m4_list_depth_sidecars(state: State<'_, AppState>) -> Vec<DepthSidecarStatusDto> {
    state
        .m4
        .sidecars
        .lock()
        .iter()
        .map(|(&id, m)| build_status_dto(id, m))
        .collect()
}

#[tauri::command]
pub fn m4_configure_depth_rois(
    state: State<'_, AppState>,
    device_id: Uuid,
    rois: Vec<NamedDepthRoi>,
) -> Vec<NamedDepthRoi> {
    if let Some(managed) = state.m4.sidecars.lock().get(&device_id) {
        *managed.rois.lock() = rois.clone();
        managed.supervisor.update_rois(rois.clone());
    }
    rois
}

#[tauri::command]
pub fn m4_set_depth_colormap(
    state: State<'_, AppState>,
    device_id: Uuid,
    palette: DepthColormap,
    near_mm: u16,
    far_mm: u16,
) -> DepthColormapConfig {
    let cfg = DepthColormapConfig {
        palette,
        near_mm: near_mm.max(50),
        far_mm: far_mm.max(near_mm.saturating_add(50)),
    };
    if let Some(managed) = state.m4.sidecars.lock().get(&device_id) {
        *managed.colormap.lock() = cfg.clone();
    }
    cfg
}

#[tauri::command]
pub fn m4_poll_depth_preview(
    state: State<'_, AppState>,
    device_id: Uuid,
) -> CmdResult<DepthPreviewPacket> {
    let guard = state.m4.sidecars.lock();
    if let Some(managed) = guard.get(&device_id) {
        let cmap = managed.colormap.lock().clone();
        let rois = managed.rois.lock().clone();
        let health = managed.supervisor.health_snapshot();

        if let Some(frame) = managed.supervisor.latest_depth_frame() {
            let mut rgba = Vec::new();
            colorize_depth_to_rgba(
                &frame.depth_mm,
                frame.header.width,
                frame.header.height,
                cmap.near_mm,
                cmap.far_mm,
                cmap.palette,
                &mut rgba,
            );
            let roi_results = extract_all_rois(
                &frame.depth_mm,
                frame.header.width,
                frame.header.height,
                &rois,
                CameraIntrinsics::default(),
            );
            let frame_ref = build_depth_ndi_frame_ref(
                device_id,
                frame.header.frame_seq,
                frame.header.width,
                frame.header.height,
                frame.header.timestamp_ns,
            );
            return Ok(DepthPreviewPacket {
                device_id,
                frame_seq: frame.header.frame_seq,
                width: frame.header.width,
                height: frame.header.height,
                palette: cmap.palette,
                near_mm: cmap.near_mm,
                far_mm: cmap.far_mm,
                rgba_base64: BASE64.encode(&rgba),
                frame_ref,
                roi_results,
                health,
            });
        }
    }
    drop(guard);

    // Fallback synthetic preview frame when sidecar hasn't emitted yet
    let mut idx = state.m4.fallback_frame_idx.lock();
    *idx += 1;
    let seq = *idx;
    let mut synth = SyntheticDepthBackend::new(160, 120);
    let frame = synth.next_frame(seq).map_err(err_str)?;
    let cmap = DepthColormapConfig::default();
    let rois = vec![NamedDepthRoi::default()];
    let mut rgba = Vec::new();
    colorize_depth_to_rgba(
        &frame.depth_mm,
        frame.width,
        frame.height,
        cmap.near_mm,
        cmap.far_mm,
        cmap.palette,
        &mut rgba,
    );
    let roi_results = extract_all_rois(
        &frame.depth_mm,
        frame.width,
        frame.height,
        &rois,
        CameraIntrinsics::default(),
    );
    let frame_ref =
        build_depth_ndi_frame_ref(device_id, seq, frame.width, frame.height, frame.timestamp_ns);
    Ok(DepthPreviewPacket {
        device_id,
        frame_seq: seq,
        width: frame.width,
        height: frame.height,
        palette: cmap.palette,
        near_mm: cmap.near_mm,
        far_mm: cmap.far_mm,
        rgba_base64: BASE64.encode(&rgba),
        frame_ref,
        roi_results,
        health: state.m4.status_or_default(device_id).health,
    })
}

// ---------- Portable `.kfreak` ZIP Bundle Commands ----------

#[tauri::command]
pub fn m4_export_kfreak_bundle(state: State<'_, AppState>) -> CmdResult<KfreakExportResult> {
    let snap = state.control.snapshot();
    let slug: String = snap
        .name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    let filename = format!("{}.kfreak", slug.trim_matches('-'));
    let proj = ProjectFile::from_graph(snap);
    let (bytes, manifest) = export_kfreak_bytes(&proj, &[], &[], &[]).map_err(err_str)?;
    Ok(KfreakExportResult {
        filename,
        size_bytes: bytes.len(),
        manifest,
        zip_base64: BASE64.encode(&bytes),
    })
}

#[tauri::command]
pub fn m4_export_kfreak_file(
    state: State<'_, AppState>,
    path: String,
) -> CmdResult<KfreakManifest> {
    let proj = ProjectFile::from_graph(state.control.snapshot());
    export_kfreak_file(path, &proj, &[], &[], &[]).map_err(err_str)
}

#[tauri::command]
pub fn m4_import_kfreak_bundle(
    state: State<'_, AppState>,
    zip_base64: String,
    reconcile_hardware: Option<bool>,
) -> CmdResult<KfreakImportResult> {
    let bytes = BASE64.decode(zip_base64.trim()).map_err(err_str)?;
    let mut bundle = import_kfreak_bytes(&bytes).map_err(err_str)?;
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
        bundle.project.reconcile_hardware(&avail);
    }
    state.protocols.stop_all();
    state
        .control
        .replace_graph(bundle.project.graph.clone())
        .map_err(err_str)?;

    Ok(KfreakImportResult {
        graph: bundle.project.graph,
        manifest: bundle.manifest,
        device_preset_count: bundle.device_presets.len(),
        group_preset_count: bundle.group_presets.len(),
        mapping_preset_count: bundle.mapping_presets.len(),
    })
}

#[tauri::command]
pub fn m4_import_kfreak_file(
    state: State<'_, AppState>,
    path: String,
    reconcile_hardware: Option<bool>,
) -> CmdResult<KfreakImportResult> {
    let mut bundle = import_kfreak_file(path).map_err(err_str)?;
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
        bundle.project.reconcile_hardware(&avail);
    }
    state.protocols.stop_all();
    state
        .control
        .replace_graph(bundle.project.graph.clone())
        .map_err(err_str)?;

    Ok(KfreakImportResult {
        graph: bundle.project.graph,
        manifest: bundle.manifest,
        device_preset_count: bundle.device_presets.len(),
        group_preset_count: bundle.group_presets.len(),
        mapping_preset_count: bundle.mapping_presets.len(),
    })
}

// ---------- Diagnostics & Performance Profiler Command ----------

#[tauri::command]
pub fn m4_get_diagnostics_snapshot(state: State<'_, AppState>) -> DiagnosticsSnapshot {
    state.m4.profiler.snapshot()
}
