//! Light bar colour: fixed, or derived from the battery level or from the mode (native / Xbox 360).

use crate::controller::Charge;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LightbarMode {
    /// The colour chosen in the profile.
    #[default]
    Fixed,
    /// From red (empty) to green (full); cyan while charging.
    Battery,
    /// One colour per mode.
    Mode,
}

/// Colours used by `LightbarMode::Mode`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(default)]
pub struct ModeColors {
    pub xbox: [u8; 3],
    pub native: [u8; 3],
}

impl Default for ModeColors {
    fn default() -> Self {
        Self { xbox: [62, 207, 142], native: [79, 140, 255] }
    }
}

const RED: [u8; 3] = [255, 60, 60];
const YELLOW: [u8; 3] = [255, 200, 60];
const GREEN: [u8; 3] = [62, 207, 142];
const CHARGING: [u8; 3] = [60, 200, 255];

fn lerp(from: [u8; 3], to: [u8; 3], t: f32) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    let mix = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
    [mix(from[0], to[0]), mix(from[1], to[1]), mix(from[2], to[2])]
}

/// Red when empty, yellow at half, green when full. Cyan while the controller is charging.
pub fn battery_color(percent: u8, charge: Charge) -> [u8; 3] {
    match charge {
        Charge::Charging => CHARGING,
        Charge::Full => GREEN,
        _ => {
            let t = f32::from(percent.min(100)) / 100.0;
            if t <= 0.5 { lerp(RED, YELLOW, t * 2.0) } else { lerp(YELLOW, GREEN, (t - 0.5) * 2.0) }
        }
    }
}

/// The colour to display. Without a known battery level, it falls back to the fixed colour.
pub fn effective(
    mode: LightbarMode,
    colors: ModeColors,
    fixed: [u8; 3],
    battery: Option<u8>,
    charge: Charge,
    xbox_mode: bool,
) -> [u8; 3] {
    match mode {
        LightbarMode::Fixed => fixed,
        LightbarMode::Mode => if xbox_mode { colors.xbox } else { colors.native },
        LightbarMode::Battery => battery.map_or(fixed, |p| battery_color(p, charge)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXED: [u8; 3] = [1, 2, 3];

    #[test]
    fn the_battery_color_goes_from_red_to_green() {
        assert_eq!(battery_color(0, Charge::Discharging), RED);
        assert_eq!(battery_color(50, Charge::Discharging), YELLOW);
        assert_eq!(battery_color(100, Charge::Discharging), GREEN);
    }

    #[test]
    fn a_low_battery_is_redder_than_a_healthy_one() {
        let low = battery_color(15, Charge::Discharging);
        let high = battery_color(85, Charge::Discharging);
        assert!(low[0] > high[0], "more red when low");
        assert!(low[1] < high[1], "less green when low");
    }

    #[test]
    fn the_color_changes_smoothly_with_the_level() {
        let mut previous = battery_color(0, Charge::Discharging);
        for percent in (5..=100).step_by(5) {
            let color = battery_color(percent, Charge::Discharging);
            let distance: i32 = (0..3).map(|i| (i32::from(color[i]) - i32::from(previous[i])).abs()).sum();
            assert!(distance < 120, "colour jump at {percent} %: {previous:?} -> {color:?}");
            previous = color;
        }
    }

    #[test]
    fn charging_has_its_own_color_whatever_the_level() {
        assert_eq!(battery_color(5, Charge::Charging), CHARGING);
        assert_eq!(battery_color(95, Charge::Charging), CHARGING);
        assert_eq!(battery_color(100, Charge::Full), GREEN);
    }

    #[test]
    fn values_above_100_are_clamped() {
        assert_eq!(battery_color(250, Charge::Discharging), GREEN);
    }

    #[test]
    fn fixed_mode_ignores_battery_and_mode() {
        let c = effective(LightbarMode::Fixed, ModeColors::default(), FIXED, Some(5), Charge::Discharging, true);
        assert_eq!(c, FIXED);
    }

    #[test]
    fn mode_colors_follow_the_virtual_pad() {
        let colors = ModeColors { xbox: [9, 9, 9], native: [7, 7, 7] };
        assert_eq!(effective(LightbarMode::Mode, colors, FIXED, None, Charge::Unknown, true), [9, 9, 9]);
        assert_eq!(effective(LightbarMode::Mode, colors, FIXED, None, Charge::Unknown, false), [7, 7, 7]);
    }

    #[test]
    fn battery_mode_without_a_known_level_keeps_the_fixed_color() {
        let c = effective(LightbarMode::Battery, ModeColors::default(), FIXED, None, Charge::Unknown, false);
        assert_eq!(c, FIXED);
    }

    #[test]
    fn battery_mode_uses_the_gradient() {
        let c = effective(LightbarMode::Battery, ModeColors::default(), FIXED, Some(100), Charge::Discharging, false);
        assert_eq!(c, GREEN);
    }

    #[test]
    fn the_mode_serializes_in_snake_case_and_defaults_to_fixed() {
        assert_eq!(serde_json::to_string(&LightbarMode::Battery).unwrap(), "\"battery\"");
        assert_eq!(LightbarMode::default(), LightbarMode::Fixed);
        let colors: ModeColors = serde_json::from_str("{}").unwrap();
        assert_eq!(colors, ModeColors::default());
    }
}
