pub mod commands;
pub mod commands_m2;
pub mod commands_m3;
pub mod commands_m4;
pub mod commands_simple;
pub mod simple_output;
pub mod state;

use state::AppState;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();

    tauri::Builder::default()
        .manage(AppState::new())
        .setup(|app| {
            app.state::<AppState>().simple.start();
            Ok(())
        })
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
            // Milestone 2: RTPMIDI & NDI Video Pipeline
            commands_m2::m2_get_ndi_runtime_status,
            commands_m2::m2_discover_ndi_sources,
            commands_m2::m2_get_ndi_node_state,
            commands_m2::m2_configure_ndi_node,
            commands_m2::m2_get_ndi_preview_frame,
            commands_m2::m2_start_rtpmidi_session,
            commands_m2::m2_stop_rtpmidi_session,
            commands_m2::m2_get_rtpmidi_status,
            commands_m2::m2_discover_rtpmidi_peers,
            commands_m2::m2_connect_rtpmidi_peer,
            commands_m2::m2_sync_rtpmidi_clock,
            commands_m2::m2_send_rtpmidi_test_midi,
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
            // Milestone 4: Depth Sidecars, .kfreak Bundles & Diagnostics Profiler
            commands_m4::m4_list_depth_vendors,
            commands_m4::m4_start_depth_sidecar,
            commands_m4::m4_stop_depth_sidecar,
            commands_m4::m4_restart_depth_sidecar,
            commands_m4::m4_simulate_sidecar_crash,
            commands_m4::m4_get_depth_sidecar_status,
            commands_m4::m4_list_depth_sidecars,
            commands_m4::m4_configure_depth_rois,
            commands_m4::m4_set_depth_colormap,
            commands_m4::m4_poll_depth_preview,
            commands_m4::m4_export_kfreak_bundle,
            commands_m4::m4_export_kfreak_file,
            commands_m4::m4_import_kfreak_bundle,
            commands_m4::m4_import_kfreak_file,
            commands_m4::m4_get_diagnostics_snapshot,
            commands_simple::simple_poll_sources,
            commands_simple::simple_set_osc_port,
            commands_simple::simple_select_source,
            commands_simple::simple_get_settings,
            commands_simple::simple_set_settings,
            commands_simple::simple_preview,
            commands_simple::simple_host_info,
            commands_simple::simple_list_displays,
            commands_simple::simple_open_output,
            commands_simple::simple_set_output_fullscreen,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Kontrolfreak Tauri application");
}
