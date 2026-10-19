//! Picking a profile automatically from the application in the foreground.
//!
//! A *game rule* pairs the name of an executable (`ForzaHorizon5.exe`) with a profile. While that
//! program's window is in the foreground its profile is used; about a second after the user leaves it
//! (for any other window, this application's included), the controllers go back to their default profile. Nothing is injected into, or read from, the games: only
//! the name of the foreground program is looked at, which needs no administrator rights.
//!
//! The decision logic (`Watcher`) is pure and tested; only `foreground` and `running_apps` talk to
//! Windows.

use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

/// Maximum number of rules.
pub const MAX_RULES: usize = 32;
const MAX_EXE_LEN: usize = 128;

/// "While this program is in the foreground, use this profile."
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct GameRule {
    /// File name of the executable, compared without regard to case.
    pub exe: String,
    pub preset: String,
}

/// The profile for a foreground program, if a rule matches it.
pub fn preset_for<'a>(rules: &'a [GameRule], exe: &str) -> Option<&'a str> {
    rules.iter().find(|r| r.exe.eq_ignore_ascii_case(exe)).map(|r| r.preset.as_str())
}

/// Brings rules coming from the interface to safe values: trimmed names, no empty or duplicate
/// executable, only profiles that exist, a bounded number of rules.
pub fn sanitize(rules: Vec<GameRule>, known_presets: &[&str]) -> Vec<GameRule> {
    let mut clean: Vec<GameRule> = Vec::new();
    for rule in rules {
        let exe: String = rule.exe.trim().chars().take(MAX_EXE_LEN).collect();
        if exe.is_empty()
            || !known_presets.contains(&rule.preset.as_str())
            || clean.iter().any(|r| r.exe.eq_ignore_ascii_case(&exe))
        {
            continue;
        }
        clean.push(GameRule { exe, preset: rule.preset });
        if clean.len() == MAX_RULES {
            break;
        }
    }
    clean
}

/// What is in the foreground.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Foreground {
    /// A window of this application.
    Own,
    /// Another program (`None` if its name cannot be read).
    App(Option<String>),
}

/// How long a non-game window must stay in the foreground before the controllers go back to their
/// default profile. Long enough for a quick Alt+Tab not to flicker the light bar, short enough not to
/// be noticed as a delay.
pub const GRACE: Duration = Duration::from_millis(1000);

/// Follows the foreground program and says when the automatic profile changes.
#[derive(Debug, Default)]
pub struct Watcher {
    current: Option<String>,
    left_at: Option<Instant>,
}

impl Watcher {
    /// The automatic profile, if one is in force.
    pub fn current(&self) -> Option<&str> {
        self.current.as_deref()
    }

    /// Feeds the foreground state. Returns `Some(new)` when the automatic profile changes (`new`
    /// being `None` when the controllers go back to their own profile), `None` otherwise.
    pub fn update(&mut self, foreground: &Foreground, rules: &[GameRule], now: Instant) -> Option<Option<String>> {
        // This application's own window is not a game: leaving the game for it ends the game profile
        // (a profile is edited through the selector, which does not depend on the game).
        let exe = match foreground {
            Foreground::Own => None,
            Foreground::App(exe) => exe.as_deref(),
        };
        let matched = exe.and_then(|exe| preset_for(rules, exe));
        match matched {
            Some(preset) => {
                self.left_at = None;
                if self.current.as_deref() == Some(preset) {
                    return None;
                }
                self.current = Some(preset.to_string());
                Some(self.current.clone())
            }
            None if self.current.is_some() => {
                let since = *self.left_at.get_or_insert(now);
                if now.duration_since(since) >= GRACE {
                    self.current = None;
                    self.left_at = None;
                    Some(None)
                } else {
                    None
                }
            }
            None => None,
        }
    }

    /// The rules changed: a profile that no rule gives any more must not stay in force.
    pub fn forget_unknown(&mut self, rules: &[GameRule]) -> Option<Option<String>> {
        let still_used = self.current.as_deref().is_some_and(|p| rules.iter().any(|r| r.preset == p));
        if self.current.is_some() && !still_used {
            self.current = None;
            self.left_at = None;
            return Some(None);
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------

/// The program whose window is in the foreground.
#[cfg(windows)]
pub fn foreground() -> Foreground {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

    // SAFETY: plain queries; the window handle is only passed back to Windows.
    let pid = unsafe {
        let window = GetForegroundWindow();
        if window.is_null() {
            return Foreground::App(None);
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(window, &mut pid);
        pid
    };
    if pid == std::process::id() {
        return Foreground::Own;
    }
    Foreground::App(exe_name(pid))
}

/// The file name of a process's executable, if it can be read (it cannot for some protected processes).
#[cfg(windows)]
fn exe_name(pid: u32) -> Option<String> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION};

    // SAFETY: the handle is closed before returning, and `buffer` outlives the call that fills it.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return None;
        }
        let mut buffer = [0u16; 1024];
        let mut length = buffer.len() as u32;
        let ok = QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut length);
        CloseHandle(process);
        if ok == 0 {
            return None;
        }
        let path = String::from_utf16_lossy(&buffer[..length as usize]);
        path.rsplit('\\').next().map(str::to_string)
    }
}

/// The programs that have a visible window, to pick a game from a list. Sorted, without duplicates,
/// and without this application.
#[cfg(windows)]
pub fn running_apps() -> Vec<String> {
    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowTextLengthW, GetWindowThreadProcessId, IsWindowVisible,
    };

    unsafe extern "system" fn collect(window: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: `lparam` is the pointer to the vector passed to `EnumWindows` below, valid for the whole call.
        unsafe {
            let pids = &mut *(lparam as *mut Vec<u32>);
            if IsWindowVisible(window) != 0 && GetWindowTextLengthW(window) > 0 {
                let mut pid = 0u32;
                GetWindowThreadProcessId(window, &mut pid);
                pids.push(pid);
            }
        }
        1
    }

    let mut pids: Vec<u32> = Vec::new();
    // SAFETY: the callback only runs during this call, with a pointer to `pids`.
    unsafe {
        EnumWindows(Some(collect), &mut pids as *mut Vec<u32> as LPARAM);
    }
    pids.sort_unstable();
    pids.dedup();
    let own = std::process::id();
    let mut names: Vec<String> = pids.into_iter().filter(|&pid| pid != own).filter_map(exe_name).collect();
    names.sort_by_key(|n| n.to_lowercase());
    names.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    names
}

#[cfg(not(windows))]
pub fn foreground() -> Foreground {
    Foreground::App(None)
}

#[cfg(not(windows))]
pub fn running_apps() -> Vec<String> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(exe: &str, preset: &str) -> GameRule {
        GameRule { exe: exe.into(), preset: preset.into() }
    }

    fn app(name: &str) -> Foreground {
        Foreground::App(Some(name.into()))
    }

    #[test]
    fn a_rule_matches_its_executable_without_regard_to_case() {
        let rules = [rule("ForzaHorizon5.exe", "Racing")];
        assert_eq!(preset_for(&rules, "forzahorizon5.EXE"), Some("Racing"));
        assert_eq!(preset_for(&rules, "chrome.exe"), None);
    }

    #[test]
    fn sanitizing_drops_empty_duplicate_and_unknown_rules() {
        let rules = vec![
            rule("  game.exe ", "A"),
            rule("GAME.EXE", "B"),
            rule("", "A"),
            rule("other.exe", "Missing"),
            rule("third.exe", "B"),
        ];
        let clean = sanitize(rules, &["A", "B"]);
        assert_eq!(clean, [rule("game.exe", "A"), rule("third.exe", "B")]);
    }

    #[test]
    fn sanitizing_bounds_the_number_of_rules() {
        let rules = (0..100).map(|i| rule(&format!("g{i}.exe"), "A")).collect();
        assert_eq!(sanitize(rules, &["A"]).len(), MAX_RULES);
    }

    #[test]
    fn the_profile_of_a_game_applies_as_soon_as_it_is_in_the_foreground() {
        let rules = [rule("game.exe", "Racing")];
        let mut w = Watcher::default();
        let t0 = Instant::now();
        assert_eq!(w.update(&app("chrome.exe"), &rules, t0), None);
        assert_eq!(w.update(&app("game.exe"), &rules, t0), Some(Some("Racing".into())));
        assert_eq!(w.update(&app("game.exe"), &rules, t0), None, "no change, nothing to report");
        assert_eq!(w.current(), Some("Racing"));
    }

    #[test]
    fn leaving_the_game_only_goes_back_after_the_grace_period() {
        let rules = [rule("game.exe", "Racing")];
        let mut w = Watcher::default();
        let t0 = Instant::now();
        w.update(&app("game.exe"), &rules, t0);
        assert_eq!(w.update(&app("chrome.exe"), &rules, t0 + Duration::from_millis(200)), None);
        assert_eq!(w.update(&app("chrome.exe"), &rules, t0 + Duration::from_millis(700)), None);
        assert_eq!(w.update(&app("chrome.exe"), &rules, t0 + Duration::from_millis(1300)), Some(None));
        assert_eq!(w.current(), None);
    }

    #[test]
    fn a_quick_alt_tab_does_not_change_the_profile() {
        let rules = [rule("game.exe", "Racing")];
        let mut w = Watcher::default();
        let t0 = Instant::now();
        let ms = Duration::from_millis;
        w.update(&app("game.exe"), &rules, t0);
        w.update(&app("discord.exe"), &rules, t0 + ms(300));
        assert_eq!(w.update(&app("game.exe"), &rules, t0 + ms(700)), None, "back before the grace period ends");
        // The grace period starts again on the next departure.
        w.update(&app("discord.exe"), &rules, t0 + ms(2000));
        assert_eq!(w.update(&app("discord.exe"), &rules, t0 + ms(2600)), None);
        assert_eq!(w.update(&app("discord.exe"), &rules, t0 + ms(3100)), Some(None));
    }

    #[test]
    fn this_application_in_the_foreground_ends_the_game_profile_like_any_other_window() {
        let rules = [rule("game.exe", "Racing")];
        let mut w = Watcher::default();
        let t0 = Instant::now();
        w.update(&app("game.exe"), &rules, t0);
        assert_eq!(w.update(&Foreground::Own, &rules, t0 + Duration::from_millis(300)), None);
        assert_eq!(w.update(&Foreground::Own, &rules, t0 + Duration::from_millis(1500)), Some(None));
        assert_eq!(w.current(), None);
    }

    #[test]
    fn going_from_one_game_to_another_switches_directly() {
        let rules = [rule("a.exe", "A"), rule("b.exe", "B")];
        let mut w = Watcher::default();
        let t0 = Instant::now();
        w.update(&app("a.exe"), &rules, t0);
        assert_eq!(w.update(&app("b.exe"), &rules, t0 + Duration::from_secs(1)), Some(Some("B".into())));
    }

    #[test]
    fn an_unreadable_program_counts_as_not_a_game() {
        let rules = [rule("game.exe", "Racing")];
        let mut w = Watcher::default();
        let t0 = Instant::now();
        w.update(&app("game.exe"), &rules, t0);
        w.update(&Foreground::App(None), &rules, t0 + Duration::from_millis(100));
        assert_eq!(w.update(&Foreground::App(None), &rules, t0 + Duration::from_secs(8)), Some(None));
    }

    #[test]
    fn removing_the_rule_of_the_profile_in_force_drops_it() {
        let rules = [rule("game.exe", "Racing")];
        let mut w = Watcher::default();
        w.update(&app("game.exe"), &rules, Instant::now());
        assert_eq!(w.forget_unknown(&rules), None);
        assert_eq!(w.forget_unknown(&[]), Some(None));
        assert_eq!(w.current(), None);
    }
}
