//! Milestone 3 Tauri IPC commands: HID Hot-Plug Watcher, 2-Tier HID Learn Wizard,
//! `.kfdevice.json` Preset Management & Auto-Rebind, and RGB Camera CV Extractors.

use crate::state::AppState;
use kf_core::{Device, HidDeviceInfo, Modifier};
use kf_hardware::{
    CameraCvStepOutput, HidDeviceDefinition, HotplugNotification, LearnWizardSnapshot,
    ParsedHidDescriptor, RgbCameraInfo, RgbCvConfig, SimulatedHidAction, SimulatedHidProfile,
};
use serde::Serialize;
use tauri::State;
use uuid::Uuid;

type CmdResult<T> = Result<T, String>;

fn err_str(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Debug, Clone, Serialize)]
pub struct SimulatedPlugResponse {
    pub info: HidDeviceInfo,
    pub notifications: Vec<HotplugNotification>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SavedHidDeviceResponse {
    pub definition: HidDeviceDefinition,
    pub preset_json: String,
    pub device: Device,
}

// ---------- HID Discovery & Hot-Plug Watcher ----------

#[tauri::command]
pub fn m3_list_hid_devices(state: State<'_, AppState>) -> CmdResult<Vec<HidDeviceInfo>> {
    state.hardware.list_hid_devices().map_err(err_str)
}

#[tauri::command]
pub fn m3_check_hid_permissions(state: State<'_, AppState>) -> CmdResult<()> {
    state.hardware.check_hid_permissions().map_err(err_str)
}

#[tauri::command]
pub fn m3_poll_hid_hotplug(state: State<'_, AppState>) -> CmdResult<Vec<HotplugNotification>> {
    state.hardware.poll_hotplug().map_err(err_str)
}

#[tauri::command]
pub fn m3_simulate_hid_plug(
    state: State<'_, AppState>,
    profile: SimulatedHidProfile,
    stable_id: Option<String>,
) -> CmdResult<SimulatedPlugResponse> {
    let (info, notifications) = state
        .hardware
        .simulate_plug_in(profile, stable_id.as_deref())
        .map_err(err_str)?;
    Ok(SimulatedPlugResponse {
        info,
        notifications,
    })
}

#[tauri::command]
pub fn m3_simulate_hid_unplug(
    state: State<'_, AppState>,
    path: String,
) -> CmdResult<Vec<HotplugNotification>> {
    state.hardware.simulate_unplug(&path).map_err(err_str)
}

#[tauri::command]
pub async fn m3_simulate_hid_actuate(
    state: State<'_, AppState>,
    path: String,
    action: SimulatedHidAction,
) -> CmdResult<usize> {
    let outputs = state
        .hardware
        .simulate_actuate(&path, action)
        .map_err(err_str)?;
    let count = outputs.len();
    if !outputs.is_empty() {
        state.protocols.dispatch_outputs(outputs).await;
    }
    Ok(count)
}

// ---------- Tier 1 Descriptor Parser & Tier 2 4-Step Learn Wizard ----------

#[tauri::command]
pub fn m3_parse_hid_descriptor(
    state: State<'_, AppState>,
    path: String,
) -> CmdResult<Option<ParsedHidDescriptor>> {
    state
        .hardware
        .parse_device_descriptor(&path)
        .map_err(err_str)
}

#[tauri::command]
pub fn m3_learn_start_baseline(
    state: State<'_, AppState>,
    device_path: String,
) -> CmdResult<LearnWizardSnapshot> {
    state
        .hardware
        .learn_start_baseline(&device_path)
        .map_err(err_str)
}

#[tauri::command]
pub fn m3_learn_poll_samples(
    state: State<'_, AppState>,
    sample_count: Option<usize>,
) -> CmdResult<LearnWizardSnapshot> {
    state
        .hardware
        .learn_poll_samples(sample_count.unwrap_or(12))
        .map_err(err_str)
}

#[tauri::command]
pub fn m3_learn_start_capture(state: State<'_, AppState>) -> CmdResult<LearnWizardSnapshot> {
    state.hardware.learn_start_capture().map_err(err_str)
}

#[tauri::command]
pub fn m3_learn_classify(
    state: State<'_, AppState>,
    alias_hint: Option<String>,
) -> CmdResult<LearnWizardSnapshot> {
    state
        .hardware
        .learn_classify(alias_hint.as_deref())
        .map_err(err_str)
}

#[tauri::command]
pub fn m3_learn_commit_control(
    state: State<'_, AppState>,
    alias: String,
    modifiers: Vec<Modifier>,
    sensitivity: Option<f64>,
    wrap: Option<bool>,
) -> CmdResult<LearnWizardSnapshot> {
    state
        .hardware
        .learn_commit_control(&alias, modifiers, sensitivity, wrap)
        .map_err(err_str)
}

#[tauri::command]
pub fn m3_learn_get_snapshot(state: State<'_, AppState>) -> LearnWizardSnapshot {
    state.hardware.learn_snapshot()
}

#[tauri::command]
pub fn m3_learn_reset(state: State<'_, AppState>) -> LearnWizardSnapshot {
    state.hardware.learn_reset()
}

// ---------- Portable `.kfdevice.json` Presets ----------

#[tauri::command]
pub fn m3_save_and_attach_learned_device(
    state: State<'_, AppState>,
    preset_name: String,
    x: Option<f64>,
    y: Option<f64>,
) -> CmdResult<SavedHidDeviceResponse> {
    let (definition, preset_json, device) = state
        .hardware
        .save_and_attach_learned_device(&preset_name, x.unwrap_or(140.0), y.unwrap_or(180.0))
        .map_err(err_str)?;
    Ok(SavedHidDeviceResponse {
        definition,
        preset_json,
        device,
    })
}

#[tauri::command]
pub fn m3_export_hid_device_preset(state: State<'_, AppState>, path: String) -> CmdResult<String> {
    let def = state
        .hardware
        .active_binding_definition(&path)
        .ok_or_else(|| format!("No learned HID preset bound to {path}"))?;
    def.to_kfdevice_json().map_err(err_str)
}

#[tauri::command]
pub fn m3_import_hid_device_preset(
    state: State<'_, AppState>,
    json: String,
) -> CmdResult<HidDeviceDefinition> {
    state
        .hardware
        .import_hid_device_preset(&json)
        .map_err(err_str)
}

#[tauri::command]
pub fn m3_list_device_presets(state: State<'_, AppState>) -> Vec<HidDeviceDefinition> {
    state.hardware.list_device_presets()
}

// ---------- RGB Camera & CV Parameter Extractors ----------

#[tauri::command]
pub fn m3_list_rgb_cameras(state: State<'_, AppState>) -> CmdResult<Vec<RgbCameraInfo>> {
    state.hardware.list_rgb_cameras().map_err(err_str)
}

#[tauri::command]
pub fn m3_update_rgb_cv_config(
    state: State<'_, AppState>,
    camera_id: String,
    config: RgbCvConfig,
) -> RgbCvConfig {
    state
        .hardware
        .update_rgb_cv_config(&camera_id, config.clone());
    config
}

#[tauri::command]
pub fn m3_get_rgb_cv_config(state: State<'_, AppState>, camera_id: String) -> RgbCvConfig {
    state.hardware.get_rgb_cv_config(&camera_id)
}

#[tauri::command]
pub fn m3_step_rgb_camera_cv(
    state: State<'_, AppState>,
    camera_id: String,
    target_device_id: Option<Uuid>,
) -> CmdResult<CameraCvStepOutput> {
    state
        .hardware
        .step_rgb_camera_cv(&camera_id, target_device_id)
        .map_err(err_str)
}
