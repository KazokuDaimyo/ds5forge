//! Short confirmation vibrations (mode change, alert…), so that an action can be felt without
//! looking at the screen.
//!
//! The module is pure: it receives the time and says whether the motor should be running.

use std::time::{Duration, Instant};

/// Motor strength during a pulse (0..=255).
pub const STRENGTH: u8 = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pattern {
    /// One short pulse: Xbox 360 mode enabled.
    Single,
    /// Two short pulses: native mode.
    Double,
    /// One long pulse: alert or error.
    Long,
}

impl Pattern {
    /// The pulses, as (start, end) offsets from the trigger.
    fn pulses(self) -> &'static [(u64, u64)] {
        match self {
            Pattern::Single => &[(0, 120)],
            Pattern::Double => &[(0, 100), (200, 300)],
            Pattern::Long => &[(0, 450)],
        }
    }
}

#[derive(Default)]
pub struct Haptics {
    /// Scheduled pulses as absolute (start, end) times.
    pulses: Vec<(Instant, Instant)>,
}

impl Haptics {
    pub fn start(&mut self, pattern: Pattern, now: Instant) {
        // A new pattern replaces the previous one: overlapping vibrations are never accumulated.
        self.pulses = pattern
            .pulses()
            .iter()
            .map(|&(from, to)| (now + Duration::from_millis(from), now + Duration::from_millis(to)))
            .collect();
    }

    /// Whether the motor should be running at `now`. Finished pulses are forgotten.
    pub fn is_on(&mut self, now: Instant) -> bool {
        self.pulses.retain(|&(_, end)| now < end);
        self.pulses.iter().any(|&(start, end)| now >= start && now < end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: fn(u64) -> Duration = Duration::from_millis;

    #[test]
    fn nothing_vibrates_by_default() {
        let mut h = Haptics::default();
        assert!(!h.is_on(Instant::now()));
    }

    #[test]
    fn a_single_pulse_is_short() {
        let (mut h, t0) = (Haptics::default(), Instant::now());
        h.start(Pattern::Single, t0);
        assert!(h.is_on(t0));
        assert!(h.is_on(t0 + MS(119)));
        assert!(!h.is_on(t0 + MS(120)));
    }

    #[test]
    fn a_double_pulse_has_a_gap_between_the_two() {
        let (mut h, t0) = (Haptics::default(), Instant::now());
        h.start(Pattern::Double, t0);
        assert!(h.is_on(t0 + MS(50)));
        assert!(!h.is_on(t0 + MS(150)), "gap between the two pulses");
        assert!(h.is_on(t0 + MS(250)));
        assert!(!h.is_on(t0 + MS(300)));
    }

    #[test]
    fn the_long_pattern_outlasts_the_others() {
        let (mut h, t0) = (Haptics::default(), Instant::now());
        h.start(Pattern::Long, t0);
        assert!(h.is_on(t0 + MS(400)));
        assert!(!h.is_on(t0 + MS(450)));
    }

    #[test]
    fn a_new_pattern_replaces_the_running_one() {
        let (mut h, t0) = (Haptics::default(), Instant::now());
        h.start(Pattern::Long, t0);
        h.start(Pattern::Single, t0 + MS(10));
        assert!(!h.is_on(t0 + MS(200)), "the old long pulse must not survive");
    }

    #[test]
    fn finished_pulses_are_forgotten() {
        let (mut h, t0) = (Haptics::default(), Instant::now());
        h.start(Pattern::Double, t0);
        h.is_on(t0 + MS(1000));
        assert!(h.pulses.is_empty());
    }
}
