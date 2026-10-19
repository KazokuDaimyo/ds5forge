//! The controller menu: opened by holding the PS button, driven with the controller itself, shown
//! in the overlay. It switches between native and Xbox 360 mode and between profiles, and shows the
//! connected controllers with their battery.
//!
//! This module is the pure logic (which button does what, when the menu opens and closes); the
//! controller thread carries out the actions and `overlay` shows the result.

use crate::controller::btn;
use serde::Serialize;
use std::time::{Duration, Instant};

/// How long PS must be held to open the menu. Longer than a tap, shorter than a wait.
pub const HOLD: Duration = Duration::from_millis(600);
/// The menu closes by itself after this long without any input.
pub const IDLE_CLOSE: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    /// Native / Xbox 360.
    Mode,
    /// Automatic, or one of the game profiles.
    Profile,
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Confirm,
    Back,
}

/// What the user asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// The selection moved: the menu must be drawn again.
    Moved,
    ToggleMode,
    /// The next (`1`) or the previous (`-1`) profile.
    CycleProfile(i8),
    Close,
}

/// The keys newly pressed between two states of the buttons (see `controller::btn`).
pub fn keys(previous: u32, now: u32) -> Vec<Key> {
    let pressed = now & !previous;
    let mut keys = Vec::new();
    for (bit, key) in [
        (btn::DPAD_UP, Key::Up),
        (btn::DPAD_DOWN, Key::Down),
        (btn::DPAD_LEFT, Key::Left),
        (btn::DPAD_RIGHT, Key::Right),
        (btn::CROSS, Key::Confirm),
        (btn::CIRCLE, Key::Back),
        (btn::PS, Key::Back),
    ] {
        if pressed & bit != 0 {
            keys.push(key);
        }
    }
    keys
}

/// Detects the long press on PS: true once per press, when it has been held for `HOLD`.
#[derive(Debug, Default)]
pub struct HoldDetector {
    since: Option<Instant>,
    fired: bool,
}

impl HoldDetector {
    pub fn update(&mut self, down: bool, now: Instant) -> bool {
        if !down {
            *self = Self::default();
            return false;
        }
        let since = *self.since.get_or_insert(now);
        if !self.fired && now.duration_since(since) >= HOLD {
            self.fired = true;
            return true;
        }
        false
    }
}

/// An open menu.
#[derive(Debug)]
pub struct Menu {
    rows: Vec<Row>,
    cursor: usize,
    /// The buttons at the previous update: only new presses count.
    buttons: u32,
    last_input: Instant,
}

impl Menu {
    /// Opens the menu. The buttons held at that moment (PS, which opened it) are not presses.
    pub fn open(has_profiles: bool, buttons: u32, now: Instant) -> Self {
        let mut rows = vec![Row::Mode];
        if has_profiles {
            rows.push(Row::Profile);
        }
        rows.push(Row::Close);
        Self { rows, cursor: 0, buttons, last_input: now }
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Feeds the buttons of a new report and returns what the user asked for.
    pub fn update(&mut self, buttons: u32, now: Instant) -> Vec<Action> {
        let keys = keys(self.buttons, buttons);
        self.buttons = buttons;
        if !keys.is_empty() {
            self.last_input = now;
        }
        keys.into_iter().map(|key| self.handle(key)).collect()
    }

    fn handle(&mut self, key: Key) -> Action {
        let row = self.rows[self.cursor];
        let n = self.rows.len();
        match key {
            Key::Up => {
                self.cursor = (self.cursor + n - 1) % n;
                Action::Moved
            }
            Key::Down => {
                self.cursor = (self.cursor + 1) % n;
                Action::Moved
            }
            Key::Left => match row {
                Row::Mode => Action::ToggleMode,
                Row::Profile => Action::CycleProfile(-1),
                Row::Close => Action::Moved,
            },
            Key::Right => match row {
                Row::Mode => Action::ToggleMode,
                Row::Profile => Action::CycleProfile(1),
                Row::Close => Action::Moved,
            },
            Key::Confirm => match row {
                Row::Mode => Action::ToggleMode,
                Row::Profile => Action::CycleProfile(1),
                Row::Close => Action::Close,
            },
            Key::Back => Action::Close,
        }
    }

    pub fn timed_out(&self, now: Instant) -> bool {
        now.duration_since(self.last_input) >= IDLE_CLOSE
    }
}

/// The input that everything else sees while the menu is open (and until every button has been
/// released afterwards): nothing is pressed. The menu takes the buttons, the game and the rules do not.
pub fn neutral(state: &crate::controller::ControllerState) -> crate::controller::ControllerState {
    crate::controller::ControllerState {
        buttons: 0,
        lx: 128,
        ly: 128,
        rx: 128,
        ry: 128,
        l2: 0,
        r2: 0,
        gyro: [0; 3],
        touch: None,
        touch_fingers: 0,
        ..state.clone()
    }
}

/// A row of the menu, for the overlay.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct MenuRow {
    /// Name of the icon drawn for the row (see `src/icons.ts`).
    pub icon: String,
    /// What the row is, for the tooltip; the overlay shows the icon.
    pub label: String,
    /// The current value, if the row has one.
    pub value: Option<String>,
}

/// The menu as the overlay draws it.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct MenuView {
    pub rows: Vec<MenuRow>,
    pub cursor: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: fn(u64) -> Duration = Duration::from_millis;

    fn open(has_profiles: bool) -> (Menu, Instant) {
        let t0 = Instant::now();
        (Menu::open(has_profiles, btn::PS, t0), t0)
    }

    #[test]
    fn only_new_presses_are_keys() {
        assert_eq!(keys(0, btn::DPAD_DOWN), [Key::Down]);
        assert!(keys(btn::DPAD_DOWN, btn::DPAD_DOWN).is_empty(), "held is not pressed again");
        assert_eq!(keys(btn::DPAD_DOWN, btn::DPAD_DOWN | btn::CROSS), [Key::Confirm]);
        assert_eq!(keys(btn::CROSS, 0), Vec::<Key>::new(), "a release is not a key");
        assert_eq!(keys(0, btn::CIRCLE), [Key::Back]);
        assert_eq!(keys(0, btn::PS), [Key::Back]);
    }

    #[test]
    fn holding_ps_opens_the_menu_once() {
        let mut hold = HoldDetector::default();
        let t0 = Instant::now();
        assert!(!hold.update(true, t0));
        assert!(!hold.update(true, t0 + MS(300)), "too early");
        assert!(hold.update(true, t0 + MS(650)));
        assert!(!hold.update(true, t0 + MS(900)), "once per press");
        assert!(!hold.update(false, t0 + MS(1000)));
        assert!(!hold.update(true, t0 + MS(1100)), "a new press starts over");
        assert!(hold.update(true, t0 + MS(1800)));
    }

    #[test]
    fn a_tap_never_opens_the_menu() {
        let mut hold = HoldDetector::default();
        let t0 = Instant::now();
        assert!(!hold.update(true, t0));
        assert!(!hold.update(true, t0 + MS(200)));
        assert!(!hold.update(false, t0 + MS(250)));
        assert!(!hold.update(false, t0 + MS(2000)));
    }

    #[test]
    fn the_button_that_opened_the_menu_does_not_close_it() {
        let (mut menu, t0) = open(true);
        assert!(menu.update(btn::PS, t0 + MS(50)).is_empty(), "PS is still held");
        assert!(menu.update(0, t0 + MS(100)).is_empty(), "releasing it does nothing");
        assert_eq!(menu.update(btn::PS, t0 + MS(200)), [Action::Close], "a new press closes");
    }

    #[test]
    fn the_cursor_wraps_around_the_rows() {
        let (mut menu, t0) = open(true);
        assert_eq!(menu.rows(), [Row::Mode, Row::Profile, Row::Close]);
        assert_eq!(menu.update(btn::DPAD_UP, t0 + MS(10)), [Action::Moved]);
        assert_eq!(menu.cursor(), 2, "up from the first row goes to the last");
        menu.update(0, t0 + MS(20));
        menu.update(btn::DPAD_DOWN, t0 + MS(30));
        assert_eq!(menu.cursor(), 0);
    }

    #[test]
    fn the_profile_row_only_exists_when_there_are_game_profiles() {
        let (menu, _) = open(false);
        assert_eq!(menu.rows(), [Row::Mode, Row::Close]);
    }

    #[test]
    fn the_mode_row_toggles_with_left_right_and_confirm() {
        let (mut menu, t0) = open(true);
        for (i, button) in [btn::DPAD_LEFT, btn::DPAD_RIGHT, btn::CROSS].into_iter().enumerate() {
            assert_eq!(menu.update(button, t0 + MS(10 + 20 * i as u64)), [Action::ToggleMode]);
            menu.update(0, t0 + MS(20 + 20 * i as u64));
        }
    }

    #[test]
    fn the_profile_row_cycles_in_both_directions() {
        let (mut menu, t0) = open(true);
        menu.update(btn::DPAD_DOWN, t0 + MS(10));
        menu.update(0, t0 + MS(20));
        assert_eq!(menu.update(btn::DPAD_RIGHT, t0 + MS(30)), [Action::CycleProfile(1)]);
        menu.update(0, t0 + MS(40));
        assert_eq!(menu.update(btn::DPAD_LEFT, t0 + MS(50)), [Action::CycleProfile(-1)]);
        menu.update(0, t0 + MS(60));
        assert_eq!(menu.update(btn::CROSS, t0 + MS(70)), [Action::CycleProfile(1)]);
    }

    #[test]
    fn close_works_from_the_row_and_from_the_back_buttons() {
        let (mut menu, t0) = open(false);
        menu.update(btn::DPAD_DOWN, t0 + MS(10));
        menu.update(0, t0 + MS(20));
        assert_eq!(menu.update(btn::CROSS, t0 + MS(30)), [Action::Close]);
        menu.update(0, t0 + MS(40));
        assert_eq!(menu.update(btn::CIRCLE, t0 + MS(50)), [Action::Close], "Circle closes from anywhere");
    }

    #[test]
    fn the_menu_closes_by_itself_after_a_while_without_input() {
        let (mut menu, t0) = open(true);
        assert!(!menu.timed_out(t0 + IDLE_CLOSE - MS(1)));
        assert!(menu.timed_out(t0 + IDLE_CLOSE));
        menu.update(btn::DPAD_DOWN, t0 + Duration::from_secs(8));
        assert!(!menu.timed_out(t0 + Duration::from_secs(12)), "an input restarts the delay");
    }

    #[test]
    fn the_neutral_input_has_nothing_pressed_and_the_sticks_centred() {
        let state = crate::controller::ControllerState {
            connected: true,
            lx: 0,
            ry: 255,
            l2: 200,
            buttons: btn::CROSS | btn::PS,
            battery: Some(55),
            ..Default::default()
        };
        let n = neutral(&state);
        assert_eq!((n.buttons, n.lx, n.ly, n.rx, n.ry, n.l2, n.r2), (0, 128, 128, 128, 128, 0, 0));
        assert!(n.connected && n.battery == Some(55), "everything else is kept");
    }
}
