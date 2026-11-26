//! Portable `.kfreak` ZIP project bundle exporter & importer with SHA-256 manifest
//! verification, packaging `.kfproj.json` together with referenced `.kfdevice.json`,
//! `.kfgroup.json`, and `.kfmapping.json` presets for single-file cross-machine transfer.

use kf_core::{DevicePreset, GroupPreset, MappingPreset, PersistenceError, ProjectFile};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;
use uuid::Uuid;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

pub const KFREAK_BUNDLE_VERSION: u32 = 1;
pub const MANIFEST_PATH: &str = "manifest.json";
pub const PROJECT_ENTRY_PATH: &str = "project.kfproj.json";

#[derive(Debug, Error)]
pub enum BundleError {
    #[error("ZIP archive error: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("Persistence error: {0}")]
    Persistence(#[from] PersistenceError),
    #[error("Missing manifest file '{MANIFEST_PATH}' in .kfreak archive")]
    MissingManifest,
    #[error("Unsupported .kfreak bundle version {found} (max supported: {KFREAK_BUNDLE_VERSION})")]
    UnsupportedVersion { found: u32 },
    #[error("Bundle entry '{path}' listed in manifest is missing from archive")]
    MissingEntry { path: String },
    #[error("SHA-256 checksum mismatch for '{path}': expected {expected}, got {actual}")]
    ChecksumMismatch {
        path: String,
        expected: String,
        actual: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KfreakManifest {
    pub bundle_version: u32,
    pub created_at_ms: u64,
    pub project_id: Uuid,
    pub project_name: String,
    pub project_file: String,
    #[serde(default)]
    pub device_presets: Vec<String>,
    #[serde(default)]
    pub group_presets: Vec<String>,
    #[serde(default)]
    pub mapping_presets: Vec<String>,
    pub checksums: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct KfreakBundle {
    pub manifest: KfreakManifest,
    pub project: ProjectFile,
    pub device_presets: Vec<DevicePreset>,
    pub group_presets: Vec<GroupPreset>,
    pub mapping_presets: Vec<MappingPreset>,
}

pub fn sha256_hex(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    let mut out = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

fn slugify(input: &str) -> String {
    let mut s = String::new();
    let mut prev_dash = false;
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            s.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !s.is_empty() {
            s.push('-');
            prev_dash = true;
        }
    }
    let trimmed = s.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "preset".into()
    } else {
        trimmed
    }
}

/// Packages a `.kfproj.json` project together with auto-extracted & extra `.kfdevice.json`,
/// `.kfgroup.json`, and `.kfmapping.json` presets plus a SHA-256 `manifest.json` into a `.kfreak` ZIP.
pub fn export_kfreak_bytes(
    project: &ProjectFile,
    extra_devices: &[DevicePreset],
    extra_groups: &[GroupPreset],
    extra_mappings: &[MappingPreset],
) -> Result<(Vec<u8>, KfreakManifest), BundleError> {
    let mut cursor = Cursor::new(Vec::new());
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    let mut checksums = BTreeMap::new();
    let mut device_paths = Vec::new();
    let mut group_paths = Vec::new();
    let mut mapping_paths = Vec::new();

    {
        let mut zip = ZipWriter::new(&mut cursor);

        // 1. Write project.kfproj.json
        let project_json = project.to_json_pretty()?;
        let project_bytes = project_json.as_bytes();
        zip.start_file(PROJECT_ENTRY_PATH, options)?;
        zip.write_all(project_bytes)?;
        checksums.insert(PROJECT_ENTRY_PATH.to_string(), sha256_hex(project_bytes));

        // 2. Collect device presets from graph + extras
        let mut all_devices: Vec<DevicePreset> = project
            .graph
            .devices
            .iter()
            .map(|d| DevicePreset::from_device(d.display_name.clone(), d.clone()))
            .collect();
        all_devices.extend(extra_devices.iter().cloned());

        for (idx, preset) in all_devices.iter().enumerate() {
            let entry_path = format!(
                "presets/devices/{:02}-{}.kfdevice.json",
                idx + 1,
                slugify(&preset.name)
            );
            let json = preset.to_json_pretty()?;
            let bytes = json.as_bytes();
            zip.start_file(&entry_path, options)?;
            zip.write_all(bytes)?;
            checksums.insert(entry_path.clone(), sha256_hex(bytes));
            device_paths.push(entry_path);
        }

        // 3. Collect rack/group presets from graph + extras
        let mut all_groups: Vec<GroupPreset> = project
            .graph
            .groups
            .iter()
            .filter_map(|g| GroupPreset::extract(&project.graph, g.id))
            .collect();
        all_groups.extend(extra_groups.iter().cloned());

        for (idx, preset) in all_groups.iter().enumerate() {
            let entry_path = format!(
                "presets/groups/{:02}-{}.kfgroup.json",
                idx + 1,
                slugify(&preset.name)
            );
            let json = preset.to_json_pretty()?;
            let bytes = json.as_bytes();
            zip.start_file(&entry_path, options)?;
            zip.write_all(bytes)?;
            checksums.insert(entry_path.clone(), sha256_hex(bytes));
            group_paths.push(entry_path);
        }

        // 4. Collect wire mapping presets from non-empty wire modifier chains + extras
        let mut all_mappings: Vec<MappingPreset> = project
            .graph
            .wires
            .iter()
            .enumerate()
            .filter(|(_, w)| !w.modifiers.is_empty())
            .map(|(i, w)| {
                MappingPreset::new(
                    format!("Wire Mapping {}", i + 1),
                    Some(format!("Extracted from wire {}", w.id)),
                    w.modifiers.clone(),
                )
            })
            .collect();
        all_mappings.extend(extra_mappings.iter().cloned());

        for (idx, preset) in all_mappings.iter().enumerate() {
            let entry_path = format!(
                "presets/mappings/{:02}-{}.kfmapping.json",
                idx + 1,
                slugify(&preset.name)
            );
            let json = preset.to_json_pretty()?;
            let bytes = json.as_bytes();
            zip.start_file(&entry_path, options)?;
            zip.write_all(bytes)?;
            checksums.insert(entry_path.clone(), sha256_hex(bytes));
            mapping_paths.push(entry_path);
        }

        let created_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let manifest = KfreakManifest {
            bundle_version: KFREAK_BUNDLE_VERSION,
            created_at_ms,
            project_id: project.graph.project_id,
            project_name: project.graph.name.clone(),
            project_file: PROJECT_ENTRY_PATH.to_string(),
            device_presets: device_paths,
            group_presets: group_paths,
            mapping_presets: mapping_paths,
            checksums,
        };

        let manifest_json = serde_json::to_string_pretty(&manifest)?;
        zip.start_file(MANIFEST_PATH, options)?;
        zip.write_all(manifest_json.as_bytes())?;
        zip.finish()?;

        return Ok((cursor.into_inner(), manifest));
    }
}

pub fn export_kfreak_file(
    path: impl AsRef<Path>,
    project: &ProjectFile,
    extra_devices: &[DevicePreset],
    extra_groups: &[GroupPreset],
    extra_mappings: &[MappingPreset],
) -> Result<KfreakManifest, BundleError> {
    let (bytes, manifest) = export_kfreak_bytes(project, extra_devices, extra_groups, extra_mappings)?;
    std::fs::write(path, bytes)?;
    Ok(manifest)
}

fn read_verified_entry(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    manifest: &KfreakManifest,
    entry_path: &str,
) -> Result<String, BundleError> {
    let mut entry = archive
        .by_name(entry_path)
        .map_err(|_| BundleError::MissingEntry {
            path: entry_path.to_string(),
        })?;
    let mut raw = Vec::new();
    entry.read_to_end(&mut raw)?;

    if let Some(expected) = manifest.checksums.get(entry_path) {
        let actual = sha256_hex(&raw);
        if &actual != expected {
            return Err(BundleError::ChecksumMismatch {
                path: entry_path.to_string(),
                expected: expected.clone(),
                actual,
            });
        }
    } else {
        return Err(BundleError::ChecksumMismatch {
            path: entry_path.to_string(),
            expected: "<missing checksum in manifest>".into(),
            actual: sha256_hex(&raw),
        });
    }

    String::from_utf8(raw).map_err(|e| {
        BundleError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("UTF-8 decode error in '{entry_path}': {e}"),
        ))
    })
}

/// Imports and verifies a `.kfreak` ZIP bundle, validating every SHA-256 entry checksum.
pub fn import_kfreak_bytes(bytes: &[u8]) -> Result<KfreakBundle, BundleError> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))?;

    let manifest: KfreakManifest = {
        let mut mf = archive
            .by_name(MANIFEST_PATH)
            .map_err(|_| BundleError::MissingManifest)?;
        let mut raw = String::new();
        mf.read_to_string(&mut raw)?;
        serde_json::from_str(&raw)?
    };

    if manifest.bundle_version > KFREAK_BUNDLE_VERSION {
        return Err(BundleError::UnsupportedVersion {
            found: manifest.bundle_version,
        });
    }

    // Verify every single checksum declared in the manifest (detects tampering on any entry).
    let declared_paths: Vec<String> = manifest.checksums.keys().cloned().collect();
    for path in &declared_paths {
        let _ = read_verified_entry(&mut archive, &manifest, path)?;
    }

    let project_json = read_verified_entry(&mut archive, &manifest, &manifest.project_file)?;
    let project = ProjectFile::from_json(&project_json)?;

    let mut device_presets = Vec::with_capacity(manifest.device_presets.len());
    for p in &manifest.device_presets {
        let json = read_verified_entry(&mut archive, &manifest, p)?;
        device_presets.push(DevicePreset::from_json(&json)?);
    }

    let mut group_presets = Vec::with_capacity(manifest.group_presets.len());
    for p in &manifest.group_presets {
        let json = read_verified_entry(&mut archive, &manifest, p)?;
        group_presets.push(GroupPreset::from_json(&json)?);
    }

    let mut mapping_presets = Vec::with_capacity(manifest.mapping_presets.len());
    for p in &manifest.mapping_presets {
        let json = read_verified_entry(&mut archive, &manifest, p)?;
        mapping_presets.push(MappingPreset::from_json(&json)?);
    }

    Ok(KfreakBundle {
        manifest,
        project,
        device_presets,
        group_presets,
        mapping_presets,
    })
}

pub fn import_kfreak_file(path: impl AsRef<Path>) -> Result<KfreakBundle, BundleError> {
    let bytes = std::fs::read(path)?;
    import_kfreak_bytes(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kf_core::{
        CurveKind, Device, DeviceKind, Modifier, Port, PortDirection, RackGroup, SignalKind, Wire,
        WorkspaceGraph,
    };

    fn sample_project() -> ProjectFile {
        let mut graph = WorkspaceGraph::new("Touring Stage Rig");
        let p_out = Port::new("depth.closest_z", "Closest Z", PortDirection::Out, SignalKind::Sensor);
        let p_in = Port::new("/dimmer/1", "Stage Wash", PortDirection::In, SignalKind::Control);
        let p_out_id = p_out.id;
        let p_in_id = p_in.id;

        let mut depth = Device::new(
            "depth.stage",
            "Stage Depth Cam",
            DeviceKind::DepthSensor {
                vendor: "realsense".into(),
                sidecar_id: None,
                roi_min_z: 0.4,
                roi_max_z: 2.8,
            },
        )
        .with_port(p_out);

        let mut osc = Device::new(
            "osc.lights",
            "Lighting Desk",
            DeviceKind::OscEndpoint {
                bind_addr: "127.0.0.1:9000".into(),
                target_addr: Some("127.0.0.1:9001".into()),
                auto_discover: true,
            },
        )
        .with_port(p_in);

        let rack = RackGroup::new("Stage Rack", vec![depth.id, osc.id]);
        depth.group_id = Some(rack.id);
        osc.group_id = Some(rack.id);

        let mut wire = Wire::new(p_out_id, p_in_id);
        wire.modifiers.push(Modifier::Curve {
            kind: CurveKind::SCurve { steepness: 4.0 },
        });

        graph.devices.push(depth);
        graph.devices.push(osc);
        graph.wires.push(wire);
        graph.groups.push(rack);
        ProjectFile::from_graph(graph)
    }

    #[test]
    fn kfreak_zip_bundle_roundtrip_and_checksum_tamper_detection() {
        let proj = sample_project();
        let (zip_bytes, manifest) = export_kfreak_bytes(&proj, &[], &[], &[]).unwrap();

        assert_eq!(manifest.project_name, "Touring Stage Rig");
        assert_eq!(manifest.device_presets.len(), 2);
        assert_eq!(manifest.group_presets.len(), 1);
        assert_eq!(manifest.mapping_presets.len(), 1);
        assert_eq!(manifest.checksums.len(), 5);

        // Clean round-trip import
        let restored = import_kfreak_bytes(&zip_bytes).unwrap();
        assert_eq!(restored.project.graph.name, proj.graph.name);
        assert_eq!(restored.project.graph.devices.len(), 2);
        assert_eq!(restored.project.graph.wires.len(), 1);
        assert_eq!(restored.device_presets.len(), 2);
        assert_eq!(restored.group_presets.len(), 1);
        assert_eq!(restored.mapping_presets.len(), 1);

        // Tamper with project.kfproj.json inside the zip without updating manifest -> must fail checksum!
        let mut tampered_cursor = Cursor::new(Vec::new());
        {
            let mut orig = ZipArchive::new(Cursor::new(zip_bytes.as_slice())).unwrap();
            let mut writer = ZipWriter::new(&mut tampered_cursor);
            let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            for i in 0..orig.len() {
                let mut f = orig.by_index(i).unwrap();
                let name = f.name().to_string();
                let mut data = Vec::new();
                f.read_to_end(&mut data).unwrap();
                if name == PROJECT_ENTRY_PATH {
                    data.extend_from_slice(b" ");
                }
                writer.start_file(&name, opts).unwrap();
                writer.write_all(&data).unwrap();
            }
            writer.finish().unwrap();
        }

        let err = import_kfreak_bytes(&tampered_cursor.into_inner()).unwrap_err();
        assert!(matches!(err, BundleError::ChecksumMismatch { .. }));
    }
}
