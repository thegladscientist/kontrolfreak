//! Kontrolfreak Depth Camera Sidecar library & 3D parameter extractor crate.
//!
//! Provides:
//! - Pluggable `DepthCameraBackend` with RealSense D400, Orbbec Femto/Astra, Azure Kinect
//!   feature-gated adapters and a default `SyntheticDepthBackend`
//! - Real-time multi-ROI 3D depth parameter extractors (`extract_roi`, `extract_all_rois`)
//! - Depth-to-RGBA / Depth-to-NDI colormap bridge (`Turbo`, `Viridis`, `Grayscale`, `DepthSlice`)

pub mod backend;
pub mod colormap;
pub mod extractor;

pub use backend::{
    create_backend, list_vendor_adapters, AzureKinectBackend, BackendError, DepthCameraBackend,
    DepthFrame, OrbbecBackend, RealSenseBackend, SyntheticDepthBackend, VendorAdapterInfo,
};
pub use colormap::{build_depth_ndi_frame_ref, colorize_depth_to_rgba, turbo_rgb, viridis_rgb};
pub use extractor::{
    extract_all_rois, extract_roi, roi_to_signal_bindings, to_depth_sample_summary,
    CameraIntrinsics,
};
