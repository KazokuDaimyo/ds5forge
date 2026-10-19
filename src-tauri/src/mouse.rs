//! Sending mouse movements and clicks to Windows (gyroscope and touchpad).

use crate::motion::MotionOut;

const MOVE: u32 = 0x0001;
const LEFT_DOWN: u32 = 0x0002;
const LEFT_UP: u32 = 0x0004;
const RIGHT_DOWN: u32 = 0x0008;
const RIGHT_UP: u32 = 0x0010;

/// The events to send, in order: (dx, dy, flags). The movement comes before the click so that the
/// click lands at the right position.
fn events(m: &MotionOut) -> Vec<(i32, i32, u32)> {
    let mut events = Vec::new();
    if m.mouse_dx != 0 || m.mouse_dy != 0 {
        events.push((m.mouse_dx, m.mouse_dy, MOVE));
    }
    match m.left_button {
        Some(true) => events.push((0, 0, LEFT_DOWN)),
        Some(false) => events.push((0, 0, LEFT_UP)),
        None => {}
    }
    match m.right_button {
        Some(true) => events.push((0, 0, RIGHT_DOWN)),
        Some(false) => events.push((0, 0, RIGHT_UP)),
        None => {}
    }
    events
}

/// Sends to Windows what the motion asks for. Does nothing when there is nothing to do.
pub fn apply(m: &MotionOut) {
    let events = events(m);
    if !events.is_empty() {
        send(&events);
    }
}

#[cfg(windows)]
fn send(events: &[(i32, i32, u32)]) {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEINPUT,
    };

    let inputs: Vec<INPUT> = events
        .iter()
        .map(|&(dx, dy, flags)| INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT { dx, dy, mouseData: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 },
            },
        })
        .collect();

    // SAFETY: `inputs` is a valid array of `INPUT` for the whole call, and the size passed is that
    // of the structure.
    unsafe {
        SendInput(inputs.len() as u32, inputs.as_ptr(), size_of::<INPUT>() as i32);
    }
}

#[cfg(not(windows))]
fn send(_events: &[(i32, i32, u32)]) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_idle_motion_sends_nothing() {
        assert!(events(&MotionOut::default()).is_empty());
    }

    #[test]
    fn movement_is_sent_as_a_relative_move() {
        let m = MotionOut { mouse_dx: 5, mouse_dy: -3, ..Default::default() };
        assert_eq!(events(&m), [(5, -3, MOVE)]);
    }

    #[test]
    fn movement_in_a_single_axis_is_still_sent() {
        let m = MotionOut { mouse_dy: 1, ..Default::default() };
        assert_eq!(events(&m), [(0, 1, MOVE)]);
    }

    #[test]
    fn clicks_map_to_press_and_release_flags() {
        let press = MotionOut { left_button: Some(true), ..Default::default() };
        assert_eq!(events(&press), [(0, 0, LEFT_DOWN)]);
        let release = MotionOut { right_button: Some(false), ..Default::default() };
        assert_eq!(events(&release), [(0, 0, RIGHT_UP)]);
    }

    #[test]
    fn the_move_comes_before_the_click() {
        let m = MotionOut { mouse_dx: 2, mouse_dy: 2, left_button: Some(true), ..Default::default() };
        assert_eq!(events(&m), [(2, 2, MOVE), (0, 0, LEFT_DOWN)]);
    }

    #[test]
    fn the_stick_output_never_produces_mouse_events() {
        let m = MotionOut { stick: (1.0, 1.0), ..Default::default() };
        assert!(events(&m).is_empty());
    }
}
