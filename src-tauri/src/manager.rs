//! Management of several controllers: detection, saved profiles and command routing.
//!
//! Each controller is identified by its MAC address. Its profile (name, colour, triggers…) is
//! saved to disk and reapplied on every connection.

use crate::bindings::{rules_from_shortcuts, with_menu_rule, AnalogBinding, Rule};
use crate::bluetooth;
use crate::controller::{self, Charge, Command, DeviceCtx};
use crate::diagnostics::{ControllerDiag, Log};
use crate::events::Events;
use crate::games::{self, GameRule};
use crate::hidhide::{Cloak, CloakStatus, NoCloak};
use crate::i18n::{self, Lang, Msg};
use crate::input::InputConfig;
use crate::lightbar::{LightbarMode, ModeColors};
use crate::menu::MenuView;
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
    /// The open menu takes the focus, so that a game in the background stops reacting to the controller.
    pub menu_focus: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { hidhide_auto: false, low_battery_alert: true, haptic_feedback: true, language: None, menu_focus: true }
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
    /// The colour of the controller, by identifier, to draw it in its colour.
    pub pad_color: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct OverlayPayload {
    /// One-off message ("Xbox 360 mode"…), shown above the list.
    pub message: Option<String>,
    /// Name of the icon drawn with the message (see `src/icons.ts`).
    pub icon: Option<String>,
    /// The controller menu, when it is open: the overlay then stays until it is closed.
    pub menu: Option<MenuView>,
    /// The overlay window must take the focus while the menu is open (see `Settings::menu_focus`).
    pub take_focus: bool,
    pub controllers: Vec<OverlayController>,
}

#[derive(Serialize, Clone, Debug)]
pub struct ControllerInfo {
    pub id: String,
    pub connected: bool,
    pub bluetooth: bool,
    /// The controller's name and the settings of the profile in force (see `effective_preset`).
    pub profile: Profile,
    /// The controller's default profile.
    pub preset: String,
    /// The colour of the controller, by identifier, if the user chose one.
    pub color: Option<String>,
    /// The game profile chosen by hand on this controller, if any.
    pub manual: Option<String>,
    /// The profile in force: the game in the foreground's, else the one chosen by hand, else `preset`.
    pub effective_preset: String,
}

/// The profiles and the game rules, for the profiles page.
#[derive(Serialize, Clone, Debug)]
pub struct PresetsInfo {
    pub presets: Vec<String>,
    pub games: Vec<GameRule>,
}

/// What is saved for each controller: its name and its default profile (the settings the user
/// edits; the interface does not show it as a profile).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
struct ControllerEntry {
    name: String,
    preset: String,
    /// A game profile the user is editing or trying out on this controller. Not saved: the
    /// controller goes back to its default profile at the next start.
    #[serde(skip)]
    manual: Option<String>,
    /// A game profile chosen from the controller menu. Not saved. Unlike `manual` it applies even
    /// while a game is in the foreground, and it wins over the profile of the game, until
    /// "Automatic" is chosen in the menu.
    #[serde(skip)]
    chosen: Option<String>,
    /// The colour of the physical controller, by identifier (see `src/colorways.ts`): it only decides
    /// how the controller is drawn. It belongs to the controller, not to a profile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    color: Option<String>,
}

/// The content of `profiles.json`. Format 1 was a map from controller identifier to profile; it is
/// converted when read, and older formats are brought up to date (see `STORE_VERSION`).
#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct Store {
    version: u32,
    controllers: BTreeMap<String, ControllerEntry>,
    /// The profiles, by name. They are shared: any controller can use any of them.
    presets: BTreeMap<String, Profile>,
    games: Vec<GameRule>,
}

/// Format 3 adds the rule that opens the controller menu to the profiles saved before it existed.
const STORE_VERSION: u32 = 3;

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
    controllers: BTreeMap<String, ControllerEntry>,
    presets: BTreeMap<String, Profile>,
    games: Vec<GameRule>,
    /// The profile that a game in the foreground imposes on every controller (not saved).
    auto_preset: Option<String>,
    /// Whether this application's window is in the foreground. A game profile chosen by hand is
    /// only in force while it is: it is for editing and trying out, so it must not follow the user
    /// into the game, where the game's own profile (or the default one) applies.
    ui_focused: bool,
    /// The controller menu is displayed (from `show_menu` to `hide_overlay`). It takes the focus, which
    /// puts a window of this application in the foreground: the game watcher must not take that for the
    /// player having left the game.
    menu_open: bool,
    active: BTreeMap<String, Active>,
    file: Option<PathBuf>,
    dirty: bool,
    settings: Settings,
    settings_dirty: bool,
    /// Controllers that WE hid: id -> HidHide instance path.
    hidden: BTreeMap<String, String>,
}

impl Inner {
    /// The profile in force for a controller: the one chosen from its menu, else the one of the game
    /// in the foreground, else the game profile chosen by hand (while this window is in the
    /// foreground), else the controller's default profile.
    fn effective_preset(&self, id: &str) -> Option<&str> {
        let entry = self.controllers.get(id)?;
        fn existing<'a>(presets: &BTreeMap<String, Profile>, name: &'a Option<String>) -> Option<&'a str> {
            name.as_deref().filter(|n| presets.contains_key(*n))
        }
        // A game only concerns the controllers that are connected: the others keep showing their own settings.
        let game = existing(&self.presets, &self.auto_preset).filter(|_| self.active.contains_key(id));
        Some(
            existing(&self.presets, &entry.chosen)
                .or(game)
                .or_else(|| existing(&self.presets, &entry.manual).filter(|_| self.ui_focused))
                .unwrap_or(entry.preset.as_str()),
        )
    }

    /// The controller as the rest of the application sees it: the settings of the profile in force,
    /// under the controller's own name.
    fn view(&self, id: &str) -> Option<Profile> {
        let entry = self.controllers.get(id)?;
        let preset = self.presets.get(self.effective_preset(id)?)?;
        Some(Profile { name: entry.name.clone(), ..preset.clone() })
    }
}

/// `base`, or `base 2`, `base 3`… if that name is taken. Never longer than `MAX_NAME_LEN`.
fn unique_name(base: &str, taken: &BTreeMap<String, Profile>) -> String {
    let base: String = base.trim().chars().take(MAX_NAME_LEN).collect();
    if !taken.contains_key(&base) {
        return base;
    }
    (2..)
        .map(|n| {
            let suffix = format!(" {n}");
            let head: String = base.chars().take(MAX_NAME_LEN - suffix.chars().count()).collect();
            format!("{head}{suffix}")
        })
        .find(|candidate| !taken.contains_key(candidate))
        .unwrap_or(base)
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
        let (store, migrated) = file.as_deref().map(|f| load_store(f, &log)).unwrap_or_default();
        let settings = file.as_deref().map(|f| load_settings(&settings_path(f), &log)).unwrap_or_default();
        if let Some(language) = settings.language {
            i18n::set_language(language);
        }
        Self {
            inner: Arc::new(Mutex::new(Inner {
                controllers: store.controllers,
                presets: store.presets,
                games: store.games,
                auto_preset: None,
                ui_focused: true,
                menu_open: false,
                active: BTreeMap::new(),
                file,
                dirty: migrated,
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
        self.lock().view(id).as_ref().map_or_else(|| Profile::default().live(), Profile::live)
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
            .controllers
            .iter()
            .map(|(id, entry)| {
                let active = inner.active.get(id);
                ControllerDiag {
                    id: id.clone(),
                    name: entry.name.clone(),
                    connected: active.is_some(),
                    bluetooth: active.is_some_and(|a| a.bluetooth),
                    path: active.map(|a| a.path.clone()),
                    battery: active.and_then(|a| a.battery),
                    charge: active.map_or(Charge::Unknown, |a| a.charge),
                    reports_per_second: active.map_or(0, |a| a.report_rate),
                    virtual_pad: inner.view(id).is_some_and(|p| p.virtual_pad),
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
            .controllers
            .iter()
            .filter_map(|(id, entry)| {
                let profile = inner.view(id)?;
                Some(ControllerInfo {
                    id: id.clone(),
                    connected: inner.active.contains_key(id),
                    bluetooth: inner.active.get(id).is_some_and(|a| a.bluetooth),
                    profile: profile.with_rules(),
                    preset: entry.preset.clone(),
                    color: entry.color.clone(),
                    manual: entry.manual.clone(),
                    effective_preset: inner.effective_preset(id).unwrap_or_default().to_string(),
                })
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
            let mut created = false;
            if !inner.controllers.contains_key(id) {
                // A new controller starts with a profile of its own, named after it, so that it
                // behaves as before until the user shares a profile between controllers.
                let count = inner.controllers.len();
                let name = Msg::ControllerName { n: count + 1 }.text();
                let preset = unique_name(&name, &inner.presets);
                let settings = Profile { lightbar: PALETTE[count % PALETTE.len()], ..Profile::default() };
                inner.presets.insert(preset.clone(), settings);
                inner.controllers.insert(id.to_string(), ControllerEntry { name, preset, ..Default::default() });
                created = true;
            }
            inner.active.insert(
                id.to_string(),
                Active { tx, bluetooth, path: path.to_string(), battery: None, charge: Charge::Unknown, report_rate: 0 },
            );
            inner.dirty |= created;
            // After the insertion: a controller that connects while a game is in the foreground gets its profile.
            inner.view(id).unwrap_or_default()
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
        let Some(mut profile) = inner.view(id) else {
            return false;
        };
        let preset = inner.effective_preset(id).unwrap_or_default().to_string();
        let before = profile.clone();
        f(&mut profile);
        profile.name = profile.name.trim().chars().take(MAX_NAME_LEN).collect();
        let changed = profile != before;
        let renamed = profile.name != before.name;
        if changed {
            // The name belongs to the controller; everything else is saved in the profile in force.
            if let Some(entry) = inner.controllers.get_mut(id) {
                entry.name = profile.name.clone();
            }
            inner.presets.insert(preset, Profile { name: String::new(), ..profile });
            inner.dirty = true;
        }
        drop(inner);
        if renamed {
            self.notify();
        }
        true
    }

    /// Chooses the colour a controller is drawn in (an identifier; `None` goes back to the default).
    /// Returns `false` if the controller is unknown.
    pub fn set_controller_color(&self, id: &str, color: Option<String>) -> bool {
        // Only a plain identifier is kept: it comes from the interface and ends up in the saved file.
        let color = color
            .map(|c| c.trim().to_ascii_lowercase())
            .filter(|c| !c.is_empty() && c.len() <= 32 && c.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-'));
        {
            let mut inner = self.lock();
            let Some(entry) = inner.controllers.get_mut(id) else {
                return false;
            };
            if entry.color != color {
                entry.color = color;
                inner.dirty = true;
            }
        }
        self.notify();
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
        let profile = self.lock().view(id);
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
        self.emit_overlay(source, message, None, None);
    }

    /// Shows a short message with an icon ("gamepad", "xbox", "power", "motion", "battery-low",
    /// "layers", "alert", "message").
    pub fn show_notice(&self, source: &str, icon: &str, message: String) {
        self.emit_overlay(source, Some(message), Some(icon.to_string()), None);
    }

    /// Shows the controller menu in the overlay, above the list of controllers. It stays displayed
    /// until `hide_overlay`.
    pub fn show_menu(&self, source: &str, menu: MenuView) {
        self.lock().menu_open = true;
        self.emit_overlay(source, None, None, Some(menu));
    }

    /// Hides the overlay at once (the menu has been closed).
    pub fn hide_overlay(&self) {
        self.lock().menu_open = false;
        self.events.emit("overlay-hide", &());
    }

    /// Whether the controller menu is displayed.
    pub fn menu_open(&self) -> bool {
        self.lock().menu_open
    }

    fn emit_overlay(&self, source: &str, message: Option<String>, icon: Option<String>, menu: Option<MenuView>) {
        let payload = {
            let inner = self.lock();
            // The overlay is about the controller that asked for it (its menu, its alert, its shortcut).
            // Without a source (a game profile coming into force, which concerns every controller),
            // all the connected controllers are shown.
            let controllers = inner
                .active
                .iter()
                .filter(|(id, _)| source.is_empty() || id.as_str() == source)
                .map(|(id, active)| {
                    let profile = inner.view(id).unwrap_or_default();
                    OverlayController {
                        id: id.clone(),
                        name: if profile.name.is_empty() { id.clone() } else { profile.name },
                        color: profile.lightbar,
                        bluetooth: active.bluetooth,
                        percent: active.battery,
                        charge: active.charge,
                        xbox_mode: profile.virtual_pad,
                        source: id == source,
                        pad_color: inner.controllers.get(id).and_then(|e| e.color.clone()),
                    }
                })
                .collect();
            let take_focus = menu.is_some() && inner.settings.menu_focus;
            OverlayPayload { message, icon, menu, take_focus, controllers }
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
                        || inner.controllers.get(*id).is_some_and(|c| c.name.eq_ignore_ascii_case(n))
                })
                .cloned()
                .collect(),
        }
    }

    // --- Profiles -------------------------------------------------------------------------

    /// The profiles and the game rules.
    pub fn presets_info(&self) -> PresetsInfo {
        let inner = self.lock();
        PresetsInfo { presets: inner.presets.keys().cloned().collect(), games: inner.games.clone() }
    }

    pub fn games(&self) -> Vec<GameRule> {
        self.lock().games.clone()
    }

    /// Sends the profile in force to a controller: light bar, LEDs, triggers, virtual controller,
    /// rules and the rest.
    fn apply_profile(&self, id: &str) {
        self.reset_to_profile(id);
        self.send(id, Command::ProfileChanged);
    }

    fn apply_all(&self) {
        let ids: Vec<String> = self.lock().active.keys().cloned().collect();
        for id in ids {
            self.apply_profile(&id);
        }
    }

    pub fn set_menu_focus(&self, enabled: bool) {
        let mut inner = self.lock();
        if inner.settings.menu_focus != enabled {
            inner.settings.menu_focus = enabled;
            inner.settings_dirty = true;
        }
    }

    /// What the menu offers for the profile row: the game profiles, and the one in use. `None`
    /// stands for "Automatic".
    pub fn menu_profiles(&self, id: &str) -> (Vec<String>, Option<String>) {
        let inner = self.lock();
        let names = inner.games.iter().map(|g| g.preset.clone()).collect();
        let chosen = inner.controllers.get(id).and_then(|e| e.chosen.clone());
        (names, chosen)
    }

    /// Moves the profile chosen from the menu to the next (`direction` 1) or previous (-1) one. The
    /// order is: Automatic, then the game profiles; it wraps around. Returns the new choice.
    pub fn cycle_chosen(&self, id: &str, direction: i8) -> Option<String> {
        let new = {
            let mut inner = self.lock();
            let options: Vec<Option<String>> =
                std::iter::once(None).chain(inner.games.iter().map(|g| Some(g.preset.clone()))).collect();
            let entry = inner.controllers.get_mut(id)?;
            let position = options.iter().position(|o| *o == entry.chosen).unwrap_or(0);
            let n = options.len() as i64;
            let next = (position as i64 + i64::from(direction)).rem_euclid(n) as usize;
            entry.chosen = options[next].clone();
            entry.chosen.clone()
        };
        self.apply_profile(id);
        self.notify();
        new
    }

    /// Tells whether this application's window is in the foreground. A game profile chosen by hand
    /// only applies while it is; the controllers are updated when that changes.
    pub fn set_ui_focus(&self, focused: bool) {
        let affected: Vec<String> = {
            let mut inner = self.lock();
            if inner.ui_focused == focused {
                return;
            }
            inner.ui_focused = focused;
            inner.controllers.iter().filter(|(_, e)| e.manual.is_some()).map(|(id, _)| id.clone()).collect()
        };
        if affected.is_empty() {
            return;
        }
        for id in &affected {
            self.apply_profile(id);
        }
        self.notify();
    }

    /// Puts a game profile on a controller by hand, to edit it or try it out; an empty name goes
    /// back to the default profile. This is not saved, only applies while this window is in the
    /// foreground (see `set_ui_focus`), and a game in the foreground still wins.
    pub fn set_controller_preset(&self, id: &str, preset: &str) -> Result<(), String> {
        {
            let mut inner = self.lock();
            let is_game_profile = inner.games.iter().any(|g| g.preset == preset);
            if !preset.is_empty() && !is_game_profile {
                return Err(Msg::PresetNotFound.text());
            }
            let Some(entry) = inner.controllers.get_mut(id) else {
                return Err(Msg::NotConnected.text());
            };
            entry.manual = (!preset.is_empty()).then(|| preset.to_string());
        }
        self.apply_profile(id);
        self.notify();
        Ok(())
    }

    /// A profile name as the user typed it: trimmed, not empty, not too long.
    fn valid_preset_name(name: &str) -> Result<String, String> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > MAX_NAME_LEN {
            return Err(Msg::PresetNameInvalid.text());
        }
        Ok(name.to_string())
    }

    /// Creates a profile as a copy of `from`. Returns its name.
    pub fn create_preset(&self, name: &str, from: &str) -> Result<String, String> {
        let name = Self::valid_preset_name(name)?;
        {
            let mut inner = self.lock();
            let Some(source) = inner.presets.get(from).cloned() else {
                return Err(Msg::PresetNotFound.text());
            };
            if inner.presets.contains_key(&name) {
                return Err(Msg::PresetExists.text());
            }
            inner.presets.insert(name.clone(), source);
            inner.dirty = true;
        }
        self.notify();
        Ok(name)
    }

    /// Creates a game profile: a copy of `from` (the settings the user has now), used while the
    /// program `exe` is in the foreground. Returns the name of the new profile.
    pub fn create_game_profile(&self, name: &str, exe: &str, from: &str) -> Result<String, String> {
        let exe = exe.trim();
        if exe.is_empty() {
            return Err(Msg::GameExeInvalid.text());
        }
        if self.lock().games.iter().any(|g| g.exe.eq_ignore_ascii_case(exe)) {
            return Err(Msg::GameExists.text());
        }
        let name = self.create_preset(name, from)?;
        let mut rules = self.games();
        rules.push(GameRule { exe: exe.to_string(), preset: name.clone() });
        self.set_games(rules);
        Ok(name)
    }

    /// Renames a profile everywhere it is used. Returns the new name.
    pub fn rename_preset(&self, old: &str, new: &str) -> Result<String, String> {
        let new = Self::valid_preset_name(new)?;
        {
            let mut inner = self.lock();
            if !inner.presets.contains_key(old) {
                return Err(Msg::PresetNotFound.text());
            }
            if new == old {
                return Ok(new);
            }
            if inner.presets.contains_key(&new) {
                return Err(Msg::PresetExists.text());
            }
            if let Some(profile) = inner.presets.remove(old) {
                inner.presets.insert(new.clone(), profile);
            }
            for entry in inner.controllers.values_mut() {
                if entry.preset == old {
                    entry.preset = new.clone();
                }
                if entry.manual.as_deref() == Some(old) {
                    entry.manual = Some(new.clone());
                }
                if entry.chosen.as_deref() == Some(old) {
                    entry.chosen = Some(new.clone());
                }
            }
            for rule in inner.games.iter_mut().filter(|r| r.preset == old) {
                rule.preset = new.clone();
            }
            if inner.auto_preset.as_deref() == Some(old) {
                inner.auto_preset = Some(new.clone());
            }
            inner.dirty = true;
        }
        self.notify();
        Ok(new)
    }

    /// Deletes a game profile together with its rule. The controllers that were showing it go back
    /// to their default profile. A default profile cannot be deleted.
    pub fn delete_preset(&self, name: &str) -> Result<(), String> {
        {
            let mut inner = self.lock();
            if !inner.presets.contains_key(name) {
                return Err(Msg::PresetNotFound.text());
            }
            // A controller's default profile is its settings: only game profiles can be deleted.
            if inner.controllers.values().any(|e| e.preset == name) {
                return Err(Msg::DefaultProfileProtected.text());
            }
            inner.presets.remove(name);
            for entry in inner.controllers.values_mut() {
                if entry.manual.as_deref() == Some(name) {
                    entry.manual = None;
                }
                if entry.chosen.as_deref() == Some(name) {
                    entry.chosen = None;
                }
            }
            inner.games.retain(|r| r.preset != name);
            if inner.auto_preset.as_deref() == Some(name) {
                inner.auto_preset = None;
            }
            inner.dirty = true;
        }
        self.apply_all();
        self.notify();
        Ok(())
    }

    /// Replaces the game rules (cleaned up: see `games::sanitize`).
    pub fn set_games(&self, rules: Vec<GameRule>) {
        {
            let mut inner = self.lock();
            let known: Vec<&str> = inner.presets.keys().map(String::as_str).collect();
            let clean = games::sanitize(rules, &known);
            if clean != inner.games {
                inner.games = clean;
                inner.dirty = true;
            }
        }
        self.notify();
    }

    /// Imposes (or, with `None`, releases) the profile of the game in the foreground on every
    /// controller. Called by the foreground watcher.
    pub fn set_auto_preset(&self, preset: Option<String>) {
        {
            let mut inner = self.lock();
            if preset.as_ref().is_some_and(|p| !inner.presets.contains_key(p)) || inner.auto_preset == preset {
                return;
            }
            inner.auto_preset = preset.clone();
        }
        // The overlay says which profile is in force now, since the change is made without the user
        // touching the application. Nothing to show if no controller is connected.
        let message = match &preset {
            Some(name) => {
                self.log.info("profiles", Msg::LogAutoProfile { name: name.clone() }.text());
                Msg::ProfileApplied { name: name.clone() }.text()
            }
            None => {
                self.log.info("profiles", Msg::LogAutoProfileEnd.text());
                Msg::ProfileDefault.text()
            }
        };
        self.apply_all();
        self.notify();
        if !self.lock().active.is_empty() {
            self.show_notice("", "layers", message);
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
            let store = Store {
                version: STORE_VERSION,
                controllers: inner.controllers.clone(),
                presets: inner.presets.clone(),
                games: inner.games.clone(),
            };
            let json = match serde_json::to_string_pretty(&store) {
                Ok(j) => j,
                Err(e) => {
                    eprintln!("profiles: serialization failed ({e})");
                    return;
                }
            };
            inner.dirty = false;
            (path, json)
        };
        backup_profiles(&path, &json);
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
                    .filter(|id| inner.view(id).is_some_and(|p| p.virtual_pad))
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
            let wants_xbox = inner.view(id).is_some_and(|p| p.virtual_pad);
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

/// Reads `profiles.json`. Returns what was read and whether it was converted from format 1 (so that
/// it gets saved in the new format). An unreadable file is copied aside, never overwritten.
fn load_store(path: &std::path::Path, log: &Log) -> (Store, bool) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return (Store::default(), false);
    };
    let unreadable = |reason: String| {
        // Never silently overwrite the user's settings.
        let backup = path.with_extension("json.bad");
        eprintln!("profiles unreadable ({reason}): copying to {}", backup.display());
        log.error("profiles", Msg::LogProfilesUnreadable { reason, backup: backup.display().to_string() }.text());
        let _ = std::fs::copy(path, backup);
        (Store::default(), false)
    };
    let value: serde_json::Value = match serde_json::from_str(&text) {
        Ok(value) => value,
        Err(e) => return unreadable(e.to_string()),
    };
    let (mut store, mut migrated) = if value.get("version").is_some() {
        match serde_json::from_value::<Store>(value) {
            Ok(store) => (store, false),
            Err(e) => return unreadable(e.to_string()),
        }
    } else {
        match serde_json::from_value::<BTreeMap<String, Profile>>(value) {
            Ok(old) => {
                // Keep the old file: the conversion must be reversible.
                let backup = path.with_file_name("profiles.v1.json");
                if !backup.exists() {
                    let _ = std::fs::copy(path, backup);
                }
                (migrate_v1(merge_duplicate_ids(old)), true)
            }
            Err(e) => return unreadable(e.to_string()),
        }
    };
    // Holding PS used to be built in; it is now a rule like the others. The profiles saved before get
    // it once: deleting it afterwards is final, since the file is then in the newest format.
    if store.version < 3 {
        for preset in store.presets.values_mut() {
            let rules = preset.rules.take().unwrap_or_else(|| rules_from_shortcuts(&preset.shortcuts));
            preset.rules = Some(with_menu_rule(rules));
        }
        migrated = true;
    }
    normalise(&mut store);
    (store, migrated)
}

/// Format 1 kept a complete profile per controller. Each one becomes a profile named after its
/// controller, which that controller keeps using: nothing changes for the user.
fn migrate_v1(old: BTreeMap<String, Profile>) -> Store {
    // Still "format 2" until the later migrations have been applied by `load_store`.
    let mut store = Store { version: 2, ..Store::default() };
    for (id, profile) in old {
        let name = if profile.name.trim().is_empty() { id.clone() } else { profile.name.clone() };
        let preset = unique_name(&name, &store.presets);
        store.presets.insert(preset.clone(), Profile { name: String::new(), ..profile });
        store.controllers.insert(id, ControllerEntry { name, preset, ..Default::default() });
    }
    store
}

/// Makes a store that was read coherent: every controller has a profile that exists, and the game
/// rules only point to existing profiles.
fn normalise(store: &mut Store) {
    store.version = STORE_VERSION;
    for entry in store.controllers.values() {
        store.presets.entry(entry.preset.clone()).or_default();
    }
    let known: Vec<&str> = store.presets.keys().map(String::as_str).collect();
    store.games = games::sanitize(std::mem::take(&mut store.games), &known);
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

/// How many automatic backups of the profiles are kept, and the minimum time between two of them.
const BACKUPS_KEPT: usize = 10;
const BACKUP_INTERVAL_SECS: u64 = 15 * 60;

/// Keeps copies of the profiles file in a `backups` folder next to it. Before a new version of the
/// file is written, the one on disk is copied there, unless another copy is less than
/// `BACKUP_INTERVAL_SECS` old (the file is rewritten at every edit: one copy per edit would drown the
/// useful ones). The newest `BACKUPS_KEPT` copies are kept. A failure here never prevents saving.
fn backup_profiles(path: &std::path::Path, new_contents: &str) {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let _ = rotate_backups(path, new_contents, now);
}

fn rotate_backups(path: &std::path::Path, new_contents: &str, now: u64) -> std::io::Result<()> {
    let Ok(existing) = std::fs::read_to_string(path) else {
        return Ok(()); // nothing to keep yet
    };
    if existing == new_contents {
        return Ok(());
    }
    let dir = path.with_file_name("backups");
    std::fs::create_dir_all(&dir)?;
    // Names are `profiles-<seconds since 1970>.json`: sorted by name, they are sorted by date.
    let mut names: Vec<String> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.starts_with("profiles-") && n.ends_with(".json"))
        .collect();
    names.sort();
    let newest = names.last().and_then(|n| n.trim_start_matches("profiles-").trim_end_matches(".json").parse::<u64>().ok());
    if newest.is_some_and(|t| now.saturating_sub(t) < BACKUP_INTERVAL_SECS) {
        return Ok(());
    }
    std::fs::write(dir.join(format!("profiles-{now}.json")), existing)?;
    names.push(format!("profiles-{now}.json"));
    names.sort();
    while names.len() > BACKUPS_KEPT {
        let oldest = names.remove(0);
        let _ = std::fs::remove_file(dir.join(oldest));
    }
    Ok(())
}

fn write_atomic(path: &std::path::Path, contents: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, path)
}

/// How often the foreground program is looked at.
const WATCH_INTERVAL: Duration = Duration::from_millis(250);

/// Starts the thread that follows the program in the foreground and applies the profile of the
/// game that is played (see `games`). It does nothing, not even a system call, while there are no
/// game rules.
pub fn start_game_watcher(controllers: Controllers) {
    let spawned = std::thread::Builder::new().name("game-watcher".into()).spawn(move || {
        let mut watcher = games::Watcher::default();
        loop {
            std::thread::sleep(WATCH_INTERVAL);
            let rules = controllers.games();
            if rules.is_empty() && watcher.current().is_none() {
                continue;
            }
            // The menu takes the focus: what is in the foreground says nothing about the game while it is open.
            if controllers.menu_open() {
                continue;
            }
            let change = watcher
                .forget_unknown(&rules)
                .or_else(|| watcher.update(&games::foreground(), &rules, std::time::Instant::now()));
            if let Some(preset) = change {
                controllers.set_auto_preset(preset);
            }
        }
    });
    if let Err(e) = spawned {
        eprintln!("game watcher: {e}");
    }
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
    fn overlay_shows_the_controller_that_asked_with_its_battery_and_mode() {
        let (events, log) = capturing_events();
        let c = Controllers::new(events, None);
        let _a = connect(&c, "AA");
        let _b = connect(&c, "BB");
        c.edit("BB", |p| p.virtual_pad = true);
        c.set_battery("AA", Some(85), Charge::Charging);
        log.lock().unwrap().clear(); // ignore the "controllers-changed" events of the connections

        c.show_overlay("AA", Some("Native mode".into()));
        c.show_overlay("BB", None);

        let log = log.lock().unwrap();
        assert_eq!(log.len(), 2);
        let (name, payload) = &log[0];
        assert_eq!(name, "overlay");
        assert_eq!(payload["message"], "Native mode");
        let list = payload["controllers"].as_array().unwrap();
        assert_eq!(list.len(), 1, "only the controller that asked");
        assert_eq!(list[0]["id"], "AA");
        assert_eq!(list[0]["percent"], 85);
        assert_eq!(list[0]["charge"], "charging");
        assert_eq!(list[0]["source"], true);
        assert_eq!(list[0]["xbox_mode"], false);

        let other = log[1].1["controllers"].as_array().unwrap();
        assert_eq!((other.len(), other[0]["id"].as_str()), (1, Some("BB")));
        assert_eq!(other[0]["xbox_mode"], true);
        assert!(other[0]["percent"].is_null());
    }

    #[test]
    fn a_notice_without_a_source_concerns_every_connected_controller() {
        let (events, log) = capturing_events();
        let c = Controllers::new(events, None);
        let _a = connect(&c, "AA");
        let _b = connect(&c, "BB");
        log.lock().unwrap().clear();
        c.show_notice("", "layers", "Profile: Racing".into());
        let log = log.lock().unwrap();
        assert_eq!(log[0].1["controllers"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn the_menu_only_shows_the_controller_that_opened_it() {
        let (events, log) = capturing_events();
        let c = Controllers::new(events, None);
        let _a = connect(&c, "AA");
        let _b = connect(&c, "BB");
        log.lock().unwrap().clear();
        c.show_menu("BB", MenuView { rows: vec![], cursor: 0 });
        let log = log.lock().unwrap();
        let list = log[0].1["controllers"].as_array().unwrap();
        assert_eq!((list.len(), list[0]["id"].as_str()), (1, Some("BB")));
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

    // --- Game profiles --------------------------------------------------------------------

    /// A controller with a game profile "Racing" for `game.exe`.
    fn with_racing_profile() -> (Controllers, Receiver<Command>) {
        let c = Controllers::new(Events::none(), None);
        let rx = connect(&c, "AA");
        c.create_game_profile("Racing", "game.exe", "Controller 1").unwrap();
        (c, rx)
    }

    #[test]
    fn a_new_controller_starts_with_a_default_profile_of_its_own() {
        let c = Controllers::new(Events::none(), None);
        let _a = connect(&c, "AA");
        let _b = connect(&c, "BB");
        let list = c.list();
        assert_eq!((list[0].preset.as_str(), list[1].preset.as_str()), ("Controller 1", "Controller 2"));
        assert_eq!(list[0].effective_preset, "Controller 1");
        assert_eq!(list[0].manual, None);
    }

    #[test]
    fn a_game_profile_starts_as_a_copy_of_the_current_settings() {
        let c = Controllers::new(Events::none(), None);
        let _a = connect(&c, "AA");
        c.edit("AA", |p| p.lightbar = [1, 2, 3]);
        c.create_game_profile("Racing", "game.exe", "Controller 1").unwrap();
        c.set_controller_preset("AA", "Racing").unwrap();
        assert_eq!(c.live_config("AA").fixed_color, [1, 2, 3]);
        assert_eq!(c.games(), [GameRule { exe: "game.exe".into(), preset: "Racing".into() }]);
    }

    #[test]
    fn editing_a_game_profile_leaves_the_default_profile_alone() {
        let (c, _rx) = with_racing_profile();
        let default_color = c.live_config("AA").fixed_color;
        c.set_controller_preset("AA", "Racing").unwrap();
        c.edit("AA", |p| {
            p.lightbar = [9, 9, 9];
            p.name = "Salon".into();
        });
        assert_eq!(c.live_config("AA").fixed_color, [9, 9, 9]);
        c.set_controller_preset("AA", "").unwrap();
        assert_eq!(c.live_config("AA").fixed_color, default_color, "back to the default settings, untouched");
        assert_eq!(c.list()[0].profile.name, "Salon", "the name belongs to the controller");
        c.set_controller_preset("AA", "Racing").unwrap();
        assert_eq!(c.live_config("AA").fixed_color, [9, 9, 9]);
    }

    #[test]
    fn choosing_a_game_profile_by_hand_is_not_saved() {
        let file = temp_file("manual");
        let c = Controllers::new(Events::none(), Some(file.clone()));
        let _a = connect(&c, "AA");
        c.create_game_profile("Racing", "game.exe", "Controller 1").unwrap();
        c.set_controller_preset("AA", "Racing").unwrap();
        assert_eq!(c.list()[0].manual.as_deref(), Some("Racing"));
        c.flush();
        let reloaded = Controllers::new(Events::none(), Some(file.clone()));
        assert_eq!(reloaded.list()[0].effective_preset, "Controller 1");
        assert_eq!(reloaded.games().len(), 1, "the game profile itself is saved");
        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn a_game_profile_chosen_by_hand_does_not_follow_the_user_out_of_the_window() {
        let (c, rx) = with_racing_profile();
        c.set_controller_preset("AA", "Racing").unwrap();
        assert_eq!(c.list()[0].effective_preset, "Racing");

        let _ = rx.try_iter().count();
        c.set_ui_focus(false); // the user goes to the game
        assert_eq!(c.list()[0].effective_preset, "Controller 1", "the default profile applies again");
        assert_eq!(c.list()[0].manual.as_deref(), Some("Racing"), "the choice is kept, only suspended");
        assert!(rx.try_iter().any(|cmd| cmd == Command::ProfileChanged), "the controller is updated");

        c.set_ui_focus(true); // back in the window
        assert_eq!(c.list()[0].effective_preset, "Racing");
    }

    #[test]
    fn the_game_in_the_foreground_applies_whether_or_not_a_profile_was_chosen_by_hand() {
        let (c, _rx) = with_racing_profile();
        c.set_controller_preset("AA", "Racing").unwrap();
        c.set_ui_focus(false);
        c.set_auto_preset(Some("Racing".into()));
        assert_eq!(c.list()[0].effective_preset, "Racing");
        c.set_auto_preset(None);
        assert_eq!(c.list()[0].effective_preset, "Controller 1", "leaving the game: default, not the hand-picked one");
    }

    #[test]
    fn losing_focus_without_a_hand_picked_profile_touches_nothing() {
        let (c, rx) = with_racing_profile();
        let _ = rx.try_iter().count();
        c.set_ui_focus(false);
        c.set_ui_focus(true);
        assert_eq!(rx.try_iter().count(), 0, "no command is sent when there is nothing to change");
    }

    // --- Controller menu ----------------------------------------------------------------

    #[test]
    fn the_profile_chosen_from_the_menu_wins_over_the_game_and_survives_leaving_it() {
        let (c, _rx) = with_racing_profile();
        c.create_game_profile("Shooter", "shoot.exe", "Controller 1").unwrap();
        assert_eq!(c.cycle_chosen("AA", 1).as_deref(), Some("Racing"));
        assert_eq!(c.list()[0].effective_preset, "Racing");

        c.set_auto_preset(Some("Shooter".into()));
        assert_eq!(c.list()[0].effective_preset, "Racing", "the menu's choice beats the game in the foreground");
        c.set_auto_preset(None);
        c.set_ui_focus(false);
        assert_eq!(c.list()[0].effective_preset, "Racing", "and it does not depend on this window's focus");
    }

    #[test]
    fn the_menu_cycles_through_automatic_and_the_game_profiles_in_both_directions() {
        let (c, _rx) = with_racing_profile();
        c.create_game_profile("Shooter", "shoot.exe", "Controller 1").unwrap();
        assert_eq!(c.cycle_chosen("AA", 1).as_deref(), Some("Racing"));
        assert_eq!(c.cycle_chosen("AA", 1).as_deref(), Some("Shooter"));
        assert_eq!(c.cycle_chosen("AA", 1), None, "after the last profile: automatic again");
        assert_eq!(c.cycle_chosen("AA", -1).as_deref(), Some("Shooter"), "and backwards from automatic");
        assert_eq!(c.cycle_chosen("ZZ", 1), None, "an unknown controller is ignored");
    }

    #[test]
    fn choosing_automatic_gives_back_the_game_and_the_default_profile() {
        let (c, _rx) = with_racing_profile();
        c.cycle_chosen("AA", 1);
        c.cycle_chosen("AA", 1); // back to automatic
        assert_eq!(c.menu_profiles("AA").1, None);
        assert_eq!(c.list()[0].effective_preset, "Controller 1");
        c.set_auto_preset(Some("Racing".into()));
        assert_eq!(c.list()[0].effective_preset, "Racing");
    }

    #[test]
    fn choosing_a_profile_from_the_menu_sends_it_to_the_controller() {
        let (c, rx) = with_racing_profile();
        c.set_controller_preset("AA", "Racing").unwrap();
        c.edit("AA", |p| p.lightbar = [200, 100, 50]);
        c.set_controller_preset("AA", "").unwrap();
        let _ = rx.try_iter().count();
        c.cycle_chosen("AA", 1);
        let cmds: Vec<_> = rx.try_iter().collect();
        assert!(cmds.contains(&Command::Lightbar(200, 100, 50)), "{cmds:?}");
        assert!(cmds.contains(&Command::ProfileChanged));
    }

    #[test]
    fn the_menu_choice_is_not_saved_and_follows_renames_and_deletions() {
        let file = temp_file("chosen");
        let c = Controllers::new(Events::none(), Some(file.clone()));
        let _a = connect(&c, "AA");
        c.create_game_profile("Racing", "game.exe", "Controller 1").unwrap();
        c.cycle_chosen("AA", 1);
        c.rename_preset("Racing", "Cars").unwrap();
        assert_eq!(c.menu_profiles("AA").1.as_deref(), Some("Cars"));
        c.flush();
        let reloaded = Controllers::new(Events::none(), Some(file.clone()));
        assert_eq!(reloaded.menu_profiles("AA").1, None, "not remembered across restarts");
        c.delete_preset("Cars").unwrap();
        assert_eq!(c.menu_profiles("AA").1, None);
        assert_eq!(c.list()[0].effective_preset, "Controller 1");
        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn the_menu_focus_setting_is_on_by_default_and_is_saved() {
        let file = temp_file("menu-setting");
        let c = Controllers::new(Events::none(), Some(file.clone()));
        assert!(c.settings().menu_focus);
        c.set_menu_focus(false);
        c.flush();
        assert!(!Controllers::new(Events::none(), Some(file.clone())).settings().menu_focus);
        let old: Settings = serde_json::from_str(r#"{"hidhide_auto":true,"controller_menu":false}"#).unwrap();
        assert!(old.menu_focus, "old settings files keep working, and the removed option is ignored");
        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn the_menu_takes_the_focus_unless_the_setting_is_off() {
        let (events, log) = capturing_events();
        let c = Controllers::new(events, None);
        let _a = connect(&c, "AA");
        let menu = || MenuView { rows: vec![], cursor: 0 };
        log.lock().unwrap().clear();

        c.show_menu("AA", menu());
        c.show_overlay("AA", Some("hello".into()));
        c.set_menu_focus(false);
        c.show_menu("AA", menu());

        let flags: Vec<bool> = log.lock().unwrap().iter().filter(|(n, _)| n == "overlay").map(|(_, p)| p["take_focus"].as_bool().unwrap()).collect();
        assert_eq!(flags, [true, false, false], "only the menu takes the focus, and only if allowed");
    }

    #[test]
    fn the_menu_is_known_to_be_open_from_its_display_to_its_hiding() {
        let c = Controllers::new(Events::none(), None);
        let _a = connect(&c, "AA");
        assert!(!c.menu_open());
        c.show_menu("AA", MenuView { rows: vec![], cursor: 0 });
        assert!(c.menu_open());
        c.show_overlay("AA", Some("a message".into()));
        assert!(c.menu_open(), "a message shown meanwhile does not close it");
        c.hide_overlay();
        assert!(!c.menu_open());
    }

    #[test]
    fn the_menu_is_shown_in_the_overlay_and_hidden_on_request() {
        let (events, log) = capturing_events();
        let c = Controllers::new(events, None);
        let _a = connect(&c, "AA");
        log.lock().unwrap().clear();
        let menu = MenuView { rows: vec![], cursor: 0 };
        c.show_menu("AA", menu);
        c.hide_overlay();
        let log = log.lock().unwrap();
        assert_eq!(log[0].0, "overlay");
        assert_eq!(log[0].1["menu"]["cursor"], 0);
        assert!(log[0].1["message"].is_null());
        assert_eq!(log[1].0, "overlay-hide");
    }

    // --- Controller colour ------------------------------------------------------------------

    #[test]
    fn a_controller_has_no_colour_until_one_is_chosen() {
        let c = Controllers::new(Events::none(), None);
        let _a = connect(&c, "AA");
        assert_eq!(c.list()[0].color, None);
    }

    #[test]
    fn the_colour_belongs_to_the_controller_and_not_to_its_profile() {
        let (c, _rx) = with_racing_profile();
        assert!(c.set_controller_color("AA", Some("cosmic-red".into())));
        c.set_controller_preset("AA", "Racing").unwrap();
        assert_eq!(c.list()[0].color.as_deref(), Some("cosmic-red"), "still there on a game profile");
        c.set_auto_preset(Some("Racing".into()));
        assert_eq!(c.list()[0].color.as_deref(), Some("cosmic-red"));
    }

    #[test]
    fn the_colour_survives_a_restart_and_can_be_cleared() {
        let file = temp_file("colour");
        let c = Controllers::new(Events::none(), Some(file.clone()));
        let _a = connect(&c, "AA");
        c.set_controller_color("AA", Some("nova-pink".into()));
        c.flush();
        let reloaded = Controllers::new(Events::none(), Some(file.clone()));
        assert_eq!(reloaded.list()[0].color.as_deref(), Some("nova-pink"));
        reloaded.set_controller_color("AA", None);
        reloaded.flush();
        assert_eq!(Controllers::new(Events::none(), Some(file.clone())).list()[0].color, None);
        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn only_a_plain_identifier_is_kept_as_a_colour() {
        let c = Controllers::new(Events::none(), None);
        let _a = connect(&c, "AA");
        c.set_controller_color("AA", Some("  Midnight-Black ".into()));
        assert_eq!(c.list()[0].color.as_deref(), Some("midnight-black"), "trimmed and lowercased");
        for bad in ["", "   ", "red;drop", "../x", &"x".repeat(33)] {
            c.set_controller_color("AA", Some(bad.into()));
            assert_eq!(c.list()[0].color, None, "{bad:?} is not an identifier");
        }
        assert!(!c.set_controller_color("ZZ", Some("white".into())), "an unknown controller is reported");
    }

    #[test]
    fn the_overlay_carries_the_colour_of_the_controller() {
        let (events, log) = capturing_events();
        let c = Controllers::new(events, None);
        let _a = connect(&c, "AA");
        c.set_controller_color("AA", Some("cobalt-blue".into()));
        log.lock().unwrap().clear();
        c.show_overlay("AA", None);
        assert_eq!(log.lock().unwrap()[0].1["controllers"][0]["pad_color"], "cobalt-blue");
    }

    #[test]
    fn a_game_only_applies_to_the_controllers_that_are_connected() {
        let (c, _rx) = with_racing_profile();
        let _b = connect(&c, "BB");
        c.unregister("BB", "path-BB");
        c.set_auto_preset(Some("Racing".into()));
        let list = c.list();
        assert_eq!(list[0].effective_preset, "Racing");
        assert_eq!(list[1].effective_preset, "Controller 2", "a disconnected controller keeps its own settings");
    }

    #[test]
    fn the_overlay_announces_the_profile_when_a_game_comes_and_goes() {
        let (events, log) = capturing_events();
        let c = Controllers::new(events, None);
        let _a = connect(&c, "AA");
        c.create_game_profile("Racing", "game.exe", "Controller 1").unwrap();
        log.lock().unwrap().clear();

        c.set_auto_preset(Some("Racing".into()));
        c.set_auto_preset(Some("Racing".into())); // no change: nothing more to announce
        c.set_auto_preset(None);

        let messages: Vec<String> = log
            .lock()
            .unwrap()
            .iter()
            .filter(|(name, _)| name == "overlay")
            .map(|(_, payload)| payload["message"].as_str().unwrap_or_default().to_string())
            .collect();
        assert_eq!(messages, ["Profile: Racing", "Default profile"]);
    }

    #[test]
    fn nothing_is_announced_when_no_controller_is_connected() {
        let (events, log) = capturing_events();
        let c = Controllers::new(events, None);
        let _a = connect(&c, "AA");
        c.create_game_profile("Racing", "game.exe", "Controller 1").unwrap();
        c.unregister("AA", "path-AA");
        log.lock().unwrap().clear();
        c.set_auto_preset(Some("Racing".into()));
        assert!(log.lock().unwrap().iter().all(|(name, _)| name != "overlay"));
    }

    #[test]
    fn a_controller_that_connects_during_a_game_gets_the_profile_of_the_game() {
        let (c, _rx) = with_racing_profile();
        c.set_auto_preset(Some("Racing".into()));
        let (tx, rx) = mpsc::channel();
        let profile = c.register("CC", "path-CC", false, tx);
        assert_eq!(c.list()[1].effective_preset, "Racing");
        assert_eq!(profile.name, "Controller 2");
        drop(rx);
    }

    #[test]
    fn only_game_profiles_can_be_chosen() {
        let (c, _rx) = with_racing_profile();
        assert!(c.set_controller_preset("AA", "Controller 1").is_err(), "a default profile is not a choice");
        assert!(c.set_controller_preset("AA", "Nope").is_err());
        assert!(c.set_controller_preset("ZZ", "Racing").is_err());
        assert_eq!(c.list()[0].manual, None);
    }

    #[test]
    fn choosing_a_game_profile_sends_it_to_the_controller() {
        let (c, rx) = with_racing_profile();
        c.set_controller_preset("AA", "Racing").unwrap();
        c.edit("AA", |p| p.lightbar = [255, 0, 0]);
        c.set_controller_preset("AA", "").unwrap();
        let _ = rx.try_iter().count();
        c.set_controller_preset("AA", "Racing").unwrap();
        let cmds: Vec<_> = rx.try_iter().collect();
        assert!(cmds.contains(&Command::Lightbar(255, 0, 0)), "{cmds:?}");
        assert!(cmds.contains(&Command::ProfileChanged));
    }

    #[test]
    fn a_game_in_the_foreground_wins_over_the_default_and_over_the_choice_made_by_hand() {
        let (c, _rx) = with_racing_profile();
        c.create_game_profile("Shooter", "shoot.exe", "Controller 1").unwrap();
        c.set_controller_preset("AA", "Shooter").unwrap();
        c.set_auto_preset(Some("Racing".into()));
        assert_eq!(c.list()[0].effective_preset, "Racing");
        c.set_auto_preset(None);
        assert_eq!(c.list()[0].effective_preset, "Shooter", "the choice made by hand is still there");
        c.set_controller_preset("AA", "").unwrap();
        assert_eq!(c.list()[0].effective_preset, "Controller 1");
    }

    #[test]
    fn a_game_applies_to_every_controller_and_releasing_it_restores_each_default() {
        let (c, _rx) = with_racing_profile();
        let _b = connect(&c, "BB");
        c.set_auto_preset(Some("Racing".into()));
        assert!(c.list().iter().all(|i| i.effective_preset == "Racing"));
        c.set_auto_preset(None);
        let list = c.list();
        assert_eq!((list[0].effective_preset.as_str(), list[1].effective_preset.as_str()), ("Controller 1", "Controller 2"));
    }

    #[test]
    fn edits_made_during_a_game_change_the_profile_of_the_game() {
        let (c, _rx) = with_racing_profile();
        c.set_auto_preset(Some("Racing".into()));
        c.edit("AA", |p| p.lightbar = [5, 5, 5]);
        c.set_auto_preset(None);
        assert_ne!(c.live_config("AA").fixed_color, [5, 5, 5]);
        c.set_auto_preset(Some("Racing".into()));
        assert_eq!(c.live_config("AA").fixed_color, [5, 5, 5]);
    }

    #[test]
    fn the_automatic_profile_is_ignored_if_it_does_not_exist_and_is_not_saved() {
        let file = temp_file("auto");
        let c = Controllers::new(Events::none(), Some(file.clone()));
        let _a = connect(&c, "AA");
        c.set_auto_preset(Some("Nope".into()));
        assert_eq!(c.list()[0].effective_preset, "Controller 1");
        c.create_game_profile("Racing", "game.exe", "Controller 1").unwrap();
        c.set_auto_preset(Some("Racing".into()));
        c.flush();
        let reloaded = Controllers::new(Events::none(), Some(file.clone()));
        assert_eq!(reloaded.list()[0].effective_preset, "Controller 1", "a game is not remembered across restarts");
        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn a_game_profile_needs_a_program_that_has_none_yet_and_a_valid_name() {
        let (c, _rx) = with_racing_profile();
        assert!(c.create_game_profile("Other", "  ", "Controller 1").is_err(), "no program");
        assert!(c.create_game_profile("Other", "GAME.EXE", "Controller 1").is_err(), "program already used");
        assert!(c.create_game_profile("Racing", "other.exe", "Controller 1").is_err(), "name already used");
        assert!(c.create_game_profile("   ", "other.exe", "Controller 1").is_err());
        assert!(c.create_game_profile("Other", "other.exe", "Missing").is_err());
        assert_eq!(c.games().len(), 1, "nothing was left behind by the failures");
        assert_eq!(c.presets_info().presets.len(), 2);
    }

    #[test]
    fn renaming_a_game_profile_updates_its_rule_and_the_controllers_showing_it() {
        let (c, _rx) = with_racing_profile();
        c.set_controller_preset("AA", "Racing").unwrap();
        assert_eq!(c.rename_preset("Racing", "Cars").unwrap(), "Cars");
        assert_eq!(c.games()[0].preset, "Cars");
        assert_eq!(c.list()[0].manual.as_deref(), Some("Cars"));
        assert!(c.rename_preset("Cars", "Controller 1").is_err());
        assert!(c.rename_preset("Missing", "X").is_err());
    }

    #[test]
    fn deleting_a_game_profile_removes_its_rule_and_the_controllers_go_back_to_their_default() {
        let (c, _rx) = with_racing_profile();
        c.set_controller_preset("AA", "Racing").unwrap();
        c.delete_preset("Racing").unwrap();
        assert!(c.games().is_empty());
        assert_eq!(c.list()[0].manual, None);
        assert_eq!(c.list()[0].effective_preset, "Controller 1");
        assert!(c.delete_preset("Missing").is_err());
    }

    #[test]
    fn a_default_profile_cannot_be_deleted() {
        let (c, _rx) = with_racing_profile();
        assert!(c.delete_preset("Controller 1").is_err());
        assert_eq!(c.list()[0].profile.lightbar, c.live_config("AA").fixed_color);
    }

    #[test]
    fn game_rules_only_keep_existing_profiles() {
        let c = Controllers::new(Events::none(), None);
        let _a = connect(&c, "AA");
        c.set_games(vec![
            GameRule { exe: "good.exe".into(), preset: "Controller 1".into() },
            GameRule { exe: "bad.exe".into(), preset: "Missing".into() },
        ]);
        assert_eq!(c.games().len(), 1);
    }

    #[test]
    fn game_profiles_survive_a_restart() {
        let file = temp_file("presets");
        let c = Controllers::new(Events::none(), Some(file.clone()));
        let _a = connect(&c, "AA");
        c.create_game_profile("Racing", "game.exe", "Controller 1").unwrap();
        c.set_controller_preset("AA", "Racing").unwrap();
        c.edit("AA", |p| p.lightbar = [4, 5, 6]);
        c.flush();

        let reloaded = Controllers::new(Events::none(), Some(file.clone()));
        assert_eq!(reloaded.games(), c.games());
        assert_eq!(reloaded.list()[0].effective_preset, "Controller 1", "starts on the default profile");
        reloaded.set_controller_preset("AA", "Racing").unwrap();
        assert_eq!(reloaded.live_config("AA").fixed_color, [4, 5, 6]);
        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn profiles_saved_by_version_1_are_converted_and_the_old_file_is_kept() {
        let file = temp_file("migrate");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let old = r#"{
            "AA:AA:AA:AA:AA:AA": {"name": "Salon", "lightbar": [1, 2, 3], "virtual_pad": true},
            "BB:BB:BB:BB:BB:BB": {"name": "Salon", "lightbar": [4, 5, 6]}
        }"#;
        std::fs::write(&file, old).unwrap();

        let c = Controllers::new(Events::none(), Some(file.clone()));
        let list = c.list();
        assert_eq!(list.len(), 2);
        assert_eq!((list[0].profile.name.as_str(), list[0].profile.lightbar, list[0].profile.virtual_pad), ("Salon", [1, 2, 3], true));
        assert_eq!((list[1].profile.name.as_str(), list[1].profile.lightbar), ("Salon", [4, 5, 6]));
        assert_ne!(list[0].preset, list[1].preset, "two controllers with the same name get two profiles");

        assert_eq!(std::fs::read_to_string(file.with_file_name("profiles.v1.json")).unwrap(), old, "the old file is kept");
        c.flush();
        assert!(std::fs::read_to_string(&file).unwrap().contains("\"version\""), "saved in the new format");
        let reloaded = Controllers::new(Events::none(), Some(file.clone()));
        assert_eq!(reloaded.list()[1].profile.lightbar, [4, 5, 6]);
        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn profiles_saved_before_the_menu_get_the_menu_rule_once_and_deleting_it_is_final() {
        let file = temp_file("menu-rule");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let overlay_rule = r#"{"inputs":["ps"],"gesture":"press","steps":[{"type":"app","action":{"type":"show_overlay"}}]}"#;
        std::fs::write(
            &file,
            format!(
                r#"{{"version":2,"controllers":{{"AA":{{"name":"Pad","preset":"Pad"}}}},"presets":{{"Pad":{{"rules":[{overlay_rule}]}}}},"games":[]}}"#
            ),
        )
        .unwrap();

        let c = Controllers::new(Events::none(), Some(file.clone()));
        let rules = c.live_config("AA").rules;
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].gesture, crate::bindings::Gesture::Tap, "the overlay rule no longer fires before the menu");
        assert_eq!(rules[1], crate::bindings::menu_rule());
        c.flush();
        assert!(std::fs::read_to_string(&file).unwrap().contains(r#""version": 3"#));

        // The user deletes the menu rule: it must stay deleted.
        c.edit("AA", |p| p.rules = Some(vec![rules[0].clone()]));
        c.flush();
        let reloaded = Controllers::new(Events::none(), Some(file.clone()));
        assert_eq!(reloaded.live_config("AA").rules.len(), 1);
        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn profiles_that_never_had_rules_get_the_default_ones_with_the_menu() {
        let file = temp_file("menu-rule-legacy");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, r#"{"AA":{"name":"Pad","shortcuts":[{"button":"mute","gesture":"press","action":"toggle_virtual_pad"}]}}"#).unwrap();
        let c = Controllers::new(Events::none(), Some(file.clone()));
        let rules = c.live_config("AA").rules;
        assert_eq!(rules.len(), 2, "the shortcut, then the menu rule");
        assert_eq!(rules[1], crate::bindings::menu_rule());
        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn a_new_controller_has_the_menu_rule_among_its_rules() {
        let c = Controllers::new(Events::none(), None);
        let _a = connect(&c, "AA");
        assert!(c.live_config("AA").rules.contains(&crate::bindings::menu_rule()));
    }

    #[test]
    fn a_store_whose_controller_points_to_a_missing_profile_is_repaired() {
        let file = temp_file("repair");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(
            &file,
            r#"{"version":2,"controllers":{"AA":{"name":"Pad","preset":"Gone"}},"presets":{},"games":[{"exe":"g.exe","preset":"Nowhere"}]}"#,
        )
        .unwrap();
        let c = Controllers::new(Events::none(), Some(file.clone()));
        assert_eq!(c.list()[0].effective_preset, "Gone");
        assert!(c.games().is_empty());
        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }

    // --- Automatic backups --------------------------------------------------------------

    fn backup_dir(tag: &str) -> PathBuf {
        let file = temp_file(tag);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        file
    }

    fn backups_of(file: &std::path::Path) -> Vec<String> {
        let dir = file.with_file_name("backups");
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .map(|d| d.filter_map(|e| e.ok()?.file_name().into_string().ok()).collect())
            .unwrap_or_default();
        names.sort();
        names
    }

    #[test]
    fn the_previous_version_of_the_file_is_kept_before_it_is_replaced() {
        let file = backup_dir("backup-basic");
        std::fs::write(&file, "old").unwrap();
        rotate_backups(&file, "new", 1_000_000).unwrap();
        assert_eq!(backups_of(&file), ["profiles-1000000.json"]);
        assert_eq!(std::fs::read_to_string(file.with_file_name("backups").join("profiles-1000000.json")).unwrap(), "old");
        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn nothing_is_kept_when_there_is_no_file_yet_or_nothing_changed() {
        let file = backup_dir("backup-none");
        rotate_backups(&file, "first", 1_000_000).unwrap();
        assert!(backups_of(&file).is_empty());
        std::fs::write(&file, "same").unwrap();
        rotate_backups(&file, "same", 1_000_000).unwrap();
        assert!(backups_of(&file).is_empty());
        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn backups_are_spaced_out_and_the_oldest_are_dropped() {
        let file = backup_dir("backup-rotate");
        std::fs::write(&file, "v").unwrap();
        rotate_backups(&file, "w", 1_000_000).unwrap();
        rotate_backups(&file, "x", 1_000_000 + 60).unwrap();
        assert_eq!(backups_of(&file).len(), 1, "a second edit a minute later does not add a copy");
        for i in 1..=15u64 {
            rotate_backups(&file, "y", 1_000_000 + i * (BACKUP_INTERVAL_SECS + 1)).unwrap();
        }
        let names = backups_of(&file);
        assert_eq!(names.len(), BACKUPS_KEPT);
        assert_eq!(names.last().unwrap(), &format!("profiles-{}.json", 1_000_000 + 15 * (BACKUP_INTERVAL_SECS + 1)));
        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn saving_profiles_keeps_a_backup_of_the_previous_file() {
        let file = temp_file("backup-flush");
        let c = Controllers::new(Events::none(), Some(file.clone()));
        let _a = connect(&c, "AA");
        c.flush();
        c.edit("AA", |p| p.lightbar = [1, 2, 3]);
        c.flush();
        assert_eq!(backups_of(&file).len(), 1);
        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn unique_names_never_exceed_the_limit() {
        let mut taken = BTreeMap::new();
        let long = "x".repeat(MAX_NAME_LEN);
        taken.insert(long.clone(), Profile::default());
        let name = unique_name(&long, &taken);
        assert_ne!(name, long);
        assert!(name.chars().count() <= MAX_NAME_LEN && name.ends_with(" 2"));
    }

}
