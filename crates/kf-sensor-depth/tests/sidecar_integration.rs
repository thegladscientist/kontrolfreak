use kf_core::{
    ControlPlane, Device, DeviceKind, MidiBackendKind, Port, PortDirection, SignalEvent,
    SignalKind, SignalRouter, SignalValue, TelemetryAggregator, WorkspaceGraph,
};
use kf_sensor_depth::{
    build_depth_ndi_frame_ref, colorize_depth_to_rgba, extract_roi, CameraIntrinsics,
};
use kf_sidecar_ipc::{
    DepthColormap, DiagnosticsProfiler, NamedDepthRoi, SidecarHealthStatus, SidecarSupervisor,
    SupervisorConfig,
};
use parking_lot::Mutex;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

fn wait_until<F>(timeout: Duration, mut cond: F) -> bool
where
    F: FnMut() -> bool,
{
    let start = Instant::now();
    while start.elapsed() < timeout {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

#[test]
fn real_out_of_process_sidecar_shm_delivery_and_3d_extraction() {
    let bin = PathBuf::from(env!("CARGO_BIN_EXE_kf-sensor-depth"));
    assert!(bin.exists(), "kf-sensor-depth binary should be built by cargo test");

    let device_id = Uuid::new_v4();
    let profiler = Arc::new(DiagnosticsProfiler::new());
    let received_frames = Arc::new(AtomicU64::new(0));
    let received_bg = Arc::clone(&received_frames);

    let mut cfg = SupervisorConfig::new(device_id, bin);
    cfg.vendor = "synthetic".into();
    cfg.width = 80;
    cfg.height = 60;
    cfg.fps = 40;
    cfg.initial_backoff_ms = 80;

    let supervisor = SidecarSupervisor::new(
        cfg,
        Some(Arc::clone(&profiler)),
        Some(Arc::new(move |_id, hdr, pixels| {
            assert_eq!(hdr.width, 80);
            assert_eq!(hdr.height, 60);
            assert_eq!(pixels.len(), 80 * 60);
            received_bg.fetch_add(1, Ordering::Relaxed);
        })),
        None,
    );

    supervisor.start().expect("supervisor starts cleanly");

    let got_frames = wait_until(Duration::from_secs(15), || {
        supervisor.status() == SidecarHealthStatus::Running
            && received_frames.load(Ordering::Relaxed) >= 5
    });
    assert!(
        got_frames,
        "expected >=5 SHM depth frames from out-of-process sidecar, status={:?}, count={}",
        supervisor.status(),
        received_frames.load(Ordering::Relaxed)
    );

    // Inspect real depth frame from SHM, run 3D ROI extraction & Depth-to-RGBA colormap
    let snap = supervisor
        .latest_depth_frame()
        .expect("latest SHM depth frame available");
    assert_eq!(snap.header.width, 80);
    assert_eq!(snap.header.height, 60);

    let roi = NamedDepthRoi::default();
    let extracted = extract_roi(
        &snap.depth_mm,
        snap.header.width,
        snap.header.height,
        &roi,
        CameraIntrinsics::default(),
    );
    assert!(extracted.valid_pixel_count > 0);
    assert!(extracted.min_depth_mm >= roi.near_mm && extracted.min_depth_mm <= roi.far_mm);
    assert!(extracted.presence);

    let mut rgba = Vec::new();
    colorize_depth_to_rgba(
        &snap.depth_mm,
        snap.header.width,
        snap.header.height,
        roi.near_mm,
        roi.far_mm,
        DepthColormap::Turbo,
        &mut rgba,
    );
    assert_eq!(rgba.len(), 80 * 60 * 4);
    let fref = build_depth_ndi_frame_ref(
        device_id,
        snap.header.frame_seq,
        snap.header.width,
        snap.header.height,
        snap.header.timestamp_ns,
    );
    assert_eq!(fref.width, 80);
    assert_eq!(fref.height, 60);

    let diag = profiler.snapshot();
    assert_eq!(diag.sidecars.len(), 1);
    assert_eq!(diag.sidecars[0].status, SidecarHealthStatus::Running);
    assert!(diag.sidecars[0].frames_received >= 5);

    supervisor.stop();
    assert_eq!(supervisor.status(), SidecarHealthStatus::Stopped);
}

#[test]
fn real_crash_isolation_sigkill_mid_stream_recovers_without_interrupting_router() {
    let bin = PathBuf::from(env!("CARGO_BIN_EXE_kf-sensor-depth"));

    // Spin up a real kf-core ControlPlane + SignalRouter to prove OSC -> MIDI keeps working
    // with zero glitches while the depth sidecar is SIGKILLed and restarted.
    let mut graph = WorkspaceGraph::new("Crash Isolation Show Rig");
    let osc_p = Port::new("/fader/1", "Fader", PortDirection::Out, SignalKind::Control);
    let midi_p = Port::new("cc.1.7", "Volume", PortDirection::In, SignalKind::Midi);
    let osc_pid = osc_p.id;
    let midi_pid = midi_p.id;
    graph.devices.push(
        Device::new(
            "osc.1",
            "TouchOSC",
            DeviceKind::OscEndpoint {
                bind_addr: "127.0.0.1:0".into(),
                target_addr: None,
                auto_discover: true,
            },
        )
        .with_port(osc_p),
    );
    graph.devices.push(
        Device::new(
            "midi.1",
            "Synth",
            DeviceKind::MidiPort {
                port_name: "Loopback".into(),
                is_virtual: true,
                channel_filter: None,
                backend: MidiBackendKind::LoopbackMock,
            },
        )
        .with_port(midi_p),
    );
    graph.wires.push(kf_core::Wire::new(osc_pid, midi_pid));

    let control = Arc::new(ControlPlane::new(graph).unwrap());
    let telemetry = Arc::new(TelemetryAggregator::default());
    let router = Arc::new(SignalRouter::new(control.compiled_handle(), telemetry));

    // Start supervised out-of-process depth sidecar
    let device_id = Uuid::new_v4();
    let profiler = Arc::new(DiagnosticsProfiler::new());
    let observed_statuses = Arc::new(Mutex::new(Vec::<SidecarHealthStatus>::new()));
    let statuses_bg = Arc::clone(&observed_statuses);

    let mut cfg = SupervisorConfig::new(device_id, bin);
    cfg.vendor = "realsense".into();
    cfg.width = 64;
    cfg.height = 48;
    cfg.fps = 40;
    cfg.heartbeat_interval_ms = 200;
    cfg.initial_backoff_ms = 100;
    cfg.max_backoff_ms = 1000;

    let supervisor = SidecarSupervisor::new(
        cfg,
        Some(Arc::clone(&profiler)),
        None,
        Some(Arc::new(move |_id, status, _reason| {
            statuses_bg.lock().push(status);
        })),
    );
    supervisor.start().unwrap();

    // Wait for initial healthy stream
    assert!(wait_until(Duration::from_secs(15), || {
        let s = supervisor.health_snapshot();
        s.status == SidecarHealthStatus::Running && s.frames_received >= 4 && s.pid.is_some()
    }));

    let first_pid = supervisor.child_pid().expect("child PID present");
    let frames_before_kill = supervisor.health_snapshot().frames_received;

    // SIGKILL / Terminate the running child process mid-stream!
    supervisor
        .simulate_crash()
        .expect("simulate_crash terminates live child");

    // Immediately verify core OSC -> MIDI routing is completely unaffected!
    for i in 0..10 {
        let outs = router.route(SignalEvent::new(
            osc_pid,
            SignalValue::Float(i as f64 / 10.0),
            SignalEvent::now_ns(),
        ));
        assert_eq!(outs.len(), 1);
        assert_eq!(outs[0].target_port_id, midi_pid);
    }

    // Verify supervisor detects crash, enters CrashedRestarting, restarts with a NEW PID,
    // and resumes delivering depth frames over SHM!
    let recovered = wait_until(Duration::from_secs(20), || {
        let snap = supervisor.health_snapshot();
        snap.status == SidecarHealthStatus::Running
            && snap.restart_count >= 1
            && snap.pid.is_some()
            && snap.pid != Some(first_pid)
            && snap.frames_received >= frames_before_kill + 4
    });

    let final_snap = supervisor.health_snapshot();
    assert!(
        recovered,
        "expected sidecar to recover after SIGKILL: snap={final_snap:?}, history={:?}",
        *observed_statuses.lock()
    );
    assert!(
        observed_statuses
            .lock()
            .contains(&SidecarHealthStatus::CrashedRestarting),
        "expected CrashedRestarting status transition to be observed"
    );
    assert_ne!(final_snap.pid, Some(first_pid));
    assert!(final_snap.restart_count >= 1);

    supervisor.stop();
}
