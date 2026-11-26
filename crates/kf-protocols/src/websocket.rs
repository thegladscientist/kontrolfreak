use crate::osc::{decode_osc_bytes, encode_osc_message, osc_args_to_signal};
use kf_core::{SignalValue, WsFraming};
use tokio_tungstenite::tungstenite::Message;

/// Decodes an incoming WebSocket message according to the endpoint's `WsFraming` mode
/// into zero or more `(logical_key, SignalValue)` pairs.
pub fn decode_ws_message(msg: &Message, framing: WsFraming) -> Vec<(String, SignalValue)> {
    match (framing, msg) {
        (WsFraming::Json, Message::Text(text)) => decode_json_payload(text),
        (WsFraming::Text, Message::Text(text)) => decode_text_payload(text),
        (WsFraming::Binary, Message::Binary(bytes)) => {
            vec![("ws.binary".to_string(), SignalValue::bytes(bytes.clone()))]
        }
        (WsFraming::OscOverWs, Message::Binary(bytes)) => match decode_osc_bytes(bytes) {
            Ok(msgs) => msgs
                .into_iter()
                .map(|m| (m.addr, osc_args_to_signal(&m.args)))
                .collect(),
            Err(_) => Vec::new(),
        },
        // Flexible fallback: text on binary or JSON on binary
        (WsFraming::Json, Message::Binary(bytes)) => {
            if let Ok(s) = std::str::from_utf8(bytes) {
                decode_json_payload(s)
            } else {
                Vec::new()
            }
        }
        _ => Vec::new(),
    }
}

/// Encodes a `(logical_key, SignalValue)` into a WebSocket `Message`.
pub fn encode_ws_message(key: &str, value: &SignalValue, framing: WsFraming) -> Option<Message> {
    match framing {
        WsFraming::Json => {
            let json_val = match value {
                SignalValue::Float(v) => serde_json::json!({ "key": key, "value": v }),
                SignalValue::Int(v) => serde_json::json!({ "key": key, "value": v }),
                SignalValue::Bool(v) => serde_json::json!({ "key": key, "value": v }),
                SignalValue::String(s) => serde_json::json!({ "key": key, "value": s.as_ref() }),
                SignalValue::Trigger => serde_json::json!({ "key": key, "trigger": true }),
                SignalValue::Vec2(v) => serde_json::json!({ "key": key, "value": v }),
                SignalValue::Vec3(v) => serde_json::json!({ "key": key, "value": v }),
                SignalValue::Bytes(b) => serde_json::json!({ "key": key, "bytes": b.len() }),
                SignalValue::FrameRef(f) => serde_json::json!({ "key": key, "frame_id": f.frame_id }),
            };
            Some(Message::Text(json_val.to_string()))
        }
        WsFraming::Text => {
            let text = match value {
                SignalValue::String(s) => format!("{key}={s}"),
                _ => format!("{key}={}", value.summary_display()),
            };
            Some(Message::Text(text))
        }
        WsFraming::Binary => match value {
            SignalValue::Bytes(b) => Some(Message::Binary(b.to_vec())),
            _ => Some(Message::Binary(
                value.as_f64().unwrap_or(0.0).to_le_bytes().to_vec(),
            )),
        },
        WsFraming::OscOverWs => encode_osc_message(key, value).ok().map(Message::Binary),
    }
}

fn decode_json_payload(text: &str) -> Vec<(String, SignalValue)> {
    let Ok(val) = serde_json::from_str::<serde_json::Value>(text) else {
        return Vec::new();
    };
    let Some(obj) = val.as_object() else {
        return Vec::new();
    };
    // Support canonical `{ "key": "/foo", "value": 0.5 }` envelope as well as flat maps `{ "x": 0.5, "y": 0.2 }`
    if let (Some(k), Some(v)) = (obj.get("key").and_then(|v| v.as_str()), obj.get("value")) {
        if let Some(sv) = json_value_to_signal(v) {
            return vec![(k.to_string(), sv)];
        }
    }
    let mut out = Vec::new();
    for (k, v) in obj {
        if let Some(sv) = json_value_to_signal(v) {
            out.push((k.clone(), sv));
        }
    }
    out
}

fn json_value_to_signal(v: &serde_json::Value) -> Option<SignalValue> {
    match v {
        serde_json::Value::Number(n) => {
            if let Some(f) = n.as_f64() {
                Some(SignalValue::Float(f))
            } else {
                n.as_i64().map(SignalValue::Int)
            }
        }
        serde_json::Value::Bool(b) => Some(SignalValue::Bool(*b)),
        serde_json::Value::String(s) => Some(SignalValue::string(s.clone())),
        serde_json::Value::Array(arr) => {
            let nums: Vec<f64> = arr.iter().filter_map(|item| item.as_f64()).collect();
            match nums.as_slice() {
                [x, y] => Some(SignalValue::Vec2([*x, *y])),
                [x, y, z] => Some(SignalValue::Vec3([*x, *y, *z])),
                _ => None,
            }
        }
        _ => None,
    }
}

fn decode_text_payload(text: &str) -> Vec<(String, SignalValue)> {
    let trimmed = text.trim();
    if let Some((k, v)) = trimmed.split_once('=').or_else(|| trimmed.split_once(':')) {
        let key = k.trim().to_string();
        let val_str = v.trim();
        let sv = if let Ok(f) = val_str.parse::<f64>() {
            SignalValue::Float(f)
        } else if val_str.eq_ignore_ascii_case("true") {
            SignalValue::Bool(true)
        } else if val_str.eq_ignore_ascii_case("false") {
            SignalValue::Bool(false)
        } else {
            SignalValue::string(val_str.to_string())
        };
        vec![(key, sv)]
    } else {
        vec![("ws.text".to_string(), SignalValue::string(trimmed.to_string()))]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ws_json_text_binary_and_osc_over_ws_framing() {
        let m_json = encode_ws_message("/fade", &SignalValue::Float(0.75), WsFraming::Json).unwrap();
        let decoded = decode_ws_message(&m_json, WsFraming::Json);
        assert_eq!(decoded, vec![("/fade".into(), SignalValue::Float(0.75))]);

        let m_text = encode_ws_message("speed", &SignalValue::Float(0.5), WsFraming::Text).unwrap();
        let decoded_t = decode_ws_message(&m_text, WsFraming::Text);
        assert_eq!(decoded_t, vec![("speed".into(), SignalValue::Float(0.5))]);

        let m_osc = encode_ws_message("/synth/cut", &SignalValue::Float(0.25), WsFraming::OscOverWs).unwrap();
        let decoded_o = decode_ws_message(&m_osc, WsFraming::OscOverWs);
        assert_eq!(decoded_o, vec![("/synth/cut".into(), SignalValue::Float(0.25))]);
    }
}
