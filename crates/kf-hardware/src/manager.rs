//! High-level `HardwareManager` coordinating HID hot-plug detection, the 2-Tier Learn Engine,
//! `.kfdevice.json` preset auto-recognition / auto-rebind, and RGB Camera CV parameter routing.

use crate::camera::{
    RgbCameraBackend, RgbCameraInfo, RgbCvConfig, RgbCvPipeline, RgbCvResult, SyntheticRgbCamera,
    SyntheticSceneParams,
};
use crate::hid::{
    fingerprint_from_hid_info, parse_hid_report_descriptor, CompositeHidBackend,
    DevicePresetLibrary, HidBackend, HidDeviceDefinition, HidHotplugWatcher,
    HidParameterSynthesizer, LearnWizardSnapshot, LearnedControlSpec, ParsedHidDescriptor,
    SimulatedHidAction, SimulatedHidBackend, SimulatedHidProfile, TwoTierHidLearnEngine,
};
use crate::{HardwareError, HardwareHotplugEvent, HardwareWatcher, HidLearnEngine};
use kf_core::{
    ControlPlane, Device, HidDeviceInfo, Modifier, RoutedOutput, SignalEvent, SignalRouter,
    SignalValue,
};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotplugNotification {
    pub event: HardwareHotplugEvent,
    pub recognized_preset_name: Option<String>,
    pub auto_rebound_device_id: Option<Uuid>,
    pub ghosted_device_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CameraCvStepOutput {
    pub camera_id: String,
    pub frame_index: u64,
    pub width: u32,
    pub height: u32,
    pub cv: RgbCvResult,
    pub routed_count: usize,
}

struct ActiveHidBinding {
    device_id: Uuid,
    definition: HidDeviceDefinition,
    synthesizer: HidParameterSynthesizer,
}

/// Manages M3 hardware subsystems (HID hot-plug, 2-tier Learn Engine, preset library, RGB CV)
/// and bridges live hardware parameters directly into `kf_core::ControlPlane` + `SignalRouter`.
pub struct HardwareManager {
    control: Arc<ControlPlane>,
    router: Arc<SignalRouter>,
    simulated_hid: Arc<SimulatedHidBackend>,
    hid_backend: Arc<CompositeHidBackend>,
    watcher: HidHotplugWatcher,
    learn_engine: Mutex<TwoTierHidLearnEngine>,
    preset_library: Mutex<DevicePresetLibrary>,
    active_hid_bindings: Mutex<HashMap<String, ActiveHidBinding>>,
    synthetic_camera: Arc<SyntheticRgbCamera>,
    cv_pipelines: Mutex<HashMap<String, RgbCvPipeline>>,
}

impl HardwareManager {
    pub fn new(control: Arc<ControlPlane>, router: Arc<SignalRouter>) -> Self {
        let simulated_hid = Arc::new(SimulatedHidBackend::new());
        #[cfg(feature = "native-hid")]
        let native_hid: Option<Arc<dyn HidBackend>> = Some(Arc::new(crate::hid::HidApiBackend));
        #[cfg(not(feature = "native-hid"))]
        let native_hid: Option<Arc<dyn HidBackend>> = None;

        let hid_backend = Arc::new(CompositeHidBackend::new(
            Arc::clone(&simulated_hid),
            native_hid,
        ));
        let watcher = HidHotplugWatcher::new(Arc::clone(&hid_backend) as Arc<dyn HidBackend>);
        let synthetic_camera = Arc::new(SyntheticRgbCamera::default_demo());

        Self {
            control,
            router,
            simulated_hid,
            hid_backend,
            watcher,
            learn_engine: Mutex::new(TwoTierHidLearnEngine::new()),
            preset_library: Mutex::new(DevicePresetLibrary::new()),
            active_hid_bindings: Mutex::new(HashMap::new()),
            synthetic_camera,
            cv_pipelines: Mutex::new(HashMap::new()),
        }
    }

    pub fn simulated_hid(&self) -> &Arc<SimulatedHidBackend> {
        &self.simulated_hid
    }

    pub fn synthetic_camera(&self) -> &Arc<SyntheticRgbCamera> {
        &self.synthetic_camera
    }

    // ---------- HID Enumeration & Hot-Plug ----------

    pub fn list_hid_devices(&self) -> Result<Vec<HidDeviceInfo>, HardwareError> {
        self.watcher.list_hid_devices()
    }

    pub fn check_hid_permissions(&self) -> Result<(), HardwareError> {
        self.watcher.check_os_permissions()
    }

    /// Polls HID hot-plug arrivals/removals, auto-ghosts disconnected `HidDevice` nodes,
    /// and auto-recognizes / auto-rebinds reconnected peripherals using saved `.kfdevice.json` presets.
    pub fn poll_hotplug(&self) -> Result<Vec<HotplugNotification>, HardwareError> {
        let raw_events = self.watcher.poll_events()?;
        let mut notifications = Vec::new();

        for ev in raw_events {
            match &ev {
                HardwareHotplugEvent::Disconnected { path, fingerprint } => {
                    tracing::debug!(hid_path = %path, "HID peripheral disconnected");
                    self.active_hid_bindings.lock().remove(path);
                    let ghosted =
                        DevicePresetLibrary::handle_disconnect(&self.control, fingerprint);
                    notifications.push(HotplugNotification {
                        event: ev,
                        recognized_preset_name: None,
                        auto_rebound_device_id: None,
                        ghosted_device_ids: ghosted,
                    });
                }
                HardwareHotplugEvent::Connected { info, .. } => {
                    let lib = self.preset_library.lock();
                    let matched_preset = lib.find_best_match(info).cloned();
                    let auto_rebound = lib.handle_reconnect(&self.control, info)?;
                    drop(lib);

                    // If we have both a recognized preset and a rebound (or matching online) node, re-arm synthesizer!
                    if let (Some(preset), Some(dev_id)) = (&matched_preset, auto_rebound) {
                        if let Ok(synth) = HidParameterSynthesizer::compile(&preset.controls) {
                            self.active_hid_bindings.lock().insert(
                                info.path.clone(),
                                ActiveHidBinding {
                                    device_id: dev_id,
                                    definition: preset.clone(),
                                    synthesizer: synth,
                                },
                            );
                        }
                    }

                    notifications.push(HotplugNotification {
                        event: ev,
                        recognized_preset_name: matched_preset.map(|p| p.name),
                        auto_rebound_device_id: auto_rebound,
                        ghosted_device_ids: Vec::new(),
                    });
                }
            }
        }

        Ok(notifications)
    }

    /// Plugs in a simulated HID device and immediately polls hot-plug notifications.
    pub fn simulate_plug_in(
        &self,
        profile: SimulatedHidProfile,
        stable_id: Option<&str>,
    ) -> Result<(HidDeviceInfo, Vec<HotplugNotification>), HardwareError> {
        let info = match stable_id {
            Some(id) => self.simulated_hid.plug_in_with_id(profile, id),
            None => self.simulated_hid.plug_in(profile),
        };
        let notifications = self.poll_hotplug()?;
        Ok((info, notifications))
    }

    /// Unplugs a simulated HID device and immediately polls hot-plug notifications.
    pub fn simulate_unplug(&self, path: &str) -> Result<Vec<HotplugNotification>, HardwareError> {
        self.simulated_hid.unplug(path);
        self.poll_hotplug()
    }

    /// Actuates a virtual control on a simulated HID device and optionally polls 1 live report.
    pub fn simulate_actuate(
        &self,
        path: &str,
        action: SimulatedHidAction,
    ) -> Result<Vec<RoutedOutput>, HardwareError> {
        self.simulated_hid.actuate(path, action)?;
        self.poll_bound_hid_report(path)
    }

    /// Reads one HID report for a bound device, synthesizes normalized/modified `kf-core`
    /// parameters, and routes them through `SignalRouter`.
    pub fn poll_bound_hid_report(&self, path: &str) -> Result<Vec<RoutedOutput>, HardwareError> {
        let Some(sample) = self.hid_backend.read_report(path)? else {
            return Ok(Vec::new());
        };

        let mut bindings = self.active_hid_bindings.lock();
        let Some(binding) = bindings.get_mut(path) else {
            return Ok(Vec::new());
        };

        let signals = binding.synthesizer.process_report(&sample);
        let device_id = binding.device_id;
        drop(bindings);

        let snap = self.control.snapshot();
        let Some(dev) = snap.devices.iter().find(|d| d.id == device_id) else {
            return Ok(Vec::new());
        };

        let mut routed = Vec::new();
        for (logical_key, value) in signals {
            if let Some(port) = dev.find_port_by_key(&logical_key) {
                let outs = self
                    .router
                    .route(SignalEvent::new(port.id, value, sample.timestamp_ns));
                routed.extend(outs);
            }
        }
        Ok(routed)
    }

    // ---------- Tier 1 Descriptor Parsing ----------

    pub fn parse_device_descriptor(
        &self,
        path: &str,
    ) -> Result<Option<ParsedHidDescriptor>, HardwareError> {
        let Some(raw) = self.hid_backend.report_descriptor(path)? else {
            return Ok(None);
        };
        let parsed = parse_hid_report_descriptor(&raw)
            .map_err(|e| HardwareError::Backend(format!("Descriptor parse failed: {e}")))?;
        Ok(Some(parsed))
    }

    // ---------- Tier 2 Interactive 4-Step Learn Wizard ----------

    pub fn learn_start_baseline(&self, device_path: &str) -> Result<LearnWizardSnapshot, HardwareError> {
        let mut engine = self.learn_engine.lock();
        engine.start_baseline(device_path)?;

        // Tier 1 pre-population if device exposes a standard HID report descriptor
        if let Ok(Some(raw_desc)) = self.hid_backend.report_descriptor(device_path) {
            if let Ok(parsed) = parse_hid_report_descriptor(&raw_desc) {
                let mappings = parsed.to_learned_mappings();
                engine.seed_from_descriptor_mappings(&mappings);
            }
        }

        Ok(engine.snapshot())
    }

    /// Reads `sample_count` packets from the active Learn device and feeds them into the Learn Engine.
    pub fn learn_poll_samples(&self, sample_count: usize) -> Result<LearnWizardSnapshot, HardwareError> {
        let count = sample_count.clamp(1, 128);
        let path = {
            let engine = self.learn_engine.lock();
            engine
                .device_path()
                .ok_or_else(|| HardwareError::Backend("No active Learn session device".into()))?
                .to_string()
        };

        let mut samples = Vec::with_capacity(count);
        for _ in 0..count {
            if let Some(s) = self.hid_backend.read_report(&path)? {
                samples.push(s);
            }
        }

        let mut engine = self.learn_engine.lock();
        for s in &samples {
            engine.ingest_report(s)?;
        }
        Ok(engine.snapshot())
    }

    pub fn learn_start_capture(&self) -> Result<LearnWizardSnapshot, HardwareError> {
        let mut engine = self.learn_engine.lock();
        engine.start_capture()?;
        Ok(engine.snapshot())
    }

    pub fn learn_classify(
        &self,
        alias_hint: Option<&str>,
    ) -> Result<LearnWizardSnapshot, HardwareError> {
        let mut engine = self.learn_engine.lock();
        engine.classify_captured_control(alias_hint)?;
        Ok(engine.snapshot())
    }

    pub fn learn_commit_control(
        &self,
        alias: &str,
        modifiers: Vec<Modifier>,
        sensitivity: Option<f64>,
        wrap: Option<bool>,
    ) -> Result<LearnWizardSnapshot, HardwareError> {
        let mut engine = self.learn_engine.lock();
        engine.commit_candidate(alias, modifiers, sensitivity, wrap)?;
        Ok(engine.snapshot())
    }

    pub fn learn_snapshot(&self) -> LearnWizardSnapshot {
        self.learn_engine.lock().snapshot()
    }

    pub fn learn_reset(&self) -> LearnWizardSnapshot {
        let mut engine = self.learn_engine.lock();
        engine.reset();
        engine.snapshot()
    }

    /// Saves the currently learned controls as a portable `.kfdevice.json` preset,
    /// registers it in the preset library, attaches/updates the `HidDevice` node in the workspace graph,
    /// and arms the live parameter synthesizer for that device path.
    pub fn save_and_attach_learned_device(
        &self,
        preset_name: &str,
        x: f64,
        y: f64,
    ) -> Result<(HidDeviceDefinition, String, Device), HardwareError> {
        let (path, controls, noise_masks) = {
            let engine = self.learn_engine.lock();
            let path = engine
                .device_path()
                .ok_or_else(|| HardwareError::Backend("No device in Learn session".into()))?
                .to_string();
            let controls: Vec<LearnedControlSpec> = engine.committed_controls().to_vec();
            let masks: HashMap<u8, Vec<u8>> = engine
                .baselines()
                .iter()
                .map(|(&rid, b)| (rid, b.noise_mask_bits.clone()))
                .collect();
            (path, controls, masks)
        };

        let info = self
            .hid_backend
            .enumerate()?
            .into_iter()
            .find(|d| d.path == path)
            .ok_or_else(|| HardwareError::Backend(format!("HID device not found: {path}")))?;

        let def = HidDeviceDefinition::from_info(preset_name, &info, controls, noise_masks);
        let json = def.to_kfdevice_json()?;

        self.preset_library.lock().upsert(def.clone());

        let mut graph_dev = def.to_graph_device(x, y);
        graph_dev.fingerprint = Some(fingerprint_from_hid_info(&info));
        let added = self
            .control
            .add_device(graph_dev)
            .map_err(|e| HardwareError::Backend(format!("Failed to add HID node: {e}")))?;

        let synth = HidParameterSynthesizer::compile(&def.controls)?;
        self.active_hid_bindings.lock().insert(
            path,
            ActiveHidBinding {
                device_id: added.id,
                definition: def.clone(),
                synthesizer: synth,
            },
        );

        Ok((def, json, added))
    }

    pub fn import_hid_device_preset(&self, json: &str) -> Result<HidDeviceDefinition, HardwareError> {
        let def = HidDeviceDefinition::from_kfdevice_json(json)?;
        self.preset_library.lock().upsert(def.clone());
        Ok(def)
    }

    pub fn list_device_presets(&self) -> Vec<HidDeviceDefinition> {
        self.preset_library.lock().list().to_vec()
    }

    pub fn bind_existing_device_to_path(
        &self,
        path: &str,
        device_id: Uuid,
        definition: HidDeviceDefinition,
    ) -> Result<(), HardwareError> {
        let synth = HidParameterSynthesizer::compile(&definition.controls)?;
        self.active_hid_bindings.lock().insert(
            path.to_string(),
            ActiveHidBinding {
                device_id,
                definition,
                synthesizer: synth,
            },
        );
        Ok(())
    }

    pub fn active_binding_definition(&self, path: &str) -> Option<HidDeviceDefinition> {
        self.active_hid_bindings
            .lock()
            .get(path)
            .map(|b| b.definition.clone())
    }

    // ---------- RGB Camera Ingestion & CV Parameter Extractors ----------

    pub fn list_rgb_cameras(&self) -> Result<Vec<RgbCameraInfo>, HardwareError> {
        #[allow(unused_mut)]
        let mut cams = self.synthetic_camera.list_cameras()?;
        #[cfg(all(
            feature = "native-camera",
            any(target_os = "windows", target_os = "macos", target_os = "linux")
        ))]
        {
            if let Ok(native) = crate::camera::NokhwaCameraBackend.list_cameras() {
                cams.extend(native);
            }
        }
        Ok(cams)
    }

    pub fn update_rgb_cv_config(&self, camera_id: &str, config: RgbCvConfig) {
        let mut map = self.cv_pipelines.lock();
        map.entry(camera_id.to_string())
            .and_modify(|p| p.set_config(config.clone()))
            .or_insert_with(|| RgbCvPipeline::new(config));
    }

    pub fn get_rgb_cv_config(&self, camera_id: &str) -> RgbCvConfig {
        self.cv_pipelines
            .lock()
            .get(camera_id)
            .map(|p| p.config().clone())
            .unwrap_or_default()
    }

    pub fn set_synthetic_scene(&self, params: SyntheticSceneParams) {
        self.synthetic_camera.set_params(params);
    }

    /// Captures one RGB frame, runs the CPU CV extractor pipeline, and routes every matching
    /// parameter port on `target_device_id` (if provided) into `kf_core::SignalRouter`.
    pub fn step_rgb_camera_cv(
        &self,
        camera_id: &str,
        target_device_id: Option<Uuid>,
    ) -> Result<CameraCvStepOutput, HardwareError> {
        let frame = self.synthetic_camera.capture_frame(camera_id)?;
        let (cv, signals) = {
            let mut map = self.cv_pipelines.lock();
            let pipe = map
                .entry(camera_id.to_string())
                .or_insert_with(|| RgbCvPipeline::new(RgbCvConfig::default()));
            pipe.process_frame(&frame.rgba, frame.width, frame.height, frame.timestamp_ns)
        };

        let mut routed_count = 0usize;
        if let Some(dev_id) = target_device_id {
            let snap = self.control.snapshot();
            if let Some(dev) = snap.devices.iter().find(|d| d.id == dev_id) {
                for (key, val) in signals {
                    if let Some(port) = dev.find_port_by_key(&key) {
                        let outs = self.router.route(SignalEvent::new(
                            port.id,
                            val,
                            frame.timestamp_ns,
                        ));
                        routed_count += outs.len();
                    }
                }
            }
        }

        Ok(CameraCvStepOutput {
            camera_id: camera_id.to_string(),
            frame_index: frame.frame_index,
            width: frame.width,
            height: frame.height,
            cv,
            routed_count,
        })
    }

    /// Helper to route a single raw `SignalValue` to a device's port by logical key.
    pub fn emit_device_signal(
        &self,
        device_id: Uuid,
        logical_key: &str,
        value: SignalValue,
    ) -> usize {
        let snap = self.control.snapshot();
        let Some(dev) = snap.devices.iter().find(|d| d.id == device_id) else {
            return 0;
        };
        let Some(port) = dev.find_port_by_key(logical_key) else {
            return 0;
        };
        self.router
            .route(SignalEvent::new(port.id, value, SignalEvent::now_ns()))
            .len()
    }
}
