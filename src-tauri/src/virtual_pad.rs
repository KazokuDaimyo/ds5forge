//! The virtual Xbox 360 controller (through the ViGEmBus driver).
//!
//! Games see a standard XInput controller. Its inputs come from the DualSense (after remapping
//! and stick tuning, see `input.rs`), and the vibrations requested by the game are sent back to
//! the DualSense motors.

use crate::bindings::PadHeld;
use crate::controller::{Command, ControllerState};
use crate::i18n::Msg;
use crate::input::{self, InputConfig, Mapped};
use std::sync::mpsc::Sender;
use vigem_client::{Client, TargetId, XButtons, XGamepad, Xbox360Wired};

pub struct VirtualPad {
    target: Xbox360Wired<Client>,
}

impl VirtualPad {
    /// Plugs in a virtual controller. `rumble` receives the vibrations requested by games.
    pub fn new(rumble: Sender<Command>) -> Result<Self, String> {
        let client = Client::connect().map_err(|e| Msg::ViGEmUnavailable { reason: e.to_string() }.text())?;
        let mut target = Xbox360Wired::new(client, TargetId::XBOX360_WIRED);
        target.plugin().map_err(|e| Msg::VirtualPadPlugFailed { reason: e.to_string() }.text())?;
        target.wait_ready().map_err(|e| Msg::VirtualPadNotReady { reason: e.to_string() }.text())?;

        let notifications = target
            .request_notification()
            .map_err(|e| Msg::VirtualPadRumbleUnavailable { reason: e.to_string() }.text())?;
        // The thread stops by itself when the virtual controller is unplugged.
        notifications.spawn_thread(move |_, data| {
            let _ = rumble.send(Command::Rumble {
                left: data.large_motor,
                right: data.small_motor,
            });
        });

        Ok(Self { target })
    }

    /// Sends the controller state. `consumed`: inputs taken by a rule. `extra_right`: movement of
    /// the right stick coming from the gyroscope. `held`: what rules press on the Xbox controller.
    pub fn update(&mut self, state: &ControllerState, consumed: u32, cfg: &InputConfig, extra_right: (f32, f32), held: &PadHeld) {
        let mapped = input::map_with(state, consumed, cfg, extra_right, held);
        let _ = self.target.update(&to_gamepad(&mapped));
    }
}

impl Drop for VirtualPad {
    fn drop(&mut self) {
        let _ = self.target.unplug();
    }
}

fn to_gamepad(m: &Mapped) -> XGamepad {
    XGamepad {
        buttons: XButtons(m.buttons),
        left_trigger: m.left_trigger,
        right_trigger: m.right_trigger,
        thumb_lx: m.left_x,
        thumb_ly: m.left_y,
        thumb_rx: m.right_x,
        thumb_ry: m.right_y,
    }
}

/// Whether the ViGEmBus driver is installed and reachable. Used by the diagnostics screen.
pub fn driver_check() -> Result<(), String> {
    Client::connect().map(|_| ()).map_err(|e| format!("{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_field_reaches_the_driver_structure() {
        let m = Mapped {
            buttons: 0x1234,
            left_trigger: 1,
            right_trigger: 2,
            left_x: -3,
            left_y: 4,
            right_x: -5,
            right_y: 6,
        };
        let g = to_gamepad(&m);
        assert_eq!(g.buttons.raw, 0x1234);
        assert_eq!((g.left_trigger, g.right_trigger), (1, 2));
        assert_eq!((g.thumb_lx, g.thumb_ly, g.thumb_rx, g.thumb_ry), (-3, 4, -5, 6));
    }
}
