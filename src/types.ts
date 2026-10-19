// Types shared by the interface. They mirror the structures of the backend (src-tauri/src/).

import { t } from "./i18n";

import type { Charge } from "./battery";

// --- Triggers (triggers.rs) ---------------------------------------------------------------

export type Kind = "off" | "feedback" | "weapon" | "vibration";
export type Side = "left" | "right";
export type TriggerEffect = { kind: Kind } & Record<string, number | string>;

// --- Rules (bindings.rs) ------------------------------------------------------------------------

/** An input that can trigger a rule: a button (BUTTON_IDS) or a stick direction. */
export type RuleInput = string;

export type RuleGesture = "press" | "release" | "tap" | "hold" | "double_tap" | "while";
export type MouseBtn = "left" | "right" | "middle" | "back" | "forward";

/** What a step presses: a virtual Xbox button, a keyboard key, a mouse button. */
export type Output =
  | { kind: "pad"; target: XTarget }
  | { kind: "key"; key: string }
  | { kind: "mouse"; button: MouseBtn };

export type LayerMode = "hold" | "toggle" | "on" | "off";

export type AppAction =
  | { type: "toggle_virtual_pad" }
  | { type: "power_off" }
  | { type: "show_overlay" }
  | { type: "toggle_motion" }
  | { type: "message"; text: string }
  | { type: "lightbar"; r: number; g: number; b: number }
  | { type: "rumble"; strength: number; ms: number }
  | { type: "layer"; name: string; mode: LayerMode };

export type Step =
  | { type: "hold" | "tap" | "down" | "up"; output: Output }
  | { type: "wait"; ms: number }
  | { type: "wheel"; amount: number }
  | { type: "mouse_move"; dx: number; dy: number }
  | { type: "app"; action: AppAction };

export type Repeat =
  | { type: "none" }
  | { type: "until_released"; interval_ms: number }
  | { type: "times"; count: number; interval_ms: number };

export interface Rule {
  enabled: boolean;
  /** Layer the rule exists in; `null`: always. */
  layer: string | null;
  inputs: RuleInput[];
  gesture: RuleGesture;
  /** Duration of the gesture (hold, short press, double press); `null`: default value. */
  ms: number | null;
  steps: Step[];
  repeat: Repeat;
  toggle: boolean;
  passthrough: boolean;
}

export type StickId = "left" | "right";

export type AnalogMode =
  | { type: "mouse"; speed: number }
  | { type: "scroll"; speed: number }
  | { type: "keys"; up: string; down: string; left: string; right: string };

export interface AnalogBinding {
  enabled: boolean;
  layer: string | null;
  stick: StickId;
  mode: AnalogMode;
  deadzone: number;
  consume: boolean;
}

/** A keyboard key offered by the backend (keys.rs). */
export interface KeyInfo {
  id: string;
  label: string;
}

// --- Lightbar (lightbar.rs) ----------------------------------------------------------------------

export type Rgb = [number, number, number];
export type LightbarMode = "fixed" | "battery" | "mode";

export interface ModeColors {
  xbox: Rgb;
  native: Rgb;
}

// --- Sticks and buttons (input.rs) ----------------------------------------------------------------

export type XTarget =
  | "a" | "b" | "x" | "y" | "lb" | "rb" | "back" | "start" | "lthumb" | "rthumb" | "guide"
  | "dpad_up" | "dpad_down" | "dpad_left" | "dpad_right" | "lt" | "rt" | "none";

export interface Remap {
  from: string;
  to: XTarget;
}

export type Curve = "linear" | "precise" | "aggressive";

export interface StickConfig {
  deadzone: number;
  outer: number;
  curve: Curve;
}

export interface InputConfig {
  remap: Remap[];
  left_stick: StickConfig;
  right_stick: StickConfig;
}

// --- Gyroscope and touchpad (motion.rs) ----------------------------------------------------------

export type GyroMode = "off" | "right_stick" | "mouse";
export type TouchpadMode = "off" | "mouse";

export interface MotionConfig {
  gyro: GyroMode;
  gyro_sensitivity: number;
  gyro_invert_x: boolean;
  gyro_invert_y: boolean;
  /** Button to hold to enable the gyroscope; `null`: always active. */
  gyro_hold: string | null;
  touchpad: TouchpadMode;
  touchpad_sensitivity: number;
}

// --- Profile and state (manager.rs, controller.rs) ----------------------------------------------------------

export interface Profile {
  name: string;
  lightbar: Rgb;
  player_leds: number;
  trigger_left: TriggerEffect | null;
  trigger_right: TriggerEffect | null;
  virtual_pad: boolean;
  /** Always filled on the interface side: the backend derives the rules from the legacy shortcuts if needed. */
  rules: Rule[] | null;
  analog: AnalogBinding[];
  lightbar_mode: LightbarMode;
  mode_colors: ModeColors;
  input: InputConfig;
  motion: MotionConfig;
}

export interface ControllerInfo {
  id: string;
  connected: boolean;
  bluetooth: boolean;
  /** The controller's name and the settings of the profile in force. */
  profile: Profile;
  /** The controller's default profile. */
  preset: string;
  /** The game profile chosen by hand on this controller, if any. */
  manual: string | null;
  /** The profile in force: the game in the foreground's, else the one chosen by hand, else `preset`. */
  effective_preset: string;
}

/** "While this program is in the foreground, use this profile" (src-tauri/src/games.rs). */
export interface GameRule {
  exe: string;
  preset: string;
}

export interface PresetsInfo {
  presets: string[];
  games: GameRule[];
}

export interface StateEvent {
  id: string;
  lx: number;
  ly: number;
  rx: number;
  ry: number;
  l2: number;
  r2: number;
  buttons: number;
  battery: number | null;
  charge: Charge;
}

// --- Buttons -----------------------------------------------------------------------------------------

// The order must match the bits of `btn` in src-tauri/src/controller.rs.
export const BUTTON_NAMES = [
  t("button.square"), t("button.cross"), t("button.circle"), t("button.triangle"), "L1", "R1", "L2", "R2",
  "Share", "Options", "L3", "R3", "PS", "Touchpad", "Mute",
  "↑", "→", "↓", "←",
];

// Button identifiers on the Rust side (enum Button), in the same order as BUTTON_NAMES.
export const BUTTON_IDS = [
  "square", "cross", "circle", "triangle", "l1", "r1", "l2", "r2",
  "share", "options", "l3", "r3", "ps", "touchpad", "mute",
  "dpad_up", "dpad_right", "dpad_down", "dpad_left",
];
