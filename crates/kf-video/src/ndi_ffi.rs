//! Dynamic NDI Runtime v6 loader via `libloading` with clean C ABI wrappers.
//!
//! Resolves:
//! - Windows: `Processing.NDI.Lib.x64.dll` (via `NDI_RUNTIME_DIR_V6` / `NDI_RUNTIME_DIR_V5` / Program Files / PATH)
//! - macOS: `libndi.dylib` (via `NDI_RUNTIME_DIR_V6` / `/Library/NDI SDK for Apple/lib/macOS` / `/usr/local/lib`)
//! - Linux: `libndi.so` / `libndi.so.6` (via `NDI_RUNTIME_DIR_V6` / `/usr/local/lib` / `/usr/lib`)

use crate::{NdiRuntimeLoader, NdiRuntimeStatus, VideoError, VideoFrame, VideoSourceInfo};
use kf_core::{FrameRef, PixelFormat};
use libloading::Library;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::Arc;
use uuid::Uuid;

pub const NDI_INSTALL_URL: &str = "https://ndi.video/tools/";

// ---------- Minimal NDI v5/v6 C ABI definitions ----------

pub const NDI_FOURCC_UYVY: u32 = u32::from_le_bytes(*b"UYVY");
pub const NDI_FOURCC_BGRA: u32 = u32::from_le_bytes(*b"BGRA");
pub const NDI_FOURCC_BGRX: u32 = u32::from_le_bytes(*b"BGRX");
pub const NDI_FOURCC_RGBA: u32 = u32::from_le_bytes(*b"RGBA");
pub const NDI_FOURCC_RGBX: u32 = u32::from_le_bytes(*b"RGBX");
pub const NDI_FOURCC_NV12: u32 = u32::from_le_bytes(*b"NV12");

pub const NDI_FRAME_TYPE_VIDEO: c_int = 1;
pub const NDI_RECV_COLOR_FORMAT_RGBX_RGBA: c_int = 2;
pub const NDI_RECV_BANDWIDTH_HIGHEST: c_int = 100;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NdiSourceC {
    pub p_ndi_name: *const c_char,
    pub p_url_address: *const c_char,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NdiFindCreateC {
    pub show_local_sources: bool,
    pub p_groups: *const c_char,
    pub p_extra_ips: *const c_char,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NdiRecvCreateV3C {
    pub source_to_connect_to: NdiSourceC,
    pub color_format: c_int,
    pub bandwidth: c_int,
    pub allow_video_fields: bool,
    pub p_ndi_recv_name: *const c_char,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NdiSendCreateC {
    pub p_ndi_name: *const c_char,
    pub p_groups: *const c_char,
    pub clock_video: bool,
    pub clock_audio: bool,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NdiVideoFrameV2C {
    pub xres: c_int,
    pub yres: c_int,
    pub fourcc: u32,
    pub frame_rate_n: c_int,
    pub frame_rate_d: c_int,
    pub picture_aspect_ratio: f32,
    pub frame_format_type: c_int,
    pub timecode: i64,
    pub p_data: *mut u8,
    pub line_stride_in_bytes: c_int,
    pub p_metadata: *const c_char,
    pub timestamp: i64,
}

impl Default for NdiVideoFrameV2C {
    fn default() -> Self {
        Self {
            xres: 0,
            yres: 0,
            fourcc: 0,
            frame_rate_n: 30000,
            frame_rate_d: 1000,
            picture_aspect_ratio: 0.0,
            frame_format_type: 1, // Progressive
            timecode: 0,
            p_data: ptr::null_mut(),
            line_stride_in_bytes: 0,
            p_metadata: ptr::null(),
            timestamp: 0,
        }
    }
}

type FnInitialize = unsafe extern "C" fn() -> bool;
type FnDestroy = unsafe extern "C" fn();
type FnVersion = unsafe extern "C" fn() -> *const c_char;
type FnFindCreateV2 = unsafe extern "C" fn(*const NdiFindCreateC) -> *mut c_void;
type FnFindWaitForSources = unsafe extern "C" fn(*mut c_void, u32) -> bool;
type FnFindGetCurrentSources = unsafe extern "C" fn(*mut c_void, *mut u32) -> *const NdiSourceC;
type FnFindDestroy = unsafe extern "C" fn(*mut c_void);
type FnRecvCreateV3 = unsafe extern "C" fn(*const NdiRecvCreateV3C) -> *mut c_void;
type FnRecvCaptureV3 = unsafe extern "C" fn(
    *mut c_void,
    *mut NdiVideoFrameV2C,
    *mut c_void,
    *mut c_void,
    u32,
) -> c_int;
type FnRecvFreeVideoV2 = unsafe extern "C" fn(*mut c_void, *const NdiVideoFrameV2C);
type FnRecvDestroy = unsafe extern "C" fn(*mut c_void);
type FnSendCreate = unsafe extern "C" fn(*const NdiSendCreateC) -> *mut c_void;
type FnSendSendVideoV2 = unsafe extern "C" fn(*mut c_void, *const NdiVideoFrameV2C);
type FnSendDestroy = unsafe extern "C" fn(*mut c_void);

struct NdiVTable {
    _library: Library,
    library_path: String,
    version: String,
    destroy: FnDestroy,
    find_create_v2: FnFindCreateV2,
    find_wait_for_sources: FnFindWaitForSources,
    find_get_current_sources: FnFindGetCurrentSources,
    find_destroy: FnFindDestroy,
    recv_create_v3: Option<FnRecvCreateV3>,
    recv_capture_v3: Option<FnRecvCaptureV3>,
    recv_free_video_v2: Option<FnRecvFreeVideoV2>,
    recv_destroy: Option<FnRecvDestroy>,
    send_create: Option<FnSendCreate>,
    send_send_video_v2: Option<FnSendSendVideoV2>,
    send_destroy: Option<FnSendDestroy>,
}

// Safety: NDI C library functions are documented thread-safe for concurrent receiver/sender handles.
unsafe impl Send for NdiVTable {}
unsafe impl Sync for NdiVTable {}

impl Drop for NdiVTable {
    fn drop(&mut self) {
        unsafe {
            (self.destroy)();
        }
    }
}

/// Returns the ordered list of platform-specific NDI Runtime v6 shared library candidate paths.
pub fn candidate_ndi_library_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    #[cfg(target_os = "windows")]
    {
        let lib_name = "Processing.NDI.Lib.x64.dll";
        for env_key in ["NDI_RUNTIME_DIR_V6", "NDI_RUNTIME_DIR_V5", "NDI_RUNTIME_DIR_V4"] {
            if let Ok(dir) = std::env::var(env_key) {
                paths.push(PathBuf::from(dir).join(lib_name));
            }
        }
        paths.push(PathBuf::from(r"C:\Program Files\NDI\NDI 6 Runtime\v6").join(lib_name));
        paths.push(PathBuf::from(r"C:\Program Files\NDI\NDI 5 Runtime\v5").join(lib_name));
        paths.push(PathBuf::from(lib_name));
    }

    #[cfg(target_os = "macos")]
    {
        let lib_name = "libndi.dylib";
        for env_key in ["NDI_RUNTIME_DIR_V6", "NDI_RUNTIME_DIR_V5"] {
            if let Ok(dir) = std::env::var(env_key) {
                paths.push(PathBuf::from(dir).join(lib_name));
            }
        }
        paths.push(PathBuf::from("/Library/NDI SDK for Apple/lib/macOS").join(lib_name));
        paths.push(PathBuf::from("/usr/local/lib").join(lib_name));
        paths.push(PathBuf::from(lib_name));
    }

    #[cfg(target_os = "linux")]
    {
        for env_key in ["NDI_RUNTIME_DIR_V6", "NDI_RUNTIME_DIR_V5"] {
            if let Ok(dir) = std::env::var(env_key) {
                paths.push(PathBuf::from(&dir).join("libndi.so.6"));
                paths.push(PathBuf::from(&dir).join("libndi.so"));
            }
        }
        paths.push(PathBuf::from("/usr/local/lib/libndi.so.6"));
        paths.push(PathBuf::from("/usr/local/lib/libndi.so"));
        paths.push(PathBuf::from("/usr/lib/libndi.so.6"));
        paths.push(PathBuf::from("/usr/lib/libndi.so"));
        paths.push(PathBuf::from("libndi.so.6"));
        paths.push(PathBuf::from("libndi.so"));
    }

    paths
}

fn try_load_ndi_vtable() -> Option<Arc<NdiVTable>> {
    for candidate in candidate_ndi_library_paths() {
        // Avoid probing missing absolute paths unnecessarily, while still allowing bare SONAMEs
        if candidate.is_absolute() && !Path::new(&candidate).exists() {
            continue;
        }
        let lib = match unsafe { Library::new(&candidate) } {
            Ok(l) => l,
            Err(_) => continue,
        };

        unsafe {
            let initialize: FnInitialize = match lib.get::<FnInitialize>(b"NDIlib_initialize\0") {
                Ok(s) => *s,
                Err(_) => continue,
            };
            let destroy: FnDestroy = match lib.get::<FnDestroy>(b"NDIlib_destroy\0") {
                Ok(s) => *s,
                Err(_) => continue,
            };
            let version_fn: FnVersion = match lib.get::<FnVersion>(b"NDIlib_version\0") {
                Ok(s) => *s,
                Err(_) => continue,
            };
            let find_create_v2: FnFindCreateV2 =
                match lib.get::<FnFindCreateV2>(b"NDIlib_find_create_v2\0") {
                    Ok(s) => *s,
                    Err(_) => continue,
                };
            let find_wait_for_sources: FnFindWaitForSources =
                match lib.get::<FnFindWaitForSources>(b"NDIlib_find_wait_for_sources\0") {
                    Ok(s) => *s,
                    Err(_) => continue,
                };
            let find_get_current_sources: FnFindGetCurrentSources =
                match lib.get::<FnFindGetCurrentSources>(b"NDIlib_find_get_current_sources\0") {
                    Ok(s) => *s,
                    Err(_) => continue,
                };
            let find_destroy: FnFindDestroy =
                match lib.get::<FnFindDestroy>(b"NDIlib_find_destroy\0") {
                    Ok(s) => *s,
                    Err(_) => continue,
                };

            let recv_create_v3 = lib
                .get::<FnRecvCreateV3>(b"NDIlib_recv_create_v3\0")
                .ok()
                .map(|s| *s);
            let recv_capture_v3 = lib
                .get::<FnRecvCaptureV3>(b"NDIlib_recv_capture_v3\0")
                .ok()
                .map(|s| *s);
            let recv_free_video_v2 = lib
                .get::<FnRecvFreeVideoV2>(b"NDIlib_recv_free_video_v2\0")
                .ok()
                .map(|s| *s);
            let recv_destroy = lib
                .get::<FnRecvDestroy>(b"NDIlib_recv_destroy\0")
                .ok()
                .map(|s| *s);
            let send_create = lib
                .get::<FnSendCreate>(b"NDIlib_send_create\0")
                .ok()
                .map(|s| *s);
            let send_send_video_v2 = lib
                .get::<FnSendSendVideoV2>(b"NDIlib_send_send_video_v2\0")
                .ok()
                .map(|s| *s);
            let send_destroy = lib
                .get::<FnSendDestroy>(b"NDIlib_send_destroy\0")
                .ok()
                .map(|s| *s);

            if !initialize() {
                tracing::warn!(
                    "NDI library loaded from {:?} but NDIlib_initialize() returned false",
                    candidate
                );
                continue;
            }

            let ver_ptr = version_fn();
            let version = if ver_ptr.is_null() {
                "NDI Runtime v6".to_string()
            } else {
                CStr::from_ptr(ver_ptr).to_string_lossy().into_owned()
            };

            return Some(Arc::new(NdiVTable {
                _library: lib,
                library_path: candidate.to_string_lossy().into_owned(),
                version,
                destroy,
                find_create_v2,
                find_wait_for_sources,
                find_get_current_sources,
                find_destroy,
                recv_create_v3,
                recv_capture_v3,
                recv_free_video_v2,
                recv_destroy,
                send_create,
                send_send_video_v2,
                send_destroy,
            }));
        }
    }

    None
}

/// Safe wrapper over dynamically loaded NDI Runtime v6 shared library.
pub struct DynamicNdiRuntime {
    vtable: Option<Arc<NdiVTable>>,
}

impl Default for DynamicNdiRuntime {
    fn default() -> Self {
        Self::probe()
    }
}

impl DynamicNdiRuntime {
    /// Attempts to dynamically locate and initialize the installed NDI Runtime v6 library.
    pub fn probe() -> Self {
        let vtable = try_load_ndi_vtable();
        if let Some(ref vt) = vtable {
            tracing::info!("Loaded NDI Runtime '{}' from {}", vt.version, vt.library_path);
        } else {
            tracing::info!(
                "NDI Runtime not found; synthetic Test Pattern sources remain fully operational"
            );
        }
        Self { vtable }
    }

    pub fn is_available(&self) -> bool {
        self.vtable.is_some()
    }

    /// Creates a real NDI video receiver bound to `source_name`.
    pub fn create_receiver(&self, source_name: &str, recv_name: &str) -> Result<NdiReceiver, VideoError> {
        let vt = self.vtable.as_ref().ok_or(VideoError::NdiRuntimeMissing)?;
        let create_fn = vt
            .recv_create_v3
            .ok_or_else(|| VideoError::Backend("NDIlib_recv_create_v3 missing".into()))?;
        let c_source = CString::new(source_name)
            .map_err(|e| VideoError::Backend(format!("invalid NDI source name: {e}")))?;
        let c_recv = CString::new(recv_name)
            .map_err(|e| VideoError::Backend(format!("invalid NDI receiver name: {e}")))?;
        let settings = NdiRecvCreateV3C {
            source_to_connect_to: NdiSourceC {
                p_ndi_name: c_source.as_ptr(),
                p_url_address: ptr::null(),
            },
            color_format: NDI_RECV_COLOR_FORMAT_RGBX_RGBA,
            bandwidth: NDI_RECV_BANDWIDTH_HIGHEST,
            allow_video_fields: false,
            p_ndi_recv_name: c_recv.as_ptr(),
        };
        let handle = unsafe { create_fn(&settings) };
        if handle.is_null() {
            return Err(VideoError::Backend(format!(
                "NDIlib_recv_create_v3 failed for source '{source_name}'"
            )));
        }
        Ok(NdiReceiver {
            vtable: Arc::clone(vt),
            handle,
            frame_counter: 0,
        })
    }

    /// Creates a real NDI video sender publishing under `stream_name`.
    pub fn create_sender(&self, stream_name: &str) -> Result<NdiSender, VideoError> {
        let vt = self.vtable.as_ref().ok_or(VideoError::NdiRuntimeMissing)?;
        let create_fn = vt
            .send_create
            .ok_or_else(|| VideoError::Backend("NDIlib_send_create missing".into()))?;
        let c_name = CString::new(stream_name)
            .map_err(|e| VideoError::Backend(format!("invalid NDI stream name: {e}")))?;
        let settings = NdiSendCreateC {
            p_ndi_name: c_name.as_ptr(),
            p_groups: ptr::null(),
            clock_video: true,
            clock_audio: false,
        };
        let handle = unsafe { create_fn(&settings) };
        if handle.is_null() {
            return Err(VideoError::Backend(format!(
                "NDIlib_send_create failed for stream '{stream_name}'"
            )));
        }
        Ok(NdiSender {
            vtable: Arc::clone(vt),
            handle,
        })
    }
}

impl NdiRuntimeLoader for DynamicNdiRuntime {
    fn probe_runtime(&self) -> NdiRuntimeStatus {
        match &self.vtable {
            Some(vt) => NdiRuntimeStatus {
                available: true,
                library_path: Some(vt.library_path.clone()),
                version: Some(vt.version.clone()),
                install_url: NDI_INSTALL_URL.to_string(),
            },
            None => NdiRuntimeStatus {
                available: false,
                library_path: None,
                version: None,
                install_url: NDI_INSTALL_URL.to_string(),
            },
        }
    }

    fn discover_sources(&self) -> Result<Vec<VideoSourceInfo>, VideoError> {
        let vt = self.vtable.as_ref().ok_or(VideoError::NdiRuntimeMissing)?;
        unsafe {
            let create_cfg = NdiFindCreateC {
                show_local_sources: true,
                p_groups: ptr::null(),
                p_extra_ips: ptr::null(),
            };
            let finder = (vt.find_create_v2)(&create_cfg);
            if finder.is_null() {
                return Err(VideoError::Backend("NDIlib_find_create_v2 returned null".into()));
            }
            let _ = (vt.find_wait_for_sources)(finder, 250);
            let mut count: u32 = 0;
            let sources_ptr = (vt.find_get_current_sources)(finder, &mut count);
            let mut results = Vec::with_capacity(count as usize);
            if !sources_ptr.is_null() && count > 0 {
                let slice = std::slice::from_raw_parts(sources_ptr, count as usize);
                for src in slice {
                    let name = if src.p_ndi_name.is_null() {
                        continue;
                    } else {
                        CStr::from_ptr(src.p_ndi_name).to_string_lossy().into_owned()
                    };
                    let url_address = if src.p_url_address.is_null() {
                        None
                    } else {
                        Some(CStr::from_ptr(src.p_url_address).to_string_lossy().into_owned())
                    };
                    results.push(VideoSourceInfo {
                        name,
                        url_address,
                        width: None,
                        height: None,
                        fps: None,
                    });
                }
            }
            (vt.find_destroy)(finder);
            Ok(results)
        }
    }
}

pub struct NdiReceiver {
    vtable: Arc<NdiVTable>,
    handle: *mut c_void,
    frame_counter: u64,
}

unsafe impl Send for NdiReceiver {}

impl NdiReceiver {
    pub fn capture_video(&mut self, source_id: Uuid, timeout_ms: u32) -> Result<Option<VideoFrame>, VideoError> {
        let capture_fn = self
            .vtable
            .recv_capture_v3
            .ok_or_else(|| VideoError::Backend("NDIlib_recv_capture_v3 missing".into()))?;
        let free_fn = self
            .vtable
            .recv_free_video_v2
            .ok_or_else(|| VideoError::Backend("NDIlib_recv_free_video_v2 missing".into()))?;

        let mut frame = NdiVideoFrameV2C::default();
        let kind = unsafe {
            capture_fn(
                self.handle,
                &mut frame,
                ptr::null_mut(),
                ptr::null_mut(),
                timeout_ms,
            )
        };
        if kind != NDI_FRAME_TYPE_VIDEO || frame.p_data.is_null() || frame.xres <= 0 || frame.yres <= 0
        {
            return Ok(None);
        }

        let width = frame.xres as u32;
        let height = frame.yres as u32;
        let format = match frame.fourcc {
            NDI_FOURCC_RGBA | NDI_FOURCC_RGBX => PixelFormat::Rgba8,
            NDI_FOURCC_BGRA | NDI_FOURCC_BGRX => PixelFormat::Bgra8,
            NDI_FOURCC_UYVY => PixelFormat::Uyvy,
            NDI_FOURCC_NV12 => PixelFormat::Nv12,
            _ => PixelFormat::Rgba8,
        };
        let stride = if frame.line_stride_in_bytes > 0 {
            frame.line_stride_in_bytes as usize
        } else {
            match format {
                PixelFormat::Uyvy => (width as usize) * 2,
                PixelFormat::Nv12 => width as usize,
                _ => (width as usize) * 4,
            }
        };
        let total_bytes = match format {
            PixelFormat::Nv12 => stride * (height as usize) * 3 / 2,
            _ => stride * (height as usize),
        };
        let data = unsafe { std::slice::from_raw_parts(frame.p_data, total_bytes).to_vec() };
        unsafe {
            free_fn(self.handle, &frame);
        }

        self.frame_counter += 1;
        Ok(Some(VideoFrame {
            meta: FrameRef {
                frame_id: self.frame_counter,
                width,
                height,
                format,
                source_id,
                timestamp_ns: kf_core::SignalEvent::now_ns(),
            },
            stride,
            data,
        }))
    }
}

impl Drop for NdiReceiver {
    fn drop(&mut self) {
        if let Some(destroy_fn) = self.vtable.recv_destroy {
            unsafe {
                destroy_fn(self.handle);
            }
        }
    }
}

pub struct NdiSender {
    vtable: Arc<NdiVTable>,
    handle: *mut c_void,
}

unsafe impl Send for NdiSender {}

impl NdiSender {
    /// Sends an RGBA8 frame out over NDI at the configured frame rate.
    pub fn send_rgba_frame(&self, frame: &VideoFrame, fps: f32) -> Result<(), VideoError> {
        let send_fn = self
            .vtable
            .send_send_video_v2
            .ok_or_else(|| VideoError::Backend("NDIlib_send_send_video_v2 missing".into()))?;
        let fps_n = (fps.clamp(1.0, 120.0) * 1000.0).round() as c_int;
        let c_frame = NdiVideoFrameV2C {
            xres: frame.meta.width as c_int,
            yres: frame.meta.height as c_int,
            fourcc: NDI_FOURCC_RGBA,
            frame_rate_n: fps_n,
            frame_rate_d: 1000,
            picture_aspect_ratio: frame.meta.width as f32 / frame.meta.height.max(1) as f32,
            frame_format_type: 1,
            timecode: 0,
            p_data: frame.data.as_ptr() as *mut u8,
            line_stride_in_bytes: frame.stride as c_int,
            p_metadata: ptr::null(),
            timestamp: 0,
        };
        unsafe {
            send_fn(self.handle, &c_frame);
        }
        Ok(())
    }
}

impl Drop for NdiSender {
    fn drop(&mut self) {
        if let Some(destroy_fn) = self.vtable.send_destroy {
            unsafe {
                destroy_fn(self.handle);
            }
        }
    }
}
