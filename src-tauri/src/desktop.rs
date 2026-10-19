//! Sending the keys, clicks, movements and wheel steps requested by rules to Windows.
//!
//! The translation of effects into raw events is pure (and therefore tested); only `send` talks to
//! Windows.

use crate::bindings::{Effect, MouseButton, Output};
use crate::keys;

const MOUSEEVENTF_MOVE: u32 = 0x0001;
const MOUSEEVENTF_LEFTDOWN: u32 = 0x0002;
const MOUSEEVENTF_LEFTUP: u32 = 0x0004;
const MOUSEEVENTF_RIGHTDOWN: u32 = 0x0008;
const MOUSEEVENTF_RIGHTUP: u32 = 0x0010;
const MOUSEEVENTF_MIDDLEDOWN: u32 = 0x0020;
const MOUSEEVENTF_MIDDLEUP: u32 = 0x0040;
const MOUSEEVENTF_XDOWN: u32 = 0x0080;
const MOUSEEVENTF_XUP: u32 = 0x0100;
const MOUSEEVENTF_WHEEL: u32 = 0x0800;
const XBUTTON1: i32 = 1;
const XBUTTON2: i32 = 2;

/// An event ready to be sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Raw {
    Key { vk: u16, up: bool, extended: bool, media: bool },
    Mouse { dx: i32, dy: i32, data: i32, flags: u32 },
}

fn mouse(flags: u32) -> Raw {
    Raw::Mouse { dx: 0, dy: 0, data: 0, flags }
}

fn button_event(button: MouseButton, up: bool) -> Raw {
    match (button, up) {
        (MouseButton::Left, false) => mouse(MOUSEEVENTF_LEFTDOWN),
        (MouseButton::Left, true) => mouse(MOUSEEVENTF_LEFTUP),
        (MouseButton::Right, false) => mouse(MOUSEEVENTF_RIGHTDOWN),
        (MouseButton::Right, true) => mouse(MOUSEEVENTF_RIGHTUP),
        (MouseButton::Middle, false) => mouse(MOUSEEVENTF_MIDDLEDOWN),
        (MouseButton::Middle, true) => mouse(MOUSEEVENTF_MIDDLEUP),
        (MouseButton::Back, up) => Raw::Mouse { dx: 0, dy: 0, data: XBUTTON1, flags: if up { MOUSEEVENTF_XUP } else { MOUSEEVENTF_XDOWN } },
        (MouseButton::Forward, up) => Raw::Mouse { dx: 0, dy: 0, data: XBUTTON2, flags: if up { MOUSEEVENTF_XUP } else { MOUSEEVENTF_XDOWN } },
    }
}

/// What an effect asks of Windows. Xbox outputs and application actions do not go through here.
pub fn translate(effect: &Effect) -> Option<Raw> {
    match effect {
        Effect::Down(output) | Effect::Up(output) => {
            let up = matches!(effect, Effect::Up(_));
            match output {
                Output::Key { key } => {
                    let def = keys::lookup(key)?;
                    Some(Raw::Key { vk: def.vk, up, extended: def.extended, media: def.media })
                }
                Output::Mouse { button } => Some(button_event(*button, up)),
                Output::Pad { .. } => None,
            }
        }
        Effect::MouseMove(dx, dy) => Some(Raw::Mouse { dx: *dx, dy: *dy, data: 0, flags: MOUSEEVENTF_MOVE }),
        Effect::Wheel(amount) => Some(Raw::Mouse { dx: 0, dy: 0, data: *amount, flags: MOUSEEVENTF_WHEEL }),
        Effect::App(_) => None,
    }
}

/// Sends to Windows what the effects ask for, in order. Does nothing when there is nothing to do.
pub fn apply(effects: &[Effect]) {
    let raw: Vec<Raw> = effects.iter().filter_map(translate).collect();
    if !raw.is_empty() {
        send(&raw);
    }
}

#[cfg(windows)]
fn send(events: &[Raw]) {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
        KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, MOUSEINPUT,
    };

    let inputs: Vec<INPUT> = events
        .iter()
        .map(|raw| match *raw {
            Raw::Key { vk, up, extended, media } => {
                let mut flags = 0;
                if extended {
                    flags |= KEYEVENTF_EXTENDEDKEY;
                }
                if up {
                    flags |= KEYEVENTF_KEYUP;
                }
                // Games read the scan code, so it is sent, except for media keys, which have none
                // and only go through their virtual-key code.
                let (vk, scan) = if media {
                    (vk, 0)
                } else {
                    // SAFETY: a plain code conversion, without any pointer.
                    let scan = unsafe { MapVirtualKeyW(u32::from(vk), 0) } as u16;
                    flags |= KEYEVENTF_SCANCODE;
                    (0, scan)
                };
                INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
                }
            }
            Raw::Mouse { dx, dy, data, flags } => INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 { mi: MOUSEINPUT { dx, dy, mouseData: data as u32, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
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
fn send(_events: &[Raw]) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bindings::AppAction;
    use crate::input::XTarget;

    fn key(k: &str) -> Output {
        Output::Key { key: k.into() }
    }

    #[test]
    fn a_key_press_and_release_translate_to_down_and_up() {
        assert_eq!(
            translate(&Effect::Down(key("A"))),
            Some(Raw::Key { vk: 0x41, up: false, extended: false, media: false })
        );
        assert_eq!(
            translate(&Effect::Up(key("A"))),
            Some(Raw::Key { vk: 0x41, up: true, extended: false, media: false })
        );
    }

    #[test]
    fn arrows_are_extended_and_media_keys_are_flagged() {
        let Some(Raw::Key { extended, media, .. }) = translate(&Effect::Down(key("Left"))) else { panic!() };
        assert!(extended && !media);
        let Some(Raw::Key { media, .. }) = translate(&Effect::Down(key("VolumeUp"))) else { panic!() };
        assert!(media);
    }

    #[test]
    fn an_unknown_key_sends_nothing() {
        assert_eq!(translate(&Effect::Down(key("Hyper"))), None);
    }

    #[test]
    fn mouse_buttons_use_their_own_flags() {
        let left = Output::Mouse { button: MouseButton::Left };
        assert_eq!(translate(&Effect::Down(left.clone())), Some(mouse(MOUSEEVENTF_LEFTDOWN)));
        assert_eq!(translate(&Effect::Up(left)), Some(mouse(MOUSEEVENTF_LEFTUP)));
        let middle = Output::Mouse { button: MouseButton::Middle };
        assert_eq!(translate(&Effect::Down(middle)), Some(mouse(MOUSEEVENTF_MIDDLEDOWN)));
    }

    #[test]
    fn side_buttons_carry_which_button_they_are() {
        let back = translate(&Effect::Down(Output::Mouse { button: MouseButton::Back }));
        assert_eq!(back, Some(Raw::Mouse { dx: 0, dy: 0, data: XBUTTON1, flags: MOUSEEVENTF_XDOWN }));
        let forward = translate(&Effect::Up(Output::Mouse { button: MouseButton::Forward }));
        assert_eq!(forward, Some(Raw::Mouse { dx: 0, dy: 0, data: XBUTTON2, flags: MOUSEEVENTF_XUP }));
    }

    #[test]
    fn moves_and_wheel_translate() {
        assert_eq!(
            translate(&Effect::MouseMove(3, -4)),
            Some(Raw::Mouse { dx: 3, dy: -4, data: 0, flags: MOUSEEVENTF_MOVE })
        );
        assert_eq!(
            translate(&Effect::Wheel(-120)),
            Some(Raw::Mouse { dx: 0, dy: 0, data: -120, flags: MOUSEEVENTF_WHEEL })
        );
    }

    #[test]
    fn xbox_outputs_and_app_actions_do_not_reach_windows() {
        assert_eq!(translate(&Effect::Down(Output::Pad { target: XTarget::A })), None);
        assert_eq!(translate(&Effect::App(AppAction::ShowOverlay)), None);
    }
}
