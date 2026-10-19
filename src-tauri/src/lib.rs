// Only `lib.rs` and `commands.rs` know about Tauri. They are excluded from the tests: Tauri's
// graphical engine does not load in a test binary without an application manifest
// (STATUS_ENTRYPOINT_NOT_FOUND), and the tested logic does not depend on it (see `events.rs`).
#![cfg_attr(test, allow(dead_code))]

mod alerts;
mod bindings;
mod bluetooth;
#[cfg(not(test))]
mod commands;
mod controller;
mod desktop;
mod diagnostics;
mod events;
mod haptics;
mod hidhide;
mod i18n;
mod input;
mod install;
mod keys;
mod lightbar;
mod manager;
mod motion;
mod mouse;
#[cfg(not(test))]
mod overlay;
mod shortcuts;
mod triggers;
mod udp;
mod virtual_pad;

#[cfg(not(test))]
use events::Events;
#[cfg(not(test))]
use i18n::Msg;
#[cfg(not(test))]
use manager::Controllers;
#[cfg(not(test))]
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, RunEvent, WindowEvent, Wry,
};

/// Main window, shown again from the notification area or by a second launch.
#[cfg(not(test))]
fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// The notification-area menu items, kept so that their labels can follow the interface language.
#[cfg(not(test))]
struct TrayMenu {
    open: MenuItem<Wry>,
    quit: MenuItem<Wry>,
}

/// Applies the current language to the notification-area menu.
#[cfg(not(test))]
pub fn refresh_tray_labels(app: &AppHandle) {
    if let Some(menu) = app.try_state::<TrayMenu>() {
        let _ = menu.open.set_text(Msg::TrayOpen.text());
        let _ = menu.quit.set_text(Msg::TrayQuit.text());
    }
}

/// Notification-area icon: left click to open, menu to quit.
#[cfg(not(test))]
fn create_tray(app: &tauri::App) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "show", Msg::TrayOpen.text(), true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", Msg::TrayQuit.text(), true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    app.manage(TrayMenu { open, quit });

    let mut tray = TrayIconBuilder::new()
        .tooltip("DS5Forge")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

/// Windows-only plugins: single instance and launch at startup.
#[cfg(all(not(test), windows))]
fn windows_plugins(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    builder
        // Must be the first plugin: a second launch wakes up the existing window.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_main_window(app)))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--minimized"]),
        ))
}

#[cfg(all(not(test), not(windows)))]
fn windows_plugins(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    builder
}

#[cfg(not(test))]
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    windows_plugins(tauri::Builder::default())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let file = app.path().app_config_dir().ok().map(|dir| dir.join("profiles.json"));
            let handle = app.handle().clone();
            app.manage(overlay::OverlayState::default());
            overlay::create(app.handle())?;

            // "overlay" opens the overlay window; every other event goes to the interface.
            let events = Events::new(move |name, payload| {
                if name == "overlay" {
                    overlay::show(&handle, payload);
                } else {
                    let _ = handle.emit(name, payload);
                }
            });
            // Created before the tray: loading the settings restores the saved language, which the
            // tray labels use.
            let controllers = Controllers::new(events.clone(), file).with_cloak(hidhide::detect());
            create_tray(app)?;

            manager::start(controllers.clone());
            let udp_status = udp::start(events, controllers.clone());

            app.manage(udp_status);
            app.manage(controllers);

            // When launched by Windows at startup (`--minimized`), the application stays in the
            // notification area.
            if !std::env::args().any(|arg| arg == "--minimized") {
                show_main_window(app.handle());
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_controllers,
            commands::rename_controller,
            commands::set_lightbar,
            commands::set_lightbar_mode,
            commands::set_player_leds,
            commands::set_rumble,
            commands::set_trigger,
            commands::set_virtual_pad,
            commands::set_input_config,
            commands::set_motion_config,
            commands::get_udp_status,
            commands::power_off_controller,
            commands::set_rules,
            commands::set_analog,
            commands::list_keys,
            commands::preview_overlay,
            commands::get_settings,
            commands::set_notifications,
            commands::set_language,
            commands::set_hidhide_auto,
            commands::hidhide_status,
            commands::components_status,
            commands::install_component,
            commands::restart_app,
            commands::check_update,
            commands::install_update,
            commands::get_diagnostics,
            commands::clear_log,
            commands::get_autostart,
            commands::set_autostart,
            commands::quit_app,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // The close button minimises the application to the notification area instead of
            // closing it, so that virtual controllers keep working. It is closed for good from the
            // tray menu or the "Quit" button.
            if let RunEvent::WindowEvent { label, event: WindowEvent::CloseRequested { api, .. }, .. } = &event {
                if label == "main" {
                    api.prevent_close();
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.hide();
                    }
                }
            }
            // On exit: stop the vibrations, unhide the controllers and save the profiles.
            if let RunEvent::Exit = event {
                if let Some(controllers) = app.try_state::<Controllers>() {
                    controllers.shutdown();
                }
            }
        });
}
