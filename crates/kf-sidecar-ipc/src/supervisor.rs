//! Out-of-process Sensor Sidecar Supervisor managing child process launch, IPC heartbeats,
//! zero-copy shared-memory frame consumption, crash detection, and exponential backoff restart.

use crate::diagnostics::{DiagnosticsProfiler, SidecarDiagnosticEntry};
use crate::ipc::{cleanup_socket_endpoint, IpcClient};
use crate::protocol::{
    DepthRoiConfig, NamedDepthRoi, ShmFrameHeader, SidecarClient, SidecarError,
    SidecarHealthStatus, SidecarRequest, SIDECAR_PROTOCOL_VERSION,
};
use crate::shm::{ShmRingHealth, ShmRingReader, ShmRingWriter};
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

/// Configuration for launching and supervising a `kf-sensor-depth` sidecar process.
#[derive(Debug, Clone)]
pub struct SupervisorConfig {
    pub sidecar_id: Uuid,
    pub binary_path: PathBuf,
    pub vendor: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub shm_slot_count: u32,
    pub initial_rois: Vec<NamedDepthRoi>,
    pub heartbeat_interval_ms: u64,
    pub initial_backoff_ms: u64,
    pub max_backoff_ms: u64,
    pub ipc_connect_timeout_ms: u64,
}

impl SupervisorConfig {
    pub fn new(sidecar_id: Uuid, binary_path: impl Into<PathBuf>) -> Self {
        Self {
            sidecar_id,
            binary_path: binary_path.into(),
            vendor: "synthetic".into(),
            width: 160,
            height: 120,
            fps: 30,
            shm_slot_count: 4,
            initial_rois: vec![NamedDepthRoi::default()],
            heartbeat_interval_ms: 400,
            initial_backoff_ms: 150,
            max_backoff_ms: 4000,
            ipc_connect_timeout_ms: 3000,
        }
    }
}

/// Snapshot of the latest depth frame read from the zero-copy shared-memory ring buffer.
#[derive(Debug, Clone)]
pub struct DepthFrameSnapshot {
    pub header: ShmFrameHeader,
    pub depth_mm: Vec<u16>,
}

/// JSON-serializable runtime health summary for a supervised sidecar.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SidecarSupervisorSnapshot {
    pub sidecar_id: Uuid,
    pub vendor: String,
    pub status: SidecarHealthStatus,
    pub pid: Option<u32>,
    pub uptime_ms: u64,
    pub restart_count: u32,
    pub consecutive_crashes: u32,
    pub last_crash_reason: Option<String>,
    pub frames_received: u64,
    pub fps: f32,
    pub width: u32,
    pub height: u32,
    pub ring_health: Option<ShmRingHealth>,
}

pub type FrameCallback = Arc<dyn Fn(Uuid, &ShmFrameHeader, &[u16]) + Send + Sync>;
pub type StatusCallback = Arc<dyn Fn(Uuid, SidecarHealthStatus, Option<String>) + Send + Sync>;

struct SharedState {
    status: Mutex<SidecarHealthStatus>,
    child: Mutex<Option<Child>>,
    pid: AtomicU32,
    restart_count: AtomicU32,
    consecutive_crashes: AtomicU32,
    frames_received: AtomicU64,
    started_at_ms: AtomicU64,
    fps: Mutex<f32>,
    last_crash_reason: Mutex<Option<String>>,
    latest_frame: RwLock<Option<DepthFrameSnapshot>>,
    ring_health: Mutex<Option<ShmRingHealth>>,
    rois: Mutex<Vec<NamedDepthRoi>>,
    rois_dirty: AtomicBool,
    force_restart: AtomicBool,
    stop_flag: AtomicBool,
}

/// Locates the `kf-sensor-depth` executable alongside the current binary, inside a Tauri
/// bundle (`externalBin` / macOS `Contents/MacOS` / `Contents/Resources`), or workspace target dir.
pub fn locate_depth_sidecar_binary() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("KF_SENSOR_DEPTH_BIN") {
        let p = PathBuf::from(explicit);
        if p.exists() {
            return Some(p);
        }
    }

    let bin_name = if cfg!(windows) {
        "kf-sensor-depth.exe"
    } else {
        "kf-sensor-depth"
    };

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            // Bundled Tauri externalBin (Windows install dir, macOS Contents/MacOS/, Linux /usr/bin)
            // and `cargo build --workspace` output directory.
            let sibling = dir.join(bin_name);
            if sibling.exists() {
                return Some(sibling);
            }
            // macOS bundle Resources fallback (`Contents/MacOS/../Resources/kf-sensor-depth`)
            let mac_resources = dir.join("../Resources").join(bin_name);
            if mac_resources.exists() {
                return Some(mac_resources);
            }
            // In cargo test, current_exe is in target/debug/deps/<test-bin>
            if let Some(parent) = dir.parent() {
                let up = parent.join(bin_name);
                if up.exists() {
                    return Some(up);
                }
            }
        }
    }

    let cwd_candidates = [
        PathBuf::from("target/debug").join(bin_name),
        PathBuf::from("target/release").join(bin_name),
        PathBuf::from("../target/debug").join(bin_name),
        PathBuf::from("../target/release").join(bin_name),
    ];
    for c in cwd_candidates {
        if c.exists() {
            return Some(c);
        }
    }

    // Check Tauri externalBin staging directory (`src-tauri/binaries/kf-sensor-depth-*`)
    for dir in [Path::new("src-tauri/binaries"), Path::new("binaries")] {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    let matches = name.starts_with("kf-sensor-depth")
                        && (!cfg!(windows) || name.ends_with(".exe"));
                    if matches && path.is_file() {
                        return Some(path);
                    }
                }
            }
        }
    }
    None
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Manages a single crash-isolated `kf-sensor-depth` child process.
pub struct SidecarSupervisor {
    config: SupervisorConfig,
    ipc_endpoint: String,
    shm_path: PathBuf,
    shared: Arc<SharedState>,
    profiler: Option<Arc<DiagnosticsProfiler>>,
    on_frame: Option<FrameCallback>,
    on_status: Option<StatusCallback>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl SidecarSupervisor {
    pub fn new(
        config: SupervisorConfig,
        profiler: Option<Arc<DiagnosticsProfiler>>,
        on_frame: Option<FrameCallback>,
        on_status: Option<StatusCallback>,
    ) -> Self {
        let tag = format!("{}-{}", config.sidecar_id.simple(), std::process::id());
        let ipc_endpoint = if cfg!(windows) {
            format!("kf-depth-{tag}")
        } else {
            std::env::temp_dir()
                .join(format!("kf-depth-{tag}.sock"))
                .to_string_lossy()
                .to_string()
        };
        let shm_path = std::env::temp_dir().join(format!("kf-depth-{tag}.shm"));

        let shared = Arc::new(SharedState {
            status: Mutex::new(SidecarHealthStatus::Stopped),
            child: Mutex::new(None),
            pid: AtomicU32::new(0),
            restart_count: AtomicU32::new(0),
            consecutive_crashes: AtomicU32::new(0),
            frames_received: AtomicU64::new(0),
            started_at_ms: AtomicU64::new(0),
            fps: Mutex::new(0.0),
            last_crash_reason: Mutex::new(None),
            latest_frame: RwLock::new(None),
            ring_health: Mutex::new(None),
            rois: Mutex::new(config.initial_rois.clone()),
            rois_dirty: AtomicBool::new(false),
            force_restart: AtomicBool::new(false),
            stop_flag: AtomicBool::new(false),
        });

        Self {
            config,
            ipc_endpoint,
            shm_path,
            shared,
            profiler,
            on_frame,
            on_status,
            worker: Mutex::new(None),
        }
    }

    pub fn sidecar_id(&self) -> Uuid {
        self.config.sidecar_id
    }

    pub fn ipc_endpoint(&self) -> &str {
        &self.ipc_endpoint
    }

    pub fn shm_path(&self) -> &Path {
        &self.shm_path
    }

    /// Starts the supervisor background thread, spawning the child sidecar process.
    pub fn start(&self) -> Result<(), SidecarError> {
        let mut guard = self.worker.lock();
        if guard.is_some() {
            return Ok(());
        }
        if !self.config.binary_path.exists() {
            return Err(SidecarError::SpawnFailed(format!(
                "sidecar binary not found at {}",
                self.config.binary_path.display()
            )));
        }

        self.shared.stop_flag.store(false, Ordering::Release);
        let config = self.config.clone();
        let ipc_endpoint = self.ipc_endpoint.clone();
        let shm_path = self.shm_path.clone();
        let shared = Arc::clone(&self.shared);
        let profiler = self.profiler.clone();
        let on_frame = self.on_frame.clone();
        let on_status = self.on_status.clone();

        let handle = std::thread::Builder::new()
            .name(format!("kf-sidecar-sup-{}", &config.sidecar_id.to_string()[..8]))
            .spawn(move || {
                supervisor_loop(
                    config,
                    ipc_endpoint,
                    shm_path,
                    shared,
                    profiler,
                    on_frame,
                    on_status,
                );
            })?;

        *guard = Some(handle);
        Ok(())
    }

    /// Gracefully stops the sidecar child process and supervisor thread.
    pub fn stop(&self) {
        self.shared.stop_flag.store(true, Ordering::Release);
        {
            let mut child_guard = self.shared.child.lock();
            if let Some(mut child) = child_guard.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        if let Some(handle) = self.worker.lock().take() {
            let _ = handle.join();
        }
        self.shared.pid.store(0, Ordering::Release);
        *self.shared.status.lock() = SidecarHealthStatus::Stopped;
        cleanup_socket_endpoint(&self.ipc_endpoint);
        let _ = std::fs::remove_file(&self.shm_path);
        if let Some(ref prof) = self.profiler {
            prof.update_sidecar(self.diagnostic_entry());
        }
    }

    /// Requests a controlled restart of the sidecar child process.
    pub fn restart(&self) {
        self.shared.force_restart.store(true, Ordering::Release);
        let mut child_guard = self.shared.child.lock();
        if let Some(child) = child_guard.as_mut() {
            let _ = child.kill();
        }
    }

    /// Deliberately terminates the running sidecar child process (SIGKILL / TerminateProcess)
    /// to demonstrate crash isolation and automatic supervisor recovery without impacting the host.
    pub fn simulate_crash(&self) -> Result<(), SidecarError> {
        let mut child_guard = self.shared.child.lock();
        if let Some(child) = child_guard.as_mut() {
            child.kill()?;
            Ok(())
        } else {
            Err(SidecarError::Disconnected(
                "no running sidecar child process to terminate".into(),
            ))
        }
    }

    pub fn update_rois(&self, rois: Vec<NamedDepthRoi>) {
        *self.shared.rois.lock() = rois;
        self.shared.rois_dirty.store(true, Ordering::Release);
    }

    pub fn status(&self) -> SidecarHealthStatus {
        *self.shared.status.lock()
    }

    pub fn child_pid(&self) -> Option<u32> {
        let p = self.shared.pid.load(Ordering::Acquire);
        if p == 0 {
            None
        } else {
            Some(p)
        }
    }

    pub fn latest_depth_frame(&self) -> Option<DepthFrameSnapshot> {
        self.shared.latest_frame.read().clone()
    }

    pub fn health_snapshot(&self) -> SidecarSupervisorSnapshot {
        let started = self.shared.started_at_ms.load(Ordering::Acquire);
        let status = *self.shared.status.lock();
        let uptime_ms = if status == SidecarHealthStatus::Running && started > 0 {
            now_ms().saturating_sub(started)
        } else {
            0
        };
        SidecarSupervisorSnapshot {
            sidecar_id: self.config.sidecar_id,
            vendor: self.config.vendor.clone(),
            status,
            pid: self.child_pid(),
            uptime_ms,
            restart_count: self.shared.restart_count.load(Ordering::Acquire),
            consecutive_crashes: self.shared.consecutive_crashes.load(Ordering::Acquire),
            last_crash_reason: self.shared.last_crash_reason.lock().clone(),
            frames_received: self.shared.frames_received.load(Ordering::Acquire),
            fps: *self.shared.fps.lock(),
            width: self.config.width,
            height: self.config.height,
            ring_health: self.shared.ring_health.lock().clone(),
        }
    }

    pub fn diagnostic_entry(&self) -> SidecarDiagnosticEntry {
        let snap = self.health_snapshot();
        SidecarDiagnosticEntry {
            sidecar_id: snap.sidecar_id,
            vendor: snap.vendor,
            status: snap.status,
            pid: snap.pid,
            uptime_ms: snap.uptime_ms,
            restart_count: snap.restart_count,
            last_crash_reason: snap.last_crash_reason,
            frames_received: snap.frames_received,
            fps: snap.fps,
            ring_buffer_utilization: snap
                .ring_health
                .as_ref()
                .map(|r| r.utilization)
                .unwrap_or(0.0),
            ring_buffer_dropped: snap
                .ring_health
                .as_ref()
                .map(|r| r.dropped_frames)
                .unwrap_or(0),
        }
    }
}

impl Drop for SidecarSupervisor {
    fn drop(&mut self) {
        self.stop();
    }
}

fn set_status(
    shared: &SharedState,
    config: &SupervisorConfig,
    profiler: &Option<Arc<DiagnosticsProfiler>>,
    on_status: &Option<StatusCallback>,
    status: SidecarHealthStatus,
    reason: Option<String>,
) {
    *shared.status.lock() = status;
    if let Some(ref r) = reason {
        *shared.last_crash_reason.lock() = Some(r.clone());
    }
    if let Some(ref cb) = on_status {
        cb(config.sidecar_id, status, reason);
    }
    push_profiler_update(shared, config, profiler);
}

fn push_profiler_update(
    shared: &SharedState,
    config: &SupervisorConfig,
    profiler: &Option<Arc<DiagnosticsProfiler>>,
) {
    let Some(ref prof) = profiler else {
        return;
    };
    let status = *shared.status.lock();
    let started = shared.started_at_ms.load(Ordering::Acquire);
    let uptime_ms = if status == SidecarHealthStatus::Running && started > 0 {
        now_ms().saturating_sub(started)
    } else {
        0
    };
    let pid_raw = shared.pid.load(Ordering::Acquire);
    let ring = shared.ring_health.lock().clone();
    prof.update_sidecar(SidecarDiagnosticEntry {
        sidecar_id: config.sidecar_id,
        vendor: config.vendor.clone(),
        status,
        pid: if pid_raw == 0 { None } else { Some(pid_raw) },
        uptime_ms,
        restart_count: shared.restart_count.load(Ordering::Acquire),
        last_crash_reason: shared.last_crash_reason.lock().clone(),
        frames_received: shared.frames_received.load(Ordering::Acquire),
        fps: *shared.fps.lock(),
        ring_buffer_utilization: ring.as_ref().map(|r| r.utilization).unwrap_or(0.0),
        ring_buffer_dropped: ring.as_ref().map(|r| r.dropped_frames).unwrap_or(0),
    });
}

fn interruptible_sleep(stop_flag: &AtomicBool, total: Duration) {
    let start = Instant::now();
    while !stop_flag.load(Ordering::Acquire) && start.elapsed() < total {
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn attempt_endpoints(base_ipc: &str, base_shm: &Path, attempt: u64) -> (String, PathBuf) {
    if attempt == 0 {
        return (base_ipc.to_string(), base_shm.to_path_buf());
    }
    let ipc = if cfg!(windows) {
        format!("{base_ipc}-r{attempt}")
    } else if let Some(stripped) = base_ipc.strip_suffix(".sock") {
        format!("{stripped}-r{attempt}.sock")
    } else {
        format!("{base_ipc}-r{attempt}")
    };
    let shm = match (base_shm.parent(), base_shm.file_stem()) {
        (Some(parent), Some(stem)) => {
            parent.join(format!("{}-r{attempt}.shm", stem.to_string_lossy()))
        }
        _ => base_shm.with_extension(format!("r{attempt}.shm")),
    };
    (ipc, shm)
}

fn supervisor_loop(
    config: SupervisorConfig,
    base_ipc_endpoint: String,
    base_shm_path: PathBuf,
    shared: Arc<SharedState>,
    profiler: Option<Arc<DiagnosticsProfiler>>,
    on_frame: Option<FrameCallback>,
    on_status: Option<StatusCallback>,
) {
    let mut is_first_launch = true;
    let mut attempt: u64 = 0;

    while !shared.stop_flag.load(Ordering::Acquire) {
        if !is_first_launch {
            let crashes = shared.consecutive_crashes.load(Ordering::Acquire).max(1);
            let exp = 1u64.checked_shl((crashes - 1).min(8)).unwrap_or(256);
            let delay_ms = config
                .initial_backoff_ms
                .saturating_mul(exp)
                .min(config.max_backoff_ms);
            tracing::warn!(
                sidecar_id = %config.sidecar_id,
                delay_ms,
                crashes,
                "Sidecar supervisor backing off before auto-restart"
            );
            interruptible_sleep(&shared.stop_flag, Duration::from_millis(delay_ms));
            if shared.stop_flag.load(Ordering::Acquire) {
                break;
            }
        }
        is_first_launch = false;

        let (ipc_endpoint, shm_path) =
            attempt_endpoints(&base_ipc_endpoint, &base_shm_path, attempt);
        attempt = attempt.wrapping_add(1);

        set_status(
            &shared,
            &config,
            &profiler,
            &on_status,
            SidecarHealthStatus::Starting,
            None,
        );

        // Pre-initialize shared-memory ring buffer so both processes share exact geometry.
        cleanup_socket_endpoint(&ipc_endpoint);
        if let Err(e) = ShmRingWriter::create(
            &shm_path,
            config.shm_slot_count,
            config.width,
            config.height,
        ) {
            let msg = format!("failed to initialize SHM ring buffer: {e}");
            shared.consecutive_crashes.fetch_add(1, Ordering::AcqRel);
            shared.restart_count.fetch_add(1, Ordering::AcqRel);
            set_status(
                &shared,
                &config,
                &profiler,
                &on_status,
                SidecarHealthStatus::CrashedRestarting,
                Some(msg),
            );
            continue;
        }

        // Launch out-of-process depth camera sidecar
        let mut cmd = Command::new(&config.binary_path);
        cmd.arg("--ipc")
            .arg(&ipc_endpoint)
            .arg("--shm")
            .arg(&shm_path)
            .arg("--vendor")
            .arg(&config.vendor)
            .arg("--width")
            .arg(config.width.to_string())
            .arg("--height")
            .arg(config.height.to_string())
            .arg("--fps")
            .arg(config.fps.to_string())
            .arg("--device-id")
            .arg(config.sidecar_id.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                let msg = format!("spawn failed: {e}");
                shared.consecutive_crashes.fetch_add(1, Ordering::AcqRel);
                shared.restart_count.fetch_add(1, Ordering::AcqRel);
                set_status(
                    &shared,
                    &config,
                    &profiler,
                    &on_status,
                    SidecarHealthStatus::CrashedRestarting,
                    Some(msg),
                );
                continue;
            }
        };

        let child_pid = child.id();
        shared.pid.store(child_pid, Ordering::Release);
        *shared.child.lock() = Some(child);

        // Connect control IPC and send Hello + StartStream + UpdateRois
        let client = match IpcClient::connect_with_retry(
            &ipc_endpoint,
            Duration::from_millis(config.ipc_connect_timeout_ms),
        ) {
            Ok(c) => c,
            Err(e) => {
                reap_child(&shared);
                shared.consecutive_crashes.fetch_add(1, Ordering::AcqRel);
                shared.restart_count.fetch_add(1, Ordering::AcqRel);
                set_status(
                    &shared,
                    &config,
                    &profiler,
                    &on_status,
                    SidecarHealthStatus::CrashedRestarting,
                    Some(format!("IPC handshake failed: {e}")),
                );
                continue;
            }
        };

        let initial_rois = shared.rois.lock().clone();
        let primary_roi = initial_rois
            .first()
            .map(|r| r.to_depth_roi_config())
            .unwrap_or_else(DepthRoiConfig::default);

        let handshake_ok = client
            .send_request(SidecarRequest::Hello {
                protocol_version: SIDECAR_PROTOCOL_VERSION,
            })
            .and_then(|_| {
                client.send_request(SidecarRequest::StartStream {
                    device_id: config.sidecar_id,
                    vendor: config.vendor.clone(),
                    roi: primary_roi,
                })
            })
            .and_then(|_| {
                client.send_request(SidecarRequest::UpdateRois {
                    device_id: config.sidecar_id,
                    rois: initial_rois,
                })
            });

        if let Err(e) = handshake_ok {
            reap_child(&shared);
            shared.consecutive_crashes.fetch_add(1, Ordering::AcqRel);
            shared.restart_count.fetch_add(1, Ordering::AcqRel);
            set_status(
                &shared,
                &config,
                &profiler,
                &on_status,
                SidecarHealthStatus::CrashedRestarting,
                Some(format!("start stream failed: {e}")),
            );
            continue;
        }

        let mut reader = match ShmRingReader::open(&shm_path) {
            Ok(r) => r,
            Err(e) => {
                reap_child(&shared);
                shared.consecutive_crashes.fetch_add(1, Ordering::AcqRel);
                shared.restart_count.fetch_add(1, Ordering::AcqRel);
                set_status(
                    &shared,
                    &config,
                    &profiler,
                    &on_status,
                    SidecarHealthStatus::CrashedRestarting,
                    Some(format!("SHM reader open failed: {e}")),
                );
                continue;
            }
        };

        shared.started_at_ms.store(now_ms(), Ordering::Release);
        shared.force_restart.store(false, Ordering::Release);
        set_status(
            &shared,
            &config,
            &profiler,
            &on_status,
            SidecarHealthStatus::Running,
            None,
        );

        let mut frame_buf: Vec<u16> = Vec::with_capacity((config.width * config.height) as usize);
        let mut last_ping = Instant::now();
        let mut fps_window_start = Instant::now();
        let mut fps_window_frames: u64 = 0;
        let mut session_frames: u64 = 0;
        let poll_interval = Duration::from_millis((1000 / config.fps.max(10) as u64 / 2).clamp(4, 15));
        let heartbeat_every = Duration::from_millis(config.heartbeat_interval_ms.max(100));

        let crash_reason: Option<String> = loop {
            if shared.stop_flag.load(Ordering::Acquire) {
                let _ = client.send_request(SidecarRequest::Shutdown);
                reap_child(&shared);
                break None;
            }

            // Check if child process exited / was SIGKILLed
            if let Some(exit_msg) = check_child_exited(&shared) {
                break Some(exit_msg);
            }

            // Push ROI updates if modified by host UI/commands
            if shared.rois_dirty.swap(false, Ordering::AcqRel) {
                let rois = shared.rois.lock().clone();
                if let Err(e) = client.send_request(SidecarRequest::UpdateRois {
                    device_id: config.sidecar_id,
                    rois,
                }) {
                    break Some(format!("IPC error during ROI update: {e}"));
                }
            }

            // Poll zero-copy SHM ring buffer for newest 16-bit depth frame
            match reader.try_read_latest_u16(&mut frame_buf) {
                Ok(Some(hdr)) => {
                    session_frames += 1;
                    fps_window_frames += 1;
                    shared.frames_received.fetch_add(1, Ordering::Relaxed);
                    if session_frames >= 3 {
                        shared.consecutive_crashes.store(0, Ordering::Release);
                    }

                    let now_ns = kf_core::SignalEvent::now_ns();
                    if now_ns > hdr.timestamp_ns {
                        let lat_us = (now_ns - hdr.timestamp_ns) / 1000;
                        if let Some(ref prof) = profiler {
                            prof.record_latency_us(lat_us);
                            prof.record_protocol_messages("DEPTH", 1);
                        }
                    }

                    *shared.latest_frame.write() = Some(DepthFrameSnapshot {
                        header: hdr,
                        depth_mm: frame_buf.clone(),
                    });
                    *shared.ring_health.lock() = Some(reader.health());
                    push_profiler_update(&shared, &config, &profiler);

                    if let Some(ref cb) = on_frame {
                        cb(config.sidecar_id, &hdr, &frame_buf);
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    break Some(format!("SHM read error: {e}"));
                }
            }

            // Roll FPS counter every 500ms
            if fps_window_start.elapsed() >= Duration::from_millis(500) {
                let dt = fps_window_start.elapsed().as_secs_f32().max(0.01);
                *shared.fps.lock() = (fps_window_frames as f32 / dt * 10.0).round() / 10.0;
                fps_window_frames = 0;
                fps_window_start = Instant::now();
                *shared.ring_health.lock() = Some(reader.health());
                push_profiler_update(&shared, &config, &profiler);
            }

            // Periodic IPC heartbeat ping
            if last_ping.elapsed() >= heartbeat_every {
                last_ping = Instant::now();
                if let Err(e) = client.send_request(SidecarRequest::HealthPing) {
                    break Some(format!("heartbeat failed: {e}"));
                }
            }

            std::thread::sleep(poll_interval);
        };

        // Drop host SHM mapping and IPC pipe handles before reaping & cleaning up files (important on Windows).
        drop(reader);
        drop(client);
        reap_child(&shared);
        cleanup_socket_endpoint(&ipc_endpoint);
        let _ = std::fs::remove_file(&shm_path);
        shared.pid.store(0, Ordering::Release);
        *shared.fps.lock() = 0.0;

        if shared.stop_flag.load(Ordering::Acquire) {
            break;
        }

        let was_forced = shared.force_restart.swap(false, Ordering::AcqRel);
        let reason = crash_reason.unwrap_or_else(|| {
            if was_forced {
                "Manual sidecar restart requested".into()
            } else {
                "Sidecar process terminated unexpectedly".into()
            }
        });

        shared.restart_count.fetch_add(1, Ordering::AcqRel);
        if !was_forced {
            shared.consecutive_crashes.fetch_add(1, Ordering::AcqRel);
        }
        set_status(
            &shared,
            &config,
            &profiler,
            &on_status,
            SidecarHealthStatus::CrashedRestarting,
            Some(reason),
        );
    }

    *shared.status.lock() = SidecarHealthStatus::Stopped;
    push_profiler_update(&shared, &config, &profiler);
}

fn check_child_exited(shared: &SharedState) -> Option<String> {
    let mut guard = shared.child.lock();
    let child = guard.as_mut()?;
    match child.try_wait() {
        Ok(Some(status)) => {
            *guard = None;
            Some(format!("Sidecar process exited with status: {status}"))
        }
        Ok(None) => None,
        Err(e) => {
            *guard = None;
            Some(format!("Failed polling child status: {e}"))
        }
    }
}

fn reap_child(shared: &SharedState) {
    let mut guard = shared.child.lock();
    if let Some(mut child) = guard.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}
