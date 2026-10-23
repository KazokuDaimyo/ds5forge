//! Commands called by the interface. Each one targets a controller by its identifier.
//! Persistent settings go through the (saved) profile and are then sent to the controller.

use crate::bindings::{self, AnalogBinding, Rule};
use crate::controller::Command;
use crate::diagnostics::{self, AppInfo, DriverState, Report};
use crate::hidhide::CloakStatus;
use crate::i18n::{Lang, Msg};
use crate::input::InputConfig;
use crate::keys::{self, KeyInfo};
use crate::lightbar::{LightbarMode, ModeColors};
use crate::manager::{ControllerInfo, Controllers, PresetsInfo, Settings};
use crate::motion::MotionConfig;
use crate::triggers::TriggerEffect;
use crate::udp::UdpStatus;
use crate::virtual_pad;
use serde::Deserialize;
use tauri::{AppHandle, Manager, State};

/// Maximum number of remapped buttons accepted: there are only 19 buttons.
const MAX_REMAPS: usize = 64;

#[tauri::command]
pub fn list_controllers(controllers: State<Controllers>) -> Vec<ControllerInfo> {
    controllers.list()
}

/// Chooses the colour the controller is drawn in (a colourway identifier; `null` for the default).
#[tauri::command]
pub fn set_controller_color(controllers: State<Controllers>, id: String, color: Option<String>) {
    controllers.set_controller_color(&id, color);
}

#[tauri::command]
pub fn rename_controller(controllers: State<Controllers>, id: String, name: String) {
    controllers.edit(&id, |p| p.name = name);
}

#[tauri::command]
pub fn set_lightbar(controllers: State<Controllers>, id: String, r: u8, g: u8, b: u8) {
    controllers.edit(&id, |p| p.lightbar = [r, g, b]);
    controllers.send(&id, Command::Lightbar(r, g, b));
}

#[tauri::command]
pub fn set_player_leds(controllers: State<Controllers>, id: String, mask: u8) {
    let mask = mask & 0x1F;
    controllers.edit(&id, |p| p.player_leds = mask);
    controllers.send(&id, Command::PlayerLeds(mask));
}

/// Vibration is momentary: it is not saved in the profile.
#[tauri::command]
pub fn set_rumble(controllers: State<Controllers>, id: String, left: u8, right: u8) {
    controllers.send(&id, Command::Rumble { left, right });
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Left,
    Right,
}

#[tauri::command]
pub fn set_trigger(controllers: State<Controllers>, id: String, side: Side, effect: TriggerEffect) {
    let right = matches!(side, Side::Right);
    controllers.edit(&id, |p| {
        if right {
            p.trigger_right = Some(effect);
        } else {
            p.trigger_left = Some(effect);
        }
    });
    controllers.send(&id, Command::Trigger { right, effect: effect.encode() });
}

#[tauri::command]
pub fn set_virtual_pad(controllers: State<Controllers>, id: String, enabled: bool) {
    controllers.edit(&id, |p| p.virtual_pad = enabled);
    controllers.send(&id, Command::VirtualPad(enabled));
}

#[tauri::command]
pub fn get_udp_status(status: State<UdpStatus>) -> UdpStatus {
    status.inner().clone()
}

/// Powers the controller off (Bluetooth only). The error is shown as is in the interface.
#[tauri::command]
pub fn power_off_controller(controllers: State<Controllers>, id: String) -> Result<(), String> {
    controllers.power_off(&id)
}

/// Saves the rules of a controller. Values coming from the interface are brought back to safe limits.
#[tauri::command]
pub fn set_rules(controllers: State<Controllers>, id: String, rules: Vec<Rule>) {
    let rules = bindings::sanitize_rules(rules);
    controllers.edit(&id, |p| p.rules = Some(rules));
    controllers.send(&id, Command::ProfileChanged);
}

/// Saves the sticks that drive the mouse, the wheel or keyboard keys.
#[tauri::command]
pub fn set_analog(controllers: State<Controllers>, id: String, analog: Vec<AnalogBinding>) {
    let analog = bindings::sanitize_analog(analog);
    controllers.edit(&id, |p| p.analog = analog);
    controllers.send(&id, Command::ProfileChanged);
}

/// Tells whether the window is in the foreground (a game profile chosen by hand only applies then).
#[tauri::command]
pub fn set_ui_focus(controllers: State<Controllers>, focused: bool) {
    controllers.set_ui_focus(focused);
}

/// The profiles and the game rules.
#[tauri::command]
pub fn get_presets(controllers: State<Controllers>) -> PresetsInfo {
    controllers.presets_info()
}

#[tauri::command]
pub fn set_controller_preset(controllers: State<Controllers>, id: String, preset: String) -> Result<(), String> {
    controllers.set_controller_preset(&id, &preset)
}

/// Creates a game profile (a copy of `from`) for the program `exe`. Returns its name.
#[tauri::command]
pub fn create_game_profile(controllers: State<Controllers>, name: String, exe: String, from: String) -> Result<String, String> {
    controllers.create_game_profile(&name, &exe, &from)
}

#[tauri::command]
pub fn rename_preset(controllers: State<Controllers>, old: String, new: String) -> Result<String, String> {
    controllers.rename_preset(&old, &new)
}

#[tauri::command]
pub fn delete_preset(controllers: State<Controllers>, name: String) -> Result<(), String> {
    controllers.delete_preset(&name)
}

/// The programs that have a visible window, to pick a game from a list. Asynchronous: it queries
/// every window and process.
#[tauri::command]
pub async fn list_running_apps() -> Vec<String> {
    crate::games::running_apps()
}

/// The keyboard keys that rules can send, labelled in the current language.
#[tauri::command]
pub fn list_keys() -> Vec<KeyInfo> {
    keys::list()
}

#[tauri::command]
pub fn get_settings(controllers: State<Controllers>) -> Settings {
    controllers.settings()
}

/// The setting is saved even if hiding fails; the error is returned to the interface.
/// Asynchronous: running HidHideCLI takes time and must not freeze the window.
#[tauri::command]
pub async fn set_hidhide_auto(controllers: State<'_, Controllers>, enabled: bool) -> Result<(), String> {
    controllers.set_hidhide_auto(enabled)
}

/// The state of HidHide, read-only.
#[tauri::command]
pub async fn hidhide_status(controllers: State<'_, Controllers>) -> Result<CloakStatus, String> {
    Ok(controllers.cloak_status())
}

/// Whether the open controller menu takes the focus away from the game.
#[tauri::command]
pub fn set_menu_focus(controllers: State<Controllers>, enabled: bool) {
    controllers.set_menu_focus(enabled);
}

#[tauri::command]
pub fn set_notifications(controllers: State<Controllers>, low_battery_alert: bool, haptic_feedback: bool) {
    controllers.set_notifications(low_battery_alert, haptic_feedback);
}

/// Changes the language of the messages produced by the backend and of the tray menu. The
/// interface itself is translated by the front end, which reloads after calling this.
#[tauri::command]
pub fn set_language(app: AppHandle, controllers: State<Controllers>, language: Lang) {
    controllers.set_language(language);
    crate::refresh_tray_labels(&app);
}

#[tauri::command]
pub fn set_lightbar_mode(controllers: State<Controllers>, id: String, mode: LightbarMode, xbox: [u8; 3], native: [u8; 3]) {
    controllers.edit(&id, |p| {
        p.lightbar_mode = mode;
        p.mode_colors = ModeColors { xbox, native };
    });
    controllers.send(&id, Command::ProfileChanged);
}

#[tauri::command]
pub fn set_input_config(controllers: State<Controllers>, id: String, mut input: InputConfig) {
    input.remap.truncate(MAX_REMAPS);
    controllers.edit(&id, |p| p.input = input);
    controllers.send(&id, Command::ProfileChanged);
}

#[tauri::command]
pub fn set_motion_config(controllers: State<Controllers>, id: String, motion: MotionConfig) {
    controllers.edit(&id, |p| p.motion = motion);
    controllers.send(&id, Command::ProfileChanged);
}

/// The full report for the diagnostics screen. Asynchronous: it queries HidHide.
#[tauri::command]
pub async fn get_diagnostics(
    app: AppHandle,
    controllers: State<'_, Controllers>,
    udp: State<'_, UdpStatus>,
) -> Result<Report, String> {
    Ok(Report {
        app: AppInfo {
            version: env!("CARGO_PKG_VERSION").to_string(),
            exe: std::env::current_exe().ok().map(|p| p.display().to_string()),
            config_dir: app.path().app_config_dir().ok().map(|p| p.display().to_string()),
            elevated: diagnostics::is_elevated(),
        },
        vigem: match virtual_pad::driver_check() {
            Ok(()) => DriverState { ok: true, message: None },
            Err(message) => DriverState { ok: false, message: Some(message) },
        },
        hidhide: controllers.cloak_status(),
        settings: controllers.settings(),
        controllers: controllers.diagnostics_controllers(),
        udp: udp.inner().clone(),
        log: controllers.log().entries(),
    })
}

#[tauri::command]
pub fn clear_log(controllers: State<Controllers>) {
    controllers.log().clear();
}

/// Which optional components are installed. HidHide is only usable (`hidhide_ready`) once the
/// application has been restarted after its installation, because it is looked for at start-up.
#[derive(serde::Serialize)]
pub struct ComponentsStatus {
    pub vigembus: bool,
    pub hidhide: bool,
    pub hidhide_ready: bool,
}

#[tauri::command]
pub async fn components_status(controllers: State<'_, Controllers>) -> Result<ComponentsStatus, String> {
    Ok(ComponentsStatus {
        vigembus: virtual_pad::driver_check().is_ok(),
        hidhide: crate::hidhide::is_installed(),
        hidhide_ready: controllers.cloak_status().installed,
    })
}

/// Installs an optional component (`"vigembus"` or `"hidhide"`) through winget, which asks Windows
/// for administrator permission. Asynchronous: the installer takes a while.
#[tauri::command]
pub async fn install_component(app: AppHandle, component: String) -> Result<crate::install::Outcome, String> {
    let component = crate::install::Component::parse(&component).ok_or_else(|| Msg::UnknownComponent.text())?;
    tauri::async_runtime::spawn_blocking(move || {
        crate::install::install(component, |url| {
            use tauri_plugin_opener::OpenerExt;
            app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The result of an update check, for the interface.
#[derive(serde::Serialize)]
pub struct UpdateInfo {
    /// The version that is running.
    pub current: String,
    /// The newer version published on GitHub, if there is one.
    pub available: Option<AvailableUpdate>,
}

#[derive(serde::Serialize)]
pub struct AvailableUpdate {
    pub version: String,
    pub notes: Option<String>,
}

/// Asks GitHub whether a newer version has been released (a single request; nothing is installed).
#[tauri::command]
pub async fn check_update(app: AppHandle) -> Result<UpdateInfo, String> {
    use tauri_plugin_updater::UpdaterExt;
    let current = app.package_info().version.to_string();
    let update = app.updater().map_err(|e| e.to_string())?.check().await.map_err(|e| e.to_string())?;
    Ok(UpdateInfo {
        current,
        available: update.map(|u| AvailableUpdate { version: u.version, notes: u.body }),
    })
}

/// Downloads the newer version, checks its signature against the public key in the configuration,
/// and runs the installer, which restarts the application. On Windows the process ends as soon as the
/// installer starts, so the controllers are released first (rumble stopped, profiles saved,
/// HidHide entries removed).
#[tauri::command]
pub async fn install_update(app: AppHandle, controllers: State<'_, Controllers>) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;
    let update = app
        .updater()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| Msg::NoUpdateAvailable.text())?;
    let bytes = update.download(|_, _| {}, || {}).await.map_err(|e| e.to_string())?;
    controllers.shutdown();
    update.install(bytes).map_err(|e| e.to_string())
}

/// Restarts the application (after installing HidHide, which is looked for at start-up).
#[tauri::command]
pub fn restart_app(app: AppHandle, controllers: State<Controllers>) {
    controllers.shutdown();
    app.restart();
}

/// Really quits the application (the window's close button only minimises it).
#[tauri::command]
pub fn quit_app(app: AppHandle) {
    app.exit(0);
}

#[cfg(windows)]
#[tauri::command]
pub fn get_autostart(app: AppHandle) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}

#[cfg(windows)]
#[tauri::command]
pub fn set_autostart(app: AppHandle, enabled: bool) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt;
    let launcher = app.autolaunch();
    if enabled { launcher.enable() } else { launcher.disable() }.map_err(|e| e.to_string())
}

#[cfg(not(windows))]
#[tauri::command]
pub fn get_autostart() -> Result<bool, String> {
    Err(Msg::AutostartWindowsOnly.text())
}

#[cfg(not(windows))]
#[tauri::command]
pub fn set_autostart(_enabled: bool) -> Result<(), String> {
    Err(Msg::AutostartWindowsOnly.text())
}
