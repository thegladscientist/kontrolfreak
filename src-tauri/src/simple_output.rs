//! Simple VJ output: one selected source, one preview, one display window.
//! The node graph stays available behind Advanced and is not required here.

use kf_protocols::midi::{is_selectable_midi_source, MidiMessage};
use kf_protocols::osc_scout::{OscDiscoveredSource, OscListenStatus, OscScout};
use kf_protocols::ProtocolManager;
use kf_video::{NdiNodeConfigPatch, VideoFitMode, VideoPipelineManager, VideoSourceInfo};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder};
use uuid::Uuid;

const PREVIEW_NODE: &str = "00000000-0000-4000-8000-000000000b71";
const OUTPUT_LABEL: &str = "output";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimpleSource {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub detail: String,
    pub value_label: String,
    pub value_norm: Option<f64>,
    pub live: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimpleOutputSettings {
    pub width: u32,
    pub height: u32,
    pub fit: String,
    pub crop_x: f32,
    pub crop_y: f32,
    pub crop_w: f32,
    pub crop_h: f32,
    pub display_index: Option<usize>,
}

impl Default for SimpleOutputSettings {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            fit: "contain".into(),
            crop_x: 0.0,
            crop_y: 0.0,
            crop_w: 1.0,
            crop_h: 1.0,
            display_index: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimplePoll {
    pub sources: Vec<SimpleSource>,
    pub osc: OscListenStatus,
    pub ndi_runtime: bool,
    pub host: HostInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostInfo {
    pub platform: String,
    pub session: String,
    pub wayland: bool,
    /// Programmatic window placement works on X11, Windows, and macOS.
    /// Wayland compositors ignore it, so the output window stays draggable.
    pub position_reliable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayInfo {
    pub index: usize,
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimplePreview {
    pub source_id: Option<String>,
    pub kind: String,
    pub label: String,
    pub detail: String,
    pub value_label: String,
    pub value_norm: Option<f64>,
    pub jpeg_base64: Option<String>,
    pub frame_width: u32,
    pub frame_height: u32,
    pub awaiting_frame: bool,
    pub using_synthetic: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputWindowStatus {
    pub open: bool,
    pub wayland: bool,
    pub positioned: bool,
    pub message: String,
}

#[derive(Clone)]
struct MidiLive {
    summary: String,
    norm: Option<f64>,
    seen_ms: u64,
}

struct HwMidi {
    name: String,
    _conn: midir::MidiInputConnection<()>,
}

pub struct SimpleOutputHub {
    scout: Arc<OscScout>,
    protocols: Arc<ProtocolManager>,
    video: Arc<VideoPipelineManager>,
    started: AtomicBool,
    selected: Mutex<Option<SimpleSource>>,
    settings: Mutex<SimpleOutputSettings>,
    midi_activity: Arc<Mutex<HashMap<String, MidiLive>>>,
    midi_hw: Mutex<Option<HwMidi>>,
    loopback_armed: Mutex<Vec<String>>,
    pattern_ready: Mutex<Option<String>>,
}

impl SimpleOutputHub {
    pub fn new(protocols: Arc<ProtocolManager>, video: Arc<VideoPipelineManager>) -> Arc<Self> {
        Arc::new(Self {
            scout: OscScout::new(),
            protocols,
            video,
            started: AtomicBool::new(false),
            selected: Mutex::new(None),
            settings: Mutex::new(SimpleOutputSettings::default()),
            midi_activity: Arc::new(Mutex::new(HashMap::new())),
            midi_hw: Mutex::new(None),
            loopback_armed: Mutex::new(Vec::new()),
            pattern_ready: Mutex::new(None),
        })
    }

    pub fn start(self: &Arc<Self>) {
        if self.started.swap(true, Ordering::SeqCst) {
            return;
        }
        self.arm_loopback();
        let scout = Arc::clone(&self.scout);
        tauri::async_runtime::spawn(async move {
            scout.serve().await;
        });
    }

    pub async fn poll(self: &Arc<Self>) -> SimplePoll {
        self.start();
        let hub = Arc::clone(self);
        let (video_sources, midi_ports, ndi_runtime) = tokio::task::spawn_blocking(move || {
            hub.refresh_midi_connection();
            let ndi_runtime = hub.video.runtime_status().available;
            let video_sources = hub.video.discover_all_sources();
            let midi_ports = hub.protocols.list_midi_ports();
            (video_sources, midi_ports, ndi_runtime)
        })
        .await
        .unwrap_or_else(|_| (Vec::new(), Vec::new(), false));

        let mut sources = osc_sources(&self.scout.snapshot());
        sources.extend(midi_sources(&midi_ports, &self.midi_activity.lock()));
        sources.extend(video_sources_to_simple(&video_sources));
        SimplePoll {
            sources,
            osc: self.scout.status(),
            ndi_runtime,
            host: host_info(),
        }
    }

    pub fn set_osc_port(&self, port: u16) -> OscListenStatus {
        if port == 0 {
            self.scout.set_listen_port(0);
        } else {
            self.scout.set_listen_port(port.clamp(1, 65535));
        }
        self.scout.status()
    }

    pub fn select(&self, source: Option<SimpleSource>) {
        let kind = source.as_ref().map(|s| s.kind.as_str());
        if kind != Some("ndi") {
            self.video.release_external_ndi();
        }
        if kind != Some("test_pattern") {
            *self.pattern_ready.lock() = None;
        }
        *self.selected.lock() = source;
        self.refresh_midi_connection();
    }

    pub fn settings(&self) -> SimpleOutputSettings {
        self.settings.lock().clone()
    }

    pub fn set_settings(&self, settings: SimpleOutputSettings) -> SimpleOutputSettings {
        let clean = sanitize_settings(settings);
        *self.settings.lock() = clean.clone();
        clean
    }

    pub fn preview(&self, max_width: u32, max_height: u32) -> SimplePreview {
        let Some(selected) = self.selected.lock().clone() else {
            return SimplePreview {
                source_id: None,
                kind: "empty".into(),
                label: "No source".into(),
                detail: "Pick a source from the list.".into(),
                value_label: String::new(),
                value_norm: None,
                jpeg_base64: None,
                frame_width: 0,
                frame_height: 0,
                awaiting_frame: false,
                using_synthetic: false,
            };
        };
        match selected.kind.as_str() {
            "osc" => self.preview_osc(&selected),
            "midi" => self.preview_midi(&selected),
            "test_pattern" => self.preview_pattern(&selected, max_width, max_height),
            "ndi" => self.preview_ndi(&selected, max_width, max_height),
            _ => idle_preview(&selected),
        }
    }

    pub fn list_displays(app: &AppHandle) -> Vec<DisplayInfo> {
        let Ok(monitors) = app.available_monitors() else {
            return Vec::new();
        };
        monitors
            .into_iter()
            .enumerate()
            .map(|(index, monitor)| {
                let pos = monitor.position();
                let size = monitor.size();
                DisplayInfo {
                    index,
                    name: monitor
                        .name()
                        .map(|name| name.to_string())
                        .unwrap_or_else(|| format!("Display {}", index + 1)),
                    x: pos.x,
                    y: pos.y,
                    width: size.width,
                    height: size.height,
                    scale: monitor.scale_factor(),
                }
            })
            .collect()
    }

    pub fn open_output(self: &Arc<Self>, app: &AppHandle) -> Result<OutputWindowStatus, String> {
        self.start();
        let host = host_info();
        let settings = self.settings();
        let displays = Self::list_displays(app);
        let chosen = settings
            .display_index
            .and_then(|index| displays.into_iter().find(|d| d.index == index));

        let window = if let Some(existing) = app.get_webview_window(OUTPUT_LABEL) {
            existing
        } else {
            let script = "window.__KF_VIEW__='output';";
            WebviewWindowBuilder::new(app, OUTPUT_LABEL, WebviewUrl::App("index.html".into()))
                .title("Kontrolfreak Output")
                .inner_size(1280.0, 720.0)
                .min_inner_size(320.0, 180.0)
                .resizable(true)
                .decorations(false)
                .focused(true)
                .visible(true)
                .initialization_script(script)
                .build()
                .map_err(|err| format!("Could not open the output window: {err}"))?
        };

        let _ = window.show();
        let _ = window.set_focus();

        if host.position_reliable {
            if let Some(display) = chosen {
                let _ = window.set_position(PhysicalPosition::new(display.x, display.y));
                let _ = window.set_size(PhysicalSize::new(display.width, display.height));
                // Give the compositor a moment to land the window before fullscreen.
                let placed = window.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(80)).await;
                    let _ = placed.set_fullscreen(true);
                });
                return Ok(OutputWindowStatus {
                    open: true,
                    wayland: false,
                    positioned: true,
                    message: format!(
                        "Output is on {} and going fullscreen. Esc leaves fullscreen.",
                        display.name
                    ),
                });
            }
            let _ = window.set_fullscreen(false);
            return Ok(OutputWindowStatus {
                open: true,
                wayland: false,
                positioned: false,
                message: "Output window is open. Choose a display to send it there, or press F for fullscreen.".into(),
            });
        }

        let _ = window.set_fullscreen(false);
        Ok(OutputWindowStatus {
            open: true,
            wayland: true,
            positioned: false,
            message: "Output window is open. Drag it to the other display, then press F or double-click for fullscreen. Esc leaves fullscreen.".into(),
        })
    }

    pub fn set_fullscreen(app: &AppHandle, enabled: bool) -> Result<bool, String> {
        let window = app
            .get_webview_window(OUTPUT_LABEL)
            .ok_or_else(|| "Open the output window first.".to_string())?;
        window
            .set_fullscreen(enabled)
            .map_err(|err| format!("Could not change fullscreen: {err}"))?;
        Ok(enabled)
    }

    fn preview_osc(&self, selected: &SimpleSource) -> SimplePreview {
        let live = self
            .scout
            .snapshot()
            .into_iter()
            .find(|row| row.id == selected.id);
        let (value_label, value_norm, detail, live_flag) = match live {
            Some(row) => (
                row.last_value.clone(),
                row.last_norm,
                osc_detail(&row),
                row.live,
            ),
            None => (
                selected.value_label.clone(),
                selected.value_norm,
                selected.detail.clone(),
                false,
            ),
        };
        SimplePreview {
            source_id: Some(selected.id.clone()),
            kind: "osc".into(),
            label: selected.label.clone(),
            detail: if live_flag {
                detail
            } else {
                format!("{detail} · quiet")
            },
            value_label,
            value_norm,
            jpeg_base64: None,
            frame_width: 0,
            frame_height: 0,
            awaiting_frame: false,
            using_synthetic: false,
        }
    }

    fn preview_midi(&self, selected: &SimpleSource) -> SimplePreview {
        let name = selected
            .id
            .strip_prefix("midi:")
            .unwrap_or(selected.label.as_str());
        let activity = self.midi_activity.lock().get(name).cloned();
        let fresh = activity
            .as_ref()
            .map(|row| now_ms().saturating_sub(row.seen_ms) < 4_000)
            .unwrap_or(false);
        SimplePreview {
            source_id: Some(selected.id.clone()),
            kind: "midi".into(),
            label: selected.label.clone(),
            detail: selected.detail.clone(),
            value_label: activity
                .as_ref()
                .map(|row| row.summary.clone())
                .unwrap_or_default(),
            value_norm: activity.and_then(|row| row.norm),
            jpeg_base64: None,
            frame_width: 0,
            frame_height: 0,
            awaiting_frame: !fresh,
            using_synthetic: false,
        }
    }

    fn preview_pattern(
        &self,
        selected: &SimpleSource,
        max_width: u32,
        max_height: u32,
    ) -> SimplePreview {
        let name = selected
            .id
            .strip_prefix("pattern:")
            .unwrap_or(&selected.label)
            .to_string();
        let node = preview_node_id();
        if self.pattern_ready.lock().as_deref() != Some(name.as_str()) {
            self.video.configure_node(
                node,
                NdiNodeConfigPatch {
                    source_name: Some(name.clone()),
                    fit_mode: Some(VideoFitMode::Stretch),
                    crop_x: Some(0.0),
                    crop_y: Some(0.0),
                    crop_w: Some(1.0),
                    crop_h: Some(1.0),
                    out_width: Some(960),
                    out_height: Some(540),
                    out_fps: Some(30.0),
                    ..NdiNodeConfigPatch::default()
                },
            );
            *self.pattern_ready.lock() = Some(name);
        }
        match self.video.render_preview(node, max_width.max(160), max_height.max(90)) {
            Ok(frame) => SimplePreview {
                source_id: Some(selected.id.clone()),
                kind: "test_pattern".into(),
                label: selected.label.clone(),
                detail: selected.detail.clone(),
                value_label: String::new(),
                value_norm: None,
                jpeg_base64: Some(frame.jpeg_base64),
                frame_width: frame.thumb_width,
                frame_height: frame.thumb_height,
                awaiting_frame: false,
                using_synthetic: true,
            },
            Err(err) => SimplePreview {
                source_id: Some(selected.id.clone()),
                kind: "test_pattern".into(),
                label: selected.label.clone(),
                detail: err.to_string(),
                value_label: String::new(),
                value_norm: None,
                jpeg_base64: None,
                frame_width: 0,
                frame_height: 0,
                awaiting_frame: true,
                using_synthetic: true,
            },
        }
    }

    fn preview_ndi(
        &self,
        selected: &SimpleSource,
        max_width: u32,
        max_height: u32,
    ) -> SimplePreview {
        let name = selected
            .id
            .strip_prefix("ndi:")
            .unwrap_or(&selected.label);
        match self
            .video
            .render_external_ndi_jpeg(name, max_width.max(160), max_height.max(90))
        {
            Ok(Some(frame)) => SimplePreview {
                source_id: Some(selected.id.clone()),
                kind: "ndi".into(),
                label: selected.label.clone(),
                detail: selected.detail.clone(),
                value_label: String::new(),
                value_norm: None,
                jpeg_base64: Some(frame.jpeg_base64),
                frame_width: frame.thumb_width,
                frame_height: frame.thumb_height,
                awaiting_frame: false,
                using_synthetic: false,
            },
            Ok(None) => SimplePreview {
                source_id: Some(selected.id.clone()),
                kind: "ndi".into(),
                label: selected.label.clone(),
                detail: "Waiting for a frame from this NDI source.".into(),
                value_label: String::new(),
                value_norm: None,
                jpeg_base64: None,
                frame_width: 0,
                frame_height: 0,
                awaiting_frame: true,
                using_synthetic: false,
            },
            Err(err) => SimplePreview {
                source_id: Some(selected.id.clone()),
                kind: "ndi".into(),
                label: selected.label.clone(),
                detail: err.to_string(),
                value_label: String::new(),
                value_norm: None,
                jpeg_base64: None,
                frame_width: 0,
                frame_height: 0,
                awaiting_frame: true,
                using_synthetic: false,
            },
        }
    }

    fn arm_loopback(&self) {
        let activity = Arc::clone(&self.midi_activity);
        let backend = self.protocols.loopback_midi();
        let mut armed = self.loopback_armed.lock();
        for port in backend.list_ports() {
            if armed.iter().any(|name| name == &port.name) {
                continue;
            }
            let key = port.name.clone();
            let act = Arc::clone(&activity);
            let name = port.name.clone();
            backend.subscribe(
                &port.name,
                Arc::new(move |bytes| record_midi(&act, &name, bytes)),
            );
            armed.push(key);
        }
    }

    fn refresh_midi_connection(&self) {
        self.arm_loopback();
        let wanted = self.selected.lock().as_ref().and_then(|source| {
            if source.kind != "midi" {
                return None;
            }
            let name = source
                .id
                .strip_prefix("midi:")
                .unwrap_or(source.label.as_str())
                .to_string();
            if source.detail.to_ascii_lowercase().contains("loopback") {
                return None;
            }
            Some(name)
        });
        let mut slot = self.midi_hw.lock();
        if slot.as_ref().map(|conn| conn.name.as_str()) == wanted.as_deref() {
            return;
        }
        *slot = None;
        let Some(name) = wanted else {
            return;
        };
        let mut input = match midir::MidiInput::new("kontrolfreak-simple-in") {
            Ok(input) => input,
            Err(err) => {
                tracing::warn!("MIDI input unavailable: {err}");
                return;
            }
        };
        input.ignore(midir::Ignore::None);
        let ports = input.ports();
        let Some(port) = ports
            .iter()
            .find(|port| input.port_name(port).ok().as_deref() == Some(name.as_str()))
        else {
            return;
        };
        let activity = Arc::clone(&self.midi_activity);
        let key = name.clone();
        match input.connect(
            port,
            "kf-simple",
            move |_stamp, bytes, _| record_midi(&activity, &key, bytes),
            (),
        ) {
            Ok(conn) => {
                *slot = Some(HwMidi { name, _conn: conn });
            }
            Err(err) => {
                tracing::warn!("Could not open MIDI input: {err}");
            }
        }
    }
}

fn idle_preview(selected: &SimpleSource) -> SimplePreview {
    SimplePreview {
        source_id: Some(selected.id.clone()),
        kind: selected.kind.clone(),
        label: selected.label.clone(),
        detail: selected.detail.clone(),
        value_label: selected.value_label.clone(),
        value_norm: selected.value_norm,
        jpeg_base64: None,
        frame_width: 0,
        frame_height: 0,
        awaiting_frame: false,
        using_synthetic: false,
    }
}

fn osc_sources(rows: &[OscDiscoveredSource]) -> Vec<SimpleSource> {
    rows.iter()
        .map(|row| SimpleSource {
            id: row.id.clone(),
            kind: "osc".into(),
            label: row.label.clone(),
            detail: osc_detail(row),
            value_label: row.last_value.clone(),
            value_norm: row.last_norm,
            live: row.live,
        })
        .collect()
}

fn osc_detail(row: &OscDiscoveredSource) -> String {
    let prefixes = if row.recent_prefixes.is_empty() {
        "no messages yet".to_string()
    } else {
        row.recent_prefixes.join("  ")
    };
    let via = if row.origin == "mdns" {
        short_service(row.service_type.as_deref().unwrap_or("mDNS"))
    } else {
        "packet".into()
    };
    let host = match row.port {
        Some(port) => format!("{}:{port}", row.host),
        None => row.host.clone(),
    };
    format!("{host} · {via} · {prefixes}")
}

fn short_service(service: &str) -> String {
    let bare = service
        .strip_suffix(".local.")
        .or_else(|| service.strip_suffix(".local"))
        .unwrap_or(service);
    bare.trim_matches('_').to_string()
}

fn midi_sources(
    ports: &[kf_protocols::midi::MidiPortInfo],
    activity: &HashMap<String, MidiLive>,
) -> Vec<SimpleSource> {
    let mut rows: Vec<_> = ports
        .iter()
        .filter(|port| is_selectable_midi_source(port))
        .collect();
    rows.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    rows.dedup_by(|a, b| a.name == b.name && a.direction == b.direction);
    let now = now_ms();
    rows.into_iter()
        .map(|port| {
            let live_row = activity.get(&port.name);
            let fresh = live_row
                .map(|row| now.saturating_sub(row.seen_ms) < 4_000)
                .unwrap_or(false);
            let kind_label = if port.backend == kf_core::MidiBackendKind::LoopbackMock {
                "Virtual loopback"
            } else if port.is_virtual {
                "Virtual"
            } else {
                "Input"
            };
            let direction = if port.direction == "bidir" {
                "in/out"
            } else {
                port.direction.as_str()
            };
            SimpleSource {
                id: format!("midi:{}", port.name),
                kind: "midi".into(),
                label: port.name.clone(),
                detail: format!("{kind_label} · {direction}"),
                value_label: live_row
                    .map(|row| row.summary.clone())
                    .unwrap_or_default(),
                value_norm: live_row.and_then(|row| row.norm),
                live: fresh,
            }
        })
        .collect()
}

fn video_sources_to_simple(sources: &[VideoSourceInfo]) -> Vec<SimpleSource> {
    let mut rows = Vec::new();
    for source in sources {
        let synthetic = source
            .url_address
            .as_deref()
            .unwrap_or("")
            .starts_with("synthetic://")
            || source.name.starts_with("Test Pattern");
        let size = match (source.width, source.height) {
            (Some(w), Some(h)) => format!("{w}×{h}"),
            _ => "Network".into(),
        };
        if synthetic {
            rows.push(SimpleSource {
                id: format!("pattern:{}", source.name),
                kind: "test_pattern".into(),
                label: source
                    .name
                    .trim_start_matches("Test Pattern: ")
                    .to_string(),
                detail: format!("Built-in · {size}"),
                value_label: String::new(),
                value_norm: None,
                live: true,
            });
        } else {
            rows.push(SimpleSource {
                id: format!("ndi:{}", source.name),
                kind: "ndi".into(),
                label: source.name.clone(),
                detail: source
                    .url_address
                    .clone()
                    .filter(|url| !url.is_empty())
                    .unwrap_or(size),
                value_label: String::new(),
                value_norm: None,
                live: true,
            });
        }
    }
    rows
}

fn record_midi(activity: &Mutex<HashMap<String, MidiLive>>, name: &str, bytes: &[u8]) {
    let Some(message) = MidiMessage::decode(bytes) else {
        return;
    };
    if matches!(message, MidiMessage::Clock) {
        return;
    }
    let (key, value) = message.to_logical_key_and_value();
    let norm = value.as_f64().map(|n| {
        if n > 1.0 {
            (n / 127.0).clamp(0.0, 1.0)
        } else {
            n.clamp(0.0, 1.0)
        }
    });
    activity.lock().insert(
        name.to_string(),
        MidiLive {
            summary: format!("{key} = {}", value.summary_display()),
            norm,
            seen_ms: now_ms(),
        },
    );
}

pub fn host_info() -> HostInfo {
    let session = std::env::var("XDG_SESSION_TYPE")
        .unwrap_or_default()
        .to_ascii_lowercase();
    let wayland = if session == "x11" {
        false
    } else if session == "wayland" {
        true
    } else {
        std::env::var_os("WAYLAND_DISPLAY").is_some() && std::env::var_os("DISPLAY").is_none()
    };
    HostInfo {
        platform: std::env::consts::OS.to_string(),
        session: if session.is_empty() {
            "unknown".into()
        } else {
            session
        },
        wayland,
        position_reliable: !wayland,
    }
}

pub fn sanitize_settings(settings: SimpleOutputSettings) -> SimpleOutputSettings {
    let fit = match settings.fit.as_str() {
        "cover" => "cover",
        "stretch" => "stretch",
        _ => "contain",
    };
    let crop_w = settings.crop_w.clamp(0.05, 1.0);
    let crop_h = settings.crop_h.clamp(0.05, 1.0);
    let crop_x = settings.crop_x.clamp(0.0, 1.0 - crop_w);
    let crop_y = settings.crop_y.clamp(0.0, 1.0 - crop_h);
    SimpleOutputSettings {
        width: settings.width.clamp(160, 7680),
        height: settings.height.clamp(90, 4320),
        fit: fit.into(),
        crop_x,
        crop_y,
        crop_w,
        crop_h,
        display_index: settings.display_index,
    }
}

fn preview_node_id() -> Uuid {
    Uuid::parse_str(PREVIEW_NODE).expect("preview node id")
}

fn now_ms() -> u64 {
    kf_core::SignalEvent::now_ms()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_clamp_fit_and_crop() {
        let clean = sanitize_settings(SimpleOutputSettings {
            width: 10,
            height: 9_000,
            fit: "nope".into(),
            crop_x: 0.9,
            crop_y: -1.0,
            crop_w: 0.5,
            crop_h: 2.0,
            display_index: Some(1),
        });
        assert_eq!(clean.width, 160);
        assert_eq!(clean.height, 4320);
        assert_eq!(clean.fit, "contain");
        assert!(clean.crop_x <= 1.0 - clean.crop_w);
        assert_eq!(clean.crop_y, 0.0);
        assert_eq!(clean.crop_h, 1.0);
        assert_eq!(clean.display_index, Some(1));
    }

    #[test]
    fn host_info_marks_wayland_when_session_says_so() {
        // Don't mutate the process environment permanently if another test reads it.
        let previous = std::env::var_os("XDG_SESSION_TYPE");
        std::env::set_var("XDG_SESSION_TYPE", "wayland");
        let info = host_info();
        assert!(info.wayland);
        assert!(!info.position_reliable);
        match previous {
            Some(value) => std::env::set_var("XDG_SESSION_TYPE", value),
            None => std::env::remove_var("XDG_SESSION_TYPE"),
        }
    }
}
