pub mod backend;
pub mod descriptor;
pub mod learn;

pub use backend::{
    fingerprint_from_hid_info, CompositeHidBackend, HidBackend, HidHotplugWatcher,
    SimulatedHidAction, SimulatedHidBackend, SimulatedHidControlState, SimulatedHidProfile,
};
#[cfg(feature = "native-hid")]
pub use backend::HidApiBackend;
pub use descriptor::{
    hid_usage_name, parse_hid_report_descriptor, sample_studio_fader_descriptor,
    DescriptorParseError, HidDescriptorField, ParsedHidDescriptor,
};
pub use learn::{
    compute_baseline_profile, BaselineProfile, DevicePresetLibrary, HidControlType,
    HidDeviceDefinition, HidParameterSynthesizer, LearnWizardSnapshot, LearnedControlSpec,
    TwoTierHidLearnEngine,
};
