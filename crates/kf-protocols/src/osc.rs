use kf_core::SignalValue;
use rosc::{OscBundle, OscMessage, OscPacket, OscTime, OscType};
use std::net::SocketAddr;
use thiserror::Error;
use tokio::net::UdpSocket;

#[derive(Debug, Error)]
pub enum OscError {
    #[error("OSC I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("OSC codec error: {0}")]
    Codec(String),
    #[error("Invalid target address '{0}'")]
    InvalidTarget(String),
}

/// Unpacks an `OscPacket` (single message or nested bundles) into a flat list of `OscMessage`s.
pub fn unpack_osc_packet(packet: OscPacket) -> Vec<OscMessage> {
    let mut out = Vec::new();
    unpack_into(packet, &mut out);
    out
}

fn unpack_into(packet: OscPacket, out: &mut Vec<OscMessage>) {
    match packet {
        OscPacket::Message(msg) => out.push(msg),
        OscPacket::Bundle(bundle) => {
            for item in bundle.content {
                unpack_into(item, out);
            }
        }
    }
}

/// Matches an OSC 1.0 address pattern (supporting `*`, `?`, `[a-z]`, `[!a-z]`, `{a,b}`)
/// against a concrete OSC address.
pub fn osc_address_matches(pattern: &str, address: &str) -> bool {
    if pattern == address {
        return true;
    }
    let pat_parts: Vec<&str> = pattern.split('/').collect();
    let addr_parts: Vec<&str> = address.split('/').collect();
    if pat_parts.len() != addr_parts.len() {
        return false;
    }
    pat_parts
        .iter()
        .zip(addr_parts.iter())
        .all(|(p, a)| match_segment(p.as_bytes(), a.as_bytes()))
}

fn match_segment(pat: &[u8], text: &[u8]) -> bool {
    if pat.is_empty() {
        return text.is_empty();
    }
    match pat[0] {
        b'*' => {
            // Skip consecutive '*'
            let mut next_p = 1;
            while next_p < pat.len() && pat[next_p] == b'*' {
                next_p += 1;
            }
            for i in 0..=text.len() {
                if match_segment(&pat[next_p..], &text[i..]) {
                    return true;
                }
            }
            false
        }
        b'?' => !text.is_empty() && match_segment(&pat[1..], &text[1..]),
        b'[' => {
            let Some(close_rel) = pat[1..].iter().position(|&b| b == b']') else {
                return !text.is_empty() && text[0] == b'[' && match_segment(&pat[1..], &text[1..]);
            };
            if text.is_empty() {
                return false;
            }
            let close_idx = 1 + close_rel;
            let class = &pat[1..close_idx];
            let (negate, inner) = if class.first() == Some(&b'!') {
                (true, &class[1..])
            } else {
                (false, class)
            };
            let ch = text[0];
            let mut matched = false;
            let mut i = 0;
            while i < inner.len() {
                if i + 2 < inner.len() && inner[i + 1] == b'-' {
                    if ch >= inner[i] && ch <= inner[i + 2] {
                        matched = true;
                    }
                    i += 3;
                } else {
                    if ch == inner[i] {
                        matched = true;
                    }
                    i += 1;
                }
            }
            if negate {
                matched = !matched;
            }
            matched && match_segment(&pat[close_idx + 1..], &text[1..])
        }
        b'{' => {
            let Some(close_rel) = pat[1..].iter().position(|&b| b == b'}') else {
                return !text.is_empty() && text[0] == b'{' && match_segment(&pat[1..], &text[1..]);
            };
            let close_idx = 1 + close_rel;
            let rest = &pat[close_idx + 1..];
            let options = &pat[1..close_idx];
            for alt in options.split(|&b| b == b',') {
                let mut combined = Vec::with_capacity(alt.len() + rest.len());
                combined.extend_from_slice(alt);
                combined.extend_from_slice(rest);
                if match_segment(&combined, text) {
                    return true;
                }
            }
            false
        }
        c => !text.is_empty() && text[0] == c && match_segment(&pat[1..], &text[1..]),
    }
}

pub fn osc_args_to_signal(args: &[OscType]) -> SignalValue {
    match args {
        [] => SignalValue::Trigger,
        [OscType::Float(f)] => SignalValue::Float(*f as f64),
        [OscType::Double(d)] => SignalValue::Float(*d),
        [OscType::Int(i)] => SignalValue::Int(*i as i64),
        [OscType::Long(l)] => SignalValue::Int(*l),
        [OscType::Bool(b)] => SignalValue::Bool(*b),
        [OscType::String(s)] => SignalValue::string(s.clone()),
        [OscType::Blob(b)] => SignalValue::bytes(b.clone()),
        [OscType::Inf] | [OscType::Nil] => SignalValue::Trigger,
        [OscType::Float(x), OscType::Float(y)] => SignalValue::Vec2([*x as f64, *y as f64]),
        [OscType::Float(x), OscType::Float(y), OscType::Float(z)] => {
            SignalValue::Vec3([*x as f64, *y as f64, *z as f64])
        }
        [first, ..] => match first {
            OscType::Float(f) => SignalValue::Float(*f as f64),
            OscType::Double(d) => SignalValue::Float(*d),
            OscType::Int(i) => SignalValue::Int(*i as i64),
            OscType::Bool(b) => SignalValue::Bool(*b),
            OscType::String(s) => SignalValue::string(s.clone()),
            OscType::Blob(b) => SignalValue::bytes(b.clone()),
            _ => SignalValue::Trigger,
        },
    }
}

pub fn signal_to_osc_args(value: &SignalValue) -> Vec<OscType> {
    match value {
        SignalValue::Float(v) => vec![OscType::Float(*v as f32)],
        SignalValue::Int(v) => vec![OscType::Int(*v as i32)],
        SignalValue::Bool(v) => vec![OscType::Bool(*v)],
        SignalValue::String(s) => vec![OscType::String(s.to_string())],
        SignalValue::Trigger => vec![OscType::Inf],
        SignalValue::Vec2([x, y]) => vec![OscType::Float(*x as f32), OscType::Float(*y as f32)],
        SignalValue::Vec3([x, y, z]) => vec![
            OscType::Float(*x as f32),
            OscType::Float(*y as f32),
            OscType::Float(*z as f32),
        ],
        SignalValue::Bytes(b) => vec![OscType::Blob(b.to_vec())],
        SignalValue::FrameRef(f) => vec![
            OscType::Int(f.width as i32),
            OscType::Int(f.height as i32),
        ],
    }
}

pub fn encode_osc_message(addr: &str, value: &SignalValue) -> Result<Vec<u8>, OscError> {
    let normalized = if addr.starts_with('/') {
        addr.to_string()
    } else {
        format!("/{addr}")
    };
    let packet = OscPacket::Message(OscMessage {
        addr: normalized,
        args: signal_to_osc_args(value),
    });
    rosc::encoder::encode(&packet).map_err(|e| OscError::Codec(e.to_string()))
}

pub fn encode_osc_bundle(messages: Vec<OscMessage>) -> Result<Vec<u8>, OscError> {
    let packet = OscPacket::Bundle(OscBundle {
        timetag: OscTime {
            seconds: 0,
            fractional: 1,
        },
        content: messages.into_iter().map(OscPacket::Message).collect(),
    });
    rosc::encoder::encode(&packet).map_err(|e| OscError::Codec(e.to_string()))
}

pub fn decode_osc_bytes(buf: &[u8]) -> Result<Vec<OscMessage>, OscError> {
    let (_, packet) =
        rosc::decoder::decode_udp(buf).map_err(|e| OscError::Codec(e.to_string()))?;
    Ok(unpack_osc_packet(packet))
}

pub async fn send_osc_udp(
    socket: &UdpSocket,
    target: SocketAddr,
    addr: &str,
    value: &SignalValue,
) -> Result<usize, OscError> {
    let bytes = encode_osc_message(addr, value)?;
    Ok(socket.send_to(&bytes, target).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc_wildcard_matching() {
        assert!(osc_address_matches("/fader/*", "/fader/1"));
        assert!(osc_address_matches("/fader/?", "/fader/9"));
        assert!(!osc_address_matches("/fader/?", "/fader/10"));
        assert!(osc_address_matches("/strip/[1-4]/gain", "/strip/3/gain"));
        assert!(!osc_address_matches("/strip/[!1-4]/gain", "/strip/3/gain"));
        assert!(osc_address_matches("/fx/{reverb,delay}/mix", "/fx/delay/mix"));
        assert!(!osc_address_matches("/fx/{reverb,delay}/mix", "/fx/chorus/mix"));
    }

    #[test]
    fn osc_bundle_encode_and_unpack() {
        let raw = encode_osc_bundle(vec![
            OscMessage {
                addr: "/a".into(),
                args: vec![OscType::Float(0.25)],
            },
            OscMessage {
                addr: "/b".into(),
                args: vec![OscType::Int(42)],
            },
        ])
        .unwrap();
        let msgs = decode_osc_bytes(&raw).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].addr, "/a");
        assert_eq!(osc_args_to_signal(&msgs[0].args), SignalValue::Float(0.25));
        assert_eq!(msgs[1].addr, "/b");
        assert_eq!(osc_args_to_signal(&msgs[1].args), SignalValue::Int(42));
    }
}
