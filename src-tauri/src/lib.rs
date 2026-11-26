pub mod commands;
pub mod commands_m3;
pub mod state;

use state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();

    tauri::Builder::default()
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            commands::get_workspace,
            commands::add_device,
            commands::remove_device,
            commands::update_device_position,
            commands::rename_device,
            commands::rename_port,
            commands::add_port,
            commands::remove_port,
            commands::update_device_kind,
            commands::create_rack,
            commands::ungroup_rack,
            commands::rename_rack,
            commands::set_rack_collapsed,
            commands::expose_macro,
            commands::remove_macro,
            commands::set_macro_value,
            commands::add_wire,
            commands::remove_wire,
            commands::toggle_wire,
            commands::update_wire_modifiers,
            commands::compute_transfer_curve,
            commands::inject_signal,
            commands::set_manual_override,
            commands::start_protocol_endpoint,
            commands::stop_protocol_endpoint,
            commands::get_protocol_statuses,
            commands::list_midi_ports,
            commands::get_virtual_midi_guidance,
            commands::export_project_json,
            commands::import_project_json,
            commands::save_project_file,
            commands::load_project_file,
            commands::export_device_preset,
            commands::import_device_preset,
            commands::export_group_preset,
            commands::import_group_preset,
            commands::export_mapping_preset,
            commands::import_mapping_preset,
            commands::set_device_status,
            commands::rebind_offline_device,
            commands::update_settings,
            commands::set_calm_mode,
            commands::update_telemetry_subscription,
            commands::subscribe_telemetry,
            commands::get_extension_capabilities,
            // M3: Hardware HID Hot-Plug, Learn Wizard & RGB Camera CV
            commands_m3::m3_list_hid_devices,
            commands_m3::m3_check_hid_permissions,
            commands_m3::m3_poll_hid_hotplug,
            commands_m3::m3_simulate_hid_plug,
            commands_m3::m3_simulate_hid_unplug,
            commands_m3::m3_simulate_hid_actuate,
            commands_m3::m3_parse_hid_descriptor,
            commands_m3::m3_learn_start_baseline,
            commands_m3::m3_learn_poll_samples,
            commands_m3::m3_learn_start_capture,
            commands_m3::m3_learn_classify,
            commands_m3::m3_learn_commit_control,
            commands_m3::m3_learn_get_snapshot,
            commands_m3::m3_learn_reset,
            commands_m3::m3_save_and_attach_learned_device,
            commands_m3::m3_export_hid_device_preset,
            commands_m3::m3_import_hid_device_preset,
            commands_m3::m3_list_device_presets,
            commands_m3::m3_list_rgb_cameras,
            commands_m3::m3_update_rgb_cv_config,
            commands_m3::m3_get_rgb_cv_config,
            commands_m3::m3_step_rgb_camera_cv,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Kontrolfreak Tauri application");
}
