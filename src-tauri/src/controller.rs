//! A connected DualSense: decoding of its input reports, encoding of its output reports, and the
//! per-controller thread that ties them to rules, motion, the virtual pad and the overlay.

use crate::alerts::{self, BatteryAlert};
use crate::bindings::{AppAction, Effect as RuleEffect, Engine, Snapshot};
use crate::desktop;
use crate::haptics::{self, Haptics, Pattern};
use crate::i18n::Msg;
use crate::lightbar::{self, LightbarMode};
use crate::manager::{Controllers, LiveConfig};
use crate::motion::Motion;
use crate::mouse;
use crate::triggers::Effect;
use crate::virtual_pad::VirtualPad;
use hidapi::{BusType, DeviceInfo, HidApi, HidDevice};
use serde::Serialize;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

const SONY_VID: u16 = 0x054C;
const DUALSENSE_PID: u16 = 0x0CE6;
const DUALSENSE_EDGE_PID: u16 = 0x0DF2;

const EMIT_INTERVAL: Duration = Duration::from_millis(16);

#[derive(Serialize, Clone, Default)]
pub struct ControllerState {
    pub connected: bool,
    pub bluetooth: bool,
    pub lx: u8,
    pub ly: u8,
    pub rx: u8,
    pub ry: u8,
    pub l2: u8,
    pub r2: u8,
    /// Bitfield: see `btn` for the order (and `BUTTON_NAMES` on the front-end side).
    pub buttons: u32,
    /// Battery level in %, `None` if the report does not contain one (simplified BT report).
    pub battery: Option<u8>,
    pub charge: Charge,
    /// Gyroscope in raw units: pitch, yaw, roll.
    pub gyro: [i16; 3],
    /// Position of the first finger on the touchpad (0..1919, 0..1079), if there is one.
    pub touch: Option<(u16, u16)>,
    /// Number of fingers on the touchpad.
    pub touch_fingers: u8,
}

#[derive(Serialize, Clone, Copy, Debug, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Charge {
    #[default]
    Unknown,
    Discharging,
    Charging,
    Full,
    /// Voltage or temperature out of range, or a charging error.
    Error,
}

/// Decodes the status byte: the low nibble gives the level (0..=10, in steps of 10 %), the high
/// nibble the charging state.
fn decode_battery(status: u8) -> (Option<u8>, Charge) {
    let level = status & 0x0F;
    let percent = (level * 10 + 5).min(100);
    match status >> 4 {
        0x0 => (Some(percent), Charge::Discharging),
        0x1 => (Some(percent), Charge::Charging),
        0x2 => (Some(100), Charge::Full),
        0xA | 0xB | 0xF => (None, Charge::Error),
        _ => (None, Charge::Unknown),
    }
}

/// Touchpad: two 4-byte fingers starting at offset 32. Bit 7 of the first byte means "no
/// contact"; x and y are encoded on 12 bits.
fn parse_touch(d: &[u8]) -> (Option<(u16, u16)>, u8) {
    if d.len() < 40 {
        return (None, 0);
    }
    let mut first = None;
    let mut fingers = 0;
    for finger in d[32..40].chunks_exact(4) {
        if finger[0] & 0x80 != 0 {
            continue;
        }
        fingers += 1;
        let x = u16::from(finger[1]) | (u16::from(finger[2] & 0x0F) << 8);
        let y = u16::from(finger[2] >> 4) | (u16::from(finger[3]) << 4);
        first.get_or_insert((x, y));
    }
    (first, fingers)
}

/// Bits of `ControllerState::buttons`.
pub mod btn {
    pub const SQUARE: u32 = 1 << 0;
    pub const CROSS: u32 = 1 << 1;
    pub const CIRCLE: u32 = 1 << 2;
    pub const TRIANGLE: u32 = 1 << 3;
    pub const L1: u32 = 1 << 4;
    pub const R1: u32 = 1 << 5;
    pub const L2: u32 = 1 << 6;
    pub const R2: u32 = 1 << 7;
    pub const SHARE: u32 = 1 << 8;
    pub const OPTIONS: u32 = 1 << 9;
    pub const L3: u32 = 1 << 10;
    pub const R3: u32 = 1 << 11;
    pub const PS: u32 = 1 << 12;
    pub const TOUCHPAD: u32 = 1 << 13;
    pub const MUTE: u32 = 1 << 14;
    pub const DPAD_UP: u32 = 1 << 15;
    pub const DPAD_RIGHT: u32 = 1 << 16;
    pub const DPAD_DOWN: u32 = 1 << 17;
    pub const DPAD_LEFT: u32 = 1 << 18;
}

/// Decodes a full input report (USB 0x01 or Bluetooth 0x31).
/// `base` is the offset of the first data byte (1 over USB, 2 over BT).
fn parse(report: &[u8], base: usize, bluetooth: bool) -> Option<ControllerState> {
    if report.len() < base + 10 {
        return None;
    }
    let d = &report[base..];
    let b0 = d[7];
    let b1 = d[8];
    let b2 = d[9];

    let mut buttons = 0u32;
    let mut set = |cond: bool, bit: u32| {
        if cond {
            buttons |= bit;
        }
    };

    set(b0 & 0x10 != 0, btn::SQUARE);
    set(b0 & 0x20 != 0, btn::CROSS);
    set(b0 & 0x40 != 0, btn::CIRCLE);
    set(b0 & 0x80 != 0, btn::TRIANGLE);

    // Directional pad: a value of 0..7 (up, then clockwise), 8 = neutral.
    match b0 & 0x0F {
        0 => set(true, btn::DPAD_UP),
        1 => {
            set(true, btn::DPAD_UP);
            set(true, btn::DPAD_RIGHT);
        }
        2 => set(true, btn::DPAD_RIGHT),
        3 => {
            set(true, btn::DPAD_RIGHT);
            set(true, btn::DPAD_DOWN);
        }
        4 => set(true, btn::DPAD_DOWN),
        5 => {
            set(true, btn::DPAD_DOWN);
            set(true, btn::DPAD_LEFT);
        }
        6 => set(true, btn::DPAD_LEFT),
        7 => {
            set(true, btn::DPAD_LEFT);
            set(true, btn::DPAD_UP);
        }
        _ => {}
    }

    set(b1 & 0x01 != 0, btn::L1);
    set(b1 & 0x02 != 0, btn::R1);
    set(b1 & 0x04 != 0, btn::L2);
    set(b1 & 0x08 != 0, btn::R2);
    set(b1 & 0x10 != 0, btn::SHARE);
    set(b1 & 0x20 != 0, btn::OPTIONS);
    set(b1 & 0x40 != 0, btn::L3);
    set(b1 & 0x80 != 0, btn::R3);
    set(b2 & 0x01 != 0, btn::PS);
    set(b2 & 0x02 != 0, btn::TOUCHPAD);
    set(b2 & 0x04 != 0, btn::MUTE);

    // The status byte is at offset 52 of the data; the simplified BT report has none.
    let (battery, charge) = d.get(52).map_or((None, Charge::Unknown), |&s| decode_battery(s));

    // Gyroscope: three little-endian 16-bit integers at offsets 15 to 20 (absent from the simplified report).
    let gyro = if d.len() >= 21 {
        let axis = |i: usize| i16::from_le_bytes([d[i], d[i + 1]]);
        [axis(15), axis(17), axis(19)]
    } else {
        [0; 3]
    };
    let (touch, touch_fingers) = parse_touch(d);

    Some(ControllerState {
        connected: true,
        bluetooth,
        lx: d[0],
        ly: d[1],
        rx: d[2],
        ry: d[3],
        l2: d[4],
        r2: d[5],
        buttons,
        battery,
        charge,
        gyro,
        touch,
        touch_fingers,
    })
}

/// Simplified Bluetooth report (ID 0x01, sent until the full mode is enabled).
/// Layout: LX LY RX RY, three button bytes, L2, R2. It is brought back to the full format for `parse`.
fn parse_bt_simple(r: &[u8]) -> Option<ControllerState> {
    if r.len() < 10 {
        return None;
    }
    let fake = [0, r[1], r[2], r[3], r[4], r[8], r[9], 0, r[5], r[6], r[7]];
    parse(&fake, 1, true)
}

/// True for the "controller" interface of a DualSense or a DualSense Edge.
pub fn is_dualsense(info: &DeviceInfo) -> bool {
    let known = info.vendor_id() == SONY_VID
        && (info.product_id() == DUALSENSE_PID || info.product_id() == DUALSENSE_EDGE_PID);
    // Usage page 1 / usage 5 = Generic Desktop / Game Pad; 0 if the system does not report it.
    let gamepad = info.usage_page() == 0 || (info.usage_page() == 0x01 && info.usage() == 0x05);
    known && gamepad
}

/// Opens the controller. Returns the device and `true` if it is connected over Bluetooth.
pub fn open(api: &HidApi, info: &DeviceInfo) -> Option<(HidDevice, bool)> {
    let path = info.path().to_string_lossy().to_ascii_lowercase();
    let bluetooth = matches!(info.bus_type(), BusType::Bluetooth) || path.contains("bth");
    let device = api.open_path(info.path()).ok()?;

    if bluetooth {
        // Reading feature report 0x05 (calibration) makes the controller switch from the
        // simplified 0x01 report to the full 0x31 report.
        let mut feature = [0u8; 41];
        feature[0] = 0x05;
        let _ = device.get_feature_report(&mut feature);
    }
    Some((device, bluetooth))
}

/// A controller's stable identifier: its MAC address, identical over USB and Bluetooth.
/// Feature report 0x09 contains the address in little-endian order at bytes 1 to 6.
pub fn identify(device: &HidDevice, info: &DeviceInfo) -> String {
    let mut buf = [0u8; 20];
    buf[0] = 0x09;
    if matches!(device.get_feature_report(&mut buf), Ok(n) if n >= 7) {
        return buf[1..7]
            .iter()
            .rev()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(":");
    }
    // Fallback: the serial number, then the device path.
    match info.serial_number() {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => info.path().to_string_lossy().into_owned(),
    }
}

// ---------------------------------------------------------------------------
// Outputs (light bar, player LEDs, rumble)
// ---------------------------------------------------------------------------

/// Orders sent by the interface to the thread that owns the device.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Command {
    Lightbar(u8, u8, u8),
    PlayerLeds(u8),
    Rumble { left: u8, right: u8 },
    Trigger { right: bool, effect: Effect },
    VirtualPad(bool),
    /// The profile changed (rules, light bar, sticks, motion): the thread reads it again.
    ProfileChanged,
}

/// The groups of controls that an output report can take over. A report only claims the groups
/// that actually changed: whatever else a game is driving (light bar, rumble, triggers) is left
/// alone instead of being overwritten with our last values on every unrelated write.
mod group {
    pub const LED: u8 = 1; // light bar and player LEDs
    pub const RUMBLE: u8 = 1 << 1;
    pub const TRIGGERS: u8 = 1 << 2;
    pub const ALL: u8 = LED | RUMBLE | TRIGGERS;
}

/// The desired output state. It is kept across reconnections.
#[derive(Clone, Default)]
struct Output {
    lightbar: [u8; 3],
    player_leds: u8,
    rumble_left: u8,
    rumble_right: u8,
    /// `None` until the user has set something: the triggers are then left alone.
    trigger_left: Option<Effect>,
    trigger_right: Option<Effect>,
    /// Virtual Xbox 360 controller enabled by the user.
    virtual_pad: bool,
    /// A confirmation vibration pulse is in progress (see `haptics`).
    blip: bool,
    /// The groups changed since the last write (see `group`).
    pending: u8,
}

impl Output {
    fn apply(&mut self, cmd: Command) {
        match cmd {
            Command::Lightbar(r, g, b) => {
                self.lightbar = [r, g, b];
                self.pending |= group::LED;
            }
            Command::PlayerLeds(mask) => {
                self.player_leds = mask & 0x1F;
                self.pending |= group::LED;
            }
            Command::Rumble { left, right } => {
                self.rumble_left = left;
                self.rumble_right = right;
                self.pending |= group::RUMBLE;
            }
            Command::Trigger { right: true, effect } => {
                self.trigger_right = Some(effect);
                self.pending |= group::TRIGGERS;
            }
            Command::Trigger { right: false, effect } => {
                self.trigger_left = Some(effect);
                self.pending |= group::TRIGGERS;
            }
            Command::VirtualPad(enabled) => self.virtual_pad = enabled,
            Command::ProfileChanged => {}
        }
    }
}

/// Size of the common part of an output report (excluding the ID and the BT-specific part).
const COMMON_LEN: usize = 47;

/// Fills the common part of the output report (identical over USB and BT).
/// Only the control groups in `groups` are claimed (see `group`). `first` asks the controller to
/// switch off the blue start-up animation of the light bar.
fn fill_common(c: &mut [u8; COMMON_LEN], out: &Output, groups: u8, first: bool) {
    c.fill(0);
    if groups & group::RUMBLE != 0 {
        c[0] |= 0x01 | 0x02; // flag0: compatible rumble + haptics selection
    }
    if groups & group::LED != 0 {
        c[1] = 0x04 | 0x10; // flag1: light bar control + player LEDs
    }
    // The confirmation pulse uses the weak motor, without cutting a game's vibration.
    c[2] = if out.blip { out.rumble_right.max(haptics::STRENGTH) } else { out.rumble_right };
    c[3] = out.rumble_left; // strong motor (left)
    if groups & group::TRIGGERS != 0 {
        if let Some(effect) = out.trigger_right {
            c[0] |= 0x04; // flag0: right trigger control
            c[10..21].copy_from_slice(&effect);
        }
        if let Some(effect) = out.trigger_left {
            c[0] |= 0x08; // flag0: left trigger control
            c[21..32].copy_from_slice(&effect);
        }
    }
    c[38] = 0x04; // flag2: improved rumble
    if first {
        c[38] |= 0x02; // flag2: light bar setup
        c[41] = 0x02; // light bar setup: switch off the default animation
    }
    c[42] = 0x00; // maximum brightness
    c[43] = out.player_leds;
    c[44..47].copy_from_slice(&out.lightbar);
}

/// Builds the complete output report, ready for `HidDevice::write`.
fn build_report(out: &Output, groups: u8, bluetooth: bool, first: bool, seq: &mut u8) -> Vec<u8> {
    let mut common = [0u8; COMMON_LEN];
    fill_common(&mut common, out, groups, first);

    if !bluetooth {
        let mut r = vec![0u8; 64];
        r[0] = 0x02;
        r[1..1 + COMMON_LEN].copy_from_slice(&common);
        return r;
    }

    let mut r = vec![0u8; 78];
    r[0] = 0x31;
    r[1] = *seq << 4; // sequence number in the high nibble
    r[2] = 0x10; // tag: the report contains HID data
    r[3..3 + COMMON_LEN].copy_from_slice(&common);
    *seq = (*seq + 1) & 0x0F;

    // CRC32 over 0xA2 (the BT HID header byte) + the report without its last 4 bytes.
    let mut h = crc32fast::Hasher::new();
    h.update(&[0xA2]);
    h.update(&r[..74]);
    r[74..78].copy_from_slice(&h.finalize().to_le_bytes());
    r
}

/// Context of a controller thread.
pub struct DeviceCtx {
    pub controllers: Controllers,
    pub id: String,
}

#[derive(Serialize, Clone)]
struct StateEvent<'a> {
    id: &'a str,
    #[serde(flatten)]
    state: &'a ControllerState,
}

/// Interval over which the input report rate is measured.
const RATE_WINDOW: Duration = Duration::from_secs(1);

/// Everything that evolves during the life of a connected controller.
struct Session {
    out: Output,
    pad: Option<VirtualPad>,
    engine: Engine,
    config: LiveConfig,
    haptics: Haptics,
    alert: BatteryAlert,
    motion: Motion,
    /// Master switch of the gyroscope and the touchpad (shortcut). Not saved.
    motion_enabled: bool,
    battery: (Option<u8>, Charge),
    /// Latest controller state, so that rules are evaluated even without a new report (a hold, a
    /// macro pause or a repetition are triggered by time).
    snapshot: Snapshot,
    /// End of a vibration requested by a rule.
    rumble_until: Option<Instant>,
    reports: u32,
    rate_since: Instant,
}

impl Drop for Session {
    /// Whatever way the loop ends (controller unplugged, error…), the keys and clicks that rules
    /// were holding are released.
    fn drop(&mut self) {
        desktop::apply(&self.engine.release_all());
    }
}

impl Session {
    fn new(ctx: &DeviceCtx) -> Self {
        let config = ctx.controllers.live_config(&ctx.id);
        Self {
            out: Output::default(),
            pad: None,
            engine: Engine::new(config.rules.clone(), config.analog.clone()),
            config,
            haptics: Haptics::default(),
            alert: BatteryAlert::default(),
            motion: Motion::default(),
            motion_enabled: true,
            battery: (None, Charge::Unknown),
            snapshot: Snapshot::default(),
            rumble_until: None,
            reports: 0,
            rate_since: Instant::now(),
        }
    }

    /// Recomputes the light bar colour. In fixed mode the colour is only re-imposed when `force`
    /// is set: otherwise a colour sent over UDP would be overwritten on every battery change.
    fn refresh_lightbar(&mut self, force: bool) -> bool {
        if self.config.lightbar_mode == LightbarMode::Fixed && !force {
            return false;
        }
        let color = lightbar::effective(
            self.config.lightbar_mode,
            self.config.mode_colors,
            self.config.fixed_color,
            self.battery.0,
            self.battery.1,
            self.pad.is_some(),
        );
        if self.out.lightbar == color {
            return false;
        }
        self.out.lightbar = color;
        self.out.pending |= group::LED;
        true
    }

    /// The profile changed: reads rules, light bar, sticks and motion again. Everything the old
    /// rules were holding (keys, clicks) is released, so that nothing stays stuck.
    fn reload_config(&mut self, ctx: &DeviceCtx) {
        self.config = ctx.controllers.live_config(&ctx.id);
        let released = self.engine.set_config(self.config.rules.clone(), self.config.analog.clone());
        desktop::apply(&released);
        self.refresh_lightbar(true);
    }

    /// End of the vibrations requested by a rule.
    fn tick_rumble(&mut self, now: Instant) {
        if self.rumble_until.is_some_and(|until| now >= until) {
            self.rumble_until = None;
            self.out.apply(Command::Rumble { left: 0, right: 0 });
        }
    }

    fn tick_haptics(&mut self, now: Instant) {
        let on = self.haptics.is_on(now);
        if on != self.out.blip {
            self.out.blip = on;
            self.out.pending |= group::RUMBLE;
        }
    }

    fn on_battery(&mut self, ctx: &DeviceCtx) {
        ctx.controllers.set_battery(&ctx.id, self.battery.0, self.battery.1);
        self.refresh_lightbar(false);

        if let Some(alert) = self.alert.update(self.battery.0, self.battery.1) {
            let settings = ctx.controllers.settings();
            if settings.low_battery_alert {
                let message = alerts::message(alert, self.battery.0.unwrap_or(0)).text();
                ctx.controllers.log().warn("battery", format!("{}: {message}", ctx.id));
                ctx.controllers.show_overlay(&ctx.id, Some(message));
                if settings.haptic_feedback {
                    self.haptics.start(Pattern::Long, Instant::now());
                }
            }
        }
    }

    /// Evaluates the rules and carries out what they ask: keys and mouse go to Windows, application
    /// actions are handled here. To be called on every report and on every loop turn.
    fn run_rules(&mut self, now: Instant, ctx: &DeviceCtx, self_tx: &Sender<Command>) {
        let effects = self.engine.update(&self.snapshot, now);
        desktop::apply(&effects);
        for effect in effects {
            if let RuleEffect::App(action) = effect {
                self.perform(action, ctx, self_tx);
            }
        }
    }

    /// Handles a new input report.
    fn on_state(&mut self, state: &ControllerState, ctx: &DeviceCtx, self_tx: &Sender<Command>) {
        self.snapshot = Snapshot::from(state);
        self.reports += 1;

        if (state.battery, state.charge) != self.battery {
            self.battery = (state.battery, state.charge);
            self.on_battery(ctx);
        }

        let motion = self.motion.update(state, &self.config.motion, self.motion_enabled, Instant::now());
        mouse::apply(&motion);

        self.run_rules(Instant::now(), ctx, self_tx);
        if let Some(pad) = self.pad.as_mut() {
            // An input taken by a rule is not passed to the virtual controller, which instead
            // receives what the rules press on it.
            let held = self.engine.held_pad();
            pad.update(state, self.engine.consumed(), &self.config.input, motion.stick, &held);
        }
    }

    /// Carries out an application action requested by a rule.
    fn perform(&mut self, action: AppAction, ctx: &DeviceCtx, self_tx: &Sender<Command>) {
        let controllers = &ctx.controllers;
        let feedback = controllers.settings().haptic_feedback;
        let now = Instant::now();
        let buzz = |haptics: &mut Haptics, pattern: Pattern| {
            if feedback {
                haptics.start(pattern, now);
            }
        };

        match action {
            AppAction::ShowOverlay => controllers.show_overlay(&ctx.id, None),

            AppAction::Message { text } => controllers.show_overlay(&ctx.id, Some(text)),

            AppAction::Lightbar { r, g, b } => {
                self.out.apply(Command::Lightbar(r, g, b));
            }

            AppAction::Rumble { strength, ms } => {
                self.out.apply(Command::Rumble { left: strength, right: strength });
                self.rumble_until = Some(now + Duration::from_millis(u64::from(ms.min(5_000))));
            }

            // Handled by the engine itself, which never emits it.
            AppAction::Layer { .. } => {}

            AppAction::ToggleVirtualPad => {
                let want = !self.out.virtual_pad;
                self.out.virtual_pad = want;
                controllers.edit(&ctx.id, |p| p.virtual_pad = want);
                let pad_error = sync_pad(ctx, &mut self.out, &mut self.pad, self_tx);
                let cloak_error = controllers.set_cloaked(&ctx.id, self.pad.is_some()).err();
                // The mode changed: a light bar tied to the mode must change colour.
                self.refresh_lightbar(false);

                let message = match (pad_error, want) {
                    (Some(error), _) => {
                        buzz(&mut self.haptics, Pattern::Long);
                        error
                    }
                    (None, true) => {
                        buzz(&mut self.haptics, Pattern::Single);
                        with_cloak_error(Msg::ModeXbox360, cloak_error)
                    }
                    (None, false) => {
                        buzz(&mut self.haptics, Pattern::Double);
                        with_cloak_error(Msg::ModeNative, cloak_error)
                    }
                };
                controllers.show_overlay(&ctx.id, Some(message));
            }

            AppAction::PowerOff => {
                controllers.show_overlay(&ctx.id, Some(Msg::PoweringOff.text()));
                if let Err(error) = controllers.power_off(&ctx.id) {
                    buzz(&mut self.haptics, Pattern::Long);
                    controllers.show_overlay(&ctx.id, Some(error));
                }
            }

            AppAction::ToggleMotion => {
                self.motion_enabled = !self.motion_enabled;
                let (message, pattern) = if self.motion_enabled {
                    (Msg::MotionEnabled, Pattern::Single)
                } else {
                    (Msg::MotionDisabled, Pattern::Double)
                };
                buzz(&mut self.haptics, pattern);
                controllers.show_overlay(&ctx.id, Some(message.text()));
            }
        }
    }
}

/// When to resend the full state after connecting. The DualSense plays an animation on start-up
/// and may ignore what it is sent meanwhile (especially over Bluetooth): the light bar colour,
/// player LEDs or triggers of the profile would then not be applied, whereas a later write goes
/// through.
const STARTUP_REPLAYS: [Duration; 4] = [
    Duration::from_millis(300),
    Duration::from_millis(900),
    Duration::from_millis(2000),
    Duration::from_millis(4000),
];

/// The resends still to be made after connecting.
struct StartupReplay {
    /// Sorted from the latest to the nearest: the next one to fall due is the last.
    pending: Vec<Instant>,
}

impl StartupReplay {
    fn new(connected_at: Instant) -> Self {
        Self { pending: STARTUP_REPLAYS.iter().rev().map(|d| connected_at + *d).collect() }
    }

    /// Whether the state should be resent now. A single resend, even if several deadlines passed.
    fn due(&mut self, now: Instant) -> bool {
        let mut due = false;
        while self.pending.last().is_some_and(|t| now >= *t) {
            self.pending.pop();
            due = true;
        }
        due
    }
}

/// The loop of one controller: reads inputs, writes outputs, evaluates rules and feeds the
/// virtual controller. Returns when the controller is unplugged.
pub fn run_device(
    ctx: DeviceCtx,
    device: HidDevice,
    bluetooth: bool,
    rx: Receiver<Command>,
    self_tx: Sender<Command>,
) {
    let events = ctx.controllers.events();
    let mut buf = [0u8; 128];
    let mut seq = 0u8;
    let mut s = Session::new(&ctx);
    ctx.controllers.release_stale_cloak(&ctx.id);

    let mut last_emit = Instant::now() - EMIT_INTERVAL;
    let mut latest: Option<ControllerState> = None;
    let mut first = true; // first write after connecting: also applies the current state
    let mut replay: Option<StartupReplay> = None;

    loop {
        let mut dirty = first;
        let mut replay_due = false;
        if let Some(r) = replay.as_mut() {
            replay_due = r.due(Instant::now());
        }
        while let Ok(cmd) = rx.try_recv() {
            if cmd == Command::ProfileChanged {
                s.reload_config(&ctx);
                continue;
            }
            s.out.apply(cmd);
            dirty = true;
        }
        s.tick_haptics(Instant::now());
        s.tick_rumble(Instant::now());

        // Right after connecting the whole state is claimed (the controller may ignore writes while
        // it plays its start-up animation); afterwards only what changed, so that a game driving
        // the light bar is not fought over by unrelated writes.
        let pending = std::mem::take(&mut s.out.pending);
        let groups = if first || replay_due { group::ALL } else { pending };

        if dirty || groups != 0 {
            sync_pad(&ctx, &mut s.out, &mut s.pad, &self_tx);
            // The physical controller is hidden from games as long as the virtual one is plugged in.
            let _ = ctx.controllers.set_cloaked(&ctx.id, s.pad.is_some());
            if first {
                s.refresh_lightbar(true); // depends on the mode, hence on the virtual controller
            }
            if first {
                s.out.pending = 0; // refresh_lightbar above is covered by the full write
            }
            if groups != 0 {
                let report = build_report(&s.out, groups, bluetooth, first, &mut seq);
                if device.write(&report).is_err() {
                    break;
                }
            }
            if first {
                replay = Some(StartupReplay::new(Instant::now()));
            }
            first = false;
        }

        // Evaluated on every turn, even without a new report: a hold is triggered by time.
        s.run_rules(Instant::now(), &ctx, &self_tx);

        // Short timeout to detect disconnection and keep the loop responsive.
        match device.read_timeout(&mut buf, 50) {
            Ok(0) => continue,
            Ok(n) => {
                let report = &buf[..n];
                let parsed = match (report[0], bluetooth) {
                    (0x01, false) if n >= 64 => parse(report, 1, false),
                    (0x01, true) => parse_bt_simple(report),
                    (0x31, true) => parse(report, 2, true),
                    _ => None,
                };
                if let Some(state) = parsed {
                    s.on_state(&state, &ctx, &self_tx);
                    latest = Some(state);
                }
            }
            Err(_) => break, // controller unplugged
        }

        if s.rate_since.elapsed() >= RATE_WINDOW {
            ctx.controllers.set_report_rate(&ctx.id, s.reports);
            s.reports = 0;
            s.rate_since = Instant::now();
        }

        if last_emit.elapsed() >= EMIT_INTERVAL {
            if let Some(state) = latest.take() {
                let _ = events.emit("controller-state", &StateEvent { id: &ctx.id, state: &state });
                last_emit = Instant::now();
            }
        }
    }
    // `s.pad` is dropped here: the virtual controller is unplugged together with the real one.
}

/// Reports in the overlay that hiding through HidHide failed, without the details: they are too
/// long for a notification, and are already in the diagnostics log.
fn with_cloak_error(base: Msg, error: Option<String>) -> String {
    match error {
        Some(_) => Msg::WithHidHideError { base: base.text() }.text(),
        None => base.text(),
    }
}

#[derive(Serialize, Clone)]
struct PadStatus {
    id: String,
    enabled: bool,
    error: Option<String>,
}

/// Plugs in or unplugs the virtual controller to reflect the user's request.
/// Returns the error message if plugging in failed.
fn sync_pad(ctx: &DeviceCtx, out: &mut Output, pad: &mut Option<VirtualPad>, rumble: &Sender<Command>) -> Option<String> {
    let (enabled, error) = match (out.virtual_pad, pad.is_some()) {
        (true, false) => match VirtualPad::new(rumble.clone()) {
            Ok(p) => {
                *pad = Some(p);
                ctx.controllers.log().info("virtual-pad", Msg::LogVirtualPadPlugged { id: ctx.id.clone() }.text());
                (true, None)
            }
            Err(e) => {
                out.virtual_pad = false;
                // Without this, the error would come back on every reconnection.
                ctx.controllers.edit(&ctx.id, |p| p.virtual_pad = false);
                ctx.controllers.log().error("virtual-pad", format!("{}: {e}", ctx.id));
                (false, Some(e))
            }
        },
        (false, true) => {
            *pad = None;
            ctx.controllers.log().info("virtual-pad", Msg::LogVirtualPadUnplugged { id: ctx.id.clone() }.text());
            (false, None)
        }
        _ => return None,
    };
    ctx.controllers.events().emit("virtual-pad", &PadStatus { id: ctx.id.clone(), enabled, error: error.clone() });
    error
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_state_is_resent_a_few_times_after_connecting() {
        let t0 = Instant::now();
        let mut replay = StartupReplay::new(t0);
        assert!(!replay.due(t0 + Duration::from_millis(100)), "too early");
        assert!(replay.due(t0 + Duration::from_millis(300)));
        assert!(!replay.due(t0 + Duration::from_millis(400)), "one resend per deadline");
        assert!(replay.due(t0 + Duration::from_millis(900)));
        assert!(replay.due(t0 + Duration::from_millis(2000)));
        assert!(replay.due(t0 + Duration::from_millis(4000)));
        assert!(!replay.due(t0 + Duration::from_secs(60)), "nothing afterwards");
    }

    #[test]
    fn late_polling_triggers_a_single_resend_for_several_missed_deadlines() {
        let t0 = Instant::now();
        let mut replay = StartupReplay::new(t0);
        assert!(replay.due(t0 + Duration::from_millis(2500)), "three deadlines have passed");
        assert!(!replay.due(t0 + Duration::from_millis(2600)));
        assert!(replay.due(t0 + Duration::from_millis(4100)), "the last one is still to come");
    }

    #[test]
    fn the_overlay_only_says_that_hidhide_failed() {
        let long = "HidHideCLI failed (exit code: 31): Error code 0x0005 at src\\FilterDriverProxy.cpp(33)";
        assert_eq!(with_cloak_error(Msg::ModeXbox360, Some(long.into())), "Xbox 360 mode · HidHide error");
        assert_eq!(with_cloak_error(Msg::ModeNative, None), "Native mode");
    }

    /// A full USB report (ID 0x01, 64 bytes) with the battery status byte.
    fn usb_report(status: u8) -> Vec<u8> {
        let mut r = vec![0u8; 64];
        r[0] = 0x01;
        r[1 + 7] = 0x08; // neutral d-pad
        r[1 + 52] = status;
        r
    }

    #[test]
    fn battery_level_is_in_tens_of_percent() {
        assert_eq!(decode_battery(0x00), (Some(5), Charge::Discharging));
        assert_eq!(decode_battery(0x04), (Some(45), Charge::Discharging));
        assert_eq!(decode_battery(0x08), (Some(85), Charge::Discharging));
    }

    #[test]
    fn battery_never_exceeds_100() {
        assert_eq!(decode_battery(0x0A), (Some(100), Charge::Discharging));
        assert_eq!(decode_battery(0x0F), (Some(100), Charge::Discharging));
    }

    #[test]
    fn charging_and_full_states() {
        assert_eq!(decode_battery(0x13), (Some(35), Charge::Charging));
        assert_eq!(decode_battery(0x2A), (Some(100), Charge::Full));
        assert_eq!(decode_battery(0x20), (Some(100), Charge::Full));
    }

    #[test]
    fn hardware_errors_hide_the_level() {
        for status in [0xA5, 0xB5, 0xF5] {
            assert_eq!(decode_battery(status), (None, Charge::Error));
        }
        assert_eq!(decode_battery(0x55), (None, Charge::Unknown));
    }

    #[test]
    fn usb_report_carries_the_battery() {
        let s = parse(&usb_report(0x16), 1, false).unwrap();
        assert_eq!((s.battery, s.charge), (Some(65), Charge::Charging));
    }

    #[test]
    fn bluetooth_report_carries_the_battery_one_byte_later() {
        let mut r = vec![0u8; 78];
        r[0] = 0x31;
        r[2 + 52] = 0x07;
        let s = parse(&r, 2, true).unwrap();
        assert_eq!((s.battery, s.charge), (Some(75), Charge::Discharging));
    }

    /// A USB report with the gyroscope and touchpad bytes filled in.
    fn report_with(f: impl FnOnce(&mut [u8])) -> Vec<u8> {
        let mut r = vec![0u8; 64];
        r[0] = 0x01;
        r[1 + 7] = 0x08;
        // An absent finger has bit 7 of its contact byte set (an all-zero report would show two).
        r[1 + 32] = 0x80;
        r[1 + 36] = 0x80;
        f(&mut r[1..]);
        r
    }

    #[test]
    fn the_gyroscope_is_three_little_endian_signed_values() {
        let r = report_with(|d| {
            d[15..17].copy_from_slice(&1000i16.to_le_bytes());
            d[17..19].copy_from_slice(&(-2000i16).to_le_bytes());
            d[19..21].copy_from_slice(&32767i16.to_le_bytes());
        });
        assert_eq!(parse(&r, 1, false).unwrap().gyro, [1000, -2000, 32767]);
    }

    #[test]
    fn a_finger_on_the_touchpad_is_decoded_from_12_bit_coordinates() {
        // x = 1000 (0x3E8), y = 500 (0x1F4): x_lo, (x_hi | y_lo << 4), y_hi.
        let r = report_with(|d| d[32..36].copy_from_slice(&[0x05, 0xE8, 0x43, 0x1F]));
        let s = parse(&r, 1, false).unwrap();
        assert_eq!((s.touch, s.touch_fingers), (Some((1000, 500)), 1));
    }

    #[test]
    fn bit_7_of_the_contact_byte_means_no_finger() {
        let r = report_with(|d| d[32..36].copy_from_slice(&[0x80, 0xE8, 0x43, 0x1F]));
        let s = parse(&r, 1, false).unwrap();
        assert_eq!((s.touch, s.touch_fingers), (None, 0));
    }

    #[test]
    fn two_fingers_are_counted_and_the_first_one_is_reported() {
        let r = report_with(|d| {
            d[32..36].copy_from_slice(&[0x01, 0x10, 0x20, 0x03]);
            d[36..40].copy_from_slice(&[0x02, 0xFF, 0xFF, 0xFF]);
        });
        let s = parse(&r, 1, false).unwrap();
        assert_eq!(s.touch_fingers, 2);
        assert_eq!(s.touch, Some((0x10 | (0 << 8), 0x2 | (0x03 << 4))));
    }

    #[test]
    fn the_largest_coordinates_stay_within_12_bits() {
        let r = report_with(|d| d[32..36].copy_from_slice(&[0x01, 0xFF, 0xFF, 0xFF]));
        let (x, y) = parse(&r, 1, false).unwrap().touch.unwrap();
        assert_eq!((x, y), (0x0FFF, 0x0FFF));
    }

    #[test]
    fn the_bluetooth_report_carries_the_gyroscope_one_byte_later() {
        let mut r = vec![0u8; 78];
        r[0] = 0x31;
        r[2 + 17..2 + 19].copy_from_slice(&(-500i16).to_le_bytes());
        assert_eq!(parse(&r, 2, true).unwrap().gyro[1], -500);
    }

    #[test]
    fn the_simple_bluetooth_report_has_no_motion_data() {
        let mut r = vec![0u8; 10];
        r[0] = 0x01;
        let s = parse_bt_simple(&r).unwrap();
        assert_eq!((s.gyro, s.touch, s.touch_fingers), ([0; 3], None, 0));
    }

    #[test]
    fn the_confirmation_pulse_never_lowers_a_games_vibration() {
        let strong = Output { blip: true, rumble_right: 255, ..Default::default() };
        let mut c = [0u8; COMMON_LEN];
        fill_common(&mut c, &strong, group::ALL, false);
        assert_eq!(c[2], 255);

        let idle = Output { blip: true, rumble_right: 0, ..Default::default() };
        fill_common(&mut c, &idle, group::ALL, false);
        assert_eq!(c[2], haptics::STRENGTH);

        let off = Output { blip: false, rumble_right: 0, ..Default::default() };
        fill_common(&mut c, &off, group::ALL, false);
        assert_eq!(c[2], 0);
    }

    #[test]
    fn a_write_only_claims_the_groups_that_changed() {
        let out = Output { lightbar: [1, 2, 3], rumble_left: 9, trigger_right: Some([1; 11]), ..Default::default() };
        let mut c = [0u8; COMMON_LEN];

        fill_common(&mut c, &out, group::RUMBLE, false);
        assert_eq!((c[0], c[1]), (0x01 | 0x02, 0), "a rumble write leaves the LEDs and triggers to the game");

        fill_common(&mut c, &out, group::LED, false);
        assert_eq!((c[0], c[1]), (0, 0x04 | 0x10), "an LED write leaves the rumble and triggers alone");

        fill_common(&mut c, &out, group::TRIGGERS, false);
        assert_eq!((c[0], c[1]), (0x04, 0), "only the trigger that was set is claimed");

        fill_common(&mut c, &out, group::ALL, false);
        assert_eq!((c[0], c[1]), (0x01 | 0x02 | 0x04, 0x04 | 0x10));
    }

    #[test]
    fn commands_mark_the_groups_they_change() {
        let mut out = Output::default();
        out.apply(Command::Rumble { left: 1, right: 1 });
        assert_eq!(out.pending, group::RUMBLE);
        out.apply(Command::Lightbar(1, 2, 3));
        out.apply(Command::VirtualPad(true));
        assert_eq!(out.pending, group::RUMBLE | group::LED);
    }

    #[test]
    fn simple_bluetooth_report_has_no_battery() {
        let mut r = vec![0u8; 10];
        r[0] = 0x01;
        let s = parse_bt_simple(&r).unwrap();
        assert_eq!((s.battery, s.charge), (None, Charge::Unknown));
    }
}
