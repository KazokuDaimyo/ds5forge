//! Encoding of the DualSense adaptive trigger effects.
//!
//! Each trigger receives an 11-byte block: a mode byte followed by parameters. The trigger travel
//! is split into 10 zones (0 = released, 9 = fully pressed). Strengths are encoded on 3 bits per
//! zone (a value of 1..=8 is stored as 0..=7).

use serde::{Deserialize, Serialize};

pub type Effect = [u8; 11];

const MODE_OFF: u8 = 0x05;
const MODE_FEEDBACK: u8 = 0x21;
const MODE_WEAPON: u8 = 0x25;
const MODE_VIBRATION: u8 = 0x26;

const OFF: Effect = [MODE_OFF, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TriggerEffect {
    Off,
    /// Constant resistance from zone `start` (0..=9) with strength `strength` (1..=8).
    Feedback { start: u8, strength: u8 },
    /// Weapon click: engages at `start` (2..=7) and releases at `end` (start+1..=8).
    Weapon { start: u8, end: u8, strength: u8 },
    /// Vibration from zone `start` (0..=9), amplitude 1..=8, frequency in Hz (1..=255).
    Vibration { start: u8, amplitude: u8, frequency: u8 },
}

/// The mask of active zones and the packed strengths, from zone `start` to the last one.
fn zones(start: u8, strength: u8) -> (u16, u32) {
    let force = u32::from(strength - 1) & 0x07;
    let mut active = 0u16;
    let mut forces = 0u32;
    for zone in start..10 {
        forces |= force << (3 * zone);
        active |= 1 << zone;
    }
    (active, forces)
}

impl TriggerEffect {
    /// Encodes the effect. Invalid parameters produce an "off" effect rather than an error, so that
    /// a badly set slider can never send a corrupted report.
    pub fn encode(self) -> Effect {
        match self {
            TriggerEffect::Off => OFF,

            TriggerEffect::Feedback { start, strength } => {
                if start > 9 || !(1..=8).contains(&strength) {
                    return OFF;
                }
                let (active, forces) = zones(start, strength);
                let f = forces.to_le_bytes();
                [
                    MODE_FEEDBACK, active as u8, (active >> 8) as u8,
                    f[0], f[1], f[2], f[3], 0, 0, 0, 0,
                ]
            }

            TriggerEffect::Weapon { start, end, strength } => {
                if !(2..=7).contains(&start) || end <= start || end > 8 || !(1..=8).contains(&strength) {
                    return OFF;
                }
                let zones = (1u16 << start) | (1u16 << end);
                [
                    MODE_WEAPON, zones as u8, (zones >> 8) as u8,
                    strength - 1, 0, 0, 0, 0, 0, 0, 0,
                ]
            }

            TriggerEffect::Vibration { start, amplitude, frequency } => {
                if start > 9 || !(1..=8).contains(&amplitude) || frequency == 0 {
                    return OFF;
                }
                let (active, forces) = zones(start, amplitude);
                let f = forces.to_le_bytes();
                [
                    MODE_VIBRATION, active as u8, (active >> 8) as u8,
                    f[0], f[1], f[2], f[3], 0, 0, frequency, 0,
                ]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_uses_mode_5() {
        assert_eq!(TriggerEffect::Off.encode()[0], 0x05);
    }

    #[test]
    fn feedback_full_range_max_strength() {
        let e = TriggerEffect::Feedback { start: 0, strength: 8 }.encode();
        // 10 active zones = 0x03FF; strength 7 on 3 bits x 10 zones = 0x3FFFFFFF
        assert_eq!(e[..7], [0x21, 0xFF, 0x03, 0xFF, 0xFF, 0xFF, 0x3F]);
    }

    #[test]
    fn feedback_starts_at_zone_5() {
        let e = TriggerEffect::Feedback { start: 5, strength: 4 }.encode();
        // zones 5..=9 -> 0x03E0; strength 3 in zones 5..=9
        assert_eq!(e[1], 0xE0);
        assert_eq!(e[2], 0x03);
        let forces = u32::from_le_bytes([e[3], e[4], e[5], e[6]]);
        assert_eq!(forces, 0b011_011_011_011_011 << 15);
    }

    #[test]
    fn weapon_encodes_start_end_and_strength() {
        let e = TriggerEffect::Weapon { start: 2, end: 5, strength: 8 }.encode();
        assert_eq!(e[..4], [0x25, 0b0010_0100, 0x00, 7]);
    }

    #[test]
    fn vibration_puts_frequency_in_byte_9() {
        let e = TriggerEffect::Vibration { start: 3, amplitude: 8, frequency: 40 }.encode();
        assert_eq!(e[0], 0x26);
        assert_eq!(e[9], 40);
    }

    #[test]
    fn invalid_parameters_fall_back_to_off() {
        assert_eq!(TriggerEffect::Feedback { start: 10, strength: 3 }.encode(), OFF);
        assert_eq!(TriggerEffect::Feedback { start: 0, strength: 0 }.encode(), OFF);
        assert_eq!(TriggerEffect::Weapon { start: 5, end: 4, strength: 3 }.encode(), OFF);
        assert_eq!(TriggerEffect::Weapon { start: 1, end: 4, strength: 3 }.encode(), OFF);
        assert_eq!(TriggerEffect::Vibration { start: 0, amplitude: 3, frequency: 0 }.encode(), OFF);
    }
}
