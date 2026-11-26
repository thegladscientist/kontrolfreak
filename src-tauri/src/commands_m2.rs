//! Milestone 2 Tauri IPC commands for RTPMIDI (RFC 6295 + mDNS-SD) and NDI Video Pipeline.

use crate::state::AppState;
use kf_core::VideoSourceInfo;
use kf_protocols::rtpmidi::{RtpMidiPeer, RtpMidiSessionStatus, RtpMidiSessionTrait};
use kf_protocols::EndpointStatus;
use kf_video::{
    NdiNodeConfigPatch, NdiNodeState, NdiPreviewPayload, NdiRuntimeStatus,
};
use std::net::SocketAddr;
use tauri::State;
use uuid::Uuid;

type CmdResult<T> = Result<T, String>;

fn err_str(e: impl std::fmt::Display) -> String {
    e.to_string()
}

// ---------- NDI Runtime & Flexible-Resolution Video Pipeline Commands ----------

#[tauri::command]
pub fn m2_get_ndi_runtime_status(state: State<'_, AppState>) -> NdiRuntimeStatus {
    state.video.runtime_status()
}

#[tauri::command]
pub fn m2_discover_ndi_sources(state: State<'_, AppState>) -> Vec<VideoSourceInfo> {
    state.video.discover_all_sources()
}

#[tauri::command]
pub fn m2_get_ndi_node_state(state: State<'_, AppState>, device_id: Uuid) -> NdiNodeState {
    state.video.get_node_state(device_id)
}

#[tauri::command]
pub fn m2_configure_ndi_node(
    state: State<'_, AppState>,
    device_id: Uuid,
    patch: NdiNodeConfigPatch,
) -> NdiNodeState {
    state.video.configure_node(device_id, patch)
}

#[tauri::command]
pub fn m2_get_ndi_preview_frame(
    state: State<'_, AppState>,
    device_id: Uuid,
    max_width: Option<u32>,
    max_height: Option<u32>,
) -> CmdResult<NdiPreviewPayload> {
    state
        .video
        .render_preview(
            device_id,
            max_width.unwrap_or(320),
            max_height.unwrap_or(180),
        )
        .map_err(err_str)
}

// ---------- Pure-Rust RTPMIDI Session & mDNS-SD Peer Discovery Commands ----------

#[tauri::command]
pub async fn m2_start_rtpmidi_session(
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
pub async fn m2_stop_rtpmidi_session(
    state: State<'_, AppState>,
    device_id: Uuid,
) -> CmdResult<()> {
    if let Some(session) = state.protocols.rtpmidi_session(device_id) {
        session.shutdown().await;
    }
    state.protocols.stop_endpoint(device_id);
    Ok(())
}

#[tauri::command]
pub fn m2_get_rtpmidi_status(
    state: State<'_, AppState>,
    device_id: Uuid,
) -> Option<RtpMidiSessionStatus> {
    state.protocols.rtpmidi_status(device_id)
}

#[tauri::command]
pub fn m2_discover_rtpmidi_peers(
    state: State<'_, AppState>,
    device_id: Uuid,
) -> Vec<RtpMidiPeer> {
    state.protocols.rtpmidi_discovered_peers(device_id)
}

#[tauri::command]
pub async fn m2_connect_rtpmidi_peer(
    state: State<'_, AppState>,
    device_id: Uuid,
    peer_control_addr: String,
) -> CmdResult<u32> {
    let addr: SocketAddr = peer_control_addr
        .parse()
        .map_err(|e| format!("Invalid peer address '{peer_control_addr}' (expected host:port): {e}"))?;
    let session = state
        .protocols
        .rtpmidi_session(device_id)
        .ok_or_else(|| format!("RTPMIDI session for device {device_id} is not running"))?;
    session.connect_peer(addr).await.map_err(err_str)
}

#[tauri::command]
pub async fn m2_sync_rtpmidi_clock(
    state: State<'_, AppState>,
    device_id: Uuid,
) -> CmdResult<usize> {
    let session = state
        .protocols
        .rtpmidi_session(device_id)
        .ok_or_else(|| format!("RTPMIDI session for device {device_id} is not running"))?;
    session.trigger_clock_sync().await.map_err(err_str)
}

#[tauri::command]
pub fn m2_send_rtpmidi_test_midi(
    state: State<'_, AppState>,
    device_id: Uuid,
    midi_bytes: Vec<u8>,
) -> CmdResult<()> {
    let session = state
        .protocols
        .rtpmidi_session(device_id)
        .ok_or_else(|| format!("RTPMIDI session for device {device_id} is not running"))?;
    session.send_midi_payload(&midi_bytes).map_err(err_str)
}
