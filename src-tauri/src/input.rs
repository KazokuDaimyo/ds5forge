//! Translation of the DualSense state into an Xbox 360 controller state: button remapping, stick
//! dead zones and sensitivity curves.
//!
//! Everything is pure and testable; `virtual_pad.rs` only passes the result to the driver.

use crate::bindings::PadHeld;
use crate::controller::ControllerState;
use crate::shortcuts::Button;
use serde::{Deserialize, Serialize};

/// XInput buttons (values imposed by Windows).
pub mod xbtn {
    pub const UP: u16 = 0x0001;
    pub const DOWN: u16 = 0x0002;
    pub const LEFT: u16 = 0x0004;
    pub const RIGHT: u16 = 0x0008;
    pub const START: u16 = 0x0010;
    pub const BACK: u16 = 0x0020;
    pub const LTHUMB: u16 = 0x0040;
    pub const RTHUMB: u16 = 0x0080;
    pub const LB: u16 = 0x0100;
    pub const RB: u16 = 0x0200;
    pub const GUIDE: u16 = 0x0400;
    pub const A: u16 = 0x1000;
    pub const B: u16 = 0x2000;
    pub const X: u16 = 0x4000;
    pub const Y: u16 = 0x8000;
}

/// What a DualSense button triggers on the Xbox controller.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum XTarget {
    A,
    B,
    X,
    Y,
    Lb,
    Rb,
    Back,
    Start,
    Lthumb,
    Rthumb,
    Guide,
    DpadUp,
    DpadDown,
    DpadLeft,
    DpadRight,
    /// Left trigger (fully pressed for a button, an analogue value for L2/R2).
    Lt,
    Rt,
    /// The button does nothing.
    None,
}

impl XTarget {
    /// The XInput bit, for the targets that are real buttons.
    pub fn bit(self) -> Option<u16> {
        Some(match self {
            XTarget::A => xbtn::A,
            XTarget::B => xbtn::B,
            XTarget::X => xbtn::X,
            XTarget::Y => xbtn::Y,
            XTarget::Lb => xbtn::LB,
            XTarget::Rb => xbtn::RB,
            XTarget::Back => xbtn::BACK,
            XTarget::Start => xbtn::START,
            XTarget::Lthumb => xbtn::LTHUMB,
            XTarget::Rthumb => xbtn::RTHUMB,
            XTarget::Guide => xbtn::GUIDE,
            XTarget::DpadUp => xbtn::UP,
            XTarget::DpadDown => xbtn::DOWN,
            XTarget::DpadLeft => xbtn::LEFT,
            XTarget::DpadRight => xbtn::RIGHT,
            XTarget::Lt | XTarget::Rt | XTarget::None => return None,
        })
    }
}

/// The default mapping, that of an Xbox controller.
pub fn default_target(button: Button) -> XTarget {
    match button {
        Button::Cross => XTarget::A,
        Button::Circle => XTarget::B,
        Button::Square => XTarget::X,
        Button::Triangle => XTarget::Y,
        Button::L1 => XTarget::Lb,
        Button::R1 => XTarget::Rb,
        Button::L2 => XTarget::Lt,
        Button::R2 => XTarget::Rt,
        Button::Share => XTarget::Back,
        Button::Options => XTarget::Start,
        Button::L3 => XTarget::Lthumb,
        Button::R3 => XTarget::Rthumb,
        Button::Ps => XTarget::Guide,
        Button::DpadUp => XTarget::DpadUp,
        Button::DpadDown => XTarget::DpadDown,
        Button::DpadLeft => XTarget::DpadLeft,
        Button::DpadRight => XTarget::DpadRight,
        // No Xbox equivalent.
        Button::Touchpad | Button::Mute => XTarget::None,
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Remap {
    pub from: Button,
    pub to: XTarget,
}

/// Sensitivity curve of a stick, applied after the dead zone.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Curve {
    /// Proportional response.
    #[default]
    Linear,
    /// Finer around the centre, for gentle aiming.
    Precise,
    /// More responsive around the centre: reacts quickly to small movements.
    Aggressive,
}

/// Settings of a stick, as a percentage of its travel.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(default)]
pub struct StickConfig {
    /// Dead zone around the centre: below it, the stick is considered motionless.
    pub deadzone: u8,
    /// Position from which the stick is considered fully deflected.
    pub outer: u8,
    pub curve: Curve,
}

impl Default for StickConfig {
    /// The neutral setting: nothing is modified.
    fn default() -> Self {
        Self { deadzone: 0, outer: 100, curve: Curve::Linear }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct InputConfig {
    /// Only the remapped buttons; the others keep their default mapping.
    pub remap: Vec<Remap>,
    pub left_stick: StickConfig,
    pub right_stick: StickConfig,
}

impl InputConfig {
    /// The target of a button: the last remap that concerns it, otherwise the default.
    pub fn target_of(&self, button: Button) -> XTarget {
        self.remap
            .iter()
            .rev()
            .find(|r| r.from == button)
            .map_or_else(|| default_target(button), |r| r.to)
    }
}

/// The Xbox controller state to send to the driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Mapped {
    pub buttons: u16,
    pub left_trigger: u8,
    pub right_trigger: u8,
    pub left_x: i16,
    pub left_y: i16,
    pub right_x: i16,
    pub right_y: i16,
}

fn to_i16(v: f32) -> i16 {
    if v < 0.0 {
        (v * 32768.0).round().max(-32768.0) as i16
    } else {
        (v * 32767.0).round().min(32767.0) as i16
    }
}

/// Applies dead zone, outer limit and curve, then adds `extra` (gyroscope).
/// Input: 0..=255 per axis (up = 0). Output: XInput values (up = positive).
fn stick(x: u8, y: u8, cfg: &StickConfig, extra: (f32, f32)) -> (i16, i16) {
    let nx = (f32::from(x) - 127.5) / 127.5;
    let ny = -(f32::from(y) - 127.5) / 127.5;

    let identity = cfg.deadzone == 0 && cfg.outer >= 100 && cfg.curve == Curve::Linear;
    let (mut sx, mut sy) = if identity {
        (nx, ny)
    } else {
        let radius = nx.hypot(ny);
        let dead = f32::from(cfg.deadzone.min(60)) / 100.0;
        // The outer limit must stay above the dead zone.
        let outer = (f32::from(cfg.outer.clamp(1, 100)) / 100.0).max(dead + 0.05);

        let scaled = if radius <= dead { 0.0 } else { ((radius - dead) / (outer - dead)).min(1.0) };
        let shaped = match cfg.curve {
            Curve::Linear => scaled,
            Curve::Precise => scaled * scaled,
            Curve::Aggressive => scaled.sqrt(),
        };
        if radius > 0.0 { (nx / radius * shaped, ny / radius * shaped) } else { (0.0, 0.0) }
    };

    sx = (sx + extra.0).clamp(-1.0, 1.0);
    sy = (sy + extra.1).clamp(-1.0, 1.0);
    (to_i16(sx), to_i16(sy))
}

/// Translates the DualSense state. `consumed`: buttons taken by a rule, which are not transmitted.
/// `extra_right`: movement added to the right stick (gyroscope), from -1 to 1.
#[cfg(test)]
pub fn map(state: &ControllerState, consumed: u32, cfg: &InputConfig, extra_right: (f32, f32)) -> Mapped {
    map_with(state, consumed, cfg, extra_right, &PadHeld::default())
}

/// Like `map`, and additionally takes what rules hold on the Xbox controller (`held`) and the
/// sticks they have taken over (which are not transmitted).
pub fn map_with(
    state: &ControllerState,
    consumed: u32,
    cfg: &InputConfig,
    extra_right: (f32, f32),
    held: &PadHeld,
) -> Mapped {
    let mut out = Mapped::default();

    for &source in &Button::ALL {
        if consumed & source.bit() != 0 {
            continue;
        }
        let pressed = state.buttons & source.bit() != 0;
        // Analogue triggers keep their value; every other button acts "fully".
        let analog = match source {
            Button::L2 => Some(state.l2),
            Button::R2 => Some(state.r2),
            _ => None,
        };
        let value = analog.unwrap_or(if pressed { 255 } else { 0 });

        match cfg.target_of(source) {
            XTarget::None => {}
            XTarget::Lt => out.left_trigger = out.left_trigger.max(value),
            XTarget::Rt => out.right_trigger = out.right_trigger.max(value),
            target => {
                if pressed {
                    out.buttons |= target.bit().unwrap_or(0);
                }
            }
        }
    }

    if !held.left_stick_taken {
        (out.left_x, out.left_y) = stick(state.lx, state.ly, &cfg.left_stick, (0.0, 0.0));
    }
    if !held.right_stick_taken {
        (out.right_x, out.right_y) = stick(state.rx, state.ry, &cfg.right_stick, extra_right);
    }

    // What the rules hold is added to the physical controller.
    out.buttons |= held.buttons;
    if held.left_trigger {
        out.left_trigger = 255;
    }
    if held.right_trigger {
        out.right_trigger = 255;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::btn;

    fn state(buttons: u32) -> ControllerState {
        ControllerState { connected: true, lx: 128, ly: 128, rx: 128, ry: 128, buttons, ..Default::default() }
    }

    fn mapped(buttons: u32) -> Mapped {
        map(&state(buttons), 0, &InputConfig::default(), (0.0, 0.0))
    }

    // --- buttons -------------------------------------------------------------------------

    #[test]
    fn face_buttons_follow_the_xbox_layout() {
        assert_eq!(mapped(btn::CROSS | btn::TRIANGLE).buttons, xbtn::A | xbtn::Y);
        assert_eq!(mapped(btn::CIRCLE | btn::SQUARE).buttons, xbtn::B | xbtn::X);
    }

    #[test]
    fn shoulders_menu_and_thumbs_keep_their_default_targets() {
        let all = btn::L1 | btn::R1 | btn::SHARE | btn::OPTIONS | btn::L3 | btn::R3 | btn::PS;
        assert_eq!(
            mapped(all).buttons,
            xbtn::LB | xbtn::RB | xbtn::BACK | xbtn::START | xbtn::LTHUMB | xbtn::RTHUMB | xbtn::GUIDE
        );
    }

    #[test]
    fn dpad_diagonals_press_two_directions() {
        assert_eq!(mapped(btn::DPAD_UP | btn::DPAD_RIGHT).buttons, xbtn::UP | xbtn::RIGHT);
    }

    #[test]
    fn touchpad_and_mute_have_no_xbox_equivalent() {
        assert_eq!(mapped(btn::TOUCHPAD | btn::MUTE).buttons, 0);
    }

    #[test]
    fn analog_triggers_pass_through() {
        let s = ControllerState { l2: 10, r2: 200, ..state(0) };
        let m = map(&s, 0, &InputConfig::default(), (0.0, 0.0));
        assert_eq!((m.left_trigger, m.right_trigger), (10, 200));
    }

    #[test]
    fn a_remapped_button_goes_to_its_new_target_only() {
        let cfg = InputConfig { remap: vec![Remap { from: Button::Cross, to: XTarget::B }], ..Default::default() };
        let m = map(&state(btn::CROSS), 0, &cfg, (0.0, 0.0));
        assert_eq!(m.buttons, xbtn::B);
    }

    #[test]
    fn two_buttons_can_be_swapped() {
        let cfg = InputConfig {
            remap: vec![
                Remap { from: Button::Cross, to: XTarget::B },
                Remap { from: Button::Circle, to: XTarget::A },
            ],
            ..Default::default()
        };
        assert_eq!(map(&state(btn::CROSS), 0, &cfg, (0.0, 0.0)).buttons, xbtn::B);
        assert_eq!(map(&state(btn::CIRCLE), 0, &cfg, (0.0, 0.0)).buttons, xbtn::A);
    }

    #[test]
    fn a_button_can_be_disabled() {
        let cfg = InputConfig { remap: vec![Remap { from: Button::Ps, to: XTarget::None }], ..Default::default() };
        assert_eq!(map(&state(btn::PS), 0, &cfg, (0.0, 0.0)).buttons, 0);
    }

    #[test]
    fn a_touchpad_press_can_be_given_a_target() {
        let cfg = InputConfig { remap: vec![Remap { from: Button::Touchpad, to: XTarget::Back }], ..Default::default() };
        assert_eq!(map(&state(btn::TOUCHPAD), 0, &cfg, (0.0, 0.0)).buttons, xbtn::BACK);
    }

    #[test]
    fn when_a_button_is_remapped_twice_the_last_remap_wins() {
        let cfg = InputConfig {
            remap: vec![
                Remap { from: Button::Cross, to: XTarget::B },
                Remap { from: Button::Cross, to: XTarget::Y },
            ],
            ..Default::default()
        };
        assert_eq!(map(&state(btn::CROSS), 0, &cfg, (0.0, 0.0)).buttons, xbtn::Y);
    }

    #[test]
    fn a_digital_button_mapped_to_a_trigger_presses_it_fully() {
        let cfg = InputConfig { remap: vec![Remap { from: Button::L1, to: XTarget::Lt }], ..Default::default() };
        let m = map(&state(btn::L1), 0, &cfg, (0.0, 0.0));
        assert_eq!((m.left_trigger, m.buttons), (255, 0));
    }

    #[test]
    fn the_analog_triggers_can_be_swapped_keeping_their_values() {
        let cfg = InputConfig {
            remap: vec![
                Remap { from: Button::L2, to: XTarget::Rt },
                Remap { from: Button::R2, to: XTarget::Lt },
            ],
            ..Default::default()
        };
        let s = ControllerState { l2: 40, r2: 180, ..state(0) };
        let m = map(&s, 0, &cfg, (0.0, 0.0));
        assert_eq!((m.left_trigger, m.right_trigger), (180, 40));
    }

    #[test]
    fn an_analog_trigger_remapped_to_a_button_presses_it_with_the_digital_click() {
        let cfg = InputConfig { remap: vec![Remap { from: Button::R2, to: XTarget::A }], ..Default::default() };
        let m = map(&state(btn::R2), 0, &cfg, (0.0, 0.0));
        assert_eq!((m.buttons, m.right_trigger), (xbtn::A, 0));
    }

    #[test]
    fn consumed_buttons_are_not_transmitted() {
        let m = map(&state(btn::PS | btn::CROSS), btn::PS, &InputConfig::default(), (0.0, 0.0));
        assert_eq!(m.buttons, xbtn::A, "PS is taken by a rule");
    }

    #[test]
    fn a_consumed_trigger_reads_zero() {
        let s = ControllerState { l2: 255, ..state(0) };
        let m = map(&s, btn::L2, &InputConfig::default(), (0.0, 0.0));
        assert_eq!(m.left_trigger, 0);
    }

    // --- sticks --------------------------------------------------------------------------

    #[test]
    fn default_sticks_reach_the_full_range_and_up_is_positive() {
        let cfg = StickConfig::default();
        assert_eq!(stick(0, 128, &cfg, (0.0, 0.0)).0, i16::MIN);
        assert_eq!(stick(255, 128, &cfg, (0.0, 0.0)).0, i16::MAX);
        assert_eq!(stick(128, 0, &cfg, (0.0, 0.0)).1, i16::MAX, "up = 0 on the DualSense");
        assert_eq!(stick(128, 255, &cfg, (0.0, 0.0)).1, i16::MIN);
    }

    #[test]
    fn default_sticks_are_per_axis_so_diagonals_are_not_clipped() {
        let (x, y) = stick(255, 0, &StickConfig::default(), (0.0, 0.0));
        assert_eq!((x, y), (i16::MAX, i16::MAX));
    }

    #[test]
    fn the_resting_position_is_near_zero() {
        let (x, y) = stick(128, 128, &StickConfig::default(), (0.0, 0.0));
        assert!(x.abs() < 300 && y.abs() < 300, "{x} {y}");
    }

    #[test]
    fn the_deadzone_silences_small_movements() {
        let cfg = StickConfig { deadzone: 15, ..Default::default() };
        assert_eq!(stick(140, 140, &cfg, (0.0, 0.0)), (0, 0)); // about 9 % of the travel
        assert_eq!(stick(128, 128, &cfg, (0.0, 0.0)), (0, 0));
    }

    #[test]
    fn beyond_the_deadzone_the_stick_ramps_up_from_zero_without_a_jump() {
        let cfg = StickConfig { deadzone: 20, ..Default::default() };
        // Just above 20 % of the travel: the value is small, not a jump of 20 %.
        let (x, _) = stick(128 + 26, 128, &cfg, (0.0, 0.0)); // about 20.4 %
        assert!(x > 0 && x < 1500, "{x}");
    }

    #[test]
    fn the_outer_limit_reaches_full_deflection_early() {
        let cfg = StickConfig { outer: 80, ..Default::default() };
        let (x, _) = stick(128 + 102, 128, &cfg, (0.0, 0.0)); // about 80 % of the travel
        assert!(x >= 32000, "{x}");
    }

    #[test]
    fn the_deadzone_keeps_the_direction_of_the_stick() {
        let cfg = StickConfig { deadzone: 10, ..Default::default() };
        let (x, y) = stick(128 + 80, 128 - 80, &cfg, (0.0, 0.0)); // up-right diagonal
        assert!(x > 0 && y > 0);
        // The two input axes are not exactly equal (offsets of 80.5 and 79.5): what must remain
        // identical is their ratio.
        let input_ratio = 80.5 / 79.5;
        let output_ratio = f32::from(x) / f32::from(y);
        assert!((output_ratio - input_ratio).abs() < 0.01, "{output_ratio} vs {input_ratio}");
    }

    #[test]
    fn the_precise_curve_is_gentler_than_linear_and_the_aggressive_one_livelier() {
        let at = |curve| stick(128 + 64, 128, &StickConfig { curve, ..Default::default() }, (0.0, 0.0)).0;
        // Linear takes the "per axis" path; compare against an equivalent linear setting.
        let linear = stick(128 + 64, 128, &StickConfig { deadzone: 1, ..Default::default() }, (0.0, 0.0)).0;
        assert!(at(Curve::Precise) < linear);
        assert!(at(Curve::Aggressive) > linear);
    }

    #[test]
    fn every_curve_still_reaches_full_deflection() {
        for curve in [Curve::Linear, Curve::Precise, Curve::Aggressive] {
            let (x, _) = stick(255, 128, &StickConfig { deadzone: 5, curve, ..Default::default() }, (0.0, 0.0));
            assert!(x >= 32000, "{curve:?}: {x}");
        }
    }

    #[test]
    fn curves_are_monotonic() {
        for curve in [Curve::Linear, Curve::Precise, Curve::Aggressive] {
            let cfg = StickConfig { deadzone: 10, curve, ..Default::default() };
            let mut last = i16::MIN;
            for x in 128..=255u8 {
                let (v, _) = stick(x, 128, &cfg, (0.0, 0.0));
                assert!(v >= last, "{curve:?} goes backwards at x={x}");
                last = v;
            }
        }
    }

    #[test]
    fn absurd_settings_are_tolerated() {
        let cfg = StickConfig { deadzone: 255, outer: 0, curve: Curve::Linear };
        let (x, y) = stick(255, 0, &cfg, (0.0, 0.0));
        assert!(x > 0 && y > 0, "an absurd dead zone must not freeze the stick forever");
    }

    #[test]
    fn the_extra_motion_is_added_to_the_right_stick_and_clamped() {
        let s = state(0);
        let m = map(&s, 0, &InputConfig::default(), (0.5, -0.25));
        assert!((i32::from(m.right_x) - 16384).abs() < 400, "{}", m.right_x);
        assert!((i32::from(m.right_y) + 8192).abs() < 400, "{}", m.right_y);
        let m = map(&s, 0, &InputConfig::default(), (5.0, -5.0));
        assert_eq!((m.right_x, m.right_y), (i16::MAX, i16::MIN));
    }

    #[test]
    fn the_extra_motion_never_touches_the_left_stick() {
        let m = map(&state(0), 0, &InputConfig::default(), (1.0, 1.0));
        assert!(m.left_x.abs() < 300 && m.left_y.abs() < 300);
    }

    #[test]
    fn each_stick_has_its_own_settings() {
        let cfg = InputConfig {
            left_stick: StickConfig { deadzone: 30, ..Default::default() },
            ..Default::default()
        };
        let s = ControllerState { lx: 140, rx: 140, ..state(0) };
        let m = map(&s, 0, &cfg, (0.0, 0.0));
        assert_eq!(m.left_x, 0);
        assert!(m.right_x > 0);
    }

    // --- what rules hold -------------------------------------------------------------------

    fn with_held(s: &ControllerState, consumed: u32, held: PadHeld) -> Mapped {
        map_with(s, consumed, &InputConfig::default(), (0.0, 0.0), &held)
    }

    #[test]
    fn buttons_held_by_rules_are_added_to_the_real_ones() {
        let held = PadHeld { buttons: xbtn::X | xbtn::LB, ..Default::default() };
        let m = with_held(&state(btn::CROSS), 0, held);
        assert_eq!(m.buttons, xbtn::A | xbtn::X | xbtn::LB);
    }

    #[test]
    fn a_consumed_button_is_replaced_by_what_the_rule_holds() {
        let held = PadHeld { buttons: xbtn::Y, ..Default::default() };
        let m = with_held(&state(btn::CROSS), btn::CROSS, held);
        assert_eq!(m.buttons, xbtn::Y, "Cross is no longer transmitted, the rule holds Y");
    }

    #[test]
    fn triggers_held_by_rules_are_fully_pressed() {
        let held = PadHeld { left_trigger: true, right_trigger: true, ..Default::default() };
        let m = with_held(&state(0), 0, held);
        assert_eq!((m.left_trigger, m.right_trigger), (255, 255));
    }

    #[test]
    fn a_stick_taken_by_a_binding_is_not_transmitted() {
        let s = ControllerState { lx: 255, rx: 255, ..state(0) };
        let held = PadHeld { right_stick_taken: true, ..Default::default() };
        let m = with_held(&s, 0, held);
        assert!(m.left_x > 0, "the left stick passes through");
        assert_eq!((m.right_x, m.right_y), (0, 0), "the right one is taken");
    }

    #[test]
    fn nothing_held_changes_nothing() {
        let s = state(btn::CROSS);
        assert_eq!(with_held(&s, 0, PadHeld::default()), map(&s, 0, &InputConfig::default(), (0.0, 0.0)));
    }

    #[test]
    fn xinput_constants_match_the_driver_library() {
        use vigem_client::XButtons;
        assert_eq!(
            [xbtn::UP, xbtn::DOWN, xbtn::LEFT, xbtn::RIGHT, xbtn::START, xbtn::BACK, xbtn::LTHUMB, xbtn::RTHUMB,
             xbtn::LB, xbtn::RB, xbtn::GUIDE, xbtn::A, xbtn::B, xbtn::X, xbtn::Y],
            [XButtons::UP, XButtons::DOWN, XButtons::LEFT, XButtons::RIGHT, XButtons::START, XButtons::BACK,
             XButtons::LTHUMB, XButtons::RTHUMB, XButtons::LB, XButtons::RB, XButtons::GUIDE, XButtons::A,
             XButtons::B, XButtons::X, XButtons::Y]
        );
    }

    #[test]
    fn the_config_round_trips_and_old_files_get_the_neutral_defaults() {
        let cfg = InputConfig {
            remap: vec![Remap { from: Button::Cross, to: XTarget::DpadUp }],
            right_stick: StickConfig { deadzone: 12, outer: 90, curve: Curve::Precise },
            ..Default::default()
        };
        let json = serde_json::to_string(&cfg).unwrap();
        assert!(json.contains(r#""from":"cross""#) && json.contains(r#""to":"dpad_up""#));
        assert_eq!(serde_json::from_str::<InputConfig>(&json).unwrap(), cfg);
        assert_eq!(serde_json::from_str::<InputConfig>("{}").unwrap(), InputConfig::default());
    }
}
