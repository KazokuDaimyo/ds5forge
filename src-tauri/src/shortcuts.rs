//! The DualSense buttons, and the legacy shortcuts (a button + a gesture = an action).
//!
//! Shortcuts are no longer evaluated here: `bindings.rs` replaced them with rules and only uses
//! them to convert profiles saved before rules existed (`rules_from_shortcuts`).

use crate::controller::btn;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// How long a button must be held to trigger a "hold" shortcut.
pub const HOLD_DURATION: Duration = Duration::from_millis(800);

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Button {
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
}

impl Button {
    /// Every button, in the bit order of `ControllerState::buttons`.
    pub const ALL: [Button; 19] = [
        Button::Square, Button::Cross, Button::Circle, Button::Triangle, Button::L1,
        Button::R1, Button::L2, Button::R2, Button::Share, Button::Options, Button::L3,
        Button::R3, Button::Ps, Button::Touchpad, Button::Mute, Button::DpadUp,
        Button::DpadRight, Button::DpadDown, Button::DpadLeft,
    ];

    /// The matching bit in `ControllerState::buttons`.
    pub fn bit(self) -> u32 {
        match self {
            Button::Square => btn::SQUARE,
            Button::Cross => btn::CROSS,
            Button::Circle => btn::CIRCLE,
            Button::Triangle => btn::TRIANGLE,
            Button::L1 => btn::L1,
            Button::R1 => btn::R1,
            Button::L2 => btn::L2,
            Button::R2 => btn::R2,
            Button::Share => btn::SHARE,
            Button::Options => btn::OPTIONS,
            Button::L3 => btn::L3,
            Button::R3 => btn::R3,
            Button::Ps => btn::PS,
            Button::Touchpad => btn::TOUCHPAD,
            Button::Mute => btn::MUTE,
            Button::DpadUp => btn::DPAD_UP,
            Button::DpadRight => btn::DPAD_RIGHT,
            Button::DpadDown => btn::DPAD_DOWN,
            Button::DpadLeft => btn::DPAD_LEFT,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Gesture {
    /// A short press.
    Press,
    /// A press held for at least `HOLD_DURATION`.
    Hold,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// Switches between native mode (the DualSense seen as is) and Xbox 360 mode.
    ToggleVirtualPad,
    /// Powers the controller off (Bluetooth only).
    PowerOff,
    /// Shows the overlay (controller batteries…).
    ShowOverlay,
    /// Enables or disables the gyroscope and the touchpad (until the next restart).
    ToggleMotion,
    /// Opens the controller menu.
    OpenMenu,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shortcut {
    pub button: Button,
    pub gesture: Gesture,
    pub action: Action,
}

/// The shortcuts given to a new controller.
pub fn default_shortcuts() -> Vec<Shortcut> {
    vec![
        Shortcut { button: Button::Mute, gesture: Gesture::Press, action: Action::ToggleVirtualPad },
        Shortcut { button: Button::Mute, gesture: Gesture::Hold, action: Action::PowerOff },
        Shortcut { button: Button::Ps, gesture: Gesture::Press, action: Action::ShowOverlay },
        Shortcut { button: Button::Ps, gesture: Gesture::Hold, action: Action::OpenMenu },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_buttons_are_listed_in_bit_order() {
        for (i, button) in Button::ALL.iter().enumerate() {
            assert_eq!(button.bit(), 1 << i, "{button:?}");
        }
    }

    #[test]
    fn shortcuts_round_trip_through_json() {
        let json = serde_json::to_string(&default_shortcuts()).unwrap();
        assert!(json.contains(r#""button":"mute""#));
        assert!(json.contains(r#""action":"toggle_virtual_pad""#));
        let back: Vec<Shortcut> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, default_shortcuts());
    }

    #[test]
    fn the_defaults_keep_their_mute_and_ps_bindings() {
        let defaults = default_shortcuts();
        assert_eq!(defaults.len(), 4);
        assert!(defaults.iter().any(|s| s.button == Button::Ps && s.gesture == Gesture::Hold && s.action == Action::OpenMenu));
        assert!(defaults.iter().any(|s| s.button == Button::Ps && s.action == Action::ShowOverlay));
    }
}
