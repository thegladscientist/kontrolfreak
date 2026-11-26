//! Milestone 3 crate: USB/Bluetooth HID Hot-Plug Watcher, 2-Tier HID Learn Engine
//! (Report Descriptor Parser + Baseline Noise-Masking Bit/Byte Diff Analyzer),
//! Portable `.kfdevice.json` Device Presets with Auto-Rebind, and RGB Camera CV Extractors.

pub mod camera;
pub mod hid;
pub mod manager;

pub use camera::{
    render_synthetic_frame, rgb_to_hsv, MotionZoneReading, NormalizedRoi, RgbCameraBackend,
    RgbCameraInfo, RgbCvConfig, RgbCvPipeline, RgbCvResult, RgbFrame, SyntheticRgbCamera,
    SyntheticSceneParams,
};
#[cfg(all(
    feature = "native-camera",
    any(target_os = "windows", target_os = "macos", target_os = "linux")
))]
pub use camera::NokhwaCameraBackend;

pub use hid::{
    compute_baseline_profile, fingerprint_from_hid_info, hid_usage_name,
    parse_hid_report_descriptor, sample_studio_fader_descriptor, BaselineProfile,
    CompositeHidBackend, DescriptorParseError, DevicePresetLibrary, HidBackend, HidControlType,
    HidDescriptorField, HidDeviceDefinition, HidHotplugWatcher, HidParameterSynthesizer,
    LearnWizardSnapshot, LearnedControlSpec, ParsedHidDescriptor, SimulatedHidAction,
    SimulatedHidBackend, SimulatedHidControlState, SimulatedHidProfile, TwoTierHidLearnEngine,
};
#[cfg(feature = "native-hid")]
pub use hid::HidApiBackend;

pub use manager::{CameraCvStepOutput, HardwareManager, HotplugNotification};

pub use kf_core::{
    DeviceFingerprint, HidControlClass, HidDeviceInfo, HidLearnStep, LearnedHidMapping, SignalValue,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum HardwareError {
    #[error("Hardware backend not implemented yet (scheduled for Milestone 3)")]
    NotImplementedYet,
    #[error("OS permission required (e.g. macOS Input Monitoring): {0}")]
    PermissionRequired(String),
    #[error("Hardware error: {0}")]
    Backend(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HidReportSample {
    pub report_id: u8,
    pub bytes: Vec<u8>,
    pub timestamp_ns: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum HardwareHotplugEvent {
    Connected {
        info: HidDeviceInfo,
        fingerprint: DeviceFingerprint,
    },
    Disconnected {
        path: String,
        fingerprint: DeviceFingerprint,
    },
}

/// Hot-plug watcher seam (implemented by `HidHotplugWatcher`).
pub trait HardwareWatcher: Send + Sync {
    fn list_hid_devices(&self) -> Result<Vec<HidDeviceInfo>, HardwareError>;
    fn check_os_permissions(&self) -> Result<(), HardwareError>;
}

/// 2-Tier HID Learn Engine seam (implemented by `TwoTierHidLearnEngine`).
pub trait HidLearnEngine: Send + Sync {
    fn step(&self) -> HidLearnStep;
    fn start_baseline(&mut self, device_path: &str) -> Result<(), HardwareError>;
    fn ingest_report(&mut self, sample: &HidReportSample) -> Result<(), HardwareError>;
    fn finalize_control(&mut self, alias: &str) -> Result<LearnedHidMapping, HardwareError>;
}

/// RGB camera CV extractor seam (implemented by `RgbCvPipeline`).
pub trait RgbCameraExtractor: Send + Sync {
    fn extractor_name(&self) -> &'static str;
    fn extract_signals(&mut self, rgba: &[u8], width: u32, height: u32) -> Vec<(String, SignalValue)>;
}

#[derive(Debug, Default)]
pub struct HardwareStub;

impl HardwareWatcher for HardwareStub {
    fn list_hid_devices(&self) -> Result<Vec<HidDeviceInfo>, HardwareError> {
        Ok(Vec::new())
    }

    fn check_os_permissions(&self) -> Result<(), HardwareError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kf_core::{
        ControlPlane, CurveKind, Device, DeviceKind, Modifier, Port, PortDirection, SignalKind,
        SignalRouter, TelemetryAggregator, WorkspaceGraph,
    };
    use std::sync::Arc;

    #[test]
    fn hardware_stub_lists_empty_cleanly() {
        let watcher = HardwareStub;
        assert!(watcher.list_hid_devices().unwrap().is_empty());
        assert!(watcher.check_os_permissions().is_ok());
    }

    #[test]
    fn tier1_hid_report_descriptor_parser_decodes_axes_wheel_and_buttons() {
        let raw = sample_studio_fader_descriptor();
        let parsed = parse_hid_report_descriptor(&raw).expect("valid descriptor parses");

        assert_eq!(parsed.usage_page, Some(0x01));
        assert_eq!(parsed.usage, Some(0x04));
        assert!(parsed.has_report_ids);
        // 4 faders + 1 dial + 1 relative wheel + 4 buttons = 10 data fields (4-bit constant padding omitted)
        assert_eq!(parsed.fields.len(), 10);

        // First 4 fields: 8-bit unsigned faders starting at bit 8 (after 8-bit Report ID)
        assert_eq!(parsed.fields[0].report_id, 1);
        assert_eq!(parsed.fields[0].usage_name, "Axis X");
        assert_eq!(parsed.fields[0].bit_offset, 8);
        assert_eq!(parsed.fields[0].bit_size, 8);
        assert_eq!(parsed.fields[0].logical_min, 0);
        assert_eq!(parsed.fields[0].logical_max, 255);
        assert!(!parsed.fields[0].is_signed);
        assert!(!parsed.fields[0].is_relative);

        // Field 4: 16-bit signed Dial at bit 40 (byte 5)
        assert_eq!(parsed.fields[4].usage_name, "Dial");
        assert_eq!(parsed.fields[4].bit_offset, 40);
        assert_eq!(parsed.fields[4].bit_size, 16);
        assert_eq!(parsed.fields[4].logical_min, -32768);
        assert_eq!(parsed.fields[4].logical_max, 32767);
        assert!(parsed.fields[4].is_signed);

        // Field 5: 8-bit relative Wheel at bit 56 (byte 7)
        assert_eq!(parsed.fields[5].usage_name, "Wheel");
        assert_eq!(parsed.fields[5].bit_offset, 56);
        assert!(parsed.fields[5].is_relative);
        assert!(parsed.fields[5].is_signed);

        // Fields 6..10: 4 x 1-bit Buttons at bit 64..67 (byte 8 bits 0..3)
        for i in 0..4 {
            let bf = &parsed.fields[6 + i];
            assert_eq!(bf.usage_page, 0x09);
            assert_eq!(bf.usage, (i + 1) as u16);
            assert_eq!(bf.bit_size, 1);
            assert_eq!(bf.bit_offset, 64 + i);
        }

        let mappings = parsed.to_learned_mappings();
        assert_eq!(mappings.len(), 10);
        assert_eq!(mappings[4].byte_offset, 5);
        assert_eq!(mappings[4].bit_width, 16);
        assert!(mappings[4].signed);
        assert_eq!(mappings[5].control_class, HidControlClass::RelativeEncoder);
        assert_eq!(mappings[6].control_class, HidControlClass::Button);
        assert_eq!(mappings[6].bit_mask, 0x01);
        assert_eq!(mappings[7].bit_mask, 0x02);
    }

    #[test]
    fn tier2_learn_wizard_end_to_end_on_noisy_imu_peripheral_and_auto_rebind() {
        let control = Arc::new(ControlPlane::new(WorkspaceGraph::new("M3 E2E")).unwrap());
        let telemetry = Arc::new(TelemetryAggregator::default());
        let router = Arc::new(SignalRouter::new(control.compiled_handle(), Arc::clone(&telemetry)));
        let mgr = HardwareManager::new(Arc::clone(&control), Arc::clone(&router));

        // 1. Plug in QuirkyImuGamepad (has rolling counter byte 1, noisy IMU bytes 2..=5, noisy footer byte 13)
        let (info, notes) = mgr
            .simulate_plug_in(SimulatedHidProfile::QuirkyImuGamepad, Some("test01"))
            .unwrap();
        assert_eq!(notes.len(), 1);
        assert!(matches!(notes[0].event, HardwareHotplugEvent::Connected { .. }));
        let path = info.path.clone();

        // 2. Step 1: Baseline idle sampling -> noise mask must mask rolling counter & noisy IMU bytes!
        mgr.learn_start_baseline(&path).unwrap();
        let snap = mgr.learn_poll_samples(40).unwrap();
        assert_eq!(snap.step, HidLearnStep::CapturingBaseline);
        assert_eq!(snap.baseline_samples, 40);

        let snap_after_base = mgr.learn_start_capture().unwrap();
        // Byte 1 (rolling counter) and Bytes 2..=5 (IMU noise) must be masked out!
        assert_eq!(snap_after_base.noise_mask_bits[1], 0xFF);
        assert_eq!(snap_after_base.noise_mask_bits[2], 0xFF);
        assert_eq!(snap_after_base.noise_mask_bits[4], 0xFF);
        // Byte 13 (timestamp low bits 0..1) masked
        assert_ne!(snap_after_base.noise_mask_bits[13], 0);
        // Clean control bytes (6, 7, 8, 9, 10, 11, 12) must have 0 noise mask
        for idx in [6, 7, 8, 9, 10, 11, 12] {
            assert_eq!(snap_after_base.noise_mask_bits[idx], 0, "byte {idx} should be clean");
        }

        // 3. Learn Control #1: 8-bit unsigned fader (byte 6)
        for v in (0..=255u16).step_by(15) {
            mgr.simulated_hid()
                .actuate(&path, SimulatedHidAction::SetFaderU8 { value: v as u8 })
                .unwrap();
            mgr.learn_poll_samples(1).unwrap();
        }
        // Return fader to rest before next control
        mgr.simulated_hid()
            .actuate(&path, SimulatedHidAction::SetFaderU8 { value: 0 })
            .unwrap();
        let c1 = mgr.learn_classify(Some("Master Fader")).unwrap();
        let cand1 = c1.candidate.expect("candidate detected");
        assert_eq!(cand1.control_type, HidControlType::Axis8);
        assert_eq!(cand1.byte_offset, 6);
        assert!(!cand1.signed);
        assert_eq!(cand1.observed_min, 0);
        assert!(cand1.observed_max >= 240);
        // Attach a hackable modifier stack (Invert + Curve + Expression)
        mgr.learn_commit_control(
            "Master Fader",
            vec![
                Modifier::Curve {
                    kind: CurveKind::Linear,
                },
                Modifier::Expression {
                    expr: "clamp(x, 0, 1)".into(),
                },
            ],
            None,
            None,
        )
        .unwrap();

        // 4. Learn Control #2: 16-bit signed LE joystick axis (bytes 7..=8)
        mgr.learn_start_capture().unwrap();
        for step in -20..=20i32 {
            let val = (step * 1500) as i16;
            mgr.simulated_hid()
                .actuate(&path, SimulatedHidAction::SetAxisI16Le { value: val })
                .unwrap();
            mgr.learn_poll_samples(1).unwrap();
        }
        mgr.simulated_hid()
            .actuate(&path, SimulatedHidAction::SetAxisI16Le { value: 0 })
            .unwrap();
        let c2 = mgr.learn_classify(Some("Joystick X")).unwrap();
        let cand2 = c2.candidate.expect("16-bit LE signed axis detected");
        assert_eq!(cand2.control_type, HidControlType::Axis16);
        assert_eq!(cand2.byte_offset, 7);
        assert!(cand2.signed, "must detect signed i16 crossing 0");
        assert!(cand2.little_endian, "must detect Little-Endian");
        assert_eq!(cand2.observed_min, -30000);
        assert_eq!(cand2.observed_max, 30000);
        mgr.learn_commit_control(
            "Joystick X",
            vec![
                Modifier::Deadzone {
                    center: 0.5,
                    radius: 0.05,
                    rescale: true,
                },
                Modifier::SmoothOneEuro {
                    min_cutoff: 1.5,
                    beta: 0.05,
                    d_cutoff: 1.0,
                },
            ],
            None,
            None,
        )
        .unwrap();

        // 5. Learn Control #3: 16-bit unsigned BE precision dial (bytes 9..=10)
        mgr.learn_start_capture().unwrap();
        for step in 0..=20u32 {
            let val = (step * 3000) as u16;
            mgr.simulated_hid()
                .actuate(&path, SimulatedHidAction::SetDialU16Be { value: val })
                .unwrap();
            mgr.learn_poll_samples(1).unwrap();
        }
        mgr.simulated_hid()
            .actuate(&path, SimulatedHidAction::SetDialU16Be { value: 0 })
            .unwrap();
        let c3 = mgr.learn_classify(Some("Precision Dial")).unwrap();
        let cand3 = c3.candidate.expect("16-bit BE unsigned dial detected");
        assert_eq!(cand3.control_type, HidControlType::Axis16);
        assert_eq!(cand3.byte_offset, 9);
        assert!(!cand3.signed, "must detect unsigned u16");
        assert!(!cand3.little_endian, "must detect Big-Endian");
        mgr.learn_commit_control("Precision Dial", Vec::new(), None, None)
            .unwrap();

        // 6. Learn Control #4: Momentary Button A (byte 11 bit 0)
        mgr.learn_start_capture().unwrap();
        mgr.learn_poll_samples(2).unwrap();
        mgr.simulated_hid()
            .actuate(&path, SimulatedHidAction::SetButtonA { pressed: true })
            .unwrap();
        mgr.learn_poll_samples(3).unwrap();
        mgr.simulated_hid()
            .actuate(&path, SimulatedHidAction::SetButtonA { pressed: false })
            .unwrap();
        mgr.learn_poll_samples(2).unwrap();
        let c4 = mgr.learn_classify(Some("Trigger A")).unwrap();
        let cand4 = c4.candidate.expect("button detected");
        assert_eq!(cand4.control_type, HidControlType::Button);
        assert_eq!(cand4.byte_offset, 11);
        assert_eq!(cand4.bit_mask, 0x01);
        mgr.learn_commit_control("Trigger A", Vec::new(), None, None)
            .unwrap();

        // 7. Learn Control #5: Relative Rotary Encoder (byte 12)
        mgr.learn_start_capture().unwrap();
        mgr.learn_poll_samples(2).unwrap();
        for delta in [1i8, 2, 1, -1, -2] {
            mgr.simulated_hid()
                .actuate(&path, SimulatedHidAction::PulseEncoder { delta })
                .unwrap();
            mgr.learn_poll_samples(1).unwrap(); // consumes delta
            mgr.learn_poll_samples(1).unwrap(); // returns to 0
        }
        let c5 = mgr.learn_classify(Some("Macro Encoder")).unwrap();
        let cand5 = c5.candidate.expect("relative encoder detected");
        assert_eq!(cand5.control_type, HidControlType::RelativeEncoder);
        assert_eq!(cand5.byte_offset, 12);
        assert!(cand5.signed);
        mgr.learn_commit_control("Macro Encoder", Vec::new(), Some(0.05), Some(false))
            .unwrap();

        // 8. Step 4: Export `.kfdevice.json` & attach to graph, wire Master Fader -> OSC target
        let (def, preset_json, hid_dev) = mgr
            .save_and_attach_learned_device("QuirkyPad Live Preset", 100.0, 120.0)
            .unwrap();
        assert_eq!(def.controls.len(), 5);
        assert_eq!(hid_dev.ports.len(), 5);

        // Verify `.kfdevice.json` round-trips cleanly
        let parsed_def = HidDeviceDefinition::from_kfdevice_json(&preset_json).unwrap();
        assert_eq!(parsed_def.vendor_id, 0x256F);
        assert_eq!(parsed_def.product_id, 0xC635);
        assert_eq!(parsed_def.controls.len(), 5);

        // Wire Master Fader -> downstream target node
        let target_port = Port::new("/dimmer", "Dimmer", PortDirection::In, SignalKind::Control);
        let target_pid = target_port.id;
        control
            .add_device(Device::new("osc.out", "Stage Lights", DeviceKind::VirtualControl).with_port(target_port))
            .unwrap();
        let fader_port_id = hid_dev.ports[0].id;
        control.add_wire(fader_port_id, target_pid).unwrap();

        // Actuate fader to full scale -> routes normalized 1.0 to target!
        let outs = mgr
            .simulate_actuate(&path, SimulatedHidAction::SetFaderU8 { value: 255 })
            .unwrap();
        let fader_out = outs
            .iter()
            .find(|o| o.target_port_id == target_pid)
            .expect("fader routed to target");
        assert!((fader_out.value.as_f64().unwrap() - 1.0).abs() < 1e-3);

        // 9. Simulate Unplug -> HidDevice enters OfflineGhost with wires intact
        let unplug_notes = mgr.simulate_unplug(&path).unwrap();
        assert_eq!(unplug_notes.len(), 1);
        assert_eq!(unplug_notes[0].ghosted_device_ids, vec![hid_dev.id]);
        let ghost_dev = control
            .snapshot()
            .devices
            .into_iter()
            .find(|d| d.id == hid_dev.id)
            .unwrap();
        assert!(ghost_dev.status.is_offline_ghost());
        assert_eq!(control.snapshot().wires.len(), 1);

        // 10. Simulate Replug -> Auto-recognized & Auto-rebound back Online, routing resumes immediately!
        let (_, replug_notes) = mgr
            .simulate_plug_in(SimulatedHidProfile::QuirkyImuGamepad, Some("test01"))
            .unwrap();
        assert_eq!(replug_notes.len(), 1);
        assert_eq!(replug_notes[0].auto_rebound_device_id, Some(hid_dev.id));
        assert_eq!(
            replug_notes[0].recognized_preset_name.as_deref(),
            Some("QuirkyPad Live Preset")
        );

        let rebound_dev = control
            .snapshot()
            .devices
            .into_iter()
            .find(|d| d.id == hid_dev.id)
            .unwrap();
        assert!(rebound_dev.status.is_online());

        // Move fader halfway -> live routed output resumes!
        let resumed_outs = mgr
            .simulate_actuate(&path, SimulatedHidAction::SetFaderU8 { value: 128 })
            .unwrap();
        let resumed_val = resumed_outs
            .iter()
            .find(|o| o.target_port_id == target_pid)
            .expect("live routing resumed after replug")
            .value
            .as_f64()
            .unwrap();
        assert!((resumed_val - 0.5).abs() < 0.05);
    }

    #[test]
    fn rgb_camera_cv_extractors_brightness_motion_color_centroid_and_zones() {
        let mut params = SyntheticSceneParams {
            width: 120,
            height: 90,
            bg_brightness: 0.10,
            orb_rgb: [20, 240, 160], // Hue ~158 deg (emerald-cyan)
            orb_radius_norm: 0.15,
            fixed_orb_pos: Some([0.25, 0.30]),
            bright_spot_pos: Some([0.80, 0.75]),
        };

        let mut pipeline = RgbCvPipeline::new(RgbCvConfig {
            target_hue_deg: 158.0,
            hue_tolerance_deg: 30.0,
            min_saturation: 0.4,
            min_value: 0.3,
            motion_pixel_threshold: 0.06,
            zone_cols: 2,
            zone_rows: 2,
            zone_trigger_threshold: 0.03,
            ..Default::default()
        });

        // Frame 1: Orb at (0.25, 0.30), pure white highlight at (0.80, 0.75)
        let f1 = render_synthetic_frame(&params, 0);
        let (res1, sigs1) = pipeline.process_frame(&f1.rgba, f1.width, f1.height, 1_000_000);

        // Brightness check: dark background + orb + bright spot -> moderate low global luma
        assert!(res1.global_luma > 0.10 && res1.global_luma < 0.35);
        // First frame has zero motion energy
        assert_eq!(res1.motion_energy, 0.0);
        // Target-color blob centroid accurately tracks orb at (0.25, 0.30)
        assert!((res1.blob_centroid[0] - 0.25).abs() < 0.03, "blob_x={}", res1.blob_centroid[0]);
        assert!((res1.blob_centroid[1] - 0.30).abs() < 0.03, "blob_y={}", res1.blob_centroid[1]);
        assert!(res1.blob_area > 0.03);
        // Brightest-point centroid accurately tracks white highlight at (0.80, 0.75)
        assert!((res1.brightest_point[0] - 0.80).abs() < 0.03, "bright_x={}", res1.brightest_point[0]);
        assert!((res1.brightest_point[1] - 0.75).abs() < 0.03, "bright_y={}", res1.brightest_point[1]);
        assert!(res1.brightest_luma > 0.95);
        assert!(sigs1.iter().any(|(k, _)| k == "cam.blob.x"));

        // Frame 2: Move orb dramatically to (0.75, 0.25) and raise background brightness
        params.fixed_orb_pos = Some([0.75, 0.25]);
        params.bg_brightness = 0.55;
        let f2 = render_synthetic_frame(&params, 1);
        let (res2, _) = pipeline.process_frame(&f2.rgba, f2.width, f2.height, 34_000_000);

        // Brightness jumped significantly
        assert!(res2.global_luma > res1.global_luma + 0.30);
        // Motion energy is strongly positive
        assert!(res2.motion_energy > 0.5, "motion_energy={}", res2.motion_energy);
        // Blob centroid moved cleanly to (0.75, 0.25)
        assert!((res2.blob_centroid[0] - 0.75).abs() < 0.03, "blob_x2={}", res2.blob_centroid[0]);
        assert!((res2.blob_centroid[1] - 0.25).abs() < 0.03, "blob_y2={}", res2.blob_centroid[1]);
        // Motion zones triggered!
        assert_eq!(res2.zones.len(), 4);
        assert!(res2.zones.iter().any(|z| z.triggered));

        // Frame 3: Identical static scene -> motion energy drops back to 0.0
        let f3 = render_synthetic_frame(&params, 2);
        let (res3, _) = pipeline.process_frame(&f3.rgba, f3.width, f3.height, 67_000_000);
        assert_eq!(res3.motion_energy, 0.0);
        assert!(res3.zones.iter().all(|z| !z.triggered));
    }

    #[test]
    fn tier2_toggle_hat_switch_and_modifier_synthesis() {
        let control = Arc::new(ControlPlane::new(WorkspaceGraph::new("Toggle & Hat")).unwrap());
        let telemetry = Arc::new(TelemetryAggregator::default());
        let router = Arc::new(SignalRouter::new(control.compiled_handle(), telemetry));
        let mgr = HardwareManager::new(control, router);

        let (info, _) = mgr
            .simulate_plug_in(SimulatedHidProfile::QuirkyImuGamepad, Some("hat01"))
            .unwrap();
        let path = info.path;

        // Baseline
        mgr.learn_start_baseline(&path).unwrap();
        mgr.learn_poll_samples(25).unwrap();

        // Learn Toggle Switch C (byte 11 bit 2) — latches on and stays on
        mgr.learn_start_capture().unwrap();
        mgr.learn_poll_samples(2).unwrap();
        mgr.simulated_hid()
            .actuate(&path, SimulatedHidAction::ToggleSwitchC)
            .unwrap();
        mgr.learn_poll_samples(4).unwrap();
        let toggle_snap = mgr.learn_classify(Some("Latch C")).unwrap();
        let toggle_spec = toggle_snap.candidate.unwrap();
        assert_eq!(toggle_spec.control_type, HidControlType::Toggle);
        assert_eq!(toggle_spec.byte_offset, 11);
        assert_eq!(toggle_spec.bit_mask, 0x04);
        // Reset switch C back to rest
        mgr.simulated_hid()
            .actuate(&path, SimulatedHidAction::ToggleSwitchC)
            .unwrap();

        // Learn 4-bit Hat Switch (byte 11 bits 4..7)
        mgr.learn_start_capture().unwrap();
        mgr.learn_poll_samples(2).unwrap();
        for dir in [0u8, 2, 4, 6, 8] {
            mgr.simulated_hid()
                .actuate(&path, SimulatedHidAction::SetHatSwitch { direction: dir })
                .unwrap();
            mgr.learn_poll_samples(1).unwrap();
        }
        let hat_snap = mgr.learn_classify(Some("D-Pad Hat")).unwrap();
        let hat_spec = hat_snap.candidate.unwrap();
        assert_eq!(hat_spec.control_type, HidControlType::HatSwitch);
        assert_eq!(hat_spec.byte_offset, 11);
        assert_eq!(hat_spec.bit_mask, 0xF0);
        assert_eq!(hat_spec.bit_width, 4);

        // Verify synthesizer with Invert + RangeMap + Expression + Hysteresis
        let mut fader_spec = LearnedControlSpec {
            logical_key: "fader.hacked".into(),
            alias: "Hacked Fader".into(),
            report_id: 1,
            byte_offset: 6,
            bit_offset: 48,
            bit_mask: 0xFF,
            bit_width: 8,
            signed: false,
            little_endian: true,
            control_type: HidControlType::Axis8,
            observed_min: 0,
            observed_max: 255,
            sensitivity: 0.02,
            wrap: false,
            modifiers: vec![
                Modifier::Invert,
                Modifier::RangeMap {
                    in_min: 0.0,
                    in_max: 1.0,
                    out_min: 0.0,
                    out_max: 100.0,
                    clamp: true,
                },
                Modifier::Expression {
                    expr: "round(x)".into(),
                },
            ],
        };
        let mut synth = HidParameterSynthesizer::compile(&[fader_spec.clone()]).unwrap();
        let sample = HidReportSample {
            report_id: 1,
            bytes: vec![1, 0, 0, 0, 0, 0, 0, 0], // raw 0 -> norm 0.0 -> invert 1.0 -> map 100.0
            timestamp_ns: 1_000_000,
        };
        let out = synth.process_report(&sample);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].1, SignalValue::Float(100.0));

        // Relative encoder wrap mode test
        fader_spec.logical_key = "enc.wrap".into();
        fader_spec.byte_offset = 12;
        fader_spec.control_type = HidControlType::RelativeEncoder;
        fader_spec.sensitivity = 0.2;
        fader_spec.wrap = true;
        fader_spec.modifiers.clear();
        let mut enc_synth = HidParameterSynthesizer::compile(&[fader_spec]).unwrap();
        let mut pkt = vec![0u8; 16];
        pkt[0] = 1;
        pkt[12] = 3u8; // +3 * 0.2 = +0.6 from 0.5 -> 1.1 rem_euclid 1.0 = 0.1
        let enc_out = enc_synth.process_report(&HidReportSample {
            report_id: 1,
            bytes: pkt,
            timestamp_ns: 2_000_000,
        });
        assert!((enc_out[0].1.as_f64().unwrap() - 0.1).abs() < 1e-6);
    }

    #[test]
    fn tier1_descriptor_auto_seeds_studio_fader_wing() {
        let control = Arc::new(ControlPlane::new(WorkspaceGraph::new("Tier 1 Seed")).unwrap());
        let telemetry = Arc::new(TelemetryAggregator::default());
        let router = Arc::new(SignalRouter::new(control.compiled_handle(), telemetry));
        let mgr = HardwareManager::new(control, router);

        let (info, _) = mgr
            .simulate_plug_in(SimulatedHidProfile::StudioFaderWing, Some("wing01"))
            .unwrap();
        let parsed = mgr
            .parse_device_descriptor(&info.path)
            .unwrap()
            .expect("StudioFaderWing exposes standard descriptor");
        assert_eq!(parsed.fields.len(), 10);

        let snap = mgr.learn_start_baseline(&info.path).unwrap();
        assert_eq!(snap.committed_controls.len(), 10);
    }
}
