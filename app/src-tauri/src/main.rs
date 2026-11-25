// Release builds on Windows open no console window.
#![cfg_attr(all(not(debug_assertions), not(test)), windows_subsystem = "windows")]

mod commands;
mod protocol;

use tauri::Manager;

fn main() {
    // Read while the process still has one thread: later reads can fail on macOS and Linux.
    let startup_offset = time::UtcOffset::current_local_offset().ok();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(move |app| {
            app.manage(commands::Shell::open(startup_offset)?);
            Ok(())
        })
        .register_asynchronous_uri_scheme_protocol("puw", protocol::handle)
        .invoke_handler(tauri::generate_handler![
            commands::app_status,
            commands::download_models,
            commands::cancel_job,
            commands::remove_downloaded_data,
            commands::add_paths,
            commands::remove_photo,
            commands::clear_photos,
            commands::list_photos,
            commands::suggest_saved,
            commands::detect,
            commands::use_saved,
            commands::edit_size,
            commands::get_area,
            commands::put_draft,
            commands::confirm_set_area,
            commands::match_all,
            commands::photo_states,
            commands::open_photo,
            commands::save_photo_area,
            commands::set_included,
            commands::removal_blockers,
            commands::preview,
            commands::check_output,
            commands::set_output_override,
            commands::remove,
            commands::save_watermark,
            commands::default_watermark_name,
            commands::list_saved,
            commands::rename_saved,
            commands::delete_saved,
            commands::get_settings,
            commands::set_settings,
        ])
        .run(tauri::generate_context!())
        .expect("error while running PhotoUnwatermark");
}
