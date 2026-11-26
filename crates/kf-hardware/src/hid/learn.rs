//! Tier 2: Interactive Baseline Noise-Masking + Bit/Byte Diff HID Learn Engine,
//! Hackable Parameter Synthesizer (with full `kf-core` modifier chain), and
//! Portable `.kfdevice.json` Device Definition Library with Auto-Rebind.

use crate::hid::backend::fingerprint_from_hid_info;
use crate::{HardwareError, HidLearnEngine, HidReportSample};
use kf_core::{
    CompiledModifierChain, ControlPlane, Device, DeviceFingerprint, DeviceKind, DevicePreset,
    DeviceStatus, HidControlClass, HidDeviceInfo, HidLearnStep, LearnedHidMapping, Modifier,
    ModifierChainState, Port, PortDirection, SignalKind, SignalValue, CURRENT_SCHEMA_VERSION,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// Fine-grained auto-classified HID control type inferred by the Tier 2 Learn Engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HidControlType {
    Button,
    Toggle,
    Axis8,
    Axis16,
    RelativeEncoder,
    HatSwitch,
}

impl HidControlType {
    pub fn to_core_class(self) -> HidControlClass {
        match self {
            HidControlType::Button => HidControlClass::Button,
            HidControlType::Toggle => HidControlClass::Toggle,
            HidControlType::Axis8 | HidControlType::Axis16 | HidControlType::HatSwitch => {
                HidControlClass::AbsoluteAxis
            }
            HidControlType::RelativeEncoder => HidControlClass::RelativeEncoder,
        }
    }

    pub fn from_core_class(cls: HidControlClass, bit_width: u8) -> Self {
        match cls {
            HidControlClass::Button => HidControlType::Button,
            HidControlClass::Toggle => HidControlType::Toggle,
            HidControlClass::RelativeEncoder => HidControlType::RelativeEncoder,
            HidControlClass::AbsoluteAxis => {
                if bit_width > 8 {
                    HidControlType::Axis16
                } else {
                    HidControlType::Axis8
                }
            }
        }
    }
}

/// Complete learned control specification including noise-masked offset/width,
/// auto-inferred type, observed min/max, encoder sensitivity, and hackable modifier stack.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LearnedControlSpec {
    pub logical_key: String,
    pub alias: String,
    pub report_id: u8,
    pub byte_offset: usize,
    pub bit_offset: usize,
    pub bit_mask: u8,
    pub bit_width: u8,
    pub signed: bool,
    pub little_endian: bool,
    pub control_type: HidControlType,
    pub observed_min: i64,
    pub observed_max: i64,
    #[serde(default = "default_sensitivity")]
    pub sensitivity: f64,
    #[serde(default)]
    pub wrap: bool,
    #[serde(default)]
    pub modifiers: Vec<Modifier>,
}

fn default_sensitivity() -> f64 {
    0.02
}

impl LearnedControlSpec {
    pub fn to_core_mapping(&self) -> LearnedHidMapping {
        LearnedHidMapping {
            logical_key: self.logical_key.clone(),
            alias: self.alias.clone(),
            report_id: self.report_id,
            byte_offset: self.byte_offset,
            bit_mask: self.bit_mask,
            bit_width: self.bit_width,
            signed: self.signed,
            little_endian: self.little_endian,
            control_class: self.control_type.to_core_class(),
            observed_min: self.observed_min,
            observed_max: self.observed_max,
        }
    }

    pub fn from_core_mapping(m: &LearnedHidMapping) -> Self {
        let trailing = if m.bit_mask == 0 {
            0
        } else {
            m.bit_mask.trailing_zeros() as usize
        };
        Self {
            logical_key: m.logical_key.clone(),
            alias: m.alias.clone(),
            report_id: m.report_id,
            byte_offset: m.byte_offset,
            bit_offset: m.byte_offset * 8 + trailing,
            bit_mask: m.bit_mask,
            bit_width: m.bit_width,
            signed: m.signed,
            little_endian: m.little_endian,
            control_type: HidControlType::from_core_class(m.control_class, m.bit_width),
            observed_min: m.observed_min,
            observed_max: m.observed_max,
            sensitivity: default_sensitivity(),
            wrap: false,
            modifiers: Vec::new(),
        }
    }

    /// Extracts the raw integer value for this control from a HID report packet.
    pub fn extract_raw(&self, report_bytes: &[u8]) -> Option<i64> {
        match self.control_type {
            HidControlType::Axis16 => {
                if self.byte_offset + 1 >= report_bytes.len() {
                    return None;
                }
                let pair = [report_bytes[self.byte_offset], report_bytes[self.byte_offset + 1]];
                let val = match (self.signed, self.little_endian) {
                    (false, true) => u16::from_le_bytes(pair) as i64,
                    (false, false) => u16::from_be_bytes(pair) as i64,
                    (true, true) => i16::from_le_bytes(pair) as i64,
                    (true, false) => i16::from_be_bytes(pair) as i64,
                };
                Some(val)
            }
            HidControlType::RelativeEncoder => {
                let b = *report_bytes.get(self.byte_offset)?;
                Some((b as i8) as i64)
            }
            HidControlType::Axis8 => {
                let b = *report_bytes.get(self.byte_offset)?;
                let val = if self.signed {
                    (b as i8) as i64
                } else {
                    b as i64
                };
                Some(val)
            }
            HidControlType::HatSwitch => {
                let b = *report_bytes.get(self.byte_offset)?;
                let shift = self.bit_mask.trailing_zeros();
                let nibble = (b & self.bit_mask) >> shift;
                Some(nibble as i64)
            }
            HidControlType::Button | HidControlType::Toggle => {
                let b = *report_bytes.get(self.byte_offset)?;
                let active = (b & self.bit_mask) != 0;
                Some(if active { 1 } else { 0 })
            }
        }
    }
}

/// Per-Report-ID baseline snapshot & automatic noise mask computed in Step 1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BaselineProfile {
    pub report_id: u8,
    pub sample_count: usize,
    pub resting_bytes: Vec<u8>,
    /// Bitmask of bits that fluctuated while untouched (1 = noise, ignore during diff).
    pub noise_mask_bits: Vec<u8>,
    /// Per-byte change frequency (0.0..=1.0) during idle baseline sampling.
    pub byte_noise_ratio: Vec<f32>,
}

/// Summary of the Learn Wizard's current state, suitable for streaming to the UI canvas strip.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LearnWizardSnapshot {
    pub step: HidLearnStep,
    pub device_path: Option<String>,
    pub active_report_id: Option<u8>,
    pub baseline_samples: usize,
    pub capture_samples: usize,
    pub resting_bytes: Vec<u8>,
    pub latest_bytes: Vec<u8>,
    pub noise_mask_bits: Vec<u8>,
    pub active_diff_bits: Vec<u8>,
    pub candidate: Option<LearnedControlSpec>,
    pub committed_controls: Vec<LearnedControlSpec>,
}

/// Interactive 2-Tier HID Learn Engine supporting Baseline Noise-Masking and Bit/Byte Diff Classification.
#[derive(Debug, Clone)]
pub struct TwoTierHidLearnEngine {
    step: HidLearnStep,
    device_path: Option<String>,
    baseline_buffers: HashMap<u8, Vec<Vec<u8>>>,
    baselines: HashMap<u8, BaselineProfile>,
    capture_buffers: HashMap<u8, Vec<Vec<u8>>>,
    active_report_id: Option<u8>,
    latest_bytes: Vec<u8>,
    active_diff_bits: Vec<u8>,
    candidate: Option<LearnedControlSpec>,
    committed: Vec<LearnedControlSpec>,
}

impl Default for TwoTierHidLearnEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl TwoTierHidLearnEngine {
    pub fn new() -> Self {
        Self {
            step: HidLearnStep::Idle,
            device_path: None,
            baseline_buffers: HashMap::new(),
            baselines: HashMap::new(),
            capture_buffers: HashMap::new(),
            active_report_id: None,
            latest_bytes: Vec::new(),
            active_diff_bits: Vec::new(),
            candidate: None,
            committed: Vec::new(),
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn device_path(&self) -> Option<&str> {
        self.device_path.as_deref()
    }

    pub fn baselines(&self) -> &HashMap<u8, BaselineProfile> {
        &self.baselines
    }

    pub fn committed_controls(&self) -> &[LearnedControlSpec] {
        &self.committed
    }

    /// Pre-populates controls discovered from Tier 1 HID Report Descriptor parsing.
    pub fn seed_from_descriptor_mappings(&mut self, mappings: &[LearnedHidMapping]) {
        for m in mappings {
            if !self.committed.iter().any(|c| c.logical_key == m.logical_key) {
                self.committed.push(LearnedControlSpec::from_core_mapping(m));
            }
        }
    }

    /// Step 1 -> Step 2 transition: finalizes baseline noise masks and starts recording one control.
    pub fn start_capture(&mut self) -> Result<BaselineProfile, HardwareError> {
        if self.baseline_buffers.is_empty() && self.baselines.is_empty() {
            return Err(HardwareError::Backend(
                "No baseline samples recorded yet. Sample idle packets first.".into(),
            ));
        }

        // Compute noise masks for every report_id seen during baseline
        for (&report_id, samples) in &self.baseline_buffers {
            if samples.is_empty() {
                continue;
            }
            let profile = compute_baseline_profile(report_id, samples);
            self.baselines.insert(report_id, profile);
        }

        self.capture_buffers.clear();
        self.candidate = None;
        self.active_diff_bits.clear();
        self.step = HidLearnStep::RecordingControl;

        let primary_id = self
            .active_report_id
            .or_else(|| self.baselines.keys().copied().next())
            .ok_or_else(|| HardwareError::Backend("Baseline profile unavailable".into()))?;
        self.active_report_id = Some(primary_id);

        Ok(self.baselines[&primary_id].clone())
    }

    /// Analyzes recorded capture packets against the noise-masked baseline and classifies the control.
    pub fn classify_captured_control(
        &mut self,
        alias_hint: Option<&str>,
    ) -> Result<LearnedControlSpec, HardwareError> {
        // Pick the report_id with the strongest unmasked activity
        let mut best_report_id = None;
        let mut best_activity = 0usize;

        for (&rid, samples) in &self.capture_buffers {
            let Some(base) = self.baselines.get(&rid) else {
                continue;
            };
            let diff = compute_unmasked_diff_bits(base, samples);
            let active_bits: usize = diff.iter().map(|b| b.count_ones() as usize).sum();
            if active_bits > best_activity {
                best_activity = active_bits;
                best_report_id = Some(rid);
            }
        }

        let report_id = best_report_id.ok_or_else(|| {
            HardwareError::Backend(
                "No non-noise bit activity detected during capture. Move a control further and retry."
                    .into(),
            )
        })?;

        let base = &self.baselines[&report_id];
        let samples = &self.capture_buffers[&report_id];
        self.active_report_id = Some(report_id);
        self.active_diff_bits = compute_unmasked_diff_bits(base, samples);

        let control_idx = self.committed.len();
        let mut spec = classify_diff(report_id, base, samples, &self.active_diff_bits, control_idx)?;
        if let Some(alias) = alias_hint {
            let trimmed = alias.trim();
            if !trimmed.is_empty() {
                spec.alias = trimmed.to_string();
            }
        }

        self.candidate = Some(spec.clone());
        self.step = HidLearnStep::ReviewAndTune;
        Ok(spec)
    }

    /// Step 3: Commits the reviewed/tuned control (with custom alias + modifier chain) into the device map.
    pub fn commit_candidate(
        &mut self,
        alias: &str,
        modifiers: Vec<Modifier>,
        sensitivity: Option<f64>,
        wrap: Option<bool>,
    ) -> Result<LearnedControlSpec, HardwareError> {
        let mut spec = match self.candidate.clone() {
            Some(c) => c,
            None => self.classify_captured_control(Some(alias))?,
        };
        if !alias.trim().is_empty() {
            spec.alias = alias.trim().to_string();
            let slug = spec
                .alias
                .to_lowercase()
                .replace(|c: char| !c.is_ascii_alphanumeric(), "_");
            spec.logical_key = format!("hid.r{}.b{}.{}", spec.report_id, spec.byte_offset, slug);
        }
        spec.modifiers = modifiers;
        if let Some(s) = sensitivity {
            spec.sensitivity = s.clamp(0.001, 0.5);
        }
        if let Some(w) = wrap {
            spec.wrap = w;
        }

        // Replace if same byte_offset + bit_mask + report_id already exists
        if let Some(pos) = self.committed.iter().position(|c| {
            c.report_id == spec.report_id
                && c.byte_offset == spec.byte_offset
                && c.bit_mask == spec.bit_mask
        }) {
            self.committed[pos] = spec.clone();
        } else {
            self.committed.push(spec.clone());
        }

        self.candidate = Some(spec.clone());
        self.step = HidLearnStep::Completed;
        Ok(spec)
    }

    /// Returns a UI-friendly snapshot of the current Learn Wizard state.
    pub fn snapshot(&self) -> LearnWizardSnapshot {
        let rid = self.active_report_id;
        let base = rid.and_then(|id| self.baselines.get(&id));
        let live_base = if base.is_none() {
            rid.and_then(|id| self.baseline_buffers.get(&id))
                .filter(|s| !s.is_empty())
                .map(|s| compute_baseline_profile(rid.unwrap_or(0), s))
        } else {
            None
        };
        let active_base = base.or(live_base.as_ref());

        let baseline_samples = rid
            .and_then(|id| self.baseline_buffers.get(&id))
            .map(|v| v.len())
            .unwrap_or_else(|| active_base.map(|b| b.sample_count).unwrap_or(0));
        let capture_samples = rid
            .and_then(|id| self.capture_buffers.get(&id))
            .map(|v| v.len())
            .unwrap_or(0);

        LearnWizardSnapshot {
            step: self.step,
            device_path: self.device_path.clone(),
            active_report_id: rid,
            baseline_samples,
            capture_samples,
            resting_bytes: active_base
                .map(|b| b.resting_bytes.clone())
                .unwrap_or_default(),
            latest_bytes: self.latest_bytes.clone(),
            noise_mask_bits: active_base
                .map(|b| b.noise_mask_bits.clone())
                .unwrap_or_default(),
            active_diff_bits: self.active_diff_bits.clone(),
            candidate: self.candidate.clone(),
            committed_controls: self.committed.clone(),
        }
    }
}

impl HidLearnEngine for TwoTierHidLearnEngine {
    fn step(&self) -> HidLearnStep {
        self.step
    }

    fn start_baseline(&mut self, device_path: &str) -> Result<(), HardwareError> {
        self.device_path = Some(device_path.to_string());
        self.baseline_buffers.clear();
        self.baselines.clear();
        self.capture_buffers.clear();
        self.active_diff_bits.clear();
        self.candidate = None;
        self.step = HidLearnStep::CapturingBaseline;
        Ok(())
    }

    fn ingest_report(&mut self, sample: &HidReportSample) -> Result<(), HardwareError> {
        if sample.bytes.is_empty() {
            return Ok(());
        }
        self.active_report_id = Some(sample.report_id);
        self.latest_bytes = sample.bytes.clone();

        match self.step {
            HidLearnStep::CapturingBaseline => {
                self.baseline_buffers
                    .entry(sample.report_id)
                    .or_default()
                    .push(sample.bytes.clone());
            }
            HidLearnStep::RecordingControl => {
                let buf = self.capture_buffers.entry(sample.report_id).or_default();
                buf.push(sample.bytes.clone());
                if let Some(base) = self.baselines.get(&sample.report_id) {
                    self.active_diff_bits = compute_unmasked_diff_bits(base, buf);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn finalize_control(&mut self, alias: &str) -> Result<LearnedHidMapping, HardwareError> {
        let spec = self.commit_candidate(alias, Vec::new(), None, None)?;
        Ok(spec.to_core_mapping())
    }
}

/// Computes the resting bytes and automatic per-bit/per-byte noise mask from idle samples.
pub fn compute_baseline_profile(report_id: u8, samples: &[Vec<u8>]) -> BaselineProfile {
    let len = samples.iter().map(|s| s.len()).max().unwrap_or(0);
    let mut resting_bytes = vec![0u8; len];
    let mut noise_mask_bits = vec![0u8; len];
    let mut byte_noise_ratio = vec![0.0f32; len];

    if samples.is_empty() || len == 0 {
        return BaselineProfile {
            report_id,
            sample_count: 0,
            resting_bytes,
            noise_mask_bits,
            byte_noise_ratio,
        };
    }

    for i in 0..len {
        let first = samples[0].get(i).copied().unwrap_or(0);
        let mut flipped_bits = 0u8;
        let mut transitions = 0usize;
        let mut distinct = HashSet::new();
        let mut freq: HashMap<u8, usize> = HashMap::new();
        let mut prev = first;

        for s in samples {
            let b = s.get(i).copied().unwrap_or(0);
            flipped_bits |= b ^ first;
            if b != prev {
                transitions += 1;
                prev = b;
            }
            distinct.insert(b);
            *freq.entry(b).or_insert(0) += 1;
        }

        // Resting value is the mode (most frequent byte during idle)
        resting_bytes[i] = freq
            .into_iter()
            .max_by_key(|&(_, count)| count)
            .map(|(val, _)| val)
            .unwrap_or(first);

        let ratio = if samples.len() > 1 {
            transitions as f32 / (samples.len() - 1) as f32
        } else {
            0.0
        };
        byte_noise_ratio[i] = ratio;

        // If a byte exhibits continuous IMU/counter jitter (>= 3 distinct values or >= 20% transitions
        // or >= 3 flickering bits), mask the entire byte so higher-bit rollovers during capture never
        // cause false detections. Otherwise mask only the exact bits that flickered.
        if distinct.len() >= 3 || ratio >= 0.2 || flipped_bits.count_ones() >= 3 {
            noise_mask_bits[i] = 0xFF;
        } else {
            noise_mask_bits[i] = flipped_bits;
        }
    }

    BaselineProfile {
        report_id,
        sample_count: samples.len(),
        resting_bytes,
        noise_mask_bits,
        byte_noise_ratio,
    }
}

fn compute_unmasked_diff_bits(base: &BaselineProfile, samples: &[Vec<u8>]) -> Vec<u8> {
    let len = base
        .resting_bytes
        .len()
        .max(samples.iter().map(|s| s.len()).max().unwrap_or(0));
    let mut diff = vec![0u8; len];
    for s in samples {
        for (i, &b) in s.iter().enumerate() {
            let rest = base.resting_bytes.get(i).copied().unwrap_or(0);
            let mask = base.noise_mask_bits.get(i).copied().unwrap_or(0);
            diff[i] |= (b ^ rest) & !mask;
        }
    }
    diff
}

#[derive(Debug, Clone, Copy)]
struct SmoothnessScore {
    signed: bool,
    little_endian: bool,
    min: i64,
    max: i64,
    score: f64,
}

fn score_16bit_interpretation(
    samples: &[Vec<u8>],
    offset: usize,
    signed: bool,
    little_endian: bool,
) -> SmoothnessScore {
    let vals: Vec<i64> = samples
        .iter()
        .filter_map(|s| {
            if offset + 1 >= s.len() {
                return None;
            }
            let pair = [s[offset], s[offset + 1]];
            let v = match (signed, little_endian) {
                (false, true) => u16::from_le_bytes(pair) as i64,
                (false, false) => u16::from_be_bytes(pair) as i64,
                (true, true) => i16::from_le_bytes(pair) as i64,
                (true, false) => i16::from_be_bytes(pair) as i64,
            };
            Some(v)
        })
        .collect();

    if vals.len() < 2 {
        return SmoothnessScore {
            signed,
            little_endian,
            min: vals.first().copied().unwrap_or(0),
            max: vals.first().copied().unwrap_or(0),
            score: f64::INFINITY,
        };
    }

    let min = *vals.iter().min().unwrap();
    let max = *vals.iter().max().unwrap();
    let span = (max - min).max(1) as f64;
    let mut total_tv = 0.0f64;
    let mut max_step = 0.0f64;
    for w in vals.windows(2) {
        let step = (w[1] - w[0]).abs() as f64;
        total_tv += step;
        if step > max_step {
            max_step = step;
        }
    }

    // Lower score = smoother, physically coherent trajectory.
    // Wrong endianness or wrap across signed/unsigned boundary produces massive max_step and TV/span.
    let tv_ratio = total_tv / span;
    let max_step_ratio = max_step / span;
    // Slight preference for unsigned when neither wrap boundary is crossed and values are non-negative.
    let signed_tiebreak = if signed { 0.005 } else { 0.0 };
    let score = tv_ratio + max_step_ratio * 3.0 + signed_tiebreak;

    SmoothnessScore {
        signed,
        little_endian,
        min,
        max,
        score,
    }
}

fn score_8bit_signedness(samples: &[Vec<u8>], offset: usize) -> (bool, i64, i64) {
    let eval = |signed: bool| -> (f64, i64, i64) {
        let vals: Vec<i64> = samples
            .iter()
            .filter_map(|s| s.get(offset).copied())
            .map(|b| if signed { (b as i8) as i64 } else { b as i64 })
            .collect();
        let min = vals.iter().copied().min().unwrap_or(0);
        let max = vals.iter().copied().max().unwrap_or(0);
        let span = (max - min).max(1) as f64;
        let mut tv = 0.0;
        let mut max_step = 0.0;
        for w in vals.windows(2) {
            let d = (w[1] - w[0]).abs() as f64;
            tv += d;
            if d > max_step {
                max_step = d;
            }
        }
        let tie = if signed { 0.005 } else { 0.0 };
        (tv / span + (max_step / span) * 3.0 + tie, min, max)
    };

    let (u_score, u_min, u_max) = eval(false);
    let (s_score, s_min, s_max) = eval(true);
    if s_score < u_score {
        (true, s_min, s_max)
    } else {
        (false, u_min, u_max)
    }
}

fn classify_diff(
    report_id: u8,
    base: &BaselineProfile,
    samples: &[Vec<u8>],
    diff_bits: &[u8],
    control_idx: usize,
) -> Result<LearnedControlSpec, HardwareError> {
    // Collect active bytes (skip byte 0 if it equals report_id and has no diff)
    let mut active_bytes: Vec<(usize, u8, usize)> = Vec::new();
    for (i, &bits) in diff_bits.iter().enumerate() {
        if bits == 0 {
            continue;
        }
        let mask = !base.noise_mask_bits.get(i).copied().unwrap_or(0);
        let distinct_count = samples
            .iter()
            .filter_map(|s| s.get(i).copied())
            .map(|b| b & mask)
            .collect::<HashSet<u8>>()
            .len();
        active_bytes.push((i, bits, distinct_count));
    }

    if active_bytes.is_empty() {
        return Err(HardwareError::Backend(
            "No active unmasked bytes found during capture".into(),
        ));
    }

    // Check for an adjacent 2-byte 16-bit axis pair first!
    for win in active_bytes.windows(2) {
        let (idx_a, bits_a, dist_a) = win[0];
        let (idx_b, bits_b, dist_b) = win[1];
        if idx_b == idx_a + 1
            && (bits_a.count_ones() + bits_b.count_ones()) >= 3
            && (dist_a >= 2 && dist_b >= 2)
        {
            // Verify both bytes are clean (not noise-masked)
            let mask_a = base.noise_mask_bits.get(idx_a).copied().unwrap_or(0);
            let mask_b = base.noise_mask_bits.get(idx_b).copied().unwrap_or(0);
            if mask_a == 0 && mask_b == 0 {
                let candidates = [
                    score_16bit_interpretation(samples, idx_a, false, true),
                    score_16bit_interpretation(samples, idx_a, true, true),
                    score_16bit_interpretation(samples, idx_a, false, false),
                    score_16bit_interpretation(samples, idx_a, true, false),
                ];
                let best = candidates
                    .into_iter()
                    .min_by(|a, b| a.score.total_cmp(&b.score))
                    .unwrap();

                let endian_label = if best.little_endian { "LE" } else { "BE" };
                let sign_label = if best.signed { "i16" } else { "u16" };
                return Ok(LearnedControlSpec {
                    logical_key: format!("hid.r{report_id}.b{idx_a}.axis16_{control_idx}"),
                    alias: format!("Axis 16-bit ({sign_label} {endian_label}) #{control_idx}"),
                    report_id,
                    byte_offset: idx_a,
                    bit_offset: idx_a * 8,
                    bit_mask: 0xFF,
                    bit_width: 16,
                    signed: best.signed,
                    little_endian: best.little_endian,
                    control_type: HidControlType::Axis16,
                    observed_min: best.min,
                    observed_max: best.max,
                    sensitivity: default_sensitivity(),
                    wrap: false,
                    modifiers: Vec::new(),
                });
            }
        }
    }

    // Otherwise pick the single byte with the most distinct values / active bits
    let (byte_idx, active_bits, _) = active_bytes
        .into_iter()
        .max_by_key(|&(_, bits, dist)| (dist, bits.count_ones() as usize))
        .unwrap();

    let rest = base.resting_bytes.get(byte_idx).copied().unwrap_or(0);
    let raw_seq: Vec<u8> = samples
        .iter()
        .filter_map(|s| s.get(byte_idx).copied())
        .collect();

    // 1. Single-bit Button or Toggle
    if active_bits.count_ones() == 1 {
        let bit_pos = active_bits.trailing_zeros() as usize;
        let rest_bit = (rest & active_bits) != 0;
        let last_bit = raw_seq.last().map(|&b| (b & active_bits) != 0).unwrap_or(rest_bit);
        let saw_return_to_rest = raw_seq
            .iter()
            .skip_while(|&&b| ((b & active_bits) != 0) == rest_bit)
            .any(|&b| ((b & active_bits) != 0) == rest_bit);

        let control_type = if !saw_return_to_rest && last_bit != rest_bit {
            HidControlType::Toggle
        } else {
            HidControlType::Button
        };

        return Ok(LearnedControlSpec {
            logical_key: format!("hid.r{report_id}.b{byte_idx}.bit{bit_pos}"),
            alias: match control_type {
                HidControlType::Toggle => format!("Toggle B{byte_idx}.{bit_pos}"),
                _ => format!("Button B{byte_idx}.{bit_pos}"),
            },
            report_id,
            byte_offset: byte_idx,
            bit_offset: byte_idx * 8 + bit_pos,
            bit_mask: active_bits,
            bit_width: 1,
            signed: false,
            little_endian: true,
            control_type,
            observed_min: 0,
            observed_max: 1,
            sensitivity: default_sensitivity(),
            wrap: false,
            modifiers: Vec::new(),
        });
    }

    // 2. 4-bit Hat Switch nibble (lower 0x0F or upper 0xF0)
    for nibble_mask in [0x0Fu8, 0xF0u8] {
        if (active_bits & !nibble_mask) == 0 {
            let shift = nibble_mask.trailing_zeros();
            let rest_nibble = (rest & nibble_mask) >> shift;
            let nibbles: HashSet<u8> = raw_seq.iter().map(|&b| (b & nibble_mask) >> shift).collect();
            let has_hat_directions = nibbles.iter().any(|&v| v <= 7);
            if rest_nibble >= 8 && has_hat_directions && nibbles.len() >= 2 && nibbles.len() <= 9 {
                return Ok(LearnedControlSpec {
                    logical_key: format!("hid.r{report_id}.b{byte_idx}.hat"),
                    alias: format!("Hat Switch B{byte_idx}"),
                    report_id,
                    byte_offset: byte_idx,
                    bit_offset: byte_idx * 8 + shift as usize,
                    bit_mask: nibble_mask,
                    bit_width: 4,
                    signed: false,
                    little_endian: true,
                    control_type: HidControlType::HatSwitch,
                    observed_min: 0,
                    observed_max: 7,
                    sensitivity: default_sensitivity(),
                    wrap: false,
                    modifiers: Vec::new(),
                });
            }
        }
    }

    // 3. Relative Encoder detection: rests at 0, emits small signed i8 deltas returning to 0
    if rest == 0 {
        let signed_vals: Vec<i8> = raw_seq.iter().map(|&b| b as i8).collect();
        let non_zero: Vec<i8> = signed_vals.iter().copied().filter(|&v| v != 0).collect();
        let max_abs = non_zero.iter().map(|&v| v.unsigned_abs()).max().unwrap_or(0);
        let zero_count = signed_vals.iter().filter(|&&v| v == 0).count();
        let returns_to_zero = signed_vals
            .windows(2)
            .filter(|w| w[0] != 0 && w[1] == 0)
            .count();

        if !non_zero.is_empty() && max_abs <= 16 && (returns_to_zero >= 1 || zero_count >= 2) {
            return Ok(LearnedControlSpec {
                logical_key: format!("hid.r{report_id}.b{byte_idx}.encoder"),
                alias: format!("Rotary Encoder B{byte_idx}"),
                report_id,
                byte_offset: byte_idx,
                bit_offset: byte_idx * 8,
                bit_mask: 0xFF,
                bit_width: 8,
                signed: true,
                little_endian: true,
                control_type: HidControlType::RelativeEncoder,
                observed_min: -127,
                observed_max: 127,
                sensitivity: default_sensitivity(),
                wrap: false,
                modifiers: Vec::new(),
            });
        }
    }

    // 4. Two discrete masked states -> Button / Toggle
    let distinct_masked: HashSet<u8> = raw_seq.iter().map(|&b| b & active_bits).collect();
    if distinct_masked.len() == 2 {
        let last_is_rest = raw_seq
            .last()
            .map(|&b| (b & active_bits) == (rest & active_bits))
            .unwrap_or(true);
        let control_type = if last_is_rest {
            HidControlType::Button
        } else {
            HidControlType::Toggle
        };
        return Ok(LearnedControlSpec {
            logical_key: format!("hid.r{report_id}.b{byte_idx}.btn_{control_idx}"),
            alias: format!("Switch B{byte_idx}"),
            report_id,
            byte_offset: byte_idx,
            bit_offset: byte_idx * 8 + active_bits.trailing_zeros() as usize,
            bit_mask: active_bits,
            bit_width: active_bits.count_ones() as u8,
            signed: false,
            little_endian: true,
            control_type,
            observed_min: 0,
            observed_max: 1,
            sensitivity: default_sensitivity(),
            wrap: false,
            modifiers: Vec::new(),
        });
    }

    // 5. Continuous 8-bit Axis (`Axis8`)
    let (signed, min, max) = score_8bit_signedness(samples, byte_idx);
    let sign_label = if signed { "i8" } else { "u8" };
    Ok(LearnedControlSpec {
        logical_key: format!("hid.r{report_id}.b{byte_idx}.axis8_{control_idx}"),
        alias: format!("Axis 8-bit ({sign_label}) #{control_idx}"),
        report_id,
        byte_offset: byte_idx,
        bit_offset: byte_idx * 8,
        bit_mask: 0xFF,
        bit_width: 8,
        signed,
        little_endian: true,
        control_type: HidControlType::Axis8,
        observed_min: min,
        observed_max: max,
        sensitivity: default_sensitivity(),
        wrap: false,
        modifiers: Vec::new(),
    })
}

/// Step 3: Live Hackable Parameter Synthesizer for a set of learned HID controls.
/// Normalizes raw HID packets and runs each control's full `kf_core::CompiledModifierChain`
/// (deadzone, invert, curve, 1-Euro smoothing, hysteresis, range map, math expression).
pub struct HidParameterSynthesizer {
    entries: Vec<ControlRuntimeEntry>,
}

struct ControlRuntimeEntry {
    spec: LearnedControlSpec,
    chain: CompiledModifierChain,
    state: ModifierChainState,
    encoder_accum: f64,
    last_value: Option<f64>,
}

impl HidParameterSynthesizer {
    pub fn compile(controls: &[LearnedControlSpec]) -> Result<Self, HardwareError> {
        let mut entries = Vec::with_capacity(controls.len());
        for spec in controls {
            let chain = CompiledModifierChain::compile(&spec.modifiers)
                .map_err(|e| HardwareError::Backend(format!("Modifier compile error: {e}")))?;
            entries.push(ControlRuntimeEntry {
                spec: spec.clone(),
                chain,
                state: ModifierChainState::default(),
                encoder_accum: 0.5,
                last_value: None,
            });
        }
        Ok(Self { entries })
    }

    /// Evaluates an incoming `HidReportSample` across all matching learned controls and returns
    /// `(logical_key, SignalValue)` pairs ready for `kf_core::SignalRouter`.
    pub fn process_report(&mut self, sample: &HidReportSample) -> Vec<(String, SignalValue)> {
        let mut out = Vec::new();
        for entry in &mut self.entries {
            if entry.spec.report_id != 0 && entry.spec.report_id != sample.report_id {
                continue;
            }
            let Some(raw) = entry.spec.extract_raw(&sample.bytes) else {
                continue;
            };

            let normalized = match entry.spec.control_type {
                HidControlType::Button | HidControlType::Toggle => {
                    if raw != 0 { 1.0 } else { 0.0 }
                }
                HidControlType::RelativeEncoder => {
                    let delta = raw as f64 * entry.spec.sensitivity;
                    let next = entry.encoder_accum + delta;
                    entry.encoder_accum = if entry.spec.wrap {
                        next.rem_euclid(1.0)
                    } else {
                        next.clamp(0.0, 1.0)
                    };
                    entry.encoder_accum
                }
                HidControlType::HatSwitch => {
                    if raw > 7 {
                        // Centered / null state -> keep last or 0.0
                        0.0
                    } else {
                        (raw as f64 / 7.0).clamp(0.0, 1.0)
                    }
                }
                HidControlType::Axis8 | HidControlType::Axis16 => {
                    let span = (entry.spec.observed_max - entry.spec.observed_min).max(1) as f64;
                    ((raw - entry.spec.observed_min) as f64 / span).clamp(0.0, 1.0)
                }
            };

            let base_signal = SignalValue::Float(normalized);
            let shaped = entry
                .chain
                .apply(base_signal, sample.timestamp_ns, &mut entry.state);

            if let Some(val) = shaped {
                entry.last_value = val.as_f64();
                out.push((entry.spec.logical_key.clone(), val));
            }
        }
        out
    }

    pub fn last_values(&self) -> HashMap<String, f64> {
        self.entries
            .iter()
            .filter_map(|e| e.last_value.map(|v| (e.spec.logical_key.clone(), v)))
            .collect()
    }
}

/// Step 4: Portable `.kfdevice.json` HID Device Definition keyed by VID/PID/UsagePage/Usage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HidDeviceDefinition {
    pub schema_version: u32,
    pub preset_id: Uuid,
    pub name: String,
    pub vendor_id: u16,
    pub product_id: u16,
    pub usage_page: u16,
    pub usage: u16,
    #[serde(default)]
    pub serial: Option<String>,
    #[serde(default)]
    pub noise_masks: HashMap<u8, Vec<u8>>,
    #[serde(default)]
    pub controls: Vec<LearnedControlSpec>,
}

impl HidDeviceDefinition {
    pub fn from_info(
        name: impl Into<String>,
        info: &HidDeviceInfo,
        controls: Vec<LearnedControlSpec>,
        noise_masks: HashMap<u8, Vec<u8>>,
    ) -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            preset_id: Uuid::new_v4(),
            name: name.into(),
            vendor_id: info.vid,
            product_id: info.pid,
            usage_page: info.usage_page,
            usage: info.usage,
            serial: info.serial.clone(),
            noise_masks,
            controls,
        }
    }

    pub fn fingerprint(&self) -> DeviceFingerprint {
        DeviceFingerprint {
            vid: Some(self.vendor_id),
            pid: Some(self.product_id),
            serial: self.serial.clone(),
            usage_page: Some(self.usage_page),
            usage: Some(self.usage),
            interface_number: None,
            port_name: Some(self.name.clone()),
            endpoint: None,
        }
    }

    /// Scores how well a connected `HidDeviceInfo` matches this device definition.
    /// 100 = exact VID+PID+serial, 85 = VID+PID+usage_page+usage, 70 = VID+PID+usage_page, 60 = VID+PID.
    pub fn match_hid_info(&self, info: &HidDeviceInfo) -> u32 {
        if self.vendor_id != info.vid || self.product_id != info.pid {
            return 0;
        }
        if let (Some(a), Some(b)) = (self.serial.as_deref(), info.serial.as_deref()) {
            if !a.is_empty() && a == b {
                return 100;
            }
        }
        if self.usage_page == info.usage_page && self.usage == info.usage {
            return 85;
        }
        if self.usage_page == info.usage_page {
            return 70;
        }
        60
    }

    /// Builds a `kf_core::Device` with one output port per learned control and its modifier stack attached.
    pub fn to_graph_device(&self, x: f64, y: f64) -> Device {
        let mut dev = Device::new(
            format!("hid.{:04x}.{:04x}", self.vendor_id, self.product_id),
            self.name.clone(),
            DeviceKind::HidDevice {
                device_preset_id: Some(self.preset_id.to_string()),
                vid: Some(self.vendor_id),
                pid: Some(self.product_id),
            },
        )
        .with_fingerprint(self.fingerprint());
        dev.ui.x = x;
        dev.ui.y = y;

        for ctrl in &self.controls {
            let sig_kind = match ctrl.control_type {
                HidControlType::Button | HidControlType::Toggle => SignalKind::Trigger,
                _ => SignalKind::Sensor,
            };
            let mut port = Port::new(
                ctrl.logical_key.clone(),
                ctrl.alias.clone(),
                PortDirection::Out,
                sig_kind,
            );
            port.modifiers = ctrl.modifiers.clone();
            dev.ports.push(port);
        }

        dev
    }

    /// Exports as a Kontrolfreak `.kfdevice.json` string compatible with `kf_core::DevicePreset`.
    pub fn to_kfdevice_json(&self) -> Result<String, HardwareError> {
        let dev = self.to_graph_device(140.0, 140.0);
        let mut preset = DevicePreset::from_device(self.name.clone(), dev);
        preset.preset_id = self.preset_id;
        preset.hid_mappings = self.controls.iter().map(|c| c.to_core_mapping()).collect();

        // Serialize standard `DevicePreset` alongside M3 `hid_definition` metadata in a single valid `.kfdevice.json`.
        let mut val = serde_json::to_value(&preset)
            .map_err(|e| HardwareError::Backend(format!("JSON serialize failed: {e}")))?;
        if let Some(obj) = val.as_object_mut() {
            obj.insert(
                "hid_definition".into(),
                serde_json::to_value(self)
                    .map_err(|e| HardwareError::Backend(format!("JSON serialize failed: {e}")))?,
            );
        }
        serde_json::to_string_pretty(&val)
            .map_err(|e| HardwareError::Backend(format!("JSON stringify failed: {e}")))
    }

    /// Imports from either a `.kfdevice.json` (`DevicePreset` + optional `hid_definition`) or raw `HidDeviceDefinition` JSON.
    pub fn from_kfdevice_json(json: &str) -> Result<Self, HardwareError> {
        let val: serde_json::Value = serde_json::from_str(json)
            .map_err(|e| HardwareError::Backend(format!("Invalid .kfdevice.json: {e}")))?;

        if let Some(embedded) = val.get("hid_definition") {
            let def: HidDeviceDefinition = serde_json::from_value(embedded.clone())
                .map_err(|e| HardwareError::Backend(format!("Invalid hid_definition: {e}")))?;
            return Ok(def);
        }

        // Try direct `HidDeviceDefinition`
        if val.get("vendor_id").is_some() && val.get("product_id").is_some() {
            let def: HidDeviceDefinition = serde_json::from_value(val)
                .map_err(|e| HardwareError::Backend(format!("Invalid HidDeviceDefinition: {e}")))?;
            return Ok(def);
        }

        // Fallback: parse plain `kf_core::DevicePreset`
        let preset = DevicePreset::from_json(json)
            .map_err(|e| HardwareError::Backend(format!("Invalid DevicePreset: {e}")))?;
        let fp = preset.device.fingerprint.unwrap_or_default();
        let controls = preset
            .hid_mappings
            .iter()
            .map(|m| {
                let mut spec = LearnedControlSpec::from_core_mapping(m);
                if let Some(port) = preset
                    .device
                    .ports
                    .iter()
                    .find(|p| p.logical_key == m.logical_key)
                {
                    spec.modifiers = port.modifiers.clone();
                }
                spec
            })
            .collect();

        Ok(Self {
            schema_version: preset.schema_version,
            preset_id: preset.preset_id,
            name: preset.name,
            vendor_id: fp.vid.unwrap_or(0),
            product_id: fp.pid.unwrap_or(0),
            usage_page: fp.usage_page.unwrap_or(0),
            usage: fp.usage.unwrap_or(0),
            serial: fp.serial,
            noise_masks: HashMap::new(),
            controls,
        })
    }
}

/// Library of saved `.kfdevice.json` definitions supporting auto-recognition & auto-rebind.
#[derive(Debug, Default, Clone)]
pub struct DevicePresetLibrary {
    definitions: Vec<HidDeviceDefinition>,
}

impl DevicePresetLibrary {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert(&mut self, def: HidDeviceDefinition) {
        if let Some(pos) = self.definitions.iter().position(|d| {
            d.preset_id == def.preset_id
                || (d.vendor_id == def.vendor_id
                    && d.product_id == def.product_id
                    && d.usage_page == def.usage_page
                    && d.usage == def.usage)
        }) {
            self.definitions[pos] = def;
        } else {
            self.definitions.push(def);
        }
    }

    pub fn list(&self) -> &[HidDeviceDefinition] {
        &self.definitions
    }

    pub fn find_best_match(&self, info: &HidDeviceInfo) -> Option<&HidDeviceDefinition> {
        self.definitions
            .iter()
            .filter_map(|d| {
                let score = d.match_hid_info(info);
                if score >= 60 {
                    Some((score, d))
                } else {
                    None
                }
            })
            .max_by_key(|&(score, _)| score)
            .map(|(_, d)| d)
    }

    /// Marks matching `HidDevice` graph nodes as `OfflineGhost` when a physical HID device disconnects.
    pub fn handle_disconnect(control: &ControlPlane, fingerprint: &DeviceFingerprint) -> Vec<Uuid> {
        let mut ghosted = Vec::new();
        let snap = control.snapshot();
        for dev in snap.devices {
            if !matches!(dev.kind, DeviceKind::HidDevice { .. }) {
                continue;
            }
            let matched = dev
                .fingerprint
                .as_ref()
                .map(|fp| fp.match_score(fingerprint) >= 65)
                .unwrap_or(false);
            if matched && dev.status.is_online() {
                if control
                    .set_device_status(
                        dev.id,
                        DeviceStatus::OfflineGhost {
                            reason: Some("HID device unplugged — waiting for replug".into()),
                            last_seen_ms: Some(kf_core::SignalEvent::now_ms()),
                        },
                    )
                    .is_ok()
                {
                    ghosted.push(dev.id);
                }
            }
        }
        ghosted
    }

    /// Auto-rebinds an `OfflineGhost` HID device node when its physical peripheral reconnects.
    pub fn handle_reconnect(
        &self,
        control: &ControlPlane,
        info: &HidDeviceInfo,
    ) -> Result<Option<Uuid>, HardwareError> {
        let fp = fingerprint_from_hid_info(info);
        let logical_hint = format!("hid.{:04x}.{:04x}", info.vid, info.pid);
        control
            .auto_rebind_discovered(&fp, Some(&logical_hint))
            .map_err(|e| HardwareError::Backend(format!("Auto-rebind failed: {e}")))
    }
}
