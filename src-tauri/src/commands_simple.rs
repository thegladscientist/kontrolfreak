//! Tauri commands for the simple VJ output screen.

use crate::simple_output::{
    host_info, DisplayInfo, HostInfo, OutputWindowStatus, SimpleOutputHub, SimpleOutputSettings,
    SimplePoll, SimplePreview, SimpleSource,
};
use crate::state::AppState;
use std::sync::Arc;
use tauri::{AppHandle, State};

#[tauri::command]
pub async fn simple_poll_sources(state: State<'_, AppState>) -> Result<SimplePoll, String> {
    let hub: Arc<SimpleOutputHub> = Arc::clone(&state.simple);
    Ok(hub.poll().await)
}

#[tauri::command]
pub fn simple_set_osc_port(
    state: State<'_, AppState>,
    port: u16,
) -> kf_protocols::osc_scout::OscListenStatus {
    state.simple.set_osc_port(port)
}

#[tauri::command]
pub fn simple_select_source(state: State<'_, AppState>, source: Option<SimpleSource>) {
    state.simple.select(source);
}

#[tauri::command]
pub fn simple_get_settings(state: State<'_, AppState>) -> SimpleOutputSettings {
    state.simple.settings()
}

#[tauri::command]
pub fn simple_set_settings(
    state: State<'_, AppState>,
    settings: SimpleOutputSettings,
) -> SimpleOutputSettings {
    state.simple.set_settings(settings)
}

#[tauri::command]
pub async fn simple_preview(
    state: State<'_, AppState>,
    max_width: u32,
    max_height: u32,
) -> Result<SimplePreview, String> {
    let hub = Arc::clone(&state.simple);
    tokio::task::spawn_blocking(move || hub.preview(max_width, max_height))
        .await
        .map_err(|err| format!("Preview failed: {err}"))
}

#[tauri::command]
pub fn simple_host_info() -> HostInfo {
    host_info()
}

#[tauri::command]
pub fn simple_list_displays(app: AppHandle) -> Vec<DisplayInfo> {
    SimpleOutputHub::list_displays(&app)
}

#[tauri::command]
pub fn simple_open_output(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<OutputWindowStatus, String> {
    state.simple.open_output(&app)
}

#[tauri::command]
pub fn simple_set_output_fullscreen(
    app: AppHandle,
    enabled: bool,
) -> Result<bool, String> {
    SimpleOutputHub::set_fullscreen(&app, enabled)
}
