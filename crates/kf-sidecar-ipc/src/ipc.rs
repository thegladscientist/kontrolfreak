//! Cross-platform local control IPC (`interprocess`) using Unix domain sockets on
//! Linux/macOS and Named Pipes on Windows with length-prefixed JSON framing.

use crate::protocol::{SidecarClient, SidecarError, SidecarHealthStatus, SidecarRequest, SidecarResponse};
use interprocess::local_socket::{prelude::*, Listener, ListenerOptions, Name, Stream};
#[cfg(unix)]
use interprocess::local_socket::GenericFilePath;
#[cfg(windows)]
use interprocess::local_socket::GenericNamespaced;
use parking_lot::Mutex;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const MAX_IPC_MESSAGE_BYTES: usize = 4 * 1024 * 1024;

/// Builds a platform-appropriate `interprocess` local socket `Name` from an endpoint string.
pub fn to_socket_name(endpoint: &str) -> Result<Name<'static>, SidecarError> {
    #[cfg(unix)]
    {
        let owned = endpoint.to_string();
        owned
            .to_fs_name::<GenericFilePath>()
            .map_err(|e| SidecarError::Protocol(format!("invalid Unix socket path '{endpoint}': {e}")))
    }
    #[cfg(windows)]
    {
        let clean = endpoint
            .rsplit(['\\', '/'])
            .next()
            .unwrap_or(endpoint)
            .to_string();
        clean
            .to_ns_name::<GenericNamespaced>()
            .map_err(|e| SidecarError::Protocol(format!("invalid Windows pipe name '{endpoint}': {e}")))
    }
}

/// Removes any stale Unix socket file before binding.
pub fn cleanup_socket_endpoint(endpoint: &str) {
    #[cfg(unix)]
    {
        let _ = std::fs::remove_file(endpoint);
    }
    #[cfg(windows)]
    {
        let _ = endpoint;
    }
}

pub fn write_framed_json<W: Write, T: serde::Serialize>(
    writer: &mut W,
    msg: &T,
) -> Result<(), SidecarError> {
    let payload = serde_json::to_vec(msg)
        .map_err(|e| SidecarError::Protocol(format!("serialize failed: {e}")))?;
    if payload.len() > MAX_IPC_MESSAGE_BYTES {
        return Err(SidecarError::Protocol("IPC message exceeds max frame size".into()));
    }
    let len_bytes = (payload.len() as u32).to_le_bytes();
    writer.write_all(&len_bytes)?;
    writer.write_all(&payload)?;
    writer.flush()?;
    Ok(())
}

pub fn read_framed_json<R: Read, T: serde::de::DeserializeOwned>(
    reader: &mut R,
) -> Result<T, SidecarError> {
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf)?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if len == 0 || len > MAX_IPC_MESSAGE_BYTES {
        return Err(SidecarError::Protocol(format!("invalid IPC frame length: {len}")));
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf)?;
    serde_json::from_slice(&buf)
        .map_err(|e| SidecarError::Protocol(format!("deserialize failed: {e}")))
}

/// Server-side local IPC endpoint hosted inside the sensor sidecar binary.
pub struct IpcServer {
    endpoint: String,
    listener: Listener,
}

impl IpcServer {
    pub fn bind(endpoint: impl Into<String>) -> Result<Self, SidecarError> {
        let endpoint = endpoint.into();
        cleanup_socket_endpoint(&endpoint);
        let name = to_socket_name(&endpoint)?;
        let listener = ListenerOptions::new().name(name).create_sync()?;
        Ok(Self { endpoint, listener })
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Runs a blocking accept/serve loop until `stop_flag` becomes true or handler returns `false`.
    pub fn serve_loop<F>(&self, stop_flag: Arc<AtomicBool>, mut handler: F) -> Result<(), SidecarError>
    where
        F: FnMut(SidecarRequest) -> (SidecarResponse, bool),
    {
        for conn in self.listener.incoming() {
            if stop_flag.load(Ordering::Acquire) {
                break;
            }
            let mut stream = match conn {
                Ok(s) => s,
                Err(e) => {
                    if stop_flag.load(Ordering::Acquire) {
                        break;
                    }
                    tracing::warn!("IPC accept error: {e}");
                    continue;
                }
            };

            loop {
                if stop_flag.load(Ordering::Acquire) {
                    return Ok(());
                }
                let req: SidecarRequest = match read_framed_json(&mut stream) {
                    Ok(r) => r,
                    Err(_) => break, // client disconnected cleanly
                };
                let (resp, keep_running) = handler(req);
                let _ = write_framed_json(&mut stream, &resp);
                if !keep_running {
                    stop_flag.store(true, Ordering::Release);
                    return Ok(());
                }
            }
        }
        Ok(())
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        cleanup_socket_endpoint(&self.endpoint);
    }
}

/// Host-side local IPC client implementing `SidecarClient`.
pub struct IpcClient {
    endpoint: String,
    stream: Mutex<Option<Stream>>,
    status: Mutex<SidecarHealthStatus>,
}

impl IpcClient {
    pub fn connect_with_retry(
        endpoint: impl Into<String>,
        timeout: Duration,
    ) -> Result<Self, SidecarError> {
        let endpoint = endpoint.into();
        let deadline = Instant::now() + timeout;

        loop {
            let name = to_socket_name(&endpoint)?;
            match Stream::connect(name) {
                Ok(stream) => {
                    return Ok(Self {
                        endpoint,
                        stream: Mutex::new(Some(stream)),
                        status: Mutex::new(SidecarHealthStatus::Running),
                    });
                }
                Err(e) => {
                    if Instant::now() >= deadline {
                        return Err(SidecarError::Timeout(format!(
                            "timed out connecting to sidecar IPC '{endpoint}': {e}"
                        )));
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        }
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn set_status(&self, status: SidecarHealthStatus) {
        *self.status.lock() = status;
    }
}

impl SidecarClient for IpcClient {
    fn status(&self) -> SidecarHealthStatus {
        *self.status.lock()
    }

    fn send_request(&self, req: SidecarRequest) -> Result<SidecarResponse, SidecarError> {
        let mut guard = self.stream.lock();
        let stream = guard
            .as_mut()
            .ok_or_else(|| SidecarError::Disconnected("IPC stream closed".into()))?;

        if let Err(e) = write_framed_json(stream, &req) {
            *guard = None;
            *self.status.lock() = SidecarHealthStatus::CrashedRestarting;
            return Err(SidecarError::Disconnected(e.to_string()));
        }

        match read_framed_json::<_, SidecarResponse>(stream) {
            Ok(resp) => Ok(resp),
            Err(e) => {
                *guard = None;
                *self.status.lock() = SidecarHealthStatus::CrashedRestarting;
                Err(SidecarError::Disconnected(e.to_string()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::SIDECAR_PROTOCOL_VERSION;

    #[test]
    fn local_ipc_request_response_roundtrip() {
        let endpoint = std::env::temp_dir()
            .join(format!("kf-ipc-test-{}.sock", uuid::Uuid::new_v4()))
            .to_string_lossy()
            .to_string();
        let server = IpcServer::bind(&endpoint).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_bg = Arc::clone(&stop);

        let handle = std::thread::spawn(move || {
            let _ = server.serve_loop(stop_bg, |req| match req {
                SidecarRequest::Hello { protocol_version } => (
                    SidecarResponse::HelloAck {
                        protocol_version,
                        sidecar_name: "test-sidecar".into(),
                    },
                    true,
                ),
                SidecarRequest::Shutdown => (
                    SidecarResponse::Status {
                        health: SidecarHealthStatus::Stopped,
                        active_streams: 0,
                    },
                    false,
                ),
                _ => (
                    SidecarResponse::Pong {
                        uptime_ms: 42,
                        pid: std::process::id(),
                        frames_produced: 7,
                    },
                    true,
                ),
            });
        });

        let client = IpcClient::connect_with_retry(&endpoint, Duration::from_secs(2)).unwrap();
        let ack = client
            .send_request(SidecarRequest::Hello {
                protocol_version: SIDECAR_PROTOCOL_VERSION,
            })
            .unwrap();
        assert_eq!(
            ack,
            SidecarResponse::HelloAck {
                protocol_version: SIDECAR_PROTOCOL_VERSION,
                sidecar_name: "test-sidecar".into(),
            }
        );

        let pong = client.send_request(SidecarRequest::HealthPing).unwrap();
        assert!(matches!(pong, SidecarResponse::Pong { frames_produced: 7, .. }));

        let _ = client.send_request(SidecarRequest::Shutdown);
        handle.join().unwrap();
    }
}
