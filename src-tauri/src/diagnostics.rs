//! Event log and diagnostic report: everything needed to understand why something does not work
//! without guessing (drivers, privileges, the state of each controller, the latest errors).

use crate::controller::Charge;
use crate::hidhide::CloakStatus;
use crate::manager::Settings;
use crate::udp::UdpStatus;
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// Maximum number of entries kept; the oldest ones are forgotten.
const CAPACITY: usize = 200;

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Info,
    Warn,
    Error,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Entry {
    /// Milliseconds since 1970 (the interface converts them to local time).
    pub time_ms: u64,
    pub level: Level,
    /// A stable identifier of the component that logged the entry (for example "hidhide"); the
    /// interface translates it into a label.
    pub source: String,
    pub message: String,
    /// How many times in a row this message was logged.
    pub count: u32,
}

/// In-memory log shared between all threads.
#[derive(Clone, Default)]
pub struct Log(Arc<Mutex<VecDeque<Entry>>>);

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

impl Log {
    pub fn push(&self, level: Level, source: &str, message: impl Into<String>) {
        let message = message.into();
        let mut entries = self.0.lock().unwrap_or_else(|e| e.into_inner());

        // A repeated message does not fill the log: repetitions are counted instead.
        if let Some(last) = entries.back_mut() {
            if last.level == level && last.source == source && last.message == message {
                last.count += 1;
                last.time_ms = now_ms();
                return;
            }
        }
        if entries.len() == CAPACITY {
            entries.pop_front();
        }
        entries.push_back(Entry { time_ms: now_ms(), level, source: source.to_string(), message, count: 1 });
    }

    pub fn info(&self, source: &str, message: impl Into<String>) {
        self.push(Level::Info, source, message);
    }
    pub fn warn(&self, source: &str, message: impl Into<String>) {
        self.push(Level::Warn, source, message);
    }
    pub fn error(&self, source: &str, message: impl Into<String>) {
        self.push(Level::Error, source, message);
    }

    pub fn entries(&self) -> Vec<Entry> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).iter().cloned().collect()
    }

    pub fn clear(&self) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct AppInfo {
    pub version: String,
    pub exe: Option<String>,
    pub config_dir: Option<String>,
    /// Whether the application runs with administrator rights. Relevant for HidHide.
    pub elevated: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct DriverState {
    pub ok: bool,
    pub message: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct ControllerDiag {
    pub id: String,
    pub name: String,
    pub connected: bool,
    pub bluetooth: bool,
    pub path: Option<String>,
    pub battery: Option<u8>,
    pub charge: Charge,
    /// Input reports received per second (about 250 over USB; a much lower value indicates a problem).
    pub reports_per_second: u32,
    pub virtual_pad: bool,
    /// Whether HidHide currently hides the physical controller from games.
    pub hidden: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct Report {
    pub app: AppInfo,
    pub vigem: DriverState,
    pub hidhide: CloakStatus,
    pub settings: Settings,
    pub controllers: Vec<ControllerDiag>,
    pub udp: UdpStatus,
    pub log: Vec<Entry>,
}

/// Whether the application runs with administrator rights.
#[cfg(windows)]
pub fn is_elevated() -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    // SAFETY: Win32 calls with valid pointers; the opened token is closed again.
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut returned = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            (&mut elevation as *mut TOKEN_ELEVATION).cast(),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        ) != 0;
        CloseHandle(token);
        ok && elevation.TokenIsElevated != 0
    }
}

#[cfg(not(windows))]
pub fn is_elevated() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_keep_their_order() {
        let log = Log::default();
        log.info("a", "one");
        log.warn("b", "two");
        log.error("c", "three");
        let messages: Vec<_> = log.entries().into_iter().map(|e| e.message).collect();
        assert_eq!(messages, ["one", "two", "three"]);
    }

    #[test]
    fn levels_and_sources_are_kept() {
        let log = Log::default();
        log.error("hidhide", "refused");
        let e = &log.entries()[0];
        assert_eq!((e.level, e.source.as_str(), e.count), (Level::Error, "hidhide", 1));
    }

    #[test]
    fn identical_consecutive_messages_are_counted_not_repeated() {
        let log = Log::default();
        for _ in 0..50 {
            log.warn("udp", "invalid JSON");
        }
        let entries = log.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].count, 50);
    }

    #[test]
    fn a_different_message_in_between_breaks_the_run() {
        let log = Log::default();
        log.warn("udp", "x");
        log.info("other", "y");
        log.warn("udp", "x");
        assert_eq!(log.entries().len(), 3);
    }

    #[test]
    fn the_same_text_from_another_source_or_level_is_a_new_entry() {
        let log = Log::default();
        log.warn("udp", "x");
        log.warn("hidhide", "x");
        log.error("hidhide", "x");
        assert_eq!(log.entries().len(), 3);
    }

    #[test]
    fn the_oldest_entries_are_dropped_when_full() {
        let log = Log::default();
        for i in 0..(CAPACITY + 25) {
            log.info("test", format!("message {i}"));
        }
        let entries = log.entries();
        assert_eq!(entries.len(), CAPACITY);
        assert_eq!(entries[0].message, "message 25");
        assert_eq!(entries.last().unwrap().message, format!("message {}", CAPACITY + 24));
    }

    #[test]
    fn clearing_empties_the_log() {
        let log = Log::default();
        log.info("a", "b");
        log.clear();
        assert!(log.entries().is_empty());
    }

    #[test]
    fn clones_share_the_same_log() {
        let log = Log::default();
        let other = log.clone();
        other.info("a", "visible everywhere");
        assert_eq!(log.entries().len(), 1);
    }

    #[test]
    fn timestamps_are_plausible_epoch_milliseconds() {
        let log = Log::default();
        log.info("a", "b");
        // After 2020: prevents a clock stuck at zero from going unnoticed.
        assert!(log.entries()[0].time_ms > 1_577_836_800_000);
    }

    #[test]
    fn the_levels_serialize_in_snake_case() {
        assert_eq!(serde_json::to_string(&Level::Warn).unwrap(), "\"warn\"");
    }
}
