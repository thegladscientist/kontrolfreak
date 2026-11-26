//! Tier 1: Pure-Rust USB HID 1.11 Report Descriptor Parser.
//!
//! Decodes standard HID descriptor byte streams into typed control fields
//! (usage page, usage, report ID, bit offset, bit size, logical min/max, signedness,
//! relative/absolute flags) so known controls can be pre-populated automatically.

use kf_core::{HidControlClass, LearnedHidMapping};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DescriptorParseError {
    #[error("Empty HID report descriptor")]
    Empty,
    #[error("Truncated HID descriptor item at byte offset {offset}")]
    Truncated { offset: usize },
    #[error("Global state stack underflow at byte offset {offset}")]
    StackUnderflow { offset: usize },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HidDescriptorField {
    pub report_id: u8,
    pub usage_page: u16,
    pub usage: u16,
    pub usage_name: String,
    pub bit_offset: usize,
    pub bit_size: u8,
    pub logical_min: i64,
    pub logical_max: i64,
    pub is_signed: bool,
    pub is_relative: bool,
    pub is_variable: bool,
    pub is_constant: bool,
}

impl HidDescriptorField {
    /// Converts a parsed HID descriptor field into a pre-populated `LearnedHidMapping`.
    pub fn to_learned_mapping(&self, index: usize) -> LearnedHidMapping {
        let byte_offset = self.bit_offset / 8;
        let bit_in_byte = (self.bit_offset % 8) as u8;
        let bit_mask = if self.bit_size >= 8 {
            0xFF
        } else {
            (((1u16 << self.bit_size) - 1) << bit_in_byte) as u8
        };

        let control_class = if self.is_relative {
            HidControlClass::RelativeEncoder
        } else if self.usage_page == 0x09 || self.bit_size == 1 {
            HidControlClass::Button
        } else {
            HidControlClass::AbsoluteAxis
        };

        let slug = self
            .usage_name
            .to_lowercase()
            .replace(|c: char| !c.is_ascii_alphanumeric(), "_");
        let logical_key = format!("hid.r{}.{}.{}", self.report_id, slug, index);

        LearnedHidMapping {
            logical_key,
            alias: self.usage_name.clone(),
            report_id: self.report_id,
            byte_offset,
            bit_mask,
            bit_width: self.bit_size,
            signed: self.is_signed,
            little_endian: true, // Standard USB HID reports are little-endian
            control_class,
            observed_min: self.logical_min,
            observed_max: self.logical_max,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedHidDescriptor {
    pub usage_page: Option<u16>,
    pub usage: Option<u16>,
    pub has_report_ids: bool,
    pub fields: Vec<HidDescriptorField>,
}

impl ParsedHidDescriptor {
    /// Converts all non-constant data fields into pre-populated `LearnedHidMapping`s.
    pub fn to_learned_mappings(&self) -> Vec<LearnedHidMapping> {
        self.fields
            .iter()
            .filter(|f| !f.is_constant)
            .enumerate()
            .map(|(i, f)| f.to_learned_mapping(i))
            .collect()
    }
}

#[derive(Debug, Clone, Default)]
struct GlobalState {
    usage_page: u16,
    logical_min: i64,
    logical_max: i64,
    report_size: u32,
    report_count: u32,
    report_id: u8,
    has_report_id: bool,
}

#[derive(Debug, Clone, Default)]
struct LocalState {
    usages: Vec<u32>,
    usage_min: Option<u32>,
    usage_max: Option<u32>,
}

impl LocalState {
    fn clear(&mut self) {
        self.usages.clear();
        self.usage_min = None;
        self.usage_max = None;
    }

    fn resolve_usage(&self, index: u32, default_page: u16) -> (u16, u16) {
        let raw = if let (Some(min), Some(max)) = (self.usage_min, self.usage_max) {
            (min + index).min(max)
        } else if !self.usages.is_empty() {
            let idx = (index as usize).min(self.usages.len() - 1);
            self.usages[idx]
        } else {
            0
        };

        if raw > 0xFFFF {
            (((raw >> 16) & 0xFFFF) as u16, (raw & 0xFFFF) as u16)
        } else {
            (default_page, (raw & 0xFFFF) as u16)
        }
    }
}

fn read_unsigned(bytes: &[u8]) -> u32 {
    match bytes.len() {
        0 => 0,
        1 => bytes[0] as u32,
        2 => u16::from_le_bytes([bytes[0], bytes[1]]) as u32,
        4 => u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        _ => 0,
    }
}

fn read_signed(bytes: &[u8]) -> i64 {
    match bytes.len() {
        0 => 0,
        1 => (bytes[0] as i8) as i64,
        2 => i16::from_le_bytes([bytes[0], bytes[1]]) as i64,
        4 => i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as i64,
        _ => 0,
    }
}

/// Returns a human-readable HID Usage name for common USB HID Usage Tables.
pub fn hid_usage_name(usage_page: u16, usage: u16) -> String {
    match (usage_page, usage) {
        (0x01, 0x04) => "Joystick".into(),
        (0x01, 0x05) => "Game Pad".into(),
        (0x01, 0x08) => "Multi-axis Controller".into(),
        (0x01, 0x30) => "Axis X".into(),
        (0x01, 0x31) => "Axis Y".into(),
        (0x01, 0x32) => "Axis Z".into(),
        (0x01, 0x33) => "Axis Rx".into(),
        (0x01, 0x34) => "Axis Ry".into(),
        (0x01, 0x35) => "Axis Rz".into(),
        (0x01, 0x36) => "Slider".into(),
        (0x01, 0x37) => "Dial".into(),
        (0x01, 0x38) => "Wheel".into(),
        (0x01, 0x39) => "Hat Switch".into(),
        (0x02, 0xBB) => "Throttle".into(),
        (0x02, 0xBA) => "Rudder".into(),
        (0x09, u) if u > 0 => format!("Button {u}"),
        (0x0C, 0xE9) => "Volume Up".into(),
        (0x0C, 0xEA) => "Volume Down".into(),
        (0x0C, 0xE2) => "Mute".into(),
        (0x0C, 0xCD) => "Play/Pause".into(),
        (page, u) if page >= 0xFF00 => format!("Vendor 0x{page:04X}:0x{u:04X}"),
        (page, u) => format!("Usage 0x{page:02X}:0x{u:02X}"),
    }
}

/// Parses a raw USB HID 1.11 report descriptor byte slice.
pub fn parse_hid_report_descriptor(bytes: &[u8]) -> Result<ParsedHidDescriptor, DescriptorParseError> {
    if bytes.is_empty() {
        return Err(DescriptorParseError::Empty);
    }

    let mut global = GlobalState::default();
    let mut global_stack: Vec<GlobalState> = Vec::new();
    let mut local = LocalState::default();

    let mut top_usage_page: Option<u16> = None;
    let mut top_usage: Option<u16> = None;
    let mut bit_offsets_by_report: std::collections::HashMap<u8, usize> =
        std::collections::HashMap::new();
    let mut fields: Vec<HidDescriptorField> = Vec::new();

    let mut pos = 0usize;
    while pos < bytes.len() {
        let prefix = bytes[pos];
        pos += 1;

        // Long item (0xFE)
        if prefix == 0xFE {
            if pos + 2 > bytes.len() {
                return Err(DescriptorParseError::Truncated { offset: pos - 1 });
            }
            let data_size = bytes[pos] as usize;
            pos += 2; // size + long item tag
            if pos + data_size > bytes.len() {
                return Err(DescriptorParseError::Truncated { offset: pos });
            }
            pos += data_size;
            continue;
        }

        let size_code = prefix & 0x03;
        let data_len = match size_code {
            0 => 0,
            1 => 1,
            2 => 2,
            3 => 4,
            _ => unreachable!(),
        };
        let item_type = (prefix >> 2) & 0x03; // 0 = Main, 1 = Global, 2 = Local
        let item_tag = (prefix >> 4) & 0x0F;

        if pos + data_len > bytes.len() {
            return Err(DescriptorParseError::Truncated { offset: pos - 1 });
        }
        let data = &bytes[pos..pos + data_len];
        pos += data_len;

        match item_type {
            // Main items
            0 => {
                match item_tag {
                    // Input (0x08)
                    0x08 => {
                        let flags = read_unsigned(data);
                        let is_constant = (flags & 0x01) != 0;
                        let is_variable = (flags & 0x02) != 0;
                        let is_relative = (flags & 0x04) != 0;

                        let report_id = global.report_id;
                        // If Report IDs are used, byte 0 of the report buffer holds the report ID (8 bits).
                        let default_start_bit = if global.has_report_id { 8 } else { 0 };
                        let bit_cursor = bit_offsets_by_report
                            .entry(report_id)
                            .or_insert(default_start_bit);

                        let is_signed = global.logical_min < 0;
                        let count = global.report_count;
                        let size = global.report_size as usize;

                        for idx in 0..count {
                            let (u_page, u_id) = local.resolve_usage(idx, global.usage_page);
                            if !is_constant && size > 0 {
                                fields.push(HidDescriptorField {
                                    report_id,
                                    usage_page: u_page,
                                    usage: u_id,
                                    usage_name: hid_usage_name(u_page, u_id),
                                    bit_offset: *bit_cursor,
                                    bit_size: (size.min(64)) as u8,
                                    logical_min: global.logical_min,
                                    logical_max: global.logical_max,
                                    is_signed,
                                    is_relative,
                                    is_variable,
                                    is_constant,
                                });
                            }
                            *bit_cursor += size;
                        }
                    }
                    // Collection (0x0A)
                    0x0A => {
                        if top_usage_page.is_none() && global.usage_page != 0 {
                            top_usage_page = Some(global.usage_page);
                        }
                        if top_usage.is_none() {
                            if let Some(&first_u) = local.usages.first() {
                                top_usage = Some((first_u & 0xFFFF) as u16);
                            }
                        }
                    }
                    // Output (0x09), Feature (0x0B), End Collection (0x0C)
                    _ => {}
                }
                local.clear();
            }
            // Global items
            1 => match item_tag {
                0x00 => global.usage_page = read_unsigned(data) as u16,
                0x01 => global.logical_min = read_signed(data),
                0x02 => {
                    // Logical Maximum is signed when Logical Minimum < 0, unsigned otherwise
                    global.logical_max = if global.logical_min < 0 {
                        read_signed(data)
                    } else {
                        read_unsigned(data) as i64
                    };
                }
                0x07 => global.report_size = read_unsigned(data),
                0x08 => {
                    global.report_id = read_unsigned(data) as u8;
                    global.has_report_id = true;
                }
                0x09 => global.report_count = read_unsigned(data),
                0x0A => global_stack.push(global.clone()),
                0x0B => {
                    global = global_stack
                        .pop()
                        .ok_or(DescriptorParseError::StackUnderflow { offset: pos })?;
                }
                _ => {}
            },
            // Local items
            2 => match item_tag {
                0x00 => local.usages.push(read_unsigned(data)),
                0x01 => local.usage_min = Some(read_unsigned(data)),
                0x02 => local.usage_max = Some(read_unsigned(data)),
                _ => {}
            },
            _ => {}
        }
    }

    Ok(ParsedHidDescriptor {
        usage_page: top_usage_page.or(if global.usage_page != 0 {
            Some(global.usage_page)
        } else {
            None
        }),
        usage: top_usage,
        has_report_ids: global.has_report_id,
        fields,
    })
}

/// Builds a realistic standard USB HID 1.11 Report Descriptor for the simulated "KF StudioFader 4"
/// controller: Report ID 1 with 4 x 8-bit unsigned faders (X, Y, Z, Slider), 1 x 16-bit signed Dial,
/// 4 x 1-bit Buttons, and 4 bits of constant padding.
pub fn sample_studio_fader_descriptor() -> Vec<u8> {
    vec![
        0x05, 0x01, // Usage Page (Generic Desktop)
        0x09, 0x04, // Usage (Joystick)
        0xA1, 0x01, // Collection (Application)
        0x85, 0x01, //   Report ID (1)
        // 4 x 8-bit unsigned faders (0..255)
        0x05, 0x01, //   Usage Page (Generic Desktop)
        0x09, 0x30, //   Usage (X)
        0x09, 0x31, //   Usage (Y)
        0x09, 0x32, //   Usage (Z)
        0x09, 0x36, //   Usage (Slider)
        0x15, 0x00, //   Logical Minimum (0)
        0x26, 0xFF, 0x00, // Logical Maximum (255)
        0x75, 0x08, //   Report Size (8)
        0x95, 0x04, //   Report Count (4)
        0x81, 0x02, //   Input (Data, Variable, Absolute)
        // 1 x 16-bit signed Dial (-32768..32767)
        0x09, 0x37, //   Usage (Dial)
        0x16, 0x00, 0x80, // Logical Minimum (-32768)
        0x26, 0xFF, 0x7F, // Logical Maximum (32767)
        0x75, 0x10, //   Report Size (16)
        0x95, 0x01, //   Report Count (1)
        0x81, 0x02, //   Input (Data, Variable, Absolute)
        // 1 x 8-bit relative Wheel (-127..127)
        0x09, 0x38, //   Usage (Wheel)
        0x15, 0x81, //   Logical Minimum (-127)
        0x25, 0x7F, //   Logical Maximum (127)
        0x75, 0x08, //   Report Size (8)
        0x95, 0x01, //   Report Count (1)
        0x81, 0x06, //   Input (Data, Variable, Relative)
        // 4 x 1-bit Buttons (Usage Page 0x09, Button 1..4)
        0x05, 0x09, //   Usage Page (Button)
        0x19, 0x01, //   Usage Minimum (1)
        0x29, 0x04, //   Usage Maximum (4)
        0x15, 0x00, //   Logical Minimum (0)
        0x25, 0x01, //   Logical Maximum (1)
        0x75, 0x01, //   Report Size (1)
        0x95, 0x04, //   Report Count (4)
        0x81, 0x02, //   Input (Data, Variable, Absolute)
        // 4 bits constant padding
        0x75, 0x04, //   Report Size (4)
        0x95, 0x01, //   Report Count (1)
        0x81, 0x03, //   Input (Constant, Variable, Absolute)
        0xC0, // End Collection
    ]
}
