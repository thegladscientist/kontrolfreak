pub mod backend;
pub mod cv;

pub use backend::{
    render_synthetic_frame, RgbCameraBackend, RgbCameraInfo, RgbFrame, SyntheticRgbCamera,
    SyntheticSceneParams,
};
#[cfg(all(
    feature = "native-camera",
    any(target_os = "windows", target_os = "macos", target_os = "linux")
))]
pub use backend::NokhwaCameraBackend;
pub use cv::{
    rgb_to_hsv, MotionZoneReading, NormalizedRoi, RgbCvConfig, RgbCvPipeline, RgbCvResult,
};
