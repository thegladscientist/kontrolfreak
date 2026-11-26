//! Crash-isolated out-of-process depth camera sidecar executable (`kf-sensor-depth`).
//! Speaks Kontrolfreak's local IPC control protocol (`kf-sidecar-ipc`) and streams
//! raw 16-bit millimeter depth frames over a zero-copy `memmap2` shared-memory ring buffer.

use kf_core::SignalEvent;
use kf_sensor_depth::{create_backend, extract_all_rois, to_depth_sample_summary, CameraIntrinsics};
use kf_sidecar_ipc::{
    IpcServer, NamedDepthRoi, RoiExtractionResult, ShmRingWriter, SidecarHealthStatus,
    SidecarRequest, SidecarResponse, SIDECAR_PROTOCOL_VERSION,
};
use parking_lot::Mutex;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

struct CliArgs {
    ipc: String,
    shm: PathBuf,
    vendor: String,
    width: u32,
    height: u32,
    fps: u32,
    device_id: Uuid,
}

fn parse_args() -> CliArgs {
    let mut ipc = String::new();
    let mut shm = PathBuf::new();
    let mut vendor = "synthetic".to_string();
    let mut width = 160u32;
    let mut height = 120u32;
    let mut fps = 30u32;
    let mut device_id = Uuid::new_v4();

    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let Some(val) = args.next() else {
            break;
        };
        match flag.as_str() {
            "--ipc" => ipc = val,
            "--shm" => shm = PathBuf::from(val),
            "--vendor" => vendor = val,
            "--width" => {
                if let Ok(v) = val.parse() {
                    width = v;
                }
            }
            "--height" => {
                if let Ok(v) = val.parse() {
                    height = v;
                }
            }
            "--fps" => {
                if let Ok(v) = val.parse() {
                    fps = v;
                }
            }
            "--device-id" => {
                if let Ok(u) = Uuid::parse_str(&val) {
                    device_id = u;
                }
            }
            _ => {}
        }
    }

    if ipc.is_empty() {
        ipc = std::env::temp_dir()
            .join(format!("kf-depth-{}.sock", device_id.simple()))
            .to_string_lossy()
            .to_string();
    }
    if shm.as_os_str().is_empty() {
        shm = std::env::temp_dir().join(format!("kf-depth-{}.shm", device_id.simple()));
    }

    CliArgs {
        ipc,
        shm,
        vendor,
        width: width.max(16),
        height: height.max(16),
        fps: fps.clamp(5, 120),
        device_id,
    }
}

fn main() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();

    let args = parse_args();
    let started_at = Instant::now();
    let stop_flag = Arc::new(AtomicBool::new(false));
    let streaming = Arc::new(AtomicBool::new(true));
    let frames_produced = Arc::new(AtomicU64::new(0));
    let rois: Arc<Mutex<Vec<NamedDepthRoi>>> = Arc::new(Mutex::new(vec![NamedDepthRoi::default()]));
    let latest_samples: Arc<Mutex<Vec<RoiExtractionResult>>> = Arc::new(Mutex::new(Vec::new()));

    let mut shm_writer = match ShmRingWriter::open(&args.shm) {
        Ok(w) => w,
        Err(_) => ShmRingWriter::create(&args.shm, 4, args.width, args.height)
            .expect("failed to create SHM ring buffer"),
    };

    let ipc_server = IpcServer::bind(&args.ipc).expect("failed to bind sidecar IPC endpoint");

    // Control IPC listener thread
    let ipc_stop = Arc::clone(&stop_flag);
    let ipc_streaming = Arc::clone(&streaming);
    let ipc_frames = Arc::clone(&frames_produced);
    let ipc_rois = Arc::clone(&rois);
    let ipc_samples = Arc::clone(&latest_samples);
    let shm_path_str = args.shm.to_string_lossy().to_string();
    let width = args.width;
    let height = args.height;
    let fps = args.fps;
    let vendor_name = args.vendor.clone();
    let default_dev_id = args.device_id;

    let _ipc_thread = std::thread::spawn(move || {
        let _ = ipc_server.serve_loop(ipc_stop, |req| match req {
            SidecarRequest::Hello { protocol_version } => (
                SidecarResponse::HelloAck {
                    protocol_version: protocol_version.min(SIDECAR_PROTOCOL_VERSION),
                    sidecar_name: format!("kf-sensor-depth ({vendor_name})"),
                },
                true,
            ),
            SidecarRequest::StartStream {
                device_id,
                vendor: _,
                roi,
            } => {
                let mut guard = ipc_rois.lock();
                if guard.is_empty() {
                    guard.push(NamedDepthRoi::from(&roi));
                } else {
                    let first = NamedDepthRoi::from(&roi);
                    guard[0].x = first.x;
                    guard[0].y = first.y;
                    guard[0].w = first.w;
                    guard[0].h = first.h;
                    guard[0].near_mm = first.near_mm;
                    guard[0].far_mm = first.far_mm;
                }
                ipc_streaming.store(true, Ordering::Release);
                (
                    SidecarResponse::StreamStarted {
                        device_id,
                        shm_path: shm_path_str.clone(),
                        width,
                        height,
                        fps,
                    },
                    true,
                )
            }
            SidecarRequest::UpdateRoi { device_id, roi } => {
                let mut guard = ipc_rois.lock();
                let converted = NamedDepthRoi::from(&roi);
                if guard.is_empty() {
                    guard.push(converted);
                } else {
                    guard[0] = converted;
                }
                (
                    SidecarResponse::RoiUpdated {
                        device_id,
                        roi_count: guard.len(),
                    },
                    true,
                )
            }
            SidecarRequest::UpdateRois {
                device_id,
                rois: next_rois,
            } => {
                let count = next_rois.len();
                *ipc_rois.lock() = next_rois;
                (
                    SidecarResponse::RoiUpdated {
                        device_id,
                        roi_count: count,
                    },
                    true,
                )
            }
            SidecarRequest::ConfigureColormap { .. } => (
                SidecarResponse::Status {
                    health: SidecarHealthStatus::Running,
                    active_streams: usize::from(ipc_streaming.load(Ordering::Acquire)),
                },
                true,
            ),
            SidecarRequest::StopStream { device_id } => {
                ipc_streaming.store(false, Ordering::Release);
                (SidecarResponse::StreamStopped { device_id }, true)
            }
            SidecarRequest::HealthPing => {
                let latest = ipc_samples.lock().clone();
                if let Some(first) = latest.first() {
                    let _ = to_depth_sample_summary(default_dev_id, first, SignalEvent::now_ms());
                }
                (
                    SidecarResponse::Pong {
                        uptime_ms: started_at.elapsed().as_millis() as u64,
                        pid: std::process::id(),
                        frames_produced: ipc_frames.load(Ordering::Acquire),
                    },
                    true,
                )
            }
            SidecarRequest::SimulateCrash => {
                // Intentionally terminate abruptly without cleanup to test host crash isolation.
                std::process::exit(137);
            }
            SidecarRequest::Shutdown => (
                SidecarResponse::Status {
                    health: SidecarHealthStatus::Stopped,
                    active_streams: 0,
                },
                false,
            ),
        });
    });

    // Main real-time depth capture -> zero-copy SHM ring buffer loop
    let mut backend = create_backend(&args.vendor, args.width, args.height, true);
    let frame_period = Duration::from_secs_f64(1.0 / (args.fps as f64));
    let mut frame_index: u64 = 0;

    while !stop_flag.load(Ordering::Acquire) {
        let tick_start = Instant::now();

        if streaming.load(Ordering::Acquire) {
            if let Ok(frame) = backend.next_frame(frame_index) {
                if shm_writer
                    .write_depth_u16(frame.width, frame.height, frame.timestamp_ns, &frame.depth_mm)
                    .is_ok()
                {
                    frames_produced.fetch_add(1, Ordering::Release);
                    let active_rois = rois.lock().clone();
                    let extracted = extract_all_rois(
                        &frame.depth_mm,
                        frame.width,
                        frame.height,
                        &active_rois,
                        CameraIntrinsics::default(),
                    );
                    *latest_samples.lock() = extracted;
                }
            }
            frame_index = frame_index.wrapping_add(1);
        }

        let elapsed = tick_start.elapsed();
        if elapsed < frame_period {
            std::thread::sleep(frame_period - elapsed);
        }
    }

    shm_writer.mark_stopped();
}
