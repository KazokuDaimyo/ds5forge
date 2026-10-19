//! Gyroscope and touchpad: turn the controller's movements into mouse movement or into a
//! deflection of the right stick.
//!
//! The module is pure: it receives the controller state and the time, and returns what to do
//! (movement, clicks, stick). The actual sending to Windows is in `mouse.rs`.

use crate::controller::{btn, ControllerState};
use crate::shortcuts::Button;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

/// Raw gyroscope units per degree/second. An approximate value (±2000 °/s over 16 bits): the
/// sensitivity slider absorbs the difference, what matters is proportionality.
const RAW_PER_DPS: f32 = 16.0;
/// Below this speed (in °/s) the controller is considered motionless: suppresses noise.
const GYRO_NOISE_DPS: f32 = 1.0;
/// Maximum deviation (raw units) from the resting value for the controller to count as still.
const STILL_RAW: f32 = 40.0;
/// Number of consecutive still reports before the bias is learned again (about 0.4 s).
const STILL_SAMPLES: u32 = 100;
/// Learning rate of the bias, applied on every still report.
const BIAS_RATE: f32 = 0.02;
/// A longer interval (pause, sleep) is brought back to this value, to avoid a cursor jump.
const MAX_STEP: Duration = Duration::from_millis(50);
/// Beyond this difference (in touchpad units) between two reports, it is a new finger rather than a slide.
const TOUCH_JUMP: i32 = 300;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GyroMode {
    #[default]
    Off,
    /// Adds the movement to the right stick of the virtual Xbox controller.
    RightStick,
    /// Moves the mouse cursor.
    Mouse,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TouchpadMode {
    #[default]
    Off,
    /// The touchpad moves the cursor; pressing it clicks.
    Mouse,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(default)]
pub struct MotionConfig {
    pub gyro: GyroMode,
    /// 1 to 100.
    pub gyro_sensitivity: u8,
    pub gyro_invert_x: bool,
    pub gyro_invert_y: bool,
    /// The gyroscope only acts while this button is held. `None`: always active.
    pub gyro_hold: Option<Button>,
    pub touchpad: TouchpadMode,
    /// 1 to 100.
    pub touchpad_sensitivity: u8,
}

impl Default for MotionConfig {
    fn default() -> Self {
        Self {
            gyro: GyroMode::Off,
            gyro_sensitivity: 30,
            gyro_invert_x: false,
            gyro_invert_y: false,
            gyro_hold: None,
            touchpad: TouchpadMode::Off,
            touchpad_sensitivity: 20,
        }
    }
}

/// What the motion asks to do for this report.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MotionOut {
    pub mouse_dx: i32,
    pub mouse_dy: i32,
    /// Deflection to add to the right stick, from -1 to 1 (x to the right, y upwards).
    pub stick: (f32, f32),
    /// `Some(true)`: press, `Some(false)`: release.
    pub left_button: Option<bool>,
    pub right_button: Option<bool>,
}

/// Estimates the gyroscope's resting value (the "bias"): a gyroscope at rest never reads exactly
/// zero, and without correction the cursor would drift on its own.
#[derive(Default)]
pub struct GyroBias {
    bias: [f32; 3],
    still: u32,
}

impl GyroBias {
    /// Returns the measurement corrected for the bias, in raw units.
    pub fn feed(&mut self, raw: [i16; 3]) -> [f32; 3] {
        let mut corrected = [0.0; 3];
        for i in 0..3 {
            corrected[i] = f32::from(raw[i]) - self.bias[i];
        }
        if corrected.iter().all(|c| c.abs() < STILL_RAW) {
            self.still += 1;
            if self.still >= STILL_SAMPLES {
                for i in 0..3 {
                    self.bias[i] += corrected[i] * BIAS_RATE;
                }
            }
        } else {
            self.still = 0;
        }
        corrected
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Click {
    Left,
    Right,
}

#[derive(Default)]
pub struct Motion {
    bias: GyroBias,
    last_time: Option<Instant>,
    /// The part of a pixel not yet sent, so that small movements are not lost.
    frac: (f32, f32),
    last_touch: Option<(u16, u16)>,
    clicking: Option<Click>,
}

impl Motion {
    /// `enabled`: the master switch (the "motion" shortcut), without touching the configuration.
    pub fn update(&mut self, s: &ControllerState, cfg: &MotionConfig, enabled: bool, now: Instant) -> MotionOut {
        let dt = self.last_time.map_or(Duration::ZERO, |t| now.saturating_duration_since(t).min(MAX_STEP));
        self.last_time = Some(now);
        let dt = dt.as_secs_f32();

        // The bias is always learned, even while the feature is off: it is ready on activation.
        let gyro = self.bias.feed(s.gyro);

        let mut out = MotionOut::default();
        let mut mouse = (0.0f32, 0.0f32);

        let hold_ok = cfg.gyro_hold.map_or(true, |b| s.buttons & b.bit() != 0);
        if enabled && hold_ok && cfg.gyro != GyroMode::Off {
            // x: pitch (up/down), y: yaw (left/right).
            let dead = |dps: f32| if dps.abs() < GYRO_NOISE_DPS { 0.0 } else { dps };
            let pitch = dead(gyro[0] / RAW_PER_DPS);
            let yaw = dead(gyro[1] / RAW_PER_DPS);
            let sx = if cfg.gyro_invert_x { -1.0 } else { 1.0 };
            let sy = if cfg.gyro_invert_y { -1.0 } else { 1.0 };
            let sens = f32::from(cfg.gyro_sensitivity.clamp(1, 100));

            match cfg.gyro {
                GyroMode::Mouse => {
                    let pixels_per_degree = sens * 0.25;
                    mouse.0 += yaw * dt * pixels_per_degree * sx;
                    mouse.1 += -pitch * dt * pixels_per_degree * sy;
                }
                GyroMode::RightStick => {
                    out.stick = (
                        (yaw * sens / 3000.0 * sx).clamp(-1.0, 1.0),
                        (pitch * sens / 3000.0 * sy).clamp(-1.0, 1.0),
                    );
                }
                GyroMode::Off => {}
            }
        }

        let touchpad_on = enabled && cfg.touchpad == TouchpadMode::Mouse;
        if touchpad_on {
            match (s.touch, self.last_touch) {
                (Some((x, y)), Some((px, py))) => {
                    let (dx, dy) = (i32::from(x) - i32::from(px), i32::from(y) - i32::from(py));
                    // A large jump is a finger landing somewhere else: it is ignored.
                    if dx.abs() < TOUCH_JUMP && dy.abs() < TOUCH_JUMP {
                        let scale = f32::from(cfg.touchpad_sensitivity.clamp(1, 100)) / 20.0;
                        mouse.0 += dx as f32 * scale;
                        mouse.1 += dy as f32 * scale;
                    }
                }
                _ => {}
            }
        }
        self.last_touch = if touchpad_on { s.touch } else { None };

        // Clicks: pressing the pad clicks; with two fingers, it is a right click.
        let pressed = touchpad_on && s.buttons & btn::TOUCHPAD != 0;
        match (pressed, self.clicking) {
            (true, None) => {
                let click = if s.touch_fingers >= 2 { Click::Right } else { Click::Left };
                self.clicking = Some(click);
                match click {
                    Click::Left => out.left_button = Some(true),
                    Click::Right => out.right_button = Some(true),
                }
            }
            (false, Some(click)) => {
                self.clicking = None;
                match click {
                    Click::Left => out.left_button = Some(false),
                    Click::Right => out.right_button = Some(false),
                }
            }
            _ => {}
        }

        // Accumulates pixel fractions: only the integer part is sent, the rest is kept.
        if mouse == (0.0, 0.0) && !touchpad_on && cfg.gyro != GyroMode::Mouse {
            self.frac = (0.0, 0.0);
        } else {
            self.frac.0 += mouse.0;
            self.frac.1 += mouse.1;
            out.mouse_dx = self.frac.0.trunc() as i32;
            out.mouse_dy = self.frac.1.trunc() as i32;
            self.frac.0 -= out.mouse_dx as f32;
            self.frac.1 -= out.mouse_dy as f32;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: fn(u64) -> Duration = Duration::from_millis;

    fn sample(gyro: [i16; 3]) -> ControllerState {
        ControllerState { connected: true, gyro, ..Default::default() }
    }

    fn mouse_cfg() -> MotionConfig {
        MotionConfig { gyro: GyroMode::Mouse, ..Default::default() }
    }

    /// Runs `n` reports of 4 ms with the same measurement; returns the accumulated movement.
    fn run(m: &mut Motion, s: &ControllerState, cfg: &MotionConfig, n: u32, t0: Instant) -> (i32, i32, Instant) {
        let (mut dx, mut dy, mut t) = (0, 0, t0);
        for _ in 0..n {
            t += MS(4);
            let out = m.update(s, cfg, true, t);
            dx += out.mouse_dx;
            dy += out.mouse_dy;
        }
        (dx, dy, t)
    }

    // --- gyroscope bias ---------------------------------------------------------------------

    #[test]
    fn the_bias_is_learned_when_the_controller_rests() {
        let mut bias = GyroBias::default();
        let mut last = [0.0; 3];
        for _ in 0..1500 {
            last = bias.feed([10, -8, 5]);
        }
        assert!(last.iter().all(|c| c.abs() < 1.0), "{last:?}");
    }

    #[test]
    fn the_bias_is_not_learned_while_the_controller_moves() {
        let mut bias = GyroBias::default();
        for _ in 0..1500 {
            bias.feed([2000, 0, 0]); // a clear rotation
        }
        let corrected = bias.feed([2000, 0, 0]);
        assert!(corrected[0] > 1900.0, "a movement must not be absorbed: {corrected:?}");
    }

    #[test]
    fn a_rest_bias_is_removed_from_later_movement() {
        let mut bias = GyroBias::default();
        for _ in 0..1500 {
            bias.feed([12, 0, 0]);
        }
        let corrected = bias.feed([12 + 800, 0, 0]);
        assert!((corrected[0] - 800.0).abs() < 2.0, "{corrected:?}");
    }

    // --- gyroscope as a mouse ---------------------------------------------------------------

    #[test]
    fn nothing_moves_when_the_gyro_is_off() {
        let mut m = Motion::default();
        let (dx, dy, _) = run(&mut m, &sample([0, 3000, 0]), &MotionConfig::default(), 200, Instant::now());
        assert_eq!((dx, dy), (0, 0));
    }

    #[test]
    fn turning_moves_the_mouse_in_proportion_to_speed_and_time() {
        let mut m = Motion::default();
        // 160 units = 10 °/s; sensitivity 30 => 7.5 px/°; 1 s => 75 px.
        let (dx, _, _) = run(&mut m, &sample([0, 160, 0]), &mouse_cfg(), 250, Instant::now());
        assert!((dx - 75).abs() <= 3, "{dx}");
    }

    #[test]
    fn twice_the_speed_moves_twice_as_far() {
        let slow = run(&mut Motion::default(), &sample([0, 160, 0]), &mouse_cfg(), 250, Instant::now()).0;
        let fast = run(&mut Motion::default(), &sample([0, 320, 0]), &mouse_cfg(), 250, Instant::now()).0;
        assert!((fast - 2 * slow).abs() <= 3, "{slow} {fast}");
    }

    #[test]
    fn small_movements_are_not_lost_to_rounding() {
        // 0.3 px per report: never a whole pixel at once, but it must accumulate.
        let mut m = Motion::default();
        let cfg = MotionConfig { gyro_sensitivity: 1, ..mouse_cfg() };
        let (dx, _, _) = run(&mut m, &sample([0, 480, 0]), &cfg, 1000, Instant::now());
        // 30 °/s × 4 s × 0.25 px/° = 30 px
        assert!((dx - 30).abs() <= 2, "{dx}");
    }

    #[test]
    fn the_axes_can_be_inverted() {
        let cfg = MotionConfig { gyro_invert_x: true, ..mouse_cfg() };
        let normal = run(&mut Motion::default(), &sample([0, 160, 0]), &mouse_cfg(), 250, Instant::now()).0;
        let inverted = run(&mut Motion::default(), &sample([0, 160, 0]), &cfg, 250, Instant::now()).0;
        assert!(normal > 0 && inverted < 0);
        let cfg = MotionConfig { gyro_invert_y: true, ..mouse_cfg() };
        let normal = run(&mut Motion::default(), &sample([160, 0, 0]), &mouse_cfg(), 250, Instant::now()).1;
        let inverted = run(&mut Motion::default(), &sample([160, 0, 0]), &cfg, 250, Instant::now()).1;
        assert!(normal.signum() == -inverted.signum() && normal != 0);
    }

    #[test]
    fn residual_noise_does_not_move_the_cursor() {
        let mut m = Motion::default();
        // 8 units ≈ 0.5 °/s: below the noise threshold.
        let (dx, dy, _) = run(&mut m, &sample([8, 8, 8]), &mouse_cfg(), 2000, Instant::now());
        assert_eq!((dx, dy), (0, 0));
    }

    #[test]
    fn a_long_pause_does_not_make_the_cursor_jump() {
        let mut m = Motion::default();
        let t0 = Instant::now();
        m.update(&sample([0, 160, 0]), &mouse_cfg(), true, t0);
        let out = m.update(&sample([0, 160, 0]), &mouse_cfg(), true, t0 + Duration::from_secs(30));
        // Without the cap, 30 s at 10 °/s would be over 2000 px; the 50 ms cap gives about 4.
        assert!(out.mouse_dx < 10, "{}", out.mouse_dx);
    }

    #[test]
    fn the_gyro_can_require_a_held_button() {
        let cfg = MotionConfig { gyro_hold: Some(Button::L2), ..mouse_cfg() };
        let still = run(&mut Motion::default(), &sample([0, 160, 0]), &cfg, 250, Instant::now()).0;
        assert_eq!(still, 0, "nothing moves without the button");

        let held = ControllerState { buttons: btn::L2, ..sample([0, 160, 0]) };
        let moved = run(&mut Motion::default(), &held, &cfg, 250, Instant::now()).0;
        assert!(moved > 50, "{moved}");
    }

    #[test]
    fn the_master_switch_stops_everything_without_touching_the_config() {
        let mut m = Motion::default();
        let t0 = Instant::now();
        let mut dx = 0;
        for i in 1..=250 {
            dx += m.update(&sample([0, 160, 0]), &mouse_cfg(), false, t0 + MS(4 * i)).mouse_dx;
        }
        assert_eq!(dx, 0);
    }

    // --- gyroscope as a stick ---------------------------------------------------------------

    #[test]
    fn the_stick_mode_deflects_in_proportion_to_speed() {
        let cfg = MotionConfig { gyro: GyroMode::RightStick, ..Default::default() };
        let mut m = Motion::default();
        // 1600 units = 100 °/s; sensitivity 30 => 100 × 30 / 3000 = 1.0 (fully deflected).
        let out = m.update(&sample([0, 1600, 0]), &cfg, true, Instant::now());
        assert!((out.stick.0 - 1.0).abs() < 0.01, "{:?}", out.stick);
        let half = Motion::default().update(&sample([0, 800, 0]), &cfg, true, Instant::now());
        assert!((half.stick.0 - 0.5).abs() < 0.01, "{:?}", half.stick);
        assert_eq!((out.mouse_dx, out.mouse_dy), (0, 0), "the stick mode does not touch the mouse");
    }

    #[test]
    fn the_stick_deflection_is_clamped() {
        let cfg = MotionConfig { gyro: GyroMode::RightStick, gyro_sensitivity: 100, ..Default::default() };
        let out = Motion::default().update(&sample([30000, 30000, 0]), &cfg, true, Instant::now());
        assert!(out.stick.0 <= 1.0 && out.stick.1 <= 1.0);
    }

    // --- touchpad ----------------------------------------------------------------------------

    fn touch_cfg() -> MotionConfig {
        MotionConfig { touchpad: TouchpadMode::Mouse, ..Default::default() }
    }

    fn touching(x: u16, y: u16) -> ControllerState {
        ControllerState { touch: Some((x, y)), touch_fingers: 1, ..Default::default() }
    }

    #[test]
    fn a_new_finger_does_not_move_the_cursor_but_dragging_does() {
        let mut m = Motion::default();
        let t0 = Instant::now();
        let first = m.update(&touching(500, 500), &touch_cfg(), true, t0);
        assert_eq!((first.mouse_dx, first.mouse_dy), (0, 0));
        let drag = m.update(&touching(520, 490), &touch_cfg(), true, t0 + MS(4));
        assert_eq!((drag.mouse_dx, drag.mouse_dy), (20, -10), "sensitivity 20 = one to one");
    }

    #[test]
    fn the_touchpad_sensitivity_scales_the_movement() {
        let cfg = MotionConfig { touchpad_sensitivity: 40, ..touch_cfg() };
        let mut m = Motion::default();
        let t0 = Instant::now();
        m.update(&touching(500, 500), &cfg, true, t0);
        let drag = m.update(&touching(510, 500), &cfg, true, t0 + MS(4));
        assert_eq!(drag.mouse_dx, 20);
    }

    #[test]
    fn lifting_and_relanding_a_finger_does_not_jump() {
        let mut m = Motion::default();
        let t0 = Instant::now();
        m.update(&touching(100, 100), &touch_cfg(), true, t0);
        m.update(&ControllerState::default(), &touch_cfg(), true, t0 + MS(4)); // finger lifted
        let relanded = m.update(&touching(1500, 800), &touch_cfg(), true, t0 + MS(8));
        assert_eq!((relanded.mouse_dx, relanded.mouse_dy), (0, 0));
    }

    #[test]
    fn a_huge_jump_between_two_reports_is_ignored() {
        let mut m = Motion::default();
        let t0 = Instant::now();
        m.update(&touching(100, 100), &touch_cfg(), true, t0);
        let jump = m.update(&touching(1800, 100), &touch_cfg(), true, t0 + MS(4));
        assert_eq!(jump.mouse_dx, 0);
    }

    #[test]
    fn pressing_the_pad_clicks_and_releasing_releases() {
        let mut m = Motion::default();
        let t0 = Instant::now();
        let pressed = ControllerState { buttons: btn::TOUCHPAD, ..touching(500, 500) };
        assert_eq!(m.update(&pressed, &touch_cfg(), true, t0).left_button, Some(true));
        assert_eq!(m.update(&pressed, &touch_cfg(), true, t0 + MS(4)).left_button, None, "no repetition");
        assert_eq!(m.update(&touching(500, 500), &touch_cfg(), true, t0 + MS(8)).left_button, Some(false));
    }

    #[test]
    fn pressing_with_two_fingers_is_a_right_click() {
        let mut m = Motion::default();
        let t0 = Instant::now();
        let two = ControllerState { buttons: btn::TOUCHPAD, touch_fingers: 2, ..touching(500, 500) };
        let out = m.update(&two, &touch_cfg(), true, t0);
        assert_eq!((out.left_button, out.right_button), (None, Some(true)));
        let release = m.update(&touching(500, 500), &touch_cfg(), true, t0 + MS(4));
        assert_eq!((release.left_button, release.right_button), (None, Some(false)));
    }

    #[test]
    fn the_same_button_that_was_pressed_is_released_even_if_the_finger_count_changes() {
        let mut m = Motion::default();
        let t0 = Instant::now();
        let one = ControllerState { buttons: btn::TOUCHPAD, ..touching(500, 500) };
        m.update(&one, &touch_cfg(), true, t0);
        let two = ControllerState { buttons: 0, touch_fingers: 2, ..touching(500, 500) };
        let out = m.update(&two, &touch_cfg(), true, t0 + MS(4));
        assert_eq!((out.left_button, out.right_button), (Some(false), None));
    }

    #[test]
    fn a_click_in_progress_is_released_when_the_function_is_switched_off() {
        let mut m = Motion::default();
        let t0 = Instant::now();
        let pressed = ControllerState { buttons: btn::TOUCHPAD, ..touching(500, 500) };
        m.update(&pressed, &touch_cfg(), true, t0);
        let off = m.update(&pressed, &touch_cfg(), false, t0 + MS(4));
        assert_eq!(off.left_button, Some(false), "otherwise the button would stay pressed");
    }

    #[test]
    fn the_touchpad_does_nothing_when_off() {
        let mut m = Motion::default();
        let t0 = Instant::now();
        let pressed = ControllerState { buttons: btn::TOUCHPAD, ..touching(500, 500) };
        m.update(&pressed, &MotionConfig::default(), true, t0);
        assert_eq!(m.update(&touching(560, 500), &MotionConfig::default(), true, t0 + MS(4)), MotionOut::default());
    }

    #[test]
    fn the_config_defaults_to_everything_off_and_round_trips() {
        let cfg = MotionConfig::default();
        assert_eq!((cfg.gyro, cfg.touchpad, cfg.gyro_hold), (GyroMode::Off, TouchpadMode::Off, None));

        let cfg = MotionConfig { gyro: GyroMode::RightStick, gyro_hold: Some(Button::L1), ..cfg };
        let json = serde_json::to_string(&cfg).unwrap();
        assert!(json.contains(r#""gyro":"right_stick""#) && json.contains(r#""gyro_hold":"l1""#));
        assert_eq!(serde_json::from_str::<MotionConfig>(&json).unwrap(), cfg);
        assert_eq!(serde_json::from_str::<MotionConfig>("{}").unwrap(), MotionConfig::default());
    }
}
