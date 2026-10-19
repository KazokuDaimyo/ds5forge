//! Management of several controllers: detection, saved profiles and command routing.
//!
//! Each controller is identified by its MAC address. Its profile (name, colour, triggers…) is
//! saved to disk and reapplied on every connection.

use crate::bindings::{rules_from_shortcuts, AnalogBinding, Rule};
use crate::bluetooth;
use crate::controller::{self, Charge, Command, DeviceCtx};
use crate::diagnostics::{ControllerDiag, Log};
use crate::events::Events;
use crate::hidhide::{Cloak, CloakStatus, NoCloak};
use crate::i18n::{self, Lang, Msg};
use crate::input::InputConfig;
use crate::lightbar::{LightbarMode, ModeColors};
use crate::motion::MotionConfig;
use crate::shortcuts::{default_shortcuts, Shortcut};
use crate::triggers::TriggerEffect;
use hidapi::HidApi;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

/// Light bar colours given to new controllers, so that they can be told apart right away.
const PALETTE: [[u8; 3]; 5] = [
    [79, 140, 255],
    [255, 90, 90],
    [62, 207, 142],
    [255, 200, 60],
    [190, 110, 255],
];

const SCAN_INTERVAL: Duration = Duration::from_millis(1500);
const MAX_NAME_LEN: usize = 32;

/// Settings remembered for one controller.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Profile {
    pub name: String,
    pub lightbar: [u8; 3],
    pub player_leds: u8,
    /// `None`: the user has not set anything, so the trigger is left alone.
    pub trigger_left: Option<TriggerEffect>,
    pub trigger_right: Option<TriggerEffect>,
    pub virtual_pad: bool,
    /// Legacy shortcuts, from before rules existed. They are only used to derive the rules of a
    /// profile that has none yet (see `rules`).
    pub shortcuts: Vec<Shortcut>,
    /// Rules (advanced remapping, macros, layers…). `None`: the profile was saved before rules
    /// existed; they are then derived from the legacy shortcuts and saved on the first edit.
    pub rules: Option<Vec<Rule>>,
    /// Sticks that drive the mouse, the wheel or keys.
    pub analog: Vec<AnalogBinding>,
    /// How the light bar colour is chosen; `lightbar` is the colour of the fixed mode.
    pub lightbar_mode: LightbarMode,
    pub mode_colors: ModeColors,
    /// Button remapping, dead zones and stick curves.
    pub input: InputConfig,
    /// Gyroscope and touchpad.
    pub motion: MotionConfig,
}

/// What a controller's thread reads again when its profile changes.
#[derive(Clone, Debug, PartialEq)]
pub struct LiveConfig {
    pub rules: Vec<Rule>,
    pub analog: Vec<AnalogBinding>,
    pub lightbar_mode: LightbarMode,
    pub mode_colors: ModeColors,
    /// Colour of the fixed mode.
    pub fixed_color: [u8; 3],
    pub input: InputConfig,
    pub motion: MotionConfig,
}

impl Profile {
    /// The profile as the interface receives it: the rules of a profile saved before rules existed
    /// are already derived from its legacy shortcuts, so that the editor can show and edit them.
    fn with_rules(&self) -> Profile {
        let mut profile = self.clone();
        profile.rules = Some(self.live().rules);
        profile
    }

    fn live(&self) -> LiveConfig {
        LiveConfig {
            rules: self.rules.clone().unwrap_or_else(|| rules_from_shortcuts(&self.shortcuts)),
            analog: self.analog.clone(),
            lightbar_mode: self.lightbar_mode,
            mode_colors: self.mode_colors,
            fixed_color: self.lightbar,
            input: self.input.clone(),
            motion: self.motion,
        }
    }
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            name: String::new(),
            lightbar: PALETTE[0],
            player_leds: 0,
            trigger_left: None,
            trigger_right: None,
            virtual_pad: false,
            shortcuts: default_shortcuts(),
            rules: None,
            analog: Vec::new(),
            lightbar_mode: LightbarMode::default(),
            mode_colors: ModeColors::default(),
            input: InputConfig::default(),
            motion: MotionConfig::default(),
        }
    }
}

impl Profile {
    /// The commands that apply this profile. With `reset`, a trigger that was never set is reset
    /// to off, to clear whatever a game may have sent in the meantime.
    pub fn commands(&self, reset: bool) -> Vec<Command> {
        let [r, g, b] = self.lightbar;
        let mut cmds = vec![Command::Lightbar(r, g, b), Command::PlayerLeds(self.player_leds)];
        for (right, effect) in [(false, self.trigger_left), (true, self.trigger_right)] {
            let effect = match (effect, reset) {
                (Some(e), _) => Some(e),
                (None, true) => Some(TriggerEffect::Off),
                (None, false) => None,
            };
            if let Some(e) = effect {
                cmds.push(Command::Trigger { right, effect: e.encode() });
            }
        }
        cmds.push(Command::VirtualPad(self.virtual_pad));
        cmds
    }
}

/// Settings shared by all controllers (saved in `settings.json`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Settings {
    /// Hide the physical controller from games (HidHide) while its Xbox 360 mode is active.
    pub hidhide_auto: bool,
    /// Show the overlay when a controller's battery gets low.
    pub low_battery_alert: bool,
    /// Vibrate the controller to confirm a mode change or signal an alert.
    pub haptic_feedback: bool,
    /// Interface language chosen by the user. `None`: not chosen yet, the interface picks one
    /// from the system language.
    pub language: Option<Lang>,
}

impl Default for Settings {
    fn default() -> Self {
        Self { hidhide_auto: false, low_battery_alert: true, haptic_feedback: true, language: None }
    }
}

/// Designates one or several controllers in the UDP API.
#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(untagged)]
pub enum Selector {
    /// Position in the list of connected controllers, sorted by identifier (starting at 0).
    Index(usize),
    /// `"all"`, an identifier (MAC address) or the name of the profile.
    Name(String),
}

/// A controller as it appears in the overlay.
#[derive(Serialize, Clone, Debug)]
pub struct OverlayController {
    pub id: String,
    pub name: String,
    pub color: [u8; 3],
    pub bluetooth: bool,
    pub percent: Option<u8>,
    pub charge: Charge,
    /// `true`: Xbox 360 mode; `false`: native mode.
    pub xbox_mode: bool,
    /// The controller whose shortcut triggered the overlay.
    pub source: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct OverlayPayload {
    /// One-off message ("Xbox 360 mode"…), shown above the list.
    pub message: Option<String>,
    pub controllers: Vec<OverlayController>,
}

#[derive(Serialize, Clone, Debug)]
pub struct ControllerInfo {
    pub id: String,
    pub connected: bool,
    pub bluetooth: bool,
    pub profile: Profile,
}

struct Active {
    tx: Sender<Command>,
    bluetooth: bool,
    path: String,
    battery: Option<u8>,
    charge: Charge,
    /// Input reports received during the last second.
    report_rate: u32,
}

struct Inner {
    profiles: BTreeMap<String, Profile>,
    active: BTreeMap<String, Active>,
    file: Option<PathBuf>,
    dirty: bool,
    settings: Settings,
    settings_dirty: bool,
    /// Controllers that WE hid: id -> HidHide instance path.
    hidden: BTreeMap<String, String>,
}

/// Tells the interface that a controller has just been hidden or unhidden (or that it failed).
#[derive(Serialize, Clone)]
struct CloakEvent {
    id: String,
    hidden: bool,
    error: Option<String>,
}

#[derive(Clone)]
pub struct Controllers {
    inner: Arc<Mutex<Inner>>,
    events: Events,
    cloak: Arc<dyn Cloak>,
    log: Log,
}

impl Controllers {
    /// `file` is `None` to save nothing (tests).
    pub fn new(events: Events, file: Option<PathBuf>) -> Self {
        let log = Log::default();
        let profiles = file.as_deref().map(|f| load_profiles(f, &log)).unwrap_or_default();
        let settings = file.as_deref().map(|f| load_settings(&settings_path(f), &log)).unwrap_or_default();
        if let Some(language) = settings.language {
            i18n::set_language(language);
        }
        Self {
            inner: Arc::new(Mutex::new(Inner {
                profiles,
                active: BTreeMap::new(),
                file,
                dirty: false,
                settings,
                settings_dirty: false,
                hidden: BTreeMap::new(),
            })),
            events,
            cloak: Arc::new(NoCloak),
            log,
        }
    }

    /// Chooses the hiding system (HidHide). Without it, nothing can be hidden.
    pub fn with_cloak(mut self, cloak: Arc<dyn Cloak>) -> Self {
        self.cloak = cloak;
        self
    }

    pub fn events(&self) -> Events {
        self.events.clone()
    }

    pub fn log(&self) -> Log {
        self.log.clone()
    }

    /// The configuration that a controller's thread applies live.
    pub fn live_config(&self, id: &str) -> LiveConfig {
        self.lock().profiles.get(id).map_or_else(|| Profile::default().live(), Profile::live)
    }

    /// Records the measured input report rate, for diagnostics.
    pub fn set_report_rate(&self, id: &str, per_second: u32) {
        if let Some(active) = self.lock().active.get_mut(id) {
            active.report_rate = per_second;
        }
    }

    /// Detailed state of every known controller, plugged in or not.
    pub fn diagnostics_controllers(&self) -> Vec<ControllerDiag> {
        let inner = self.lock();
        inner
            .profiles
            .iter()
            .map(|(id, profile)| {
                let active = inner.active.get(id);
                ControllerDiag {
                    id: id.clone(),
                    name: profile.name.clone(),
                    connected: active.is_some(),
                    bluetooth: active.is_some_and(|a| a.bluetooth),
                    path: active.map(|a| a.path.clone()),
                    battery: active.and_then(|a| a.battery),
                    charge: active.map_or(Charge::Unknown, |a| a.charge),
                    reports_per_second: active.map_or(0, |a| a.report_rate),
                    virtual_pad: profile.virtual_pad,
                    hidden: inner.hidden.contains_key(id),
                }
            })
            .collect()
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // A panicking thread must not make every other thread unusable.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn notify(&self) {
        self.events.emit("controllers-changed", &());
    }

    pub fn list(&self) -> Vec<ControllerInfo> {
        let inner = self.lock();
        inner
            .profiles
            .iter()
            .map(|(id, profile)| ControllerInfo {
                id: id.clone(),
                connected: inner.active.contains_key(id),
                bluetooth: inner.active.get(id).is_some_and(|a| a.bluetooth),
                profile: profile.with_rules(),
            })
            .collect()
    }

    pub fn is_active_path(&self, path: &str) -> bool {
        self.lock().active.values().any(|a| a.path == path)
    }

    pub fn is_active_id(&self, id: &str) -> bool {
        self.lock().active.contains_key(id)
    }

    /// Registers a connected controller and returns its profile (created if it does not exist).
    pub fn register(&self, id: &str, path: &str, bluetooth: bool, tx: Sender<Command>) -> Profile {
        let profile = {
            let mut inner = self.lock();
            let count = inner.profiles.len();
            let mut created = false;
            let profile = inner
                .profiles
                .entry(id.to_string())
                .or_insert_with(|| {
                    created = true;
                    Profile {
                        name: Msg::ControllerName { n: count + 1 }.text(),
                        lightbar: PALETTE[count % PALETTE.len()],
                        ..Profile::default()
                    }
                })
                .clone();
            inner.active.insert(
                id.to_string(),
                Active { tx, bluetooth, path: path.to_string(), battery: None, charge: Charge::Unknown, report_rate: 0 },
            );
            inner.dirty |= created;
            profile
        };
        let link = if bluetooth { "Bluetooth" } else { "USB" };
        self.log.info("controller", Msg::LogConnected { id: id.to_string(), link }.text());
        self.notify();
        profile
    }

    /// Removes a disconnected controller. Ignored if `path` is no longer the registered one.
    pub fn unregister(&self, id: &str, path: &str) {
        let (removed, instance) = {
            let mut inner = self.lock();
            if inner.active.get(id).is_some_and(|a| a.path == path) {
                inner.active.remove(id);
                (true, inner.hidden.remove(id))
            } else {
                (false, None)
            }
        };
        if removed {
            self.log.info("controller", Msg::LogDisconnected { id: id.to_string() }.text());
        }
        // An unplugged controller must not leave a hidden entry behind in HidHide.
        if let Some(instance) = instance {
            let error = self.cloak.unhide(&instance).err();
            self.events.emit("cloak", &CloakEvent { id: id.to_string(), hidden: false, error });
        }
        self.notify();
    }

    /// Modifies an existing profile. Returns `false` if the controller is unknown.
    pub fn edit(&self, id: &str, f: impl FnOnce(&mut Profile)) -> bool {
        let mut inner = self.lock();
        let Some(profile) = inner.profiles.get_mut(id) else {
            return false;
        };
        let before = profile.clone();
        f(profile);
        profile.name = profile.name.trim().chars().take(MAX_NAME_LEN).collect();
        let changed = *profile != before;
        let renamed = profile.name != before.name;
        inner.dirty |= changed;
        drop(inner);
        if renamed {
            self.notify();
        }
        true
    }

    /// Sends a command to a connected controller (no effect if it is absent).
    pub fn send(&self, id: &str, cmd: Command) {
        if let Some(active) = self.lock().active.get(id) {
            let _ = active.tx.send(cmd);
        }
    }

    /// Reapplies the saved profile, for example after a game has changed the light bar.
    pub fn reset_to_profile(&self, id: &str) {
        let profile = self.lock().profiles.get(id).cloned();
        if let Some(profile) = profile {
            for cmd in profile.commands(true) {
                self.send(id, cmd);
            }
        }
    }

    /// Remembers a connected controller's battery (for the overlay).
    pub fn set_battery(&self, id: &str, percent: Option<u8>, charge: Charge) {
        if let Some(active) = self.lock().active.get_mut(id) {
            active.battery = percent;
            active.charge = charge;
        }
    }

    /// Shows the overlay: the state of every connected controller, with an optional message.
    pub fn show_overlay(&self, source: &str, message: Option<String>) {
        let payload = {
            let inner = self.lock();
            let controllers = inner
                .active
                .iter()
                .map(|(id, active)| {
                    let profile = inner.profiles.get(id).cloned().unwrap_or_default();
                    OverlayController {
                        id: id.clone(),
                        name: if profile.name.is_empty() { id.clone() } else { profile.name },
                        color: profile.lightbar,
                        bluetooth: active.bluetooth,
                        percent: active.battery,
                        charge: active.charge,
                        xbox_mode: profile.virtual_pad,
                        source: id == source,
                    }
                })
                .collect();
            OverlayPayload { message, controllers }
        };
        self.events.emit("overlay", &payload);
    }

    /// Powers a controller off by closing its Bluetooth link. Not possible over USB: the cable
    /// powers it, so it cannot turn off.
    pub fn power_off(&self, id: &str) -> Result<(), String> {
        let result = self.try_power_off(id);
        if let Err(error) = &result {
            self.log.warn("power-off", format!("{id}: {error}"));
        }
        result
    }

    fn try_power_off(&self, id: &str) -> Result<(), String> {
        let bluetooth = self.lock().active.get(id).map(|a| a.bluetooth);
        match bluetooth {
            None => return Err(Msg::NotConnected.text()),
            Some(false) => return Err(Msg::PowerOffUsb.text()),
            Some(true) => {}
        }
        let mac = bluetooth::parse_mac(id).ok_or_else(|| Msg::UnknownBluetoothAddress.text())?;
        // Stop the rumble first: a controller powered off mid-rumble sometimes gets stuck.
        self.send(id, Command::Rumble { left: 0, right: 0 });
        bluetooth::disconnect(mac)
    }

    /// Identifiers of the **connected** controllers designated by `selector`.
    pub fn resolve(&self, selector: Option<&Selector>) -> Vec<String> {
        let inner = self.lock();
        let connected: Vec<&String> = inner.active.keys().collect();
        match selector {
            None => connected.into_iter().cloned().collect(),
            Some(Selector::Index(i)) => connected.get(*i).map(|s| vec![(*s).clone()]).unwrap_or_default(),
            Some(Selector::Name(n)) if n.eq_ignore_ascii_case("all") => {
                connected.into_iter().cloned().collect()
            }
            Some(Selector::Name(n)) => connected
                .into_iter()
                .filter(|id| {
                    id.eq_ignore_ascii_case(n)
                        || inner.profiles.get(*id).is_some_and(|p| p.name.eq_ignore_ascii_case(n))
                })
                .cloned()
                .collect(),
        }
    }

    /// Writes the profiles and the settings to disk if they changed.
    pub fn flush(&self) {
        self.flush_profiles();
        self.flush_settings();
    }

    fn flush_settings(&self) {
        let (path, json) = {
            let mut inner = self.lock();
            if !inner.settings_dirty {
                return;
            }
            let Some(file) = inner.file.clone() else {
                inner.settings_dirty = false;
                return;
            };
            let Ok(json) = serde_json::to_string_pretty(&inner.settings) else {
                return;
            };
            inner.settings_dirty = false;
            (settings_path(&file), json)
        };
        if let Err(e) = write_atomic(&path, &json) {
            eprintln!("settings: write failed ({e})");
            self.log.error("settings", Msg::LogWriteFailed { reason: e.to_string() }.text());
            self.lock().settings_dirty = true;
        }
    }

    fn flush_profiles(&self) {
        let (path, json) = {
            let mut inner = self.lock();
            if !inner.dirty {
                return;
            }
            let Some(path) = inner.file.clone() else {
                inner.dirty = false;
                return;
            };
            let json = match serde_json::to_string_pretty(&inner.profiles) {
                Ok(j) => j,
                Err(e) => {
                    eprintln!("profiles: serialization failed ({e})");
                    return;
                }
            };
            inner.dirty = false;
            (path, json)
        };
        if let Err(e) = write_atomic(&path, &json) {
            eprintln!("profiles: write failed ({e})");
            self.log.error("profiles", Msg::LogWriteFailed { reason: e.to_string() }.text());
            self.lock().dirty = true; // try again on the next pass
        }
    }

    /// On exit: stops the rumble and saves the profiles.
    pub fn shutdown(&self) {
        let ids: Vec<String> = self.lock().active.keys().cloned().collect();
        for id in &ids {
            self.send(id, Command::Rumble { left: 0, right: 0 });
        }
        self.flush();
        // Without this, quitting in Xbox mode would leave the physical controller hidden from games.
        let hidden = std::mem::take(&mut self.lock().hidden);
        for instance in hidden.values() {
            let _ = self.cloak.unhide(instance);
        }
        if !ids.is_empty() {
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    // --- Settings -------------------------------------------------------------------------

    pub fn settings(&self) -> Settings {
        self.lock().settings.clone()
    }

    /// Changes the interface language: the backend messages follow immediately, the choice is
    /// saved, and the interface windows are told so that they can update themselves.
    pub fn set_language(&self, language: Lang) {
        {
            let mut inner = self.lock();
            if inner.settings.language != Some(language) {
                inner.settings.language = Some(language);
                inner.settings_dirty = true;
            }
        }
        i18n::set_language(language);
        self.events.emit("language-changed", &language);
    }

    /// Low-battery alert and confirmation vibrations.
    pub fn set_notifications(&self, low_battery_alert: bool, haptic_feedback: bool) {
        let mut inner = self.lock();
        if inner.settings.low_battery_alert != low_battery_alert || inner.settings.haptic_feedback != haptic_feedback {
            inner.settings.low_battery_alert = low_battery_alert;
            inner.settings.haptic_feedback = haptic_feedback;
            inner.settings_dirty = true;
        }
    }

    // --- Hiding the physical controller (HidHide) -----------------------------------------

    pub fn cloak_status(&self) -> CloakStatus {
        self.cloak.status()
    }

    /// Enables or disables automatic hiding, and applies it right away to the controllers
    /// concerned. Returns the first error met (the setting is saved all the same).
    pub fn set_hidhide_auto(&self, enabled: bool) -> Result<(), String> {
        let ids: Vec<String> = {
            let mut inner = self.lock();
            if inner.settings.hidhide_auto != enabled {
                inner.settings.hidhide_auto = enabled;
                inner.settings_dirty = true;
            }
            if enabled {
                inner
                    .active
                    .keys()
                    .filter(|id| inner.profiles.get(*id).is_some_and(|p| p.virtual_pad))
                    .cloned()
                    .collect()
            } else {
                inner.hidden.keys().cloned().collect()
            }
        };

        let mut first_error = None;
        for id in ids {
            if let Err(e) = self.set_cloaked(&id, enabled) {
                first_error.get_or_insert(e);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    /// To be called when a controller connects: if it should be in native mode, unhides it.
    /// Recovers from a previous session killed before it could unhide (Ctrl+C, crash), which the
    /// in-memory tracking cannot know about.
    pub fn release_stale_cloak(&self, id: &str) {
        let path = {
            let inner = self.lock();
            let wants_xbox = inner.profiles.get(id).is_some_and(|p| p.virtual_pad);
            if !inner.settings.hidhide_auto || wants_xbox || inner.hidden.contains_key(id) {
                return;
            }
            inner.active.get(id).map(|a| a.path.clone())
        };
        if let Some(path) = path {
            let _ = self.cloak.unhide_link(&path);
        }
    }

    /// Hides (`want`) or unhides the physical controller. Has no effect while the automatic
    /// setting is off. Returns the final state, and notifies the interface when it changes or fails.
    pub fn set_cloaked(&self, id: &str, want: bool) -> Result<bool, String> {
        let before = self.lock().hidden.contains_key(id);
        let result = self.apply_cloak(id, want);
        let after = self.lock().hidden.contains_key(id);
        let error = result.as_ref().err().cloned();
        if before != after || error.is_some() {
            match &error {
                Some(e) => self.log.error("hidhide", format!("{id}: {e}")),
                None if after => self.log.info("hidhide", Msg::LogHidden { id: id.to_string() }.text()),
                None => self.log.info("hidhide", Msg::LogUnhidden { id: id.to_string() }.text()),
            }
            self.events.emit("cloak", &CloakEvent { id: id.to_string(), hidden: after, error });
        }
        result
    }

    fn apply_cloak(&self, id: &str, want: bool) -> Result<bool, String> {
        if want {
            let path = {
                let inner = self.lock();
                if !inner.settings.hidhide_auto {
                    return Ok(false);
                }
                if inner.hidden.contains_key(id) {
                    return Ok(true);
                }
                inner.active.get(id).map(|a| a.path.clone())
            }
            .ok_or_else(|| Msg::ControllerMissing.text())?;
            // Outside the lock: running HidHideCLI takes a while.
            let instance = self.cloak.hide(&path)?;
            self.lock().hidden.insert(id.to_string(), instance);
            Ok(true)
        } else {
            let Some(instance) = self.lock().hidden.remove(id) else {
                return Ok(false);
            };
            if let Err(e) = self.cloak.unhide(&instance) {
                self.lock().hidden.insert(id.to_string(), instance); // still hidden
                return Err(e);
            }
            Ok(false)
        }
    }
}

/// `settings.json` sits next to `profiles.json`.
fn settings_path(profiles_file: &std::path::Path) -> PathBuf {
    profiles_file.with_file_name("settings.json")
}

fn load_settings(path: &std::path::Path, log: &Log) -> Settings {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Settings::default();
    };
    serde_json::from_str(&text).unwrap_or_else(|e| {
        eprintln!("settings unreadable ({e}): using defaults");
        log.error("settings", Msg::LogSettingsUnreadable { reason: e.to_string() }.text());
        Settings::default()
    })
}

fn load_profiles(path: &std::path::Path, log: &Log) -> BTreeMap<String, Profile> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    match serde_json::from_str::<BTreeMap<String, Profile>>(&text) {
        Ok(profiles) => merge_duplicate_ids(profiles),
        Err(e) => {
            // Never silently overwrite the user's settings.
            let backup = path.with_extension("json.bad");
            eprintln!("profiles unreadable ({e}): copying to {}", backup.display());
            let reason = e.to_string();
            log.error("profiles", Msg::LogProfilesUnreadable { reason, backup: backup.display().to_string() }.text());
            let _ = std::fs::copy(path, backup);
            BTreeMap::new()
        }
    }
}

/// Older versions could register the same controller under two identifiers: its MAC address
/// (`D0:BC:C1:92:48:0F`) and, when the controller did not answer in time after a Bluetooth
/// reconnection, its serial number (`d0bcc192480f`). Such an identifier is renamed to the canonical
/// form; if a profile already exists under that form, it is the one in use and the duplicate is dropped.
fn merge_duplicate_ids(profiles: BTreeMap<String, Profile>) -> BTreeMap<String, Profile> {
    let mut merged = BTreeMap::new();
    let mut duplicates = Vec::new();
    for (id, profile) in profiles {
        match controller::mac_from_serial(&id) {
            Some(canonical) if canonical != id => duplicates.push((canonical, profile)),
            _ => {
                merged.insert(id, profile);
            }
        }
    }
    for (canonical, profile) in duplicates {
        merged.entry(canonical).or_insert(profile);
    }
    merged
}

fn write_atomic(path: &std::path::Path, contents: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, path)
}

/// Starts the thread that detects plugged-in controllers and starts one thread per controller.
pub fn start(controllers: Controllers) {
    std::thread::Builder::new()
        .name("controller-supervisor".into())
        .spawn(move || supervise(controllers))
        .expect("could not start the controller supervisor");
}

fn supervise(controllers: Controllers) {
    let mut api = match HidApi::new() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("hidapi: {e}");
            return;
        }
    };

    loop {
        if api.refresh_devices().is_ok() {
            let infos: Vec<_> = api.device_list().filter(|d| controller::is_dualsense(d)).cloned().collect();
            for info in infos {
                let path = info.path().to_string_lossy().into_owned();
                if controllers.is_active_path(&path) {
                    continue;
                }
                let Some((device, bluetooth)) = controller::open(&api, &info) else {
                    continue;
                };
                let id = controller::identify(&device, &info);
                if controllers.is_active_id(&id) {
                    continue; // the same controller, seen through another interface
                }

                let (tx, rx) = mpsc::channel();
                // The profile goes into the channel before the thread starts: it is part of the first report.
                let profile = controllers.register(&id, &path, bluetooth, tx.clone());
                for cmd in profile.commands(false) {
                    let _ = tx.send(cmd);
                }

                let ctx = DeviceCtx { controllers: controllers.clone(), id: id.clone() };
                let controllers = controllers.clone();
                let spawned = std::thread::Builder::new()
                    .name(format!("dualsense-{id}"))
                    .spawn(move || {
                        controller::run_device(ctx, device, bluetooth, rx, tx);
                        controllers.unregister(&id, &path);
                    });
                if let Err(e) = spawned {
                    eprintln!("controller thread: {e}");
                }
            }
        }
        controllers.flush();
        std::thread::sleep(SCAN_INTERVAL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::Receiver;

    fn connect(c: &Controllers, id: &str) -> Receiver<Command> {
        let (tx, rx) = mpsc::channel();
        c.register(id, &format!("path-{id}"), false, tx);
        rx
    }

    fn temp_file(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("dualsense-test-{tag}-{}-{nanos}", std::process::id())).join("profiles.json")
    }

    #[test]
    fn new_controllers_get_distinct_names_and_colors() {
        let c = Controllers::new(Events::none(), None);
        let _a = connect(&c, "AA");
        let _b = connect(&c, "BB");
        let _d = connect(&c, "CC");
        let list = c.list();
        let colors: Vec<_> = list.iter().map(|i| i.profile.lightbar).collect();
        assert_eq!(list[0].profile.name, "Controller 1");
        assert_eq!(list[2].profile.name, "Controller 3");
        assert_ne!(colors[0], colors[1]);
        assert_ne!(colors[1], colors[2]);
    }

    #[test]
    fn default_names_follow_the_current_language() {
        i18n::set_language(Lang::Fr);
        let c = Controllers::new(Events::none(), None);
        let _a = connect(&c, "AA");
        assert_eq!(c.list()[0].profile.name, "Manette 1");
    }

    #[test]
    fn reconnecting_keeps_the_existing_profile() {
        let c = Controllers::new(Events::none(), None);
        let rx = connect(&c, "AA");
        c.edit("AA", |p| p.lightbar = [1, 2, 3]);
        c.unregister("AA", "path-AA");
        drop(rx);
        let (tx, _rx) = mpsc::channel();
        let profile = c.register("AA", "other-path", true, tx);
        assert_eq!(profile.lightbar, [1, 2, 3]);
        assert_eq!(c.list().len(), 1);
    }

    #[test]
    fn commands_go_only_to_the_requested_controller() {
        let c = Controllers::new(Events::none(), None);
        let rx_a = connect(&c, "AA");
        let rx_b = connect(&c, "BB");
        c.send("BB", Command::Lightbar(9, 9, 9));
        assert!(rx_a.try_recv().is_err());
        assert_eq!(rx_b.try_recv().unwrap(), Command::Lightbar(9, 9, 9));
    }

    #[test]
    fn selectors_resolve_by_index_id_name_and_all() {
        let c = Controllers::new(Events::none(), None);
        let _a = connect(&c, "AA");
        let _b = connect(&c, "BB");
        c.edit("BB", |p| p.name = "Living room".into());

        assert_eq!(c.resolve(None), ["AA", "BB"]);
        assert_eq!(c.resolve(Some(&Selector::Name("ALL".into()))), ["AA", "BB"]);
        assert_eq!(c.resolve(Some(&Selector::Index(1))), ["BB"]);
        assert_eq!(c.resolve(Some(&Selector::Name("aa".into()))), ["AA"]);
        assert_eq!(c.resolve(Some(&Selector::Name("living room".into()))), ["BB"]);
        assert!(c.resolve(Some(&Selector::Index(2))).is_empty());
        assert!(c.resolve(Some(&Selector::Name("unknown".into()))).is_empty());
    }

    #[test]
    fn disconnected_controllers_are_not_targeted_but_stay_listed() {
        let c = Controllers::new(Events::none(), None);
        let _a = connect(&c, "AA");
        c.unregister("AA", "path-AA");
        assert!(c.resolve(None).is_empty());
        assert!(!c.list()[0].connected);
    }

    #[test]
    fn stale_unregister_does_not_remove_a_newer_connection() {
        let c = Controllers::new(Events::none(), None);
        let _old = connect(&c, "AA");
        let (tx, _rx) = mpsc::channel();
        c.register("AA", "new-path", false, tx);
        c.unregister("AA", "path-AA"); // the old thread ends late
        assert!(c.is_active_id("AA"));
    }

    #[test]
    fn name_is_trimmed_and_truncated() {
        let c = Controllers::new(Events::none(), None);
        let _a = connect(&c, "AA");
        c.edit("AA", |p| p.name = format!("  {}  ", "x".repeat(100)));
        assert_eq!(c.list()[0].profile.name.chars().count(), MAX_NAME_LEN);
    }

    #[test]
    fn editing_an_unknown_controller_reports_false() {
        let c = Controllers::new(Events::none(), None);
        assert!(!c.edit("ZZ", |p| p.player_leds = 1));
    }

    #[test]
    fn profiles_survive_a_restart() {
        let file = temp_file("roundtrip");
        let c = Controllers::new(Events::none(), Some(file.clone()));
        let _a = connect(&c, "AA");
        c.edit("AA", |p| {
            p.name = "Living room".into();
            p.lightbar = [10, 20, 30];
            p.trigger_right = Some(TriggerEffect::Weapon { start: 3, end: 6, strength: 8 });
            p.virtual_pad = true;
        });
        c.flush();

        let reloaded = Controllers::new(Events::none(), Some(file.clone()));
        let profile = &reloaded.list()[0].profile;
        assert_eq!(profile.name, "Living room");
        assert_eq!(profile.lightbar, [10, 20, 30]);
        assert_eq!(profile.trigger_right, Some(TriggerEffect::Weapon { start: 3, end: 6, strength: 8 }));
        assert!(profile.virtual_pad);
        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn a_profile_saved_under_a_serial_number_moves_to_the_mac_address_form() {
        let mut profiles = BTreeMap::new();
        profiles.insert("d0bcc192480f".to_string(), Profile { name: "Old".into(), ..Profile::default() });
        let merged = merge_duplicate_ids(profiles);
        assert_eq!(merged.keys().collect::<Vec<_>>(), ["D0:BC:C1:92:48:0F"]);
        assert_eq!(merged["D0:BC:C1:92:48:0F"].name, "Old");
    }

    #[test]
    fn when_both_forms_exist_the_mac_address_profile_is_kept() {
        let mut profiles = BTreeMap::new();
        profiles.insert("4cb99b3df6d9".to_string(), Profile { name: "Duplicate".into(), ..Profile::default() });
        profiles.insert("4C:B9:9B:3D:F6:D9".to_string(), Profile { name: "In use".into(), ..Profile::default() });
        profiles.insert("serial-number".to_string(), Profile { name: "Other".into(), ..Profile::default() });
        let merged = merge_duplicate_ids(profiles);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged["4C:B9:9B:3D:F6:D9"].name, "In use");
        assert_eq!(merged["serial-number"].name, "Other", "identifiers that are not MAC addresses are left alone");
    }

    #[test]
    fn corrupt_profile_file_is_backed_up_not_overwritten() {
        let file = temp_file("corrupt");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "{ this is not json").unwrap();

        let c = Controllers::new(Events::none(), Some(file.clone()));
        assert!(c.list().is_empty());
        let backup = file.with_extension("json.bad");
        assert_eq!(std::fs::read_to_string(backup).unwrap(), "{ this is not json");
        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn profile_commands_skip_untouched_triggers_unless_resetting() {
        let p = Profile::default();
        let normal = p.commands(false);
        let reset = p.commands(true);
        assert_eq!(normal.iter().filter(|c| matches!(c, Command::Trigger { .. })).count(), 0);
        assert_eq!(reset.iter().filter(|c| matches!(c, Command::Trigger { .. })).count(), 2);
        assert!(matches!(normal.last(), Some(Command::VirtualPad(false))));
    }

    #[test]
    fn reset_to_profile_sends_the_saved_state() {
        let c = Controllers::new(Events::none(), None);
        let rx = connect(&c, "AA");
        c.edit("AA", |p| p.lightbar = [5, 6, 7]);
        c.reset_to_profile("AA");
        let cmds: Vec<_> = rx.try_iter().collect();
        assert_eq!(cmds[0], Command::Lightbar(5, 6, 7));
    }

    #[test]
    fn power_off_refuses_unknown_and_disconnected_controllers() {
        let c = Controllers::new(Events::none(), None);
        assert!(c.power_off("AA:BB:CC:DD:EE:FF").unwrap_err().contains("not connected"));

        let _rx = connect(&c, "AA:BB:CC:DD:EE:FF");
        c.unregister("AA:BB:CC:DD:EE:FF", "path-AA:BB:CC:DD:EE:FF");
        assert!(c.power_off("AA:BB:CC:DD:EE:FF").unwrap_err().contains("not connected"));
    }

    #[test]
    fn power_off_is_refused_over_usb() {
        let c = Controllers::new(Events::none(), None);
        let rx = connect(&c, "AA:BB:CC:DD:EE:FF"); // `connect` simulates a USB link
        let err = c.power_off("AA:BB:CC:DD:EE:FF").unwrap_err();
        assert!(err.contains("USB"));
        assert!(rx.try_recv().is_err(), "nothing must be sent to the controller");
    }

    #[test]
    fn power_off_needs_a_mac_style_identifier() {
        let c = Controllers::new(Events::none(), None);
        let (tx, _rx) = mpsc::channel();
        c.register("serial-number", "path", true, tx); // Bluetooth, but with a fallback identifier
        assert!(c.power_off("serial-number").unwrap_err().contains("Bluetooth address"));
    }

    #[test]
    fn new_profiles_get_the_default_shortcuts() {
        let c = Controllers::new(Events::none(), None);
        let _a = connect(&c, "AA");
        assert_eq!(c.live_config("AA").rules, rules_from_shortcuts(&default_shortcuts()));
    }

    #[test]
    fn profiles_saved_before_rules_existed_derive_them_from_their_shortcuts() {
        let p: Profile = serde_json::from_str(r#"{"shortcuts":[{"button":"cross","gesture":"press","action":"show_overlay"}]}"#).unwrap();
        assert_eq!(p.rules, None);
        let live = p.live();
        assert_eq!(live.rules.len(), 1);
        assert_eq!(live.rules[0].inputs, [crate::bindings::Input::Cross]);
    }

    #[test]
    fn the_interface_receives_the_derived_rules_of_an_old_profile() {
        let old: Profile = serde_json::from_str(r#"{"name":"Old"}"#).unwrap();
        assert_eq!(old.rules, None);
        assert_eq!(old.with_rules().rules, Some(rules_from_shortcuts(&default_shortcuts())));
        let c = Controllers::new(Events::none(), None);
        let _a = connect(&c, "AA");
        assert!(c.list()[0].profile.rules.is_some());
    }

    #[test]
    fn saved_rules_take_over_from_the_old_shortcuts() {
        let mut p = Profile::default();
        p.rules = Some(Vec::new());
        assert!(p.live().rules.is_empty(), "a saved empty list is respected");
    }

    #[test]
    fn profiles_saved_before_shortcuts_existed_get_the_defaults() {
        let p: Profile = serde_json::from_str(r#"{"name":"Old","virtual_pad":true}"#).unwrap();
        assert_eq!(p.shortcuts, default_shortcuts());
    }

    #[test]
    fn an_empty_shortcut_list_is_kept_as_chosen() {
        let p: Profile = serde_json::from_str(r#"{"shortcuts":[]}"#).unwrap();
        assert!(p.shortcuts.is_empty());
    }

    fn capturing_events() -> (Events, Arc<Mutex<Vec<(String, serde_json::Value)>>>) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let sink = log.clone();
        (Events::new(move |name, value| sink.lock().unwrap().push((name.to_string(), value))), log)
    }

    #[test]
    fn overlay_lists_connected_controllers_with_their_battery_and_mode() {
        let (events, log) = capturing_events();
        let c = Controllers::new(events, None);
        let _a = connect(&c, "AA");
        let _b = connect(&c, "BB");
        c.edit("BB", |p| p.virtual_pad = true);
        c.set_battery("AA", Some(85), Charge::Charging);
        log.lock().unwrap().clear(); // ignore the "controllers-changed" events of the connections

        c.show_overlay("AA", Some("Native mode".into()));

        let log = log.lock().unwrap();
        assert_eq!(log.len(), 1);
        let (name, payload) = &log[0];
        assert_eq!(name, "overlay");
        assert_eq!(payload["message"], "Native mode");
        let list = payload["controllers"].as_array().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0]["percent"], 85);
        assert_eq!(list[0]["charge"], "charging");
        assert_eq!(list[0]["source"], true);
        assert_eq!(list[0]["xbox_mode"], false);
        assert_eq!(list[1]["source"], false);
        assert_eq!(list[1]["xbox_mode"], true);
        assert!(list[1]["percent"].is_null());
    }

    #[test]
    fn overlay_skips_disconnected_controllers() {
        let (events, log) = capturing_events();
        let c = Controllers::new(events, None);
        let _a = connect(&c, "AA");
        let _b = connect(&c, "BB");
        c.unregister("BB", "path-BB");
        log.lock().unwrap().clear();
        c.show_overlay("AA", None);
        let log = log.lock().unwrap();
        assert_eq!(log[0].1["controllers"].as_array().unwrap().len(), 1);
    }

    // --- HidHide ------------------------------------------------------------------------

    #[derive(Default)]
    struct FakeCloak {
        hidden: Mutex<Vec<String>>,
        hide_calls: Mutex<u32>,
        /// Symbolic links passed to `unhide_link`.
        released: Mutex<Vec<String>>,
        fail: bool,
    }

    impl Cloak for FakeCloak {
        fn hide(&self, link: &str) -> Result<String, String> {
            *self.hide_calls.lock().unwrap() += 1;
            if self.fail {
                return Err("refused".into());
            }
            let instance = format!("instance:{link}");
            self.hidden.lock().unwrap().push(instance.clone());
            Ok(instance)
        }
        fn unhide(&self, instance: &str) -> Result<(), String> {
            if self.fail {
                return Err("refused".into());
            }
            self.hidden.lock().unwrap().retain(|i| i != instance);
            Ok(())
        }
        fn unhide_link(&self, link: &str) -> Result<(), String> {
            self.released.lock().unwrap().push(link.to_string());
            Ok(())
        }
        fn status(&self) -> CloakStatus {
            CloakStatus { installed: true, cloak_active: Some(true), app_registered: Some(true), error: None }
        }
    }

    fn with_cloak(fail: bool) -> (Controllers, Arc<FakeCloak>) {
        let cloak = Arc::new(FakeCloak { fail, ..Default::default() });
        (Controllers::new(Events::none(), None).with_cloak(cloak.clone()), cloak)
    }

    fn enable_auto(c: &Controllers) {
        c.set_hidhide_auto(true).unwrap();
    }

    #[test]
    fn nothing_is_hidden_while_the_automatic_setting_is_off() {
        let (c, cloak) = with_cloak(false);
        let _a = connect(&c, "AA");
        assert_eq!(c.set_cloaked("AA", true), Ok(false));
        assert_eq!(*cloak.hide_calls.lock().unwrap(), 0);
    }

    #[test]
    fn the_real_device_is_hidden_by_its_own_path_and_only_once() {
        let (c, cloak) = with_cloak(false);
        let _a = connect(&c, "AA");
        enable_auto(&c);
        assert_eq!(c.set_cloaked("AA", true), Ok(true));
        assert_eq!(c.set_cloaked("AA", true), Ok(true));
        assert_eq!(*cloak.hide_calls.lock().unwrap(), 1, "HidHide must not be run again needlessly");
        assert_eq!(*cloak.hidden.lock().unwrap(), ["instance:path-AA"]);
    }

    #[test]
    fn leaving_xbox_mode_unhides_the_device() {
        let (c, cloak) = with_cloak(false);
        let _a = connect(&c, "AA");
        enable_auto(&c);
        c.set_cloaked("AA", true).unwrap();
        assert_eq!(c.set_cloaked("AA", false), Ok(false));
        assert!(cloak.hidden.lock().unwrap().is_empty());
    }

    #[test]
    fn unplugging_a_controller_unhides_it() {
        let (c, cloak) = with_cloak(false);
        let _a = connect(&c, "AA");
        enable_auto(&c);
        c.set_cloaked("AA", true).unwrap();
        c.unregister("AA", "path-AA");
        assert!(cloak.hidden.lock().unwrap().is_empty());
    }

    #[test]
    fn a_late_unregister_does_not_unhide_a_newer_connection() {
        let (c, cloak) = with_cloak(false);
        let _old = connect(&c, "AA");
        enable_auto(&c);
        c.set_cloaked("AA", true).unwrap();
        c.unregister("AA", "another-path"); // does not match the current connection
        assert_eq!(cloak.hidden.lock().unwrap().len(), 1);
    }

    #[test]
    fn shutdown_unhides_everything_we_hid() {
        let (c, cloak) = with_cloak(false);
        let _a = connect(&c, "AA");
        let _b = connect(&c, "BB");
        enable_auto(&c);
        c.set_cloaked("AA", true).unwrap();
        c.set_cloaked("BB", true).unwrap();
        c.shutdown();
        assert!(cloak.hidden.lock().unwrap().is_empty());
    }

    #[test]
    fn a_controller_in_native_mode_is_released_at_connection() {
        let (c, cloak) = with_cloak(false);
        enable_auto(&c);
        let _a = connect(&c, "AA"); // default profile: native mode
        c.release_stale_cloak("AA");
        assert_eq!(*cloak.released.lock().unwrap(), ["path-AA"]);
    }

    #[test]
    fn a_controller_in_xbox_mode_is_not_released_at_connection() {
        let (c, cloak) = with_cloak(false);
        enable_auto(&c);
        let _a = connect(&c, "AA");
        c.edit("AA", |p| p.virtual_pad = true);
        c.release_stale_cloak("AA");
        assert!(cloak.released.lock().unwrap().is_empty());
    }

    #[test]
    fn nothing_is_released_when_the_automatic_setting_is_off() {
        let (c, cloak) = with_cloak(false);
        let _a = connect(&c, "AA");
        c.release_stale_cloak("AA"); // the user manages HidHide themselves: leave it alone
        assert!(cloak.released.lock().unwrap().is_empty());
    }

    #[test]
    fn turning_the_setting_on_hides_only_controllers_in_xbox_mode() {
        let (c, cloak) = with_cloak(false);
        let _a = connect(&c, "AA");
        let _b = connect(&c, "BB");
        c.edit("BB", |p| p.virtual_pad = true);
        enable_auto(&c);
        assert_eq!(*cloak.hidden.lock().unwrap(), ["instance:path-BB"]);
    }

    #[test]
    fn turning_the_setting_off_unhides_everything() {
        let (c, cloak) = with_cloak(false);
        let _a = connect(&c, "AA");
        c.edit("AA", |p| p.virtual_pad = true);
        enable_auto(&c);
        c.set_hidhide_auto(false).unwrap();
        assert!(cloak.hidden.lock().unwrap().is_empty());
        assert!(!c.settings().hidhide_auto);
    }

    #[test]
    fn a_refused_hide_reports_the_error_and_marks_nothing_hidden() {
        let (c, _cloak) = with_cloak(true);
        let _a = connect(&c, "AA");
        enable_auto(&c);
        assert_eq!(c.set_cloaked("AA", true), Err("refused".to_string()));
        assert!(c.lock().hidden.is_empty());
    }

    #[test]
    fn a_refused_unhide_keeps_tracking_the_device() {
        let (c, cloak) = with_cloak(false);
        let _a = connect(&c, "AA");
        enable_auto(&c);
        c.set_cloaked("AA", true).unwrap();
        let failing = Arc::new(FakeCloak { fail: true, ..Default::default() });
        let c = Controllers { cloak: failing, ..c };
        assert!(c.set_cloaked("AA", false).is_err());
        assert_eq!(c.lock().hidden.len(), 1, "it must be possible to try again");
        drop(cloak);
    }

    #[test]
    fn hiding_a_missing_controller_fails() {
        let (c, _) = with_cloak(false);
        enable_auto(&c);
        assert!(c.set_cloaked("ZZ", true).is_err());
    }

    #[test]
    fn without_hidhide_enabling_the_setting_still_saves_it_but_reports_the_error() {
        let c = Controllers::new(Events::none(), None); // NoCloak by default
        let _a = connect(&c, "AA");
        c.edit("AA", |p| p.virtual_pad = true);
        assert!(c.set_hidhide_auto(true).unwrap_err().contains("not installed"));
        assert!(c.settings().hidhide_auto);
    }

    #[test]
    fn cloak_events_fire_on_changes_and_errors_but_not_on_no_ops() {
        let (events, log) = capturing_events();
        let cloak = Arc::new(FakeCloak::default());
        let c = Controllers::new(events, None).with_cloak(cloak);
        let _a = connect(&c, "AA");
        enable_auto(&c);
        log.lock().unwrap().clear();

        c.set_cloaked("AA", true).unwrap(); // change
        c.set_cloaked("AA", true).unwrap(); // no-op
        c.set_cloaked("AA", false).unwrap(); // change
        c.set_cloaked("AA", false).unwrap(); // no-op

        let log = log.lock().unwrap();
        let cloak_events: Vec<_> = log.iter().filter(|(n, _)| n == "cloak").collect();
        assert_eq!(cloak_events.len(), 2);
        assert_eq!(cloak_events[0].1["hidden"], true);
        assert_eq!(cloak_events[1].1["hidden"], false);
    }

    #[test]
    fn the_setting_survives_a_restart() {
        let file = temp_file("settings");
        let c = Controllers::new(Events::none(), Some(file.clone()));
        c.set_hidhide_auto(true).ok();
        c.flush();
        assert!(Controllers::new(Events::none(), Some(file.clone())).settings().hidhide_auto);
        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn missing_or_broken_settings_fall_back_to_defaults() {
        assert!(!Controllers::new(Events::none(), None).settings().hidhide_auto);
        let file = temp_file("settings-broken");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(settings_path(&file), "not json").unwrap();
        assert_eq!(Controllers::new(Events::none(), Some(file.clone())).settings(), Settings::default());
        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }

    // --- Language -----------------------------------------------------------------------

    #[test]
    fn no_language_is_chosen_until_the_user_picks_one() {
        assert_eq!(Settings::default().language, None);
        let old: Settings = serde_json::from_str(r#"{"hidhide_auto":true}"#).unwrap();
        assert_eq!(old.language, None);
    }

    #[test]
    fn changing_the_language_updates_the_messages_and_notifies_the_interface() {
        let (events, log) = capturing_events();
        let c = Controllers::new(events, None);
        c.set_language(Lang::Fr);
        assert_eq!(i18n::language(), Lang::Fr);
        assert_eq!(c.settings().language, Some(Lang::Fr));
        let log = log.lock().unwrap();
        assert_eq!(log.last().unwrap().0, "language-changed");
        assert_eq!(log.last().unwrap().1, "fr");
    }

    #[test]
    fn the_language_survives_a_restart_and_is_applied_at_start_up() {
        let file = temp_file("language");
        let c = Controllers::new(Events::none(), Some(file.clone()));
        c.set_language(Lang::Fr);
        c.flush();

        i18n::set_language(Lang::En);
        let reloaded = Controllers::new(Events::none(), Some(file.clone()));
        assert_eq!(reloaded.settings().language, Some(Lang::Fr));
        assert_eq!(i18n::language(), Lang::Fr);
        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }

    // --- Further settings ---------------------------------------------------------------

    #[test]
    fn new_features_are_off_or_neutral_by_default() {
        let p = Profile::default();
        assert_eq!(p.lightbar_mode, LightbarMode::Fixed);
        assert_eq!(p.input, InputConfig::default());
        assert_eq!(p.motion, MotionConfig::default());
        assert_eq!(p.motion.gyro, crate::motion::GyroMode::Off);
        assert_eq!(p.motion.touchpad, crate::motion::TouchpadMode::Off);
    }

    #[test]
    fn profiles_saved_before_these_features_still_load() {
        let p: Profile = serde_json::from_str(r#"{"name":"Old","lightbar":[1,2,3],"virtual_pad":true}"#).unwrap();
        assert_eq!((p.name.as_str(), p.lightbar, p.virtual_pad), ("Old", [1, 2, 3], true));
        assert_eq!(p.motion, MotionConfig::default());
    }

    #[test]
    fn the_notification_settings_are_on_by_default_even_for_old_settings_files() {
        let s = Settings::default();
        assert!(s.low_battery_alert && s.haptic_feedback && !s.hidhide_auto);
        let old: Settings = serde_json::from_str(r#"{"hidhide_auto":true}"#).unwrap();
        assert!(old.hidhide_auto && old.low_battery_alert && old.haptic_feedback);
    }

    #[test]
    fn notification_settings_can_be_changed_and_survive_a_restart() {
        let file = temp_file("notifications");
        let c = Controllers::new(Events::none(), Some(file.clone()));
        c.set_notifications(false, false);
        c.flush();
        let s = Controllers::new(Events::none(), Some(file.clone())).settings();
        assert!(!s.low_battery_alert && !s.haptic_feedback);
        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn the_live_config_reflects_profile_edits() {
        let c = Controllers::new(Events::none(), None);
        let _a = connect(&c, "AA");
        c.edit("AA", |p| {
            p.lightbar = [1, 2, 3];
            p.lightbar_mode = LightbarMode::Battery;
            p.motion.gyro = crate::motion::GyroMode::Mouse;
        });
        let live = c.live_config("AA");
        assert_eq!(live.fixed_color, [1, 2, 3]);
        assert_eq!(live.lightbar_mode, LightbarMode::Battery);
        assert_eq!(live.motion.gyro, crate::motion::GyroMode::Mouse);
        assert_eq!(live.rules, rules_from_shortcuts(&default_shortcuts()));
    }

    #[test]
    fn the_live_config_of_an_unknown_controller_is_the_neutral_default() {
        let c = Controllers::new(Events::none(), None);
        assert_eq!(c.live_config("ZZ"), Profile::default().live());
    }

    // --- Diagnostics --------------------------------------------------------------------

    #[test]
    fn the_diagnostic_lists_connected_and_remembered_controllers() {
        let c = Controllers::new(Events::none(), None);
        let _a = connect(&c, "AA");
        let _b = connect(&c, "BB");
        c.unregister("BB", "path-BB");
        c.set_battery("AA", Some(85), Charge::Charging);
        c.set_report_rate("AA", 248);

        let list = c.diagnostics_controllers();
        assert_eq!(list.len(), 2);
        let (a, b) = (&list[0], &list[1]);
        assert!(a.connected && a.path.as_deref() == Some("path-AA"));
        assert_eq!((a.battery, a.charge, a.reports_per_second), (Some(85), Charge::Charging, 248));
        assert!(!b.connected && b.path.is_none() && b.reports_per_second == 0);
    }

    #[test]
    fn the_diagnostic_shows_which_controllers_are_hidden() {
        let (c, _) = with_cloak(false);
        let _a = connect(&c, "AA");
        enable_auto(&c);
        c.set_cloaked("AA", true).unwrap();
        assert!(c.diagnostics_controllers()[0].hidden);
    }

    #[test]
    fn connections_and_disconnections_are_logged() {
        let c = Controllers::new(Events::none(), None);
        let _a = connect(&c, "AA");
        c.unregister("AA", "path-AA");
        let messages: Vec<_> = c.log().entries().into_iter().map(|e| e.message).collect();
        assert!(messages.iter().any(|m| m.contains("AA connected (USB)")), "{messages:?}");
        assert!(messages.iter().any(|m| m.contains("AA disconnected")), "{messages:?}");
    }

    #[test]
    fn a_late_unregister_is_not_logged_as_a_disconnection() {
        let c = Controllers::new(Events::none(), None);
        let _a = connect(&c, "AA");
        c.unregister("AA", "another-path");
        assert!(!c.log().entries().iter().any(|e| e.message.contains("disconnected")));
    }

    #[test]
    fn hidhide_failures_and_power_off_refusals_are_logged() {
        let (c, _) = with_cloak(true);
        let _a = connect(&c, "AA");
        enable_auto(&c);
        let _ = c.set_cloaked("AA", true);
        let _ = c.power_off("AA");
        let entries = c.log().entries();
        assert!(entries.iter().any(|e| e.source == "hidhide" && e.message.contains("refused")), "{entries:?}");
        assert!(entries.iter().any(|e| e.source == "power-off"), "{entries:?}");
    }

    #[test]
    fn a_corrupt_profile_file_is_reported_in_the_log() {
        let file = temp_file("corrupt-log");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "not json").unwrap();
        let c = Controllers::new(Events::none(), Some(file.clone()));
        assert!(c.log().entries().iter().any(|e| e.source == "profiles" && e.message.contains("unreadable")));
        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn missing_fields_in_old_profile_files_use_defaults() {
        let p: Profile = serde_json::from_str(r#"{"name":"Old"}"#).unwrap();
        assert_eq!(p.name, "Old");
        assert_eq!(p.lightbar, PALETTE[0]);
    }
}
