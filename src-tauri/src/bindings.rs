//! Rules engine: "when this input makes this gesture, do this".
//!
//! A **rule** pairs inputs (one or more buttons, or a stick direction) with a
//! **gesture** (press, release, short press, hold, double press, or "while held"), and runs a list of
//! **steps**: press a key or an Xbox button, wait, click, scroll, trigger an application action…
//! A rule can repeat (turbo) or latch (toggle),
//! and can exist in a single **layer** (a set of rules that takes over while the layer is active).
//!
//! **Analog bindings** make a stick drive the mouse, the wheel or keys.
//!
//! The engine is pure: it receives the controller state and the time, and returns effects to carry out.
//! It knows neither the controller nor Windows, which makes it fully testable.

use crate::controller::ControllerState;
use crate::input::XTarget;
use crate::keys;
use crate::shortcuts::{self, Button, Shortcut};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------------------------
// Model (what is saved in profiles)
// ---------------------------------------------------------------------------------------------

/// What can trigger a rule: a button, or a stick direction.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Input {
    Square,
    Cross,
    Circle,
    Triangle,
    L1,
    R1,
    L2,
    R2,
    Share,
    Options,
    L3,
    R3,
    Ps,
    Touchpad,
    Mute,
    DpadUp,
    DpadRight,
    DpadDown,
    DpadLeft,
    LeftStickUp,
    LeftStickRight,
    LeftStickDown,
    LeftStickLeft,
    RightStickUp,
    RightStickRight,
    RightStickDown,
    RightStickLeft,
}

/// Number of physical buttons: they are the first inputs, in the bit order of
/// `ControllerState::buttons`.
const BUTTON_COUNT: u32 = 19;
const BUTTON_MASK: u32 = (1 << BUTTON_COUNT) - 1;

impl Input {
    fn mask(self) -> u32 {
        1 << (self as u32)
    }
}

impl From<Button> for Input {
    fn from(b: Button) -> Self {
        match b {
            Button::Square => Input::Square,
            Button::Cross => Input::Cross,
            Button::Circle => Input::Circle,
            Button::Triangle => Input::Triangle,
            Button::L1 => Input::L1,
            Button::R1 => Input::R1,
            Button::L2 => Input::L2,
            Button::R2 => Input::R2,
            Button::Share => Input::Share,
            Button::Options => Input::Options,
            Button::L3 => Input::L3,
            Button::R3 => Input::R3,
            Button::Ps => Input::Ps,
            Button::Touchpad => Input::Touchpad,
            Button::Mute => Input::Mute,
            Button::DpadUp => Input::DpadUp,
            Button::DpadRight => Input::DpadRight,
            Button::DpadDown => Input::DpadDown,
            Button::DpadLeft => Input::DpadLeft,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Gesture {
    /// As soon as the input is pressed.
    #[default]
    Press,
    /// When the input is released.
    Release,
    /// Short press: fires on release, if the press lasted less than `ms`.
    Tap,
    /// Long press: fires once, when the input has been held for `ms`.
    Hold,
    /// Two quick presses (less than `ms` between the release of the first and the second press).
    DoubleTap,
    /// While the input is held: "held" outputs stay pressed until release.
    While,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
}

/// What a step presses: a button of the virtual Xbox controller, a key, a mouse button.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Output {
    Pad { target: XTarget },
    Key { key: String },
    Mouse { button: MouseButton },
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LayerMode {
    /// Activates the layer while the rule is held ("while held" gesture).
    Hold,
    /// Activates the layer if it is inactive, deactivates it otherwise.
    Toggle,
    On,
    Off,
}

/// What the application can do by itself.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AppAction {
    /// Toggles between native mode and Xbox 360 mode.
    ToggleVirtualPad,
    /// Powers the controller off (Bluetooth only).
    PowerOff,
    /// Shows the overlay.
    ShowOverlay,
    /// Opens the controller menu (the same one as holding PS).
    OpenMenu,
    /// Enables or disables the gyroscope and the touchpad.
    ToggleMotion,
    /// Shows a text in the overlay.
    Message { text: String },
    /// Changes the light bar colour.
    Lightbar { r: u8, g: u8, b: u8 },
    /// Vibrates the controller for a moment.
    Rumble { strength: u8, ms: u32 },
    /// Manages a layer (handled by the engine itself).
    Layer { name: String, mode: LayerMode },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Step {
    /// Presses and **holds**: with the "while held" gesture, until release; otherwise, a short press.
    Hold { output: Output },
    /// Short press (press then release).
    Tap { output: Output },
    /// Presses without releasing (release with `Up`, otherwise it is done when the rule ends).
    Down { output: Output },
    Up { output: Output },
    /// Pause, in milliseconds.
    Wait { ms: u32 },
    /// One wheel notch (positive: upwards); a wheel has 120 units per notch.
    Wheel { amount: i32 },
    /// Relative mouse movement, in pixels.
    MouseMove { dx: i32, dy: i32 },
    App { action: AppAction },
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Repeat {
    #[default]
    None,
    /// Starts over every `interval_ms` while the input is held (turbo). "While held" gesture.
    UntilReleased { interval_ms: u32 },
    /// Starts over up to `count` times in total.
    Times { count: u32, interval_ms: u32 },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(default)]
pub struct Rule {
    pub enabled: bool,
    /// Layer the rule exists in; `None`: always (base layer).
    pub layer: Option<String>,
    /// One input, or several for a combination (all held together).
    pub inputs: Vec<Input>,
    pub gesture: Gesture,
    /// Duration of the gesture: hold (800 ms by default), short press (800 ms), double press (300 ms).
    pub ms: Option<u32>,
    pub steps: Vec<Step>,
    pub repeat: Repeat,
    /// Each trigger alternates: the rule starts "held", then stops at the next one.
    pub toggle: bool,
    /// Keep passing the physical input to the virtual controller, in addition to the rule.
    pub passthrough: bool,
}

impl Default for Rule {
    fn default() -> Self {
        Self {
            enabled: true,
            layer: None,
            inputs: Vec::new(),
            gesture: Gesture::Press,
            ms: None,
            steps: Vec::new(),
            repeat: Repeat::None,
            toggle: false,
            passthrough: false,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StickId {
    Left,
    Right,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AnalogMode {
    /// The stick moves the mouse. `speed` from 1 to 100.
    Mouse { speed: u8 },
    /// The stick scrolls (vertically). `speed` from 1 to 100.
    Scroll { speed: u8 },
    /// Each stick direction presses a key (for example WASD).
    Keys { up: String, down: String, left: String, right: String },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(default)]
pub struct AnalogBinding {
    pub enabled: bool,
    pub layer: Option<String>,
    pub stick: StickId,
    pub mode: AnalogMode,
    /// Dead zone, as a percentage of the travel.
    pub deadzone: u8,
    /// Stop passing this stick to the virtual controller.
    pub consume: bool,
}

impl Default for AnalogBinding {
    fn default() -> Self {
        Self {
            enabled: true,
            layer: None,
            stick: StickId::Right,
            mode: AnalogMode::Mouse { speed: 40 },
            deadzone: 15,
            consume: true,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Limits and validation
// ---------------------------------------------------------------------------------------------

pub const MAX_RULES: usize = 64;
pub const MAX_ANALOG: usize = 8;
const MAX_STEPS: usize = 64;
const MAX_INPUTS: usize = 4;
/// Minimum interval between two repetitions: avoids flooding Windows with events.
const MIN_INTERVAL_MS: u32 = 10;
const MAX_WAIT_MS: u32 = 60_000;
const MAX_REPEAT_TIMES: u32 = 1_000;
const DEFAULT_HOLD_MS: u32 = shortcuts::HOLD_DURATION.as_millis() as u32;
const DEFAULT_DOUBLE_MS: u32 = 300;
/// Duration of a short press (press then release).
const PULSE: Duration = Duration::from_millis(40);

fn valid_output(output: &Output) -> bool {
    match output {
        Output::Key { key } => keys::lookup(key).is_some(),
        Output::Pad { target } => *target != XTarget::None,
        Output::Mouse { .. } => true,
    }
}

fn clean_step(step: Step) -> Option<Step> {
    match &step {
        Step::Hold { output } | Step::Tap { output } | Step::Down { output } | Step::Up { output }
            if !valid_output(output) =>
        {
            None
        }
        Step::Wait { ms } => Some(Step::Wait { ms: (*ms).min(MAX_WAIT_MS) }),
        _ => Some(step),
    }
}

/// Brings rules coming from the interface back to safe values: limits, duplicates, unknown keys.
pub fn sanitize_rules(rules: Vec<Rule>) -> Vec<Rule> {
    rules
        .into_iter()
        .take(MAX_RULES)
        .map(|mut rule| {
            let mut seen = Vec::new();
            for input in rule.inputs.drain(..) {
                if !seen.contains(&input) && seen.len() < MAX_INPUTS {
                    seen.push(input);
                }
            }
            rule.inputs = seen;
            rule.steps = rule.steps.into_iter().take(MAX_STEPS).filter_map(clean_step).collect();
            rule.ms = rule.ms.map(|ms| ms.clamp(50, 10_000));
            rule.repeat = match rule.repeat {
                Repeat::None => Repeat::None,
                Repeat::UntilReleased { interval_ms } => {
                    Repeat::UntilReleased { interval_ms: interval_ms.clamp(MIN_INTERVAL_MS, MAX_WAIT_MS) }
                }
                Repeat::Times { count, interval_ms } => Repeat::Times {
                    count: count.clamp(1, MAX_REPEAT_TIMES),
                    interval_ms: interval_ms.clamp(MIN_INTERVAL_MS, MAX_WAIT_MS),
                },
            };
            rule.layer = rule.layer.map(|l| l.trim().to_string()).filter(|l| !l.is_empty());
            rule
        })
        .collect()
}

pub fn sanitize_analog(bindings: Vec<AnalogBinding>) -> Vec<AnalogBinding> {
    bindings
        .into_iter()
        .take(MAX_ANALOG)
        .filter(|b| match &b.mode {
            AnalogMode::Keys { up, down, left, right } => {
                [up, down, left, right].iter().all(|k| keys::lookup(k).is_some())
            }
            _ => true,
        })
        .map(|mut b| {
            b.deadzone = b.deadzone.min(80);
            b.mode = match b.mode {
                AnalogMode::Mouse { speed } => AnalogMode::Mouse { speed: speed.clamp(1, 100) },
                AnalogMode::Scroll { speed } => AnalogMode::Scroll { speed: speed.clamp(1, 100) },
                keys => keys,
            };
            b.layer = b.layer.map(|l| l.trim().to_string()).filter(|l| !l.is_empty());
            b
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Migration of the legacy shortcuts
// ---------------------------------------------------------------------------------------------

/// Legacy shortcuts (one button, one gesture, one action) become rules. A "press" that
/// shares its button with a "hold" becomes a short press: that was already its behaviour.
pub fn rules_from_shortcuts(list: &[Shortcut]) -> Vec<Rule> {
    list.iter()
        .map(|s| {
            let gesture = match s.gesture {
                shortcuts::Gesture::Hold => Gesture::Hold,
                shortcuts::Gesture::Press => {
                    let has_hold = list
                        .iter()
                        .any(|o| o.button == s.button && o.gesture == shortcuts::Gesture::Hold);
                    if has_hold { Gesture::Tap } else { Gesture::Press }
                }
            };
            let action = match s.action {
                shortcuts::Action::ToggleVirtualPad => AppAction::ToggleVirtualPad,
                shortcuts::Action::PowerOff => AppAction::PowerOff,
                shortcuts::Action::ShowOverlay => AppAction::ShowOverlay,
                shortcuts::Action::ToggleMotion => AppAction::ToggleMotion,
                shortcuts::Action::OpenMenu => AppAction::OpenMenu,
            };
            Rule {
                inputs: vec![s.button.into()],
                gesture,
                steps: vec![Step::App { action }],
                ..Rule::default()
            }
        })
        .collect()
}

/// The rule that opens the controller menu by holding PS. It is one of the default rules.
pub fn menu_rule() -> Rule {
    Rule {
        inputs: vec![Input::Ps],
        gesture: Gesture::Hold,
        steps: vec![Step::App { action: AppAction::OpenMenu }],
        ..Rule::default()
    }
}

/// Adds the rule that opens the menu to rules saved before the menu existed (holding PS was then
/// built in, and is now only this rule). Nothing is added if a rule already opens the menu. The
/// default "PS press → show the overlay" becomes a short press, as it does for a new profile, so that
/// the overlay does not flash just before the menu opens.
pub fn with_menu_rule(mut rules: Vec<Rule>) -> Vec<Rule> {
    let opens_menu = |r: &Rule| r.steps.iter().any(|s| matches!(s, Step::App { action: AppAction::OpenMenu }));
    if rules.iter().any(opens_menu) {
        return rules;
    }
    for r in &mut rules {
        let default_overlay_rule = r.inputs == [Input::Ps]
            && r.gesture == Gesture::Press
            && r.layer.is_none()
            && r.steps == [Step::App { action: AppAction::ShowOverlay }];
        if default_overlay_rule {
            r.gesture = Gesture::Tap;
        }
    }
    rules.push(menu_rule());
    rules
}

// ---------------------------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------------------------

/// What the engine needs to know about the controller to evaluate rules.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub buttons: u32,
    pub lx: u8,
    pub ly: u8,
    pub rx: u8,
    pub ry: u8,
}

impl From<&ControllerState> for Snapshot {
    fn from(s: &ControllerState) -> Self {
        Self { buttons: s.buttons, lx: s.lx, ly: s.ly, rx: s.rx, ry: s.ry }
    }
}

/// What the engine asks the outside world to carry out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// An output has just been pressed / released (Xbox outputs are read through `held_pad`).
    Down(Output),
    Up(Output),
    MouseMove(i32, i32),
    Wheel(i32),
    App(AppAction),
}

/// Buttons of the virtual Xbox controller held by rules, to be added to those of the physical controller.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PadHeld {
    pub buttons: u16,
    pub left_trigger: bool,
    pub right_trigger: bool,
    /// The stick must no longer be passed on (an analog binding takes it).
    pub left_stick_taken: bool,
    pub right_stick_taken: bool,
}

#[derive(Default)]
struct RuleState {
    /// Start of the current press.
    down_since: Option<Instant>,
    /// The "hold" gesture has already fired for this press.
    fired: bool,
    /// Release of the last press, for the double press.
    last_release: Option<Instant>,
    /// Latched rule (toggle mode).
    locked: bool,
    /// The rule was active on the previous turn: otherwise, an input that is already held does not trigger it.
    was_enabled: bool,
}

struct Runner {
    rule: usize,
    pc: usize,
    iteration: u32,
    wait_until: Option<Instant>,
    /// "Held" outputs stay pressed until the rule explicitly ends.
    sustained: bool,
    /// All steps have been played (a "held" runner stays alive until release).
    done: bool,
    /// Outputs pressed by this runner: released when it ends.
    held: Vec<Output>,
    layers: Vec<String>,
}

#[derive(Default)]
struct AnalogState {
    keys: [bool; 4],
}

pub struct Engine {
    rules: Vec<Rule>,
    masks: Vec<u32>,
    states: Vec<RuleState>,
    analog: Vec<AnalogBinding>,
    analog_states: Vec<AnalogState>,

    previous: u32,
    /// Stick directions currently considered pressed (with hysteresis).
    dirs: u32,
    held: HashMap<Output, u32>,
    held_layers: HashMap<String, u32>,
    toggled_layers: Vec<String>,
    runners: Vec<Runner>,
    timers: Vec<(Instant, Output)>,
    last_tick: Option<Instant>,
    mouse_acc: (f32, f32),
    wheel_acc: f32,

    consumed: u32,
    sticks_taken: (bool, bool),
}

/// Beyond this value (as a fraction of the travel), a stick direction is "pressed";
/// it only becomes released again below `DIR_OFF`.
const DIR_ON: f32 = 0.55;
const DIR_OFF: f32 = 0.40;
/// Safety net: maximum steps played per turn for a single runner.
const MAX_STEPS_PER_TICK: usize = 256;

fn norm(v: u8) -> f32 {
    (f32::from(v) - 127.5) / 127.5
}

impl Engine {
    pub fn new(rules: Vec<Rule>, analog: Vec<AnalogBinding>) -> Self {
        let mut e = Self {
            rules: Vec::new(),
            masks: Vec::new(),
            states: Vec::new(),
            analog: Vec::new(),
            analog_states: Vec::new(),
            previous: 0,
            dirs: 0,
            held: HashMap::new(),
            held_layers: HashMap::new(),
            toggled_layers: Vec::new(),
            runners: Vec::new(),
            timers: Vec::new(),
            last_tick: None,
            mouse_acc: (0.0, 0.0),
            wheel_acc: 0.0,
            consumed: 0,
            sticks_taken: (false, false),
        };
        e.load(rules, analog);
        e
    }

    fn load(&mut self, rules: Vec<Rule>, analog: Vec<AnalogBinding>) {
        self.masks = rules.iter().map(|r| r.inputs.iter().fold(0, |m, i| m | i.mask())).collect();
        self.states = rules.iter().map(|_| RuleState::default()).collect();
        self.rules = rules;
        self.analog_states = analog.iter().map(|_| AnalogState::default()).collect();
        self.analog = analog;
    }

    /// Replaces the rules. Everything that was pressed is released (no stuck key ever), and
    /// presses in progress trigger nothing until they are released.
    pub fn set_config(&mut self, rules: Vec<Rule>, analog: Vec<AnalogBinding>) -> Vec<Effect> {
        let released = self.release_all();
        self.load(rules, analog);
        // Inputs that are already held count as "already seen": no phantom trigger.
        for state in &mut self.states {
            state.was_enabled = true;
            state.fired = true;
        }
        released
    }

    /// Releases every pressed output and abandons whatever is in progress.
    pub fn release_all(&mut self) -> Vec<Effect> {
        let mut effects: Vec<Effect> = self
            .held
            .keys()
            .cloned()
            .map(Effect::Up)
            .collect();
        // Stable order, for reproducible results.
        effects.sort_by_key(|e| format!("{e:?}"));
        self.held.clear();
        self.held_layers.clear();
        self.toggled_layers.clear();
        self.runners.clear();
        self.timers.clear();
        for s in &mut self.states {
            *s = RuleState::default();
        }
        for a in &mut self.analog_states {
            a.keys = [false; 4];
        }
        self.mouse_acc = (0.0, 0.0);
        self.wheel_acc = 0.0;
        self.consumed = 0;
        self.sticks_taken = (false, false);
        effects
    }

    /// Physical inputs not to pass to the virtual controller (bits of `ControllerState::buttons`).
    pub fn consumed(&self) -> u32 {
        self.consumed
    }

    /// Layers currently active.
    pub fn active_layers(&self) -> Vec<String> {
        let mut layers: Vec<String> = self.toggled_layers.clone();
        for (name, count) in &self.held_layers {
            if *count > 0 && !layers.contains(name) {
                layers.push(name.clone());
            }
        }
        layers.sort();
        layers
    }

    /// What the rules hold on the virtual Xbox controller.
    pub fn held_pad(&self) -> PadHeld {
        let mut pad = PadHeld::default();
        for (output, count) in &self.held {
            if *count == 0 {
                continue;
            }
            if let Output::Pad { target } = output {
                match target {
                    XTarget::Lt => pad.left_trigger = true,
                    XTarget::Rt => pad.right_trigger = true,
                    other => pad.buttons |= other.bit().unwrap_or(0),
                }
            }
        }
        pad.left_stick_taken = self.sticks_taken.0;
        pad.right_stick_taken = self.sticks_taken.1;
        pad
    }

    // --- held outputs (counted: two rules may hold the same key) --------------------------------

    fn down(&mut self, output: &Output, effects: &mut Vec<Effect>) {
        if !valid_output(output) {
            return;
        }
        let count = self.held.entry(output.clone()).or_insert(0);
        *count += 1;
        if *count == 1 {
            effects.push(Effect::Down(output.clone()));
        }
    }

    fn up(&mut self, output: &Output, effects: &mut Vec<Effect>) {
        if let Some(count) = self.held.get_mut(output) {
            *count -= 1;
            if *count == 0 {
                self.held.remove(output);
                effects.push(Effect::Up(output.clone()));
            }
        }
    }

    // --- reading the inputs ----------------------------------------------------------------------

    fn read_inputs(&mut self, snap: &Snapshot) -> u32 {
        let mut dirs = 0u32;
        let sticks = [
            (norm(snap.lx), norm(snap.ly), Input::LeftStickUp, Input::LeftStickRight, Input::LeftStickDown, Input::LeftStickLeft),
            (norm(snap.rx), norm(snap.ry), Input::RightStickUp, Input::RightStickRight, Input::RightStickDown, Input::RightStickLeft),
        ];
        for (x, y, up, right, down, left) in sticks {
            // y grows downwards: a stick pushed up has a negative y.
            for (input, value) in [(up, -y), (right, x), (down, y), (left, -x)] {
                let was = self.dirs & input.mask() != 0;
                let now = if was { value > DIR_OFF } else { value > DIR_ON };
                if now {
                    dirs |= input.mask();
                }
            }
        }
        self.dirs = dirs;
        (snap.buttons & BUTTON_MASK) | dirs
    }

    // --- layers ------------------------------------------------------------------------------------

    fn layer_active(&self, name: &str) -> bool {
        self.toggled_layers.iter().any(|l| l == name) || self.held_layers.get(name).is_some_and(|c| *c > 0)
    }

    /// Which rules are in force: those of the base layer and those of the active layers.
    /// A layer rule that shares an input with a base rule replaces it.
    fn enabled_rules(&self) -> Vec<bool> {
        let mut enabled: Vec<bool> = self
            .rules
            .iter()
            .enumerate()
            .map(|(i, r)| {
                r.enabled
                    && self.masks[i] != 0
                    && !r.steps.is_empty()
                    && r.layer.as_deref().is_none_or(|l| self.layer_active(l))
            })
            .collect();
        let layer_inputs = self
            .rules
            .iter()
            .enumerate()
            .filter(|(i, r)| enabled[*i] && r.layer.is_some())
            .fold(0u32, |m, (i, _)| m | self.masks[i]);
        for (i, r) in self.rules.iter().enumerate() {
            if r.layer.is_none() && self.masks[i] & layer_inputs != 0 {
                enabled[i] = false;
            }
        }
        enabled
    }

    // --- running the rules ------------------------------------------------------------------------

    fn start(&mut self, rule: usize, sustained: bool, now: Instant, effects: &mut Vec<Effect>) {
        // A rule still running its macro is not started again on top of itself.
        if self.runners.iter().any(|r| r.rule == rule) {
            return;
        }
        self.runners.push(Runner {
            rule,
            pc: 0,
            iteration: 0,
            wait_until: None,
            sustained,
            done: false,
            held: Vec::new(),
            layers: Vec::new(),
        });
        self.advance(self.runners.len() - 1, now, effects);
    }

    /// Ends a rule's runner: releases whatever it was holding.
    fn stop(&mut self, rule: usize, effects: &mut Vec<Effect>) {
        if let Some(pos) = self.runners.iter().position(|r| r.rule == rule) {
            let runner = self.runners.remove(pos);
            self.release_runner(runner, effects);
        }
    }

    fn release_runner(&mut self, runner: Runner, effects: &mut Vec<Effect>) {
        for output in runner.held {
            self.up(&output, effects);
        }
        for layer in runner.layers {
            if let Some(count) = self.held_layers.get_mut(&layer) {
                *count -= 1;
                if *count == 0 {
                    self.held_layers.remove(&layer);
                }
            }
        }
    }

    /// Advances a runner: plays the steps up to the next pause.
    fn advance(&mut self, index: usize, now: Instant, effects: &mut Vec<Effect>) {
        for _ in 0..MAX_STEPS_PER_TICK {
            let runner = &mut self.runners[index];
            if runner.done {
                return;
            }
            if let Some(until) = runner.wait_until {
                if now < until {
                    return;
                }
                runner.wait_until = None;
            }
            let rule = &self.rules[runner.rule];

            if runner.pc >= rule.steps.len() {
                // End of a pass: start over, or stop.
                let again = match rule.repeat {
                    Repeat::UntilReleased { interval_ms } if runner.sustained => Some(interval_ms),
                    Repeat::Times { count, interval_ms } if runner.iteration + 1 < count => Some(interval_ms),
                    _ => None,
                };
                match again {
                    Some(interval) => {
                        runner.iteration += 1;
                        runner.pc = 0;
                        runner.wait_until = Some(now + Duration::from_millis(u64::from(interval.max(MIN_INTERVAL_MS))));
                        return;
                    }
                    None => {
                        runner.done = true;
                        return;
                    }
                }
            }

            let step = rule.steps[runner.pc].clone();
            let sustained = runner.sustained;
            runner.pc += 1;

            match step {
                Step::Wait { ms } => {
                    runner.wait_until = Some(now + Duration::from_millis(u64::from(ms)));
                    return;
                }
                Step::Tap { output } => self.tap(&output, now, effects),
                Step::Hold { output } if sustained => {
                    self.runners[index].held.push(output.clone());
                    self.down(&output, effects);
                }
                Step::Hold { output } => self.tap(&output, now, effects),
                Step::Down { output } => {
                    self.runners[index].held.push(output.clone());
                    self.down(&output, effects);
                }
                Step::Up { output } => {
                    let held = &mut self.runners[index].held;
                    if let Some(pos) = held.iter().position(|o| *o == output) {
                        held.remove(pos);
                        self.up(&output, effects);
                    }
                }
                Step::Wheel { amount } => effects.push(Effect::Wheel(amount)),
                Step::MouseMove { dx, dy } => effects.push(Effect::MouseMove(dx, dy)),
                Step::App { action: AppAction::Layer { name, mode } } => {
                    self.layer_step(index, &name, mode);
                }
                Step::App { action } => effects.push(Effect::App(action)),
            }
        }
    }

    /// Short press: press now, release in a moment.
    fn tap(&mut self, output: &Output, now: Instant, effects: &mut Vec<Effect>) {
        if !valid_output(output) {
            return;
        }
        self.down(output, effects);
        self.timers.push((now + PULSE, output.clone()));
    }

    fn layer_step(&mut self, runner: usize, name: &str, mode: LayerMode) {
        let sustained = self.runners[runner].sustained;
        match mode {
            LayerMode::Hold if sustained => {
                *self.held_layers.entry(name.to_string()).or_insert(0) += 1;
                self.runners[runner].layers.push(name.to_string());
            }
            // Without a "while held" gesture, "hold" makes no sense: toggle instead.
            LayerMode::Hold | LayerMode::Toggle => {
                if let Some(pos) = self.toggled_layers.iter().position(|l| l == name) {
                    self.toggled_layers.remove(pos);
                } else {
                    self.toggled_layers.push(name.to_string());
                }
            }
            LayerMode::On => {
                if !self.toggled_layers.iter().any(|l| l == name) {
                    self.toggled_layers.push(name.to_string());
                }
            }
            LayerMode::Off => self.toggled_layers.retain(|l| l != name),
        }
    }

    fn fire(&mut self, rule: usize, sustained: bool, now: Instant, effects: &mut Vec<Effect>) {
        if self.rules[rule].toggle {
            self.states[rule].locked = !self.states[rule].locked;
            if self.states[rule].locked {
                self.start(rule, true, now, effects);
            } else {
                self.stop(rule, effects);
            }
        } else {
            self.start(rule, sustained, now, effects);
        }
    }

    /// To be called on every report **and** regularly even without a new report: a hold, a
    /// macro pause or a repetition are triggered by time. Returns the effects to carry out.
    pub fn update(&mut self, snap: &Snapshot, now: Instant) -> Vec<Effect> {
        let mut effects = Vec::new();
        let dt = self
            .last_tick
            .map_or(0.0, |t| now.saturating_duration_since(t).as_secs_f32().min(0.1));
        self.last_tick = Some(now);

        let pressed = self.read_inputs(snap);

        // Deferred releases of short presses
        let due: Vec<Output> = {
            let (due, later): (Vec<_>, Vec<_>) = self.timers.drain(..).partition(|(t, _)| *t <= now);
            self.timers = later;
            due.into_iter().map(|(_, o)| o).collect()
        };
        for output in due {
            self.up(&output, &mut effects);
        }

        let enabled = self.enabled_rules();
        for i in 0..self.rules.len() {
            if !enabled[i] {
                // Disabled (layer left, rule replaced…): cut what it was holding.
                if self.states[i].was_enabled {
                    self.stop(i, &mut effects);
                    self.states[i] = RuleState::default();
                }
                continue;
            }
            let mask = self.masks[i];
            let down = pressed & mask == mask;
            // The previous state comes from the input itself, not from the rule: a rule that comes into force
            // (layer activated) therefore does not see an input that was already held as a press.
            let was = self.previous & mask == mask;
            self.states[i].was_enabled = true;
            let rising = down && !was;
            let falling = !down && was;
            let ms = u64::from(self.rules[i].ms.unwrap_or(match self.rules[i].gesture {
                Gesture::DoubleTap => DEFAULT_DOUBLE_MS,
                _ => DEFAULT_HOLD_MS,
            }));
            let limit = Duration::from_millis(ms);

            match self.rules[i].gesture {
                Gesture::Press => {
                    if rising {
                        self.fire(i, false, now, &mut effects);
                    }
                }
                Gesture::Release => {
                    if falling {
                        self.fire(i, false, now, &mut effects);
                    }
                }
                Gesture::While => {
                    if rising {
                        self.fire(i, true, now, &mut effects);
                    } else if falling && !self.rules[i].toggle {
                        self.stop(i, &mut effects);
                    }
                }
                Gesture::Tap => {
                    if rising {
                        self.states[i].down_since = Some(now);
                    } else if falling {
                        if let Some(since) = self.states[i].down_since.take() {
                            if now.saturating_duration_since(since) < limit {
                                self.fire(i, false, now, &mut effects);
                            }
                        }
                    }
                }
                Gesture::Hold => {
                    if rising {
                        self.states[i].down_since = Some(now);
                        self.states[i].fired = false;
                    } else if falling {
                        self.states[i].down_since = None;
                    } else if down && !self.states[i].fired {
                        if let Some(since) = self.states[i].down_since {
                            if now.saturating_duration_since(since) >= limit {
                                self.states[i].fired = true;
                                self.fire(i, false, now, &mut effects);
                            }
                        }
                    }
                }
                Gesture::DoubleTap => {
                    if falling {
                        // The release that follows a double press is not the start of the next one.
                        if std::mem::take(&mut self.states[i].fired) {
                            self.states[i].last_release = None;
                        } else {
                            self.states[i].last_release = Some(now);
                        }
                    } else if rising {
                        let recent = self.states[i]
                            .last_release
                            .is_some_and(|t| now.saturating_duration_since(t) <= limit);
                        if recent {
                            self.states[i].last_release = None;
                            self.states[i].fired = true;
                            self.fire(i, false, now, &mut effects);
                        }
                    }
                }
            }
        }

        // Macros and repetitions in progress
        let mut index = 0;
        while index < self.runners.len() {
            self.advance(index, now, &mut effects);
            // A finished runner that is not "held" releases what it was holding.
            let finished = self.runners[index].done && !self.runners[index].sustained;
            if finished {
                let mut runner = self.runners.remove(index);
                // Outputs left pressed are only released after a short press: otherwise a macro
                // "Shift then A" would let go of Shift the moment A is pressed.
                for output in runner.held.drain(..) {
                    self.timers.push((now + PULSE, output));
                }
                self.release_runner(runner, &mut effects);
            } else {
                index += 1;
            }
        }

        // Layers may have changed during this turn: rules that no longer exist release what they
        // were holding immediately, without waiting for the next turn.
        let enabled_now = self.enabled_rules();
        for i in 0..self.rules.len() {
            if enabled[i] && !enabled_now[i] {
                self.stop(i, &mut effects);
                self.states[i] = RuleState::default();
            }
        }

        let layers = self.active_layers();
        self.update_analog(snap, &layers, dt, &mut effects);

        // Inputs taken by a rule: not passed to the virtual controller.
        let mut consumed = 0u32;
        for i in 0..self.rules.len() {
            if !enabled_now[i] || self.rules[i].passthrough {
                continue;
            }
            let mask = self.masks[i];
            if mask.count_ones() == 1 || pressed & mask == mask {
                consumed |= mask & BUTTON_MASK;
            }
        }
        self.consumed = consumed;
        self.previous = pressed;
        effects
    }

    fn update_analog(&mut self, snap: &Snapshot, layers: &[String], dt: f32, effects: &mut Vec<Effect>) {
        let mut taken = (false, false);
        for i in 0..self.analog.len() {
            let binding = self.analog[i].clone();
            let active = binding.enabled && binding.layer.as_ref().is_none_or(|l| layers.contains(l));
            let (x, y) = match binding.stick {
                StickId::Left => (norm(snap.lx), norm(snap.ly)),
                StickId::Right => (norm(snap.rx), norm(snap.ry)),
            };
            if !active {
                // Leaving a layer releases the keys this stick was holding.
                if let AnalogMode::Keys { up, down, left, right } = &binding.mode {
                    for (n, key) in [up, down, left, right].into_iter().enumerate() {
                        if std::mem::replace(&mut self.analog_states[i].keys[n], false) {
                            self.up(&Output::Key { key: key.clone() }, effects);
                        }
                    }
                }
                continue;
            }
            if binding.consume {
                match binding.stick {
                    StickId::Left => taken.0 = true,
                    StickId::Right => taken.1 = true,
                }
            }

            let dead = f32::from(binding.deadzone) / 100.0;
            let magnitude = x.hypot(y);
            let strength = if magnitude <= dead { 0.0 } else { ((magnitude - dead) / (1.0 - dead)).min(1.0) };
            let (ux, uy) = if magnitude > 0.0 { (x / magnitude, y / magnitude) } else { (0.0, 0.0) };

            match &binding.mode {
                AnalogMode::Mouse { speed } => {
                    // Slightly finer response near the centre, for gentle aiming.
                    let pixels = strength * strength.sqrt() * f32::from(*speed) * 15.0 * dt;
                    self.mouse_acc.0 += ux * pixels;
                    self.mouse_acc.1 += uy * pixels;
                    let (dx, dy) = (self.mouse_acc.0.trunc(), self.mouse_acc.1.trunc());
                    if dx != 0.0 || dy != 0.0 {
                        self.mouse_acc.0 -= dx;
                        self.mouse_acc.1 -= dy;
                        effects.push(Effect::MouseMove(dx as i32, dy as i32));
                    }
                }
                AnalogMode::Scroll { speed } => {
                    // Upwards: positive wheel. 120 per notch.
                    self.wheel_acc += -uy * strength * f32::from(*speed) * 12.0 * dt;
                    let units = self.wheel_acc.trunc();
                    if units != 0.0 {
                        self.wheel_acc -= units;
                        effects.push(Effect::Wheel(units as i32));
                    }
                }
                AnalogMode::Keys { up, down, left, right } => {
                    let wants = [-y, y, -x, x];
                    for (n, key) in [up, down, left, right].into_iter().enumerate() {
                        let was = self.analog_states[i].keys[n];
                        let is = if was { wants[n] > DIR_OFF.max(dead) } else { wants[n] > DIR_ON.max(dead) };
                        if is != was {
                            self.analog_states[i].keys[n] = is;
                            let output = Output::Key { key: key.clone() };
                            if is {
                                self.down(&output, effects);
                            } else {
                                self.up(&output, effects);
                            }
                        }
                    }
                }
            }
        }
        self.sticks_taken = taken;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::btn;

    const MS: fn(u64) -> Duration = Duration::from_millis;

    fn snap(buttons: u32) -> Snapshot {
        Snapshot { buttons, lx: 128, ly: 128, rx: 128, ry: 128 }
    }

    fn key(k: &str) -> Output {
        Output::Key { key: k.into() }
    }
    fn pad(t: XTarget) -> Output {
        Output::Pad { target: t }
    }
    fn rule(input: Input, gesture: Gesture, steps: Vec<Step>) -> Rule {
        Rule { inputs: vec![input], gesture, steps, ..Rule::default() }
    }
    fn tap(k: &str) -> Step {
        Step::Tap { output: key(k) }
    }
    fn hold(k: &str) -> Step {
        Step::Hold { output: key(k) }
    }
    fn app(action: AppAction) -> Step {
        Step::App { action }
    }
    /// The sticks taken by an analog binding: (left, right).
    fn taken(e: &Engine) -> (bool, bool) {
        let pad = e.held_pad();
        (pad.left_stick_taken, pad.right_stick_taken)
    }
    fn engine(rules: Vec<Rule>) -> (Engine, Instant) {
        (Engine::new(rules, vec![]), Instant::now())
    }
    fn downs(effects: &[Effect]) -> Vec<Output> {
        effects.iter().filter_map(|e| if let Effect::Down(o) = e { Some(o.clone()) } else { None }).collect()
    }
    fn ups(effects: &[Effect]) -> Vec<Output> {
        effects.iter().filter_map(|e| if let Effect::Up(o) = e { Some(o.clone()) } else { None }).collect()
    }

    // --- gestures ----------------------------------------------------------------------------------

    #[test]
    fn the_menu_can_be_opened_by_a_rule() {
        let action: AppAction = serde_json::from_str(r#"{"type":"open_menu"}"#).unwrap();
        assert_eq!(action, AppAction::OpenMenu);
        let (mut e, t0) = engine(vec![rule(Input::Share, Gesture::Hold, vec![app(AppAction::OpenMenu)])]);
        e.update(&snap(btn::SHARE), t0);
        assert_eq!(e.update(&snap(btn::SHARE), t0 + MS(800)), [Effect::App(AppAction::OpenMenu)]);
    }

    #[test]
    fn press_fires_once_when_the_button_goes_down() {
        let (mut e, t0) = engine(vec![rule(Input::Ps, Gesture::Press, vec![app(AppAction::ShowOverlay)])]);
        assert_eq!(e.update(&snap(btn::PS), t0), [Effect::App(AppAction::ShowOverlay)]);
        assert!(e.update(&snap(btn::PS), t0 + MS(5000)).is_empty(), "no repetition while held");
        assert!(e.update(&snap(0), t0 + MS(5100)).is_empty(), "nothing on release");
    }

    #[test]
    fn release_fires_when_the_button_goes_up() {
        let (mut e, t0) = engine(vec![rule(Input::Ps, Gesture::Release, vec![app(AppAction::ShowOverlay)])]);
        assert!(e.update(&snap(btn::PS), t0).is_empty());
        assert_eq!(e.update(&snap(0), t0 + MS(100)), [Effect::App(AppAction::ShowOverlay)]);
    }

    #[test]
    fn tap_fires_on_release_only_if_short() {
        let (mut e, t0) = engine(vec![rule(Input::Mute, Gesture::Tap, vec![app(AppAction::ToggleVirtualPad)])]);
        assert!(e.update(&snap(btn::MUTE), t0).is_empty(), "not before release");
        assert_eq!(e.update(&snap(0), t0 + MS(200)), [Effect::App(AppAction::ToggleVirtualPad)]);

        e.update(&snap(btn::MUTE), t0 + MS(1000));
        assert!(e.update(&snap(0), t0 + MS(1900)).is_empty(), "trop long pour un appui bref");
    }

    #[test]
    fn hold_fires_once_after_its_duration() {
        let (mut e, t0) = engine(vec![rule(Input::Mute, Gesture::Hold, vec![app(AppAction::PowerOff)])]);
        e.update(&snap(btn::MUTE), t0);
        assert!(e.update(&snap(btn::MUTE), t0 + MS(700)).is_empty(), "too early");
        assert_eq!(e.update(&snap(btn::MUTE), t0 + MS(800)), [Effect::App(AppAction::PowerOff)]);
        assert!(e.update(&snap(btn::MUTE), t0 + MS(3000)).is_empty(), "une seule fois");
        assert!(e.update(&snap(0), t0 + MS(3100)).is_empty());
    }

    #[test]
    fn hold_does_not_fire_when_released_early() {
        let (mut e, t0) = engine(vec![rule(Input::Mute, Gesture::Hold, vec![app(AppAction::PowerOff)])]);
        e.update(&snap(btn::MUTE), t0);
        e.update(&snap(btn::MUTE), t0 + MS(500));
        assert!(e.update(&snap(0), t0 + MS(600)).is_empty());
        assert!(e.update(&snap(0), t0 + MS(2000)).is_empty());
    }

    #[test]
    fn a_custom_hold_duration_is_used() {
        let mut r = rule(Input::Mute, Gesture::Hold, vec![app(AppAction::PowerOff)]);
        r.ms = Some(2000);
        let (mut e, t0) = engine(vec![r]);
        e.update(&snap(btn::MUTE), t0);
        assert!(e.update(&snap(btn::MUTE), t0 + MS(1999)).is_empty());
        assert_eq!(e.update(&snap(btn::MUTE), t0 + MS(2000)).len(), 1);
    }

    #[test]
    fn tap_and_hold_on_the_same_button_do_not_both_fire() {
        let (mut e, t0) = engine(vec![
            rule(Input::Mute, Gesture::Tap, vec![app(AppAction::ToggleVirtualPad)]),
            rule(Input::Mute, Gesture::Hold, vec![app(AppAction::PowerOff)]),
        ]);
        e.update(&snap(btn::MUTE), t0);
        assert_eq!(e.update(&snap(btn::MUTE), t0 + MS(800)), [Effect::App(AppAction::PowerOff)]);
        assert!(e.update(&snap(0), t0 + MS(900)).is_empty(), "the release after a hold is not a short press");
    }

    #[test]
    fn double_tap_needs_two_presses_close_together() {
        let (mut e, t0) = engine(vec![rule(Input::Cross, Gesture::DoubleTap, vec![app(AppAction::ShowOverlay)])]);
        assert!(e.update(&snap(btn::CROSS), t0).is_empty());
        assert!(e.update(&snap(0), t0 + MS(50)).is_empty());
        assert_eq!(e.update(&snap(btn::CROSS), t0 + MS(200)).len(), 1, "second press within the window");
        e.update(&snap(0), t0 + MS(250));
        assert!(e.update(&snap(btn::CROSS), t0 + MS(900)).is_empty(), "trop tard : c'est un nouveau premier appui");
    }

    #[test]
    fn a_double_tap_does_not_fire_a_third_time_on_a_third_press() {
        let (mut e, t0) = engine(vec![rule(Input::Cross, Gesture::DoubleTap, vec![app(AppAction::ShowOverlay)])]);
        e.update(&snap(btn::CROSS), t0);
        e.update(&snap(0), t0 + MS(50));
        assert_eq!(e.update(&snap(btn::CROSS), t0 + MS(100)).len(), 1);
        e.update(&snap(0), t0 + MS(150));
        assert!(e.update(&snap(btn::CROSS), t0 + MS(200)).is_empty(), "il faut deux nouveaux appuis");
    }

    // --- held outputs and short presses ---------------------------------------------------------------

    #[test]
    fn while_holds_the_output_as_long_as_the_button_is_held() {
        let (mut e, t0) = engine(vec![rule(Input::Square, Gesture::While, vec![hold("E")])]);
        assert_eq!(downs(&e.update(&snap(btn::SQUARE), t0)), [key("E")]);
        assert!(e.update(&snap(btn::SQUARE), t0 + MS(500)).is_empty());
        assert_eq!(ups(&e.update(&snap(0), t0 + MS(600))), [key("E")]);
    }

    #[test]
    fn hold_step_on_an_impulse_gesture_is_a_short_press() {
        let (mut e, t0) = engine(vec![rule(Input::Square, Gesture::Press, vec![hold("E")])]);
        let fx = e.update(&snap(btn::SQUARE), t0);
        assert_eq!(downs(&fx), [key("E")]);
        assert!(ups(&e.update(&snap(btn::SQUARE), t0 + MS(10))).is_empty(), "not released yet");
        assert_eq!(ups(&e.update(&snap(btn::SQUARE), t0 + MS(60))), [key("E")], "released after the short press");
    }

    #[test]
    fn xbox_outputs_are_reported_as_held_pad_state() {
        let (mut e, t0) = engine(vec![
            rule(Input::Square, Gesture::While, vec![Step::Hold { output: pad(XTarget::A) }]),
            rule(Input::Circle, Gesture::While, vec![Step::Hold { output: pad(XTarget::Rt) }]),
        ]);
        e.update(&snap(btn::SQUARE | btn::CIRCLE), t0);
        let held = e.held_pad();
        assert_eq!(held.buttons, crate::input::xbtn::A);
        assert!(held.right_trigger && !held.left_trigger);
        e.update(&snap(0), t0 + MS(10));
        assert_eq!(e.held_pad(), PadHeld::default());
    }

    #[test]
    fn two_rules_holding_the_same_key_release_it_only_when_both_are_done() {
        let (mut e, t0) = engine(vec![
            rule(Input::Square, Gesture::While, vec![hold("E")]),
            rule(Input::Circle, Gesture::While, vec![hold("E")]),
        ]);
        assert_eq!(downs(&e.update(&snap(btn::SQUARE), t0)).len(), 1);
        assert!(downs(&e.update(&snap(btn::SQUARE | btn::CIRCLE), t0 + MS(10))).is_empty(), "already pressed");
        assert!(ups(&e.update(&snap(btn::CIRCLE), t0 + MS(20))).is_empty(), "the other rule still holds it");
        assert_eq!(ups(&e.update(&snap(0), t0 + MS(30))).len(), 1);
    }

    #[test]
    fn mouse_buttons_wheel_and_moves_are_effects() {
        let (mut e, t0) = engine(vec![rule(
            Input::Cross,
            Gesture::Press,
            vec![
                Step::Tap { output: Output::Mouse { button: MouseButton::Left } },
                Step::Wheel { amount: -120 },
                Step::MouseMove { dx: 10, dy: -5 },
            ],
        )]);
        let fx = e.update(&snap(btn::CROSS), t0);
        assert!(fx.contains(&Effect::Down(Output::Mouse { button: MouseButton::Left })));
        assert!(fx.contains(&Effect::Wheel(-120)));
        assert!(fx.contains(&Effect::MouseMove(10, -5)));
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let (mut e, t0) = engine(vec![rule(Input::Cross, Gesture::Press, vec![tap("Hyper")])]);
        assert!(e.update(&snap(btn::CROSS), t0).is_empty());
    }

    // --- macros and repetitions --------------------------------------------------------------------------

    #[test]
    fn a_macro_plays_its_steps_with_pauses() {
        let (mut e, t0) = engine(vec![rule(
            Input::Triangle,
            Gesture::Press,
            vec![tap("A"), Step::Wait { ms: 100 }, tap("B")],
        )]);
        assert_eq!(downs(&e.update(&snap(btn::TRIANGLE), t0)), [key("A")]);
        assert!(downs(&e.update(&snap(btn::TRIANGLE), t0 + MS(60))).is_empty(), "B attend la fin de la pause");
        assert_eq!(ups(&e.update(&snap(btn::TRIANGLE), t0 + MS(60))), Vec::<Output>::new(), "A is already released");
        assert_eq!(downs(&e.update(&snap(btn::TRIANGLE), t0 + MS(100))), [key("B")]);
    }

    #[test]
    fn explicit_down_is_released_at_the_end_of_the_rule() {
        let (mut e, t0) = engine(vec![rule(
            Input::Cross,
            Gesture::Press,
            vec![Step::Down { output: key("Shift") }, tap("A")],
        )]);
        let fx = e.update(&snap(btn::CROSS), t0);
        assert_eq!(downs(&fx), [key("Shift"), key("A")]);
        assert!(ups(&fx).is_empty(), "Shift stays pressed as long as A is");
        // end of the steps: the forgotten key is released, after A
        assert_eq!(ups(&e.update(&snap(btn::CROSS), t0 + MS(60))), [key("A"), key("Shift")]);
    }

    #[test]
    fn up_releases_before_the_end() {
        let (mut e, t0) = engine(vec![rule(
            Input::Cross,
            Gesture::Press,
            vec![Step::Down { output: key("Shift") }, Step::Wait { ms: 50 }, Step::Up { output: key("Shift") }],
        )]);
        assert_eq!(downs(&e.update(&snap(btn::CROSS), t0)), [key("Shift")]);
        assert_eq!(ups(&e.update(&snap(btn::CROSS), t0 + MS(60))), [key("Shift")]);
    }

    #[test]
    fn turbo_repeats_while_held_and_stops_on_release() {
        let mut r = rule(Input::Cross, Gesture::While, vec![tap("F")]);
        r.repeat = Repeat::UntilReleased { interval_ms: 100 };
        let (mut e, t0) = engine(vec![r]);
        let mut count = downs(&e.update(&snap(btn::CROSS), t0)).len();
        for step in 1..=10u64 {
            count += downs(&e.update(&snap(btn::CROSS), t0 + MS(step * 50))).len();
        }
        assert!((5..=7).contains(&count), "environ une fois toutes les 100 ms, vu {count}");
        e.update(&snap(0), t0 + MS(600));
        assert!(downs(&e.update(&snap(0), t0 + MS(800))).is_empty(), "stopped on release");
    }

    #[test]
    fn repeat_times_stops_after_the_count() {
        let mut r = rule(Input::Cross, Gesture::Press, vec![tap("F")]);
        r.repeat = Repeat::Times { count: 3, interval_ms: 100 };
        let (mut e, t0) = engine(vec![r]);
        let mut total = downs(&e.update(&snap(btn::CROSS), t0)).len();
        for step in 1..=30u64 {
            total += downs(&e.update(&snap(btn::CROSS), t0 + MS(step * 50))).len();
        }
        assert_eq!(total, 3);
    }

    #[test]
    fn repeat_with_no_steps_and_an_interval_does_not_spin() {
        let mut r = rule(Input::Cross, Gesture::While, vec![Step::Wait { ms: 0 }]);
        r.repeat = Repeat::UntilReleased { interval_ms: 10 };
        let (mut e, t0) = engine(vec![r]);
        let _ = e.update(&snap(btn::CROSS), t0);
        let _ = e.update(&snap(btn::CROSS), t0 + MS(5));
    }

    #[test]
    fn a_running_macro_is_not_restarted_by_a_new_trigger() {
        let (mut e, t0) = engine(vec![rule(Input::Cross, Gesture::Press, vec![tap("A"), Step::Wait { ms: 500 }, tap("B")])]);
        assert_eq!(downs(&e.update(&snap(btn::CROSS), t0)), [key("A")]);
        e.update(&snap(0), t0 + MS(100));
        assert!(downs(&e.update(&snap(btn::CROSS), t0 + MS(200))).is_empty(), "the first run is not finished");
    }

    // --- toggle ----------------------------------------------------------------------------------------------

    #[test]
    fn toggle_locks_the_output_until_the_next_trigger() {
        let mut r = rule(Input::Square, Gesture::Press, vec![hold("W")]);
        r.toggle = true;
        let (mut e, t0) = engine(vec![r]);
        assert_eq!(downs(&e.update(&snap(btn::SQUARE), t0)), [key("W")]);
        assert!(ups(&e.update(&snap(0), t0 + MS(100))).is_empty(), "still held after the release");
        assert!(ups(&e.update(&snap(0), t0 + MS(500))).is_empty());
        assert_eq!(ups(&e.update(&snap(btn::SQUARE), t0 + MS(600))), [key("W")], "the second press releases it");
        assert!(downs(&e.update(&snap(0), t0 + MS(700))).is_empty());
    }

    // --- combinations ----------------------------------------------------------------------------------------

    #[test]
    fn a_chord_needs_all_its_inputs() {
        let mut r = rule(Input::L1, Gesture::Press, vec![app(AppAction::ShowOverlay)]);
        r.inputs = vec![Input::L1, Input::R1];
        let (mut e, t0) = engine(vec![r]);
        assert!(e.update(&snap(btn::L1), t0).is_empty());
        assert_eq!(e.update(&snap(btn::L1 | btn::R1), t0 + MS(20)).len(), 1);
        assert!(e.update(&snap(btn::L1 | btn::R1), t0 + MS(40)).is_empty());
    }

    #[test]
    fn a_chord_is_consumed_only_while_it_is_complete() {
        let mut r = rule(Input::L1, Gesture::Press, vec![app(AppAction::ShowOverlay)]);
        r.inputs = vec![Input::L1, Input::R1];
        let (mut e, t0) = engine(vec![r]);
        e.update(&snap(btn::L1), t0);
        assert_eq!(e.consumed(), 0, "L1 seul passe comme d'habitude");
        e.update(&snap(btn::L1 | btn::R1), t0 + MS(20));
        assert_eq!(e.consumed(), btn::L1 | btn::R1);
    }

    // --- consumed inputs ---------------------------------------------------------------------------------------

    #[test]
    fn bound_buttons_are_consumed_and_others_are_not() {
        let (mut e, t0) = engine(vec![
            rule(Input::Mute, Gesture::Tap, vec![app(AppAction::ShowOverlay)]),
            rule(Input::Ps, Gesture::Press, vec![app(AppAction::ShowOverlay)]),
        ]);
        e.update(&snap(0), t0);
        assert_eq!(e.consumed(), btn::MUTE | btn::PS);
    }

    #[test]
    fn passthrough_keeps_the_physical_button() {
        let mut r = rule(Input::Cross, Gesture::Press, vec![tap("E")]);
        r.passthrough = true;
        let (mut e, t0) = engine(vec![r]);
        e.update(&snap(btn::CROSS), t0);
        assert_eq!(e.consumed(), 0);
    }

    #[test]
    fn disabled_and_empty_rules_consume_nothing() {
        let mut off = rule(Input::Cross, Gesture::Press, vec![tap("E")]);
        off.enabled = false;
        let empty = rule(Input::Circle, Gesture::Press, vec![]);
        let (mut e, t0) = engine(vec![off, empty]);
        assert!(e.update(&snap(btn::CROSS | btn::CIRCLE), t0).is_empty());
        assert_eq!(e.consumed(), 0);
    }

    // --- layers --------------------------------------------------------------------------------------------------

    fn layer_hold_rule() -> Rule {
        rule(
            Input::L2,
            Gesture::While,
            vec![app(AppAction::Layer { name: "viser".into(), mode: LayerMode::Hold })],
        )
    }

    #[test]
    fn layer_rules_only_exist_while_the_layer_is_held() {
        let mut in_layer = rule(Input::Cross, Gesture::Press, vec![tap("F")]);
        in_layer.layer = Some("viser".into());
        let (mut e, t0) = engine(vec![layer_hold_rule(), in_layer]);

        assert!(downs(&e.update(&snap(btn::CROSS), t0)).is_empty(), "couche inactive");
        e.update(&snap(0), t0 + MS(10));
        e.update(&snap(btn::L2), t0 + MS(20));
        assert_eq!(e.active_layers(), ["viser"]);
        assert_eq!(downs(&e.update(&snap(btn::L2 | btn::CROSS), t0 + MS(40))), [key("F")]);
        e.update(&snap(0), t0 + MS(100));
        assert!(e.active_layers().is_empty(), "L2 released: layer left");
    }

    #[test]
    fn a_layer_rule_replaces_the_base_rule_on_the_same_button() {
        let base = rule(Input::Cross, Gesture::Press, vec![tap("A")]);
        let mut layered = rule(Input::Cross, Gesture::Press, vec![tap("B")]);
        layered.layer = Some("viser".into());
        let (mut e, t0) = engine(vec![layer_hold_rule(), base, layered]);

        assert_eq!(downs(&e.update(&snap(btn::CROSS), t0)), [key("A")], "couche de base");
        e.update(&snap(0), t0 + MS(100));
        e.update(&snap(btn::L2), t0 + MS(120));
        assert_eq!(downs(&e.update(&snap(btn::L2 | btn::CROSS), t0 + MS(140))), [key("B")], "la couche remplace");
    }

    #[test]
    fn base_rules_on_other_buttons_still_work_in_a_layer() {
        let base = rule(Input::Circle, Gesture::Press, vec![tap("A")]);
        let mut layered = rule(Input::Cross, Gesture::Press, vec![tap("B")]);
        layered.layer = Some("viser".into());
        let (mut e, t0) = engine(vec![layer_hold_rule(), base, layered]);
        e.update(&snap(btn::L2), t0);
        assert_eq!(downs(&e.update(&snap(btn::L2 | btn::CIRCLE), t0 + MS(20))), [key("A")]);
    }

    #[test]
    fn leaving_a_layer_releases_what_its_rules_were_holding() {
        let mut held = rule(Input::Cross, Gesture::While, vec![hold("F")]);
        held.layer = Some("viser".into());
        let (mut e, t0) = engine(vec![layer_hold_rule(), held]);
        e.update(&snap(btn::L2), t0);
        assert_eq!(downs(&e.update(&snap(btn::L2 | btn::CROSS), t0 + MS(20))), [key("F")]);
        // release L2 while keeping Cross: the layer's rule disappears, and so does its key
        assert_eq!(ups(&e.update(&snap(btn::CROSS), t0 + MS(40))), [key("F")]);
    }

    #[test]
    fn a_rule_entering_force_does_not_see_an_already_held_button_as_a_press() {
        let mut layered = rule(Input::Cross, Gesture::Press, vec![tap("B")]);
        layered.layer = Some("viser".into());
        let (mut e, t0) = engine(vec![layer_hold_rule(), layered]);
        e.update(&snap(btn::CROSS), t0); // Cross held before the layer
        e.update(&snap(btn::CROSS | btn::L2), t0 + MS(20));
        let fx = e.update(&snap(btn::CROSS | btn::L2), t0 + MS(40));
        assert!(downs(&fx).is_empty(), "Cross was already held when the layer came into force");
    }

    #[test]
    fn toggle_layer_flips_on_each_trigger() {
        let toggle = rule(
            Input::Share,
            Gesture::Press,
            vec![app(AppAction::Layer { name: "x".into(), mode: LayerMode::Toggle })],
        );
        let (mut e, t0) = engine(vec![toggle]);
        e.update(&snap(btn::SHARE), t0);
        assert_eq!(e.active_layers(), ["x"]);
        e.update(&snap(0), t0 + MS(50));
        e.update(&snap(btn::SHARE), t0 + MS(100));
        assert!(e.active_layers().is_empty());
    }

    #[test]
    fn layer_on_and_off_are_explicit() {
        let on = rule(Input::Share, Gesture::Press, vec![app(AppAction::Layer { name: "x".into(), mode: LayerMode::On })]);
        let off = rule(Input::Options, Gesture::Press, vec![app(AppAction::Layer { name: "x".into(), mode: LayerMode::Off })]);
        let (mut e, t0) = engine(vec![on, off]);
        e.update(&snap(btn::SHARE), t0);
        e.update(&snap(0), t0 + MS(20));
        e.update(&snap(btn::SHARE), t0 + MS(40));
        assert_eq!(e.active_layers(), ["x"], "« activer » est idempotent");
        e.update(&snap(btn::OPTIONS), t0 + MS(60));
        assert!(e.active_layers().is_empty());
    }

    // --- safety: nothing stays stuck -------------------------------------------------------------------------------

    #[test]
    fn release_all_lets_go_of_everything() {
        let (mut e, t0) = engine(vec![rule(Input::Square, Gesture::While, vec![hold("E"), Step::Hold { output: pad(XTarget::A) }])]);
        e.update(&snap(btn::SQUARE), t0);
        let fx = e.release_all();
        assert_eq!(ups(&fx).len(), 2);
        assert_eq!(e.held_pad(), PadHeld::default());
        assert!(e.update(&snap(0), t0 + MS(10)).is_empty());
    }

    #[test]
    fn changing_the_config_releases_held_outputs_and_ignores_buttons_already_down() {
        let (mut e, t0) = engine(vec![rule(Input::Square, Gesture::While, vec![hold("E")])]);
        e.update(&snap(btn::SQUARE), t0);
        let fx = e.set_config(vec![rule(Input::Square, Gesture::Press, vec![tap("Q")])], vec![]);
        assert_eq!(ups(&fx), [key("E")]);
        assert!(e.update(&snap(btn::SQUARE), t0 + MS(20)).is_empty(), "Square was already held: no phantom trigger");
        e.update(&snap(0), t0 + MS(40));
        assert_eq!(downs(&e.update(&snap(btn::SQUARE), t0 + MS(60))), [key("Q")]);
    }

    // --- sticks as inputs --------------------------------------------------------------------------------------------

    #[test]
    fn a_stick_direction_can_trigger_a_rule() {
        let (mut e, t0) = engine(vec![rule(Input::LeftStickUp, Gesture::While, vec![hold("W")])]);
        let up = Snapshot { ly: 0, ..snap(0) };
        assert_eq!(downs(&e.update(&up, t0)), [key("W")]);
        assert_eq!(ups(&e.update(&snap(0), t0 + MS(20))), [key("W")]);
    }

    #[test]
    fn stick_directions_have_hysteresis() {
        let (mut e, t0) = engine(vec![rule(Input::RightStickRight, Gesture::While, vec![hold("D")])]);
        let at = |rx: u8| Snapshot { rx, ..snap(0) };
        assert!(downs(&e.update(&at(190), t0)).is_empty(), "0.49: below the entry threshold");
        assert_eq!(downs(&e.update(&at(205), t0 + MS(10))).len(), 1, "0.61: pressed");
        assert!(ups(&e.update(&at(185), t0 + MS(20))).is_empty(), "0.45: still pressed (hysteresis)");
        assert_eq!(ups(&e.update(&at(170), t0 + MS(30))).len(), 1, "0.33: released");
    }

    // --- analog bindings -----------------------------------------------------------------------------------------------

    fn analog(mode: AnalogMode) -> AnalogBinding {
        AnalogBinding { mode, stick: StickId::Right, ..AnalogBinding::default() }
    }

    #[test]
    fn a_stick_inside_the_deadzone_does_nothing() {
        let mut e = Engine::new(vec![], vec![analog(AnalogMode::Mouse { speed: 50 })]);
        let t0 = Instant::now();
        let near_center = Snapshot { rx: 135, ..snap(0) };
        e.update(&near_center, t0);
        assert!(e.update(&near_center, t0 + MS(16)).is_empty());
    }

    #[test]
    fn a_stick_moves_the_mouse_in_its_direction() {
        let mut e = Engine::new(vec![], vec![analog(AnalogMode::Mouse { speed: 50 })]);
        let t0 = Instant::now();
        let right = Snapshot { rx: 255, ry: 128, ..snap(0) };
        e.update(&right, t0);
        let mut dx = 0;
        let mut dy = 0;
        for step in 1..=30u64 {
            for fx in e.update(&right, t0 + MS(step * 16)) {
                if let Effect::MouseMove(x, y) = fx {
                    dx += x;
                    dy += y;
                }
            }
        }
        assert!(dx > 100, "it moved to the right : {dx}");
        assert!(dy.abs() <= 2, "no vertical drift : {dy}");
    }

    #[test]
    fn mouse_speed_scales_the_movement() {
        let travel = |speed| {
            let mut e = Engine::new(vec![], vec![analog(AnalogMode::Mouse { speed })]);
            let t0 = Instant::now();
            let s = Snapshot { rx: 255, ..snap(0) };
            e.update(&s, t0);
            (1..=30u64)
                .flat_map(|n| e.update(&s, t0 + MS(n * 16)))
                .filter_map(|fx| if let Effect::MouseMove(x, _) = fx { Some(x) } else { None })
                .sum::<i32>()
        };
        assert!(travel(80) > travel(20) * 2);
    }

    #[test]
    fn a_stick_pushed_up_scrolls_up() {
        let mut e = Engine::new(vec![], vec![analog(AnalogMode::Scroll { speed: 50 })]);
        let t0 = Instant::now();
        let up = Snapshot { ry: 0, ..snap(0) };
        e.update(&up, t0);
        let total: i32 = (1..=20u64)
            .flat_map(|n| e.update(&up, t0 + MS(n * 16)))
            .filter_map(|fx| if let Effect::Wheel(w) = fx { Some(w) } else { None })
            .sum();
        assert!(total > 0, "molette positive = vers le haut : {total}");
    }

    #[test]
    fn a_stick_can_press_wasd_keys() {
        let keys = AnalogMode::Keys { up: "W".into(), down: "S".into(), left: "A".into(), right: "D".into() };
        let mut e = Engine::new(vec![], vec![analog(keys)]);
        let t0 = Instant::now();
        let up_left = Snapshot { rx: 20, ry: 20, ..snap(0) };
        let fx = e.update(&up_left, t0);
        let pressed = downs(&fx);
        assert!(pressed.contains(&key("W")) && pressed.contains(&key("A")), "{pressed:?}");
        assert!(!pressed.contains(&key("S")) && !pressed.contains(&key("D")));
        let fx = e.update(&snap(0), t0 + MS(16));
        assert_eq!(ups(&fx).len(), 2);
    }

    #[test]
    fn a_consumed_stick_is_taken_from_the_virtual_pad() {
        let mut e = Engine::new(vec![], vec![analog(AnalogMode::Mouse { speed: 50 })]);
        e.update(&snap(0), Instant::now());
        assert_eq!(taken(&e), (false, true));
        let mut keep = analog(AnalogMode::Mouse { speed: 50 });
        keep.consume = false;
        let mut e = Engine::new(vec![], vec![keep]);
        e.update(&snap(0), Instant::now());
        assert_eq!(taken(&e), (false, false));
    }

    #[test]
    fn analog_bindings_follow_layers() {
        let mut b = analog(AnalogMode::Mouse { speed: 50 });
        b.layer = Some("souris".into());
        let toggle = rule(
            Input::Share,
            Gesture::Press,
            vec![app(AppAction::Layer { name: "souris".into(), mode: LayerMode::Toggle })],
        );
        let mut e = Engine::new(vec![toggle], vec![b]);
        let t0 = Instant::now();
        e.update(&snap(0), t0);
        assert_eq!(taken(&e), (false, false), "couche inactive");
        e.update(&snap(btn::SHARE), t0 + MS(16));
        e.update(&snap(0), t0 + MS(32));
        assert_eq!(taken(&e), (false, true));
    }

    #[test]
    fn leaving_a_layer_releases_the_keys_a_stick_was_holding() {
        let mut b = analog(AnalogMode::Keys { up: "W".into(), down: "S".into(), left: "A".into(), right: "D".into() });
        b.layer = Some("x".into());
        let on = rule(Input::Share, Gesture::Press, vec![app(AppAction::Layer { name: "x".into(), mode: LayerMode::On })]);
        let off = rule(Input::Options, Gesture::Press, vec![app(AppAction::Layer { name: "x".into(), mode: LayerMode::Off })]);
        let mut e = Engine::new(vec![on, off], vec![b]);
        let t0 = Instant::now();
        e.update(&snap(btn::SHARE), t0);
        let up = Snapshot { rx: 128, ry: 0, ..snap(0) };
        assert_eq!(downs(&e.update(&up, t0 + MS(16))), [key("W")]);
        let fx = e.update(&Snapshot { buttons: btn::OPTIONS, ..up }, t0 + MS(32));
        let fx2 = e.update(&Snapshot { buttons: btn::OPTIONS, ..up }, t0 + MS(48));
        assert!(ups(&fx).contains(&key("W")) || ups(&fx2).contains(&key("W")));
    }

    // --- migration, validation, serialization --------------------------------------------------------------------------

    #[test]
    fn the_default_shortcuts_become_equivalent_rules() {
        let rules = rules_from_shortcuts(&shortcuts::default_shortcuts());
        assert_eq!(rules.len(), 4);
        assert_eq!(rules[0].gesture, Gesture::Tap, "Mute shares its button with a hold");
        assert_eq!(rules[1].gesture, Gesture::Hold);
        assert_eq!(rules[2].gesture, Gesture::Tap, "PS shares its button with a hold too");
        assert_eq!(rules[2].steps, [app(AppAction::ShowOverlay)]);
        assert_eq!(rules[3], menu_rule(), "holding PS opens the menu: it is a rule like the others");
    }

    #[test]
    fn rules_saved_before_the_menu_get_the_menu_rule_once() {
        let saved = rules_from_shortcuts(&[
            shortcuts::Shortcut { button: shortcuts::Button::Ps, gesture: shortcuts::Gesture::Press, action: shortcuts::Action::ShowOverlay },
            shortcuts::Shortcut { button: shortcuts::Button::Mute, gesture: shortcuts::Gesture::Press, action: shortcuts::Action::ToggleVirtualPad },
        ]);
        assert_eq!(saved[0].gesture, Gesture::Press);

        let updated = with_menu_rule(saved.clone());
        assert_eq!(updated.len(), 3);
        assert_eq!(updated[0].gesture, Gesture::Tap, "the overlay no longer flashes before the menu");
        assert_eq!(updated[2], menu_rule());
        assert_eq!(updated[1], saved[1], "other rules are untouched");

        assert_eq!(with_menu_rule(updated.clone()), updated, "a rule that opens the menu is enough: nothing more is added");
        let own = vec![rule(Input::Share, Gesture::Hold, vec![app(AppAction::OpenMenu)])];
        assert_eq!(with_menu_rule(own.clone()), own, "a menu rule of the user's own is respected");
    }

    #[test]
    fn the_overlay_rule_is_only_made_a_short_press_when_it_is_the_default_one() {
        let custom = rule(Input::Ps, Gesture::Press, vec![tap("A")]);
        let updated = with_menu_rule(vec![custom.clone()]);
        assert_eq!(updated[0], custom);
        let mut layered = rule(Input::Ps, Gesture::Press, vec![app(AppAction::ShowOverlay)]);
        layered.layer = Some("Shift".into());
        assert_eq!(with_menu_rule(vec![layered.clone()])[0], layered);
    }

    #[test]
    fn migrated_rules_behave_like_the_old_shortcuts() {
        let (mut e, t0) = engine(rules_from_shortcuts(&shortcuts::default_shortcuts()));
        e.update(&snap(btn::MUTE), t0);
        assert_eq!(e.update(&snap(0), t0 + MS(200)), [Effect::App(AppAction::ToggleVirtualPad)]);
        e.update(&snap(btn::MUTE), t0 + MS(1000));
        assert_eq!(e.update(&snap(btn::MUTE), t0 + MS(1800)), [Effect::App(AppAction::PowerOff)]);
        // PS: a short press shows the overlay, a long one opens the menu.
        e.update(&snap(btn::PS), t0 + MS(3000));
        assert_eq!(e.update(&snap(0), t0 + MS(3100)), [Effect::App(AppAction::ShowOverlay)]);
        e.update(&snap(btn::PS), t0 + MS(4000));
        assert_eq!(e.update(&snap(btn::PS), t0 + MS(4800)), [Effect::App(AppAction::OpenMenu)]);
        assert_ne!(e.consumed() & btn::PS, 0, "PS is taken by the rule while it is held");
    }

    #[test]
    fn sanitizing_enforces_limits_and_drops_unknown_keys() {
        let many = vec![rule(Input::Cross, Gesture::Press, vec![tap("A")]); MAX_RULES + 10];
        assert_eq!(sanitize_rules(many).len(), MAX_RULES);

        let bad = Rule {
            inputs: vec![Input::Cross, Input::Cross, Input::Circle, Input::Square, Input::Triangle, Input::L1],
            steps: vec![tap("A"), tap("Hyper"), Step::Wait { ms: 999_999 }],
            repeat: Repeat::Times { count: 10_000_000, interval_ms: 0 },
            ms: Some(1),
            layer: Some("  ".into()),
            ..Rule::default()
        };
        let clean = sanitize_rules(vec![bad]).remove(0);
        assert_eq!(clean.inputs.len(), MAX_INPUTS, "duplicates removed, then the limit");
        assert_eq!(clean.steps, [tap("A"), Step::Wait { ms: MAX_WAIT_MS }]);
        assert_eq!(clean.repeat, Repeat::Times { count: MAX_REPEAT_TIMES, interval_ms: MIN_INTERVAL_MS });
        assert_eq!(clean.ms, Some(50));
        assert_eq!(clean.layer, None, "an empty layer name is not a layer");
    }

    #[test]
    fn sanitizing_analog_bindings_checks_keys_and_speed() {
        let bad_keys = analog(AnalogMode::Keys { up: "W".into(), down: "Hyper".into(), left: "A".into(), right: "D".into() });
        let fast = analog(AnalogMode::Mouse { speed: 250 });
        let clean = sanitize_analog(vec![bad_keys, fast]);
        assert_eq!(clean.len(), 1);
        assert_eq!(clean[0].mode, AnalogMode::Mouse { speed: 100 });
    }

    #[test]
    fn rules_round_trip_through_json() {
        let r = Rule {
            layer: Some("viser".into()),
            inputs: vec![Input::L1, Input::LeftStickUp],
            gesture: Gesture::DoubleTap,
            steps: vec![tap("A"), Step::Wait { ms: 50 }, Step::Hold { output: pad(XTarget::Lt) }, app(AppAction::Message { text: "ok".into() })],
            repeat: Repeat::UntilReleased { interval_ms: 80 },
            toggle: true,
            ..Rule::default()
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains(r#""gesture":"double_tap""#));
        assert!(json.contains(r#""type":"until_released""#));
        assert!(json.contains(r#""kind":"key""#));
        let back: Rule = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn a_minimal_rule_gets_sensible_defaults() {
        let r: Rule = serde_json::from_str(r#"{"inputs":["cross"],"steps":[{"type":"tap","output":{"kind":"key","key":"E"}}]}"#).unwrap();
        assert!(r.enabled);
        assert_eq!(r.gesture, Gesture::Press);
        assert_eq!(r.repeat, Repeat::None);
        assert!(!r.toggle && !r.passthrough);
    }

    #[test]
    fn inputs_are_in_button_bit_order() {
        for (i, button) in Button::ALL.iter().enumerate() {
            assert_eq!(Input::from(*button).mask(), 1 << i, "{button:?}");
            assert_eq!(button.bit(), 1 << i);
        }
        assert_eq!(Input::LeftStickUp as u32, BUTTON_COUNT);
    }
}
