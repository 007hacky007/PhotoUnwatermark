// Release builds on Windows open no console window.
#![cfg_attr(all(not(debug_assertions), not(test)), windows_subsystem = "windows")]

mod commands;
mod protocol;
mod window;

use tauri::{Emitter, Manager, RunEvent};

/// What the window hears when the app is asked to quit during a removal.
const QUIT_REQUESTED: &str = "quit-requested";

/// The app menu's Quit, which goes through [`RunEvent::ExitRequested`] (the standard item
/// ends the app at once).
#[cfg(target_os = "macos")]
const QUIT_MENU_ID: &str = "quit";

/// The standard Mac menu with its Quit item replaced by one the app can hold back.
#[cfg(target_os = "macos")]
fn mac_menu(app: &tauri::AppHandle) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    use tauri::menu::{Menu, MenuItem};
    let menu = Menu::default(app)?;
    let Some(app_menu) = menu
        .items()?
        .into_iter()
        .next()
        .and_then(|i| i.as_submenu().cloned())
    else {
        return Ok(menu);
    };
    let items = app_menu.items()?;
    let quit = items.iter().enumerate().rev().find_map(|(at, item)| {
        let text = item.as_predefined_menuitem()?.text().ok()?;
        text.starts_with("Quit").then_some((at, text))
    });
    if let Some((at, text)) = quit {
        app_menu.remove_at(at)?;
        let item = MenuItem::with_id(app, QUIT_MENU_ID, text, true, Some("CmdOrCtrl+Q"))?;
        app_menu.insert(&item, at)?;
    }
    Ok(menu)
}

/// Shrinks the main window to the screen it opens on (keeping the usual size where it fits)
/// and centres it, so its bottom never sits under the taskbar. Nothing changes when the screen
/// cannot be read.
fn fit_main_window(app: &tauri::App) {
    use tauri::{LogicalSize, PhysicalPosition};
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    let Ok(Some(monitor)) = win.current_monitor() else {
        return;
    };
    let area = monitor.work_area();
    let frame = match (win.outer_size(), win.inner_size()) {
        (Ok(o), Ok(i)) => (
            o.width.saturating_sub(i.width),
            o.height.saturating_sub(i.height),
        ),
        _ => (0, 0),
    };
    let f = window::fit(
        (
            area.position.x,
            area.position.y,
            area.size.width,
            area.size.height,
        ),
        frame,
        monitor.scale_factor(),
    );
    let _ = win.set_min_size(Some(LogicalSize::new(f.min.0, f.min.1)));
    let _ = win.set_size(LogicalSize::new(f.size.0, f.size.1));
    let _ = win.set_position(PhysicalPosition::new(f.position.0, f.position.1));
}

fn main() {
    // Read while the process still has one thread: later reads can fail on macOS and Linux.
    let startup_offset = time::UtcOffset::current_local_offset().ok();
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init());
    #[cfg(target_os = "macos")]
    let builder = builder.menu(mac_menu).on_menu_event(|app, event| {
        if event.id() == QUIT_MENU_ID {
            app.exit(0);
        }
    });
    builder
        .setup(move |app| {
            app.manage(commands::Shell::open(startup_offset)?);
            fit_main_window(app);
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
            commands::drop_draft,
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
            commands::device_status,
            commands::device_fix,
        ])
        .build(tauri::generate_context!())
        .expect("error while running PhotoUnwatermark")
        .run(|app, event| {
            if let RunEvent::ExitRequested { code, api, .. } = event {
                let removing = app
                    .try_state::<commands::Shell>()
                    .is_some_and(|s| s.removing());
                if commands::hold_exit(code, removing) {
                    api.prevent_exit();
                    let _ = app.emit(QUIT_REQUESTED, ());
                }
            }
        });
}
