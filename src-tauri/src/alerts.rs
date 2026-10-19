//! Low-battery alerts: warn once when the level becomes low, once more when it becomes critical,
//! and never repeat on every report.

use crate::controller::Charge;
use crate::i18n::Msg;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alert {
    /// Band 1 (10–20 %).
    Low,
    /// Band 0 (0–10 %).
    Critical,
}

/// Above this band (30 %) the alerts are armed again, so that a level hovering around a threshold
/// does not repeat the alert.
const REARM_LEVEL: u8 = 3;

#[derive(Default)]
pub struct BatteryAlert {
    low_sent: bool,
    critical_sent: bool,
}

impl BatteryAlert {
    /// To be called whenever the battery changes. `percent` is the middle of the reported band,
    /// as everywhere else in the application.
    pub fn update(&mut self, percent: Option<u8>, charge: Charge) -> Option<Alert> {
        // A controller that is charging is not in danger; re-arm for the next discharge.
        if matches!(charge, Charge::Charging | Charge::Full) {
            *self = Self::default();
            return None;
        }
        let level = percent? / 10;

        if level >= REARM_LEVEL {
            *self = Self::default();
            return None;
        }
        if level == 0 && !self.critical_sent {
            self.critical_sent = true;
            self.low_sent = true; // no "low" alert after a "critical" one
            return Some(Alert::Critical);
        }
        if level <= 1 && !self.low_sent {
            self.low_sent = true;
            return Some(Alert::Low);
        }
        None
    }
}

/// The overlay message for an alert, with the battery band (for example "10–20 %").
pub fn message(alert: Alert, percent: u8) -> Msg {
    let level = u16::from((percent / 10).min(10));
    let (from, to) = (level * 10, level * 10 + 10);
    match alert {
        Alert::Low => Msg::BatteryLow { from, to },
        Alert::Critical => Msg::BatteryCritical { from, to },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Lang;
    use Charge::*;

    #[test]
    fn healthy_battery_stays_quiet() {
        let mut a = BatteryAlert::default();
        for percent in [95, 85, 55, 35] {
            assert_eq!(a.update(Some(percent), Discharging), None);
        }
    }

    #[test]
    fn low_alert_fires_once_when_the_level_reaches_the_low_band() {
        let mut a = BatteryAlert::default();
        assert_eq!(a.update(Some(25), Discharging), None);
        assert_eq!(a.update(Some(15), Discharging), Some(Alert::Low));
        assert_eq!(a.update(Some(15), Discharging), None, "no repetition");
    }

    #[test]
    fn critical_alert_follows_the_low_one() {
        let mut a = BatteryAlert::default();
        assert_eq!(a.update(Some(15), Discharging), Some(Alert::Low));
        assert_eq!(a.update(Some(5), Discharging), Some(Alert::Critical));
        assert_eq!(a.update(Some(5), Discharging), None);
    }

    #[test]
    fn going_straight_to_critical_skips_the_low_alert() {
        let mut a = BatteryAlert::default();
        assert_eq!(a.update(Some(5), Discharging), Some(Alert::Critical));
        assert_eq!(a.update(Some(15), Discharging), None, "a low alert never follows a critical one");
    }

    #[test]
    fn a_controller_that_connects_already_low_warns_immediately() {
        let mut a = BatteryAlert::default();
        assert_eq!(a.update(Some(15), Discharging), Some(Alert::Low));
    }

    #[test]
    fn charging_silences_and_rearms() {
        let mut a = BatteryAlert::default();
        assert_eq!(a.update(Some(5), Discharging), Some(Alert::Critical));
        assert_eq!(a.update(Some(5), Charging), None);
        assert_eq!(a.update(Some(5), Discharging), Some(Alert::Critical), "re-armed by charging");
    }

    #[test]
    fn a_full_battery_never_alerts() {
        let mut a = BatteryAlert::default();
        assert_eq!(a.update(Some(100), Full), None);
    }

    #[test]
    fn hovering_around_a_threshold_does_not_repeat() {
        let mut a = BatteryAlert::default();
        assert_eq!(a.update(Some(15), Discharging), Some(Alert::Low));
        assert_eq!(a.update(Some(25), Discharging), None); // slightly higher: not re-armed
        assert_eq!(a.update(Some(15), Discharging), None);
    }

    #[test]
    fn recovering_above_thirty_percent_rearms() {
        let mut a = BatteryAlert::default();
        assert_eq!(a.update(Some(15), Discharging), Some(Alert::Low));
        assert_eq!(a.update(Some(45), Discharging), None);
        assert_eq!(a.update(Some(15), Discharging), Some(Alert::Low));
    }

    #[test]
    fn unknown_levels_and_errors_do_nothing() {
        let mut a = BatteryAlert::default();
        assert_eq!(a.update(None, Unknown), None);
        assert_eq!(a.update(None, Error), None);
        assert_eq!(a.update(Some(5), Unknown), Some(Alert::Critical), "known level, unknown state");
    }

    #[test]
    fn messages_show_the_range_in_both_languages() {
        assert_eq!(message(Alert::Low, 15).text_in(Lang::En), "Low battery (10–20%)");
        assert_eq!(message(Alert::Critical, 5).text_in(Lang::En), "Critical battery (0–10%)");
        assert_eq!(message(Alert::Low, 15).text_in(Lang::Fr), "Batterie faible (10–20 %)");
        assert_eq!(message(Alert::Critical, 5).text_in(Lang::Fr), "Batterie critique (0–10 %)");
    }
}
