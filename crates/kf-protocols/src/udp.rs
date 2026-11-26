use kf_core::{SignalValue, UdpFormat};

/// Parses a raw UDP datagram into `(logical_key, SignalValue)` pairs using the configured `UdpFormat`.
pub fn parse_udp_payload(bytes: &[u8], format: UdpFormat) -> Vec<(String, SignalValue)> {
    match format {
        UdpFormat::Utf8 => {
            let Ok(text) = std::str::from_utf8(bytes) else {
                return Vec::new();
            };
            let trimmed = text.trim();
            if let Some((k, v)) = trimmed.split_once('=').or_else(|| trimmed.split_once(':')) {
                let key = k.trim().to_string();
                let val_str = v.trim();
                let sv = val_str
                    .parse::<f64>()
                    .map(SignalValue::Float)
                    .unwrap_or_else(|_| SignalValue::string(val_str.to_string()));
                vec![(key, sv)]
            } else if let Ok(f) = trimmed.parse::<f64>() {
                vec![("udp.value".to_string(), SignalValue::Float(f))]
            } else {
                vec![("udp.utf8".to_string(), SignalValue::string(trimmed.to_string()))]
            }
        }
        UdpFormat::Json => {
            let Ok(text) = std::str::from_utf8(bytes) else {
                return Vec::new();
            };
            let Ok(val) = serde_json::from_str::<serde_json::Value>(text) else {
                return Vec::new();
            };
            let Some(obj) = val.as_object() else {
                return Vec::new();
            };
            if let (Some(k), Some(v)) = (obj.get("key").and_then(|x| x.as_str()), obj.get("value")) {
                if let Some(f) = v.as_f64() {
                    return vec![(k.to_string(), SignalValue::Float(f))];
                }
            }
            obj.iter()
                .filter_map(|(k, v)| {
                    if let Some(f) = v.as_f64() {
                        Some((k.clone(), SignalValue::Float(f)))
                    } else if let Some(b) = v.as_bool() {
                        Some((k.clone(), SignalValue::Bool(b)))
                    } else {
                        v.as_str()
                            .map(|s| (k.clone(), SignalValue::string(s.to_string())))
                    }
                })
                .collect()
        }
        UdpFormat::Csv => {
            let Ok(text) = std::str::from_utf8(bytes) else {
                return Vec::new();
            };
            text.trim()
                .split(',')
                .enumerate()
                .map(|(i, cell)| {
                    let c = cell.trim();
                    let sv = c
                        .parse::<f64>()
                        .map(SignalValue::Float)
                        .unwrap_or_else(|_| SignalValue::string(c.to_string()));
                    (format!("col.{i}"), sv)
                })
                .collect()
        }
        UdpFormat::Hex => {
            // Accept either raw binary bytes or ASCII hex string like "01 ff 7f"
            if let Ok(text) = std::str::from_utf8(bytes) {
                if let Some(parsed) = parse_hex_str(text) {
                    return vec![("udp.hex".to_string(), SignalValue::bytes(parsed))];
                }
            }
            vec![("udp.hex".to_string(), SignalValue::bytes(bytes.to_vec()))]
        }
    }
}

/// Formats an outbound `(logical_key, SignalValue)` into a UDP datagram according to `UdpFormat`.
pub fn format_udp_payload(key: &str, value: &SignalValue, format: UdpFormat) -> Vec<u8> {
    match format {
        UdpFormat::Utf8 => format!("{key}={}", value.summary_display()).into_bytes(),
        UdpFormat::Json => {
            let json = match value {
                SignalValue::Float(v) => serde_json::json!({ "key": key, "value": v }),
                SignalValue::Int(v) => serde_json::json!({ "key": key, "value": v }),
                SignalValue::Bool(v) => serde_json::json!({ "key": key, "value": v }),
                SignalValue::String(s) => serde_json::json!({ "key": key, "value": s.as_ref() }),
                _ => serde_json::json!({ "key": key, "value": value.as_f64().unwrap_or(0.0) }),
            };
            json.to_string().into_bytes()
        }
        UdpFormat::Csv => match value {
            SignalValue::Vec2([x, y]) => format!("{x:.4},{y:.4}").into_bytes(),
            SignalValue::Vec3([x, y, z]) => format!("{x:.4},{y:.4},{z:.4}").into_bytes(),
            _ => format!("{}", value.as_f64().unwrap_or(0.0)).into_bytes(),
        },
        UdpFormat::Hex => match value {
            SignalValue::Bytes(b) => encode_hex_str(b).into_bytes(),
            _ => {
                let byte = (value.as_f64().unwrap_or(0.0).clamp(0.0, 1.0) * 255.0).round() as u8;
                format!("{byte:02x}").into_bytes()
            }
        },
    }
}

fn parse_hex_str(s: &str) -> Option<Vec<u8>> {
    let cleaned: String = s.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    if cleaned.is_empty() || cleaned.len() % 2 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(cleaned.len() / 2);
    for chunk in cleaned.as_bytes().chunks(2) {
        let pair = std::str::from_utf8(chunk).ok()?;
        let byte = u8::from_str_radix(pair, 16).ok()?;
        out.push(byte);
    }
    Some(out)
}

fn encode_hex_str(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn udp_formats_roundtrip() {
        let u = format_udp_payload("ch1", &SignalValue::Float(0.42), UdpFormat::Utf8);
        assert_eq!(
            parse_udp_payload(&u, UdpFormat::Utf8),
            vec![("ch1".into(), SignalValue::Float(0.42))]
        );

        let j = format_udp_payload("temp", &SignalValue::Float(21.5), UdpFormat::Json);
        assert_eq!(
            parse_udp_payload(&j, UdpFormat::Json),
            vec![("temp".into(), SignalValue::Float(21.5))]
        );

        let c = parse_udp_payload(b"0.1, 0.5, 0.9", UdpFormat::Csv);
        assert_eq!(c.len(), 3);
        assert_eq!(c[1], ("col.1".into(), SignalValue::Float(0.5)));

        let h = format_udp_payload("raw", &SignalValue::bytes(vec![0xDE, 0xAD, 0xBE, 0xEF]), UdpFormat::Hex);
        assert_eq!(
            parse_udp_payload(&h, UdpFormat::Hex),
            vec![("udp.hex".into(), SignalValue::bytes(vec![0xDE, 0xAD, 0xBE, 0xEF]))]
        );
    }
}
