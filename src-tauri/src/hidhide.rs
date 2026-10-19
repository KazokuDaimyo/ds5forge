//! Hiding the physical controller from games through HidHide (https://github.com/nefarius/HidHide).
//!
//! In Xbox 360 mode, games must only see the virtual controller: otherwise they also read the
//! DualSense and the inputs are doubled. HidHide is driven through its command line
//! (`HidHideCLI.exe`), and **every change is verified by reading HidHide's state back**, rather
//! than trusting the program's exit code.

use crate::i18n::Msg;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// What the controller manager expects from a hiding system.
pub trait Cloak: Send + Sync {
    /// Hides the device identified by its HID symbolic link (`DeviceInfo::path`) and returns its
    /// instance path, to be kept so that the device can be unhidden even once unplugged.
    fn hide(&self, symbolic_link: &str) -> Result<String, String>;
    /// Unhides a device. Does nothing if it is not hidden.
    fn unhide(&self, instance_path: &str) -> Result<(), String>;
    /// Unhides a device that is still plugged in, found by its symbolic link. Used to recover from
    /// a hide left behind by a previous session (application killed before its cleanup).
    fn unhide_link(&self, symbolic_link: &str) -> Result<(), String>;
    fn status(&self) -> CloakStatus;
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct CloakStatus {
    pub installed: bool,
    /// Whether hiding is globally active. `None` if HidHide could not be queried.
    pub cloak_active: Option<bool>,
    /// Whether DS5Forge can see the hidden devices.
    pub app_registered: Option<bool>,
    pub error: Option<String>,
}

/// Runs `HidHideCLI`: returns its standard output, or an error message.
pub trait Cli: Send + Sync {
    fn run(&self, args: &[&str]) -> Result<String, String>;
}

pub struct HidHide {
    cli: Box<dyn Cli>,
    /// Path of this executable: it must be registered with HidHide to see hidden devices.
    app_exe: String,
}

struct State {
    apps: Vec<String>,
    hidden: Vec<String>,
    cloak_on: bool,
}

fn contains_ci(list: &[String], value: &str) -> bool {
    list.iter().any(|v| v.eq_ignore_ascii_case(value))
}

/// Extracts the quoted argument of `--flag "value"` lines.
fn quoted_values(output: &str, flag: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| line.trim().strip_prefix(flag))
        .filter_map(|rest| {
            let start = rest.find('"')? + 1;
            let end = rest.rfind('"')?;
            (end >= start).then(|| rest[start..end].to_string())
        })
        .collect()
}

#[derive(Deserialize)]
struct GamingGroup {
    devices: Vec<GamingDevice>,
}

#[derive(Deserialize)]
struct GamingDevice {
    #[serde(rename = "symbolicLink")]
    symbolic_link: String,
    #[serde(rename = "deviceInstancePath")]
    instance_path: String,
}

/// HidHideCLI refuses to run without sufficient rights and says so with exit code 31
/// ("Access is denied", Windows error 0x0005).
fn is_access_denied(error: &str) -> bool {
    let error = error.to_lowercase();
    error.contains("access is denied") || error.contains("0x0005") || error.contains("exit code: 31")
}

/// The advice that fits an access refusal: either rights are missing, or another program holds
/// HidHide. Its control device can only be opened by one program at a time, so a refusal while
/// already running as administrator means that another program has it open.
fn denied_message() -> String {
    let elevated = crate::diagnostics::is_elevated();
    Msg::HidHideAccessDenied { elevated }.text()
}

/// The message used when HidHide did not apply a change without reporting an error. If the
/// application already runs as administrator, advising to run it as administrator would be wrong:
/// the message says so instead, and gives the details of what HidHide reports.
fn not_applied(detail: &str) -> String {
    let elevated = crate::diagnostics::is_elevated();
    Msg::HidHideNotApplied { elevated, detail: detail.to_string() }.text()
}

impl HidHide {
    pub fn new(cli: Box<dyn Cli>, app_exe: String) -> Self {
        Self { cli, app_exe }
    }

    /// Runs HidHideCLI. An access refusal (for reads as well as writes: HidHide may refuse both
    /// without sufficient rights) becomes actionable advice instead of the raw program output.
    fn run(&self, args: &[&str]) -> Result<String, String> {
        self.cli.run(args).map_err(|error| {
            if is_access_denied(&error) { denied_message() } else { error }
        })
    }

    /// A command that modifies HidHide's configuration.
    fn write(&self, args: &[&str]) -> Result<(), String> {
        self.run(args).map(|_| ())
    }

    fn state(&self) -> Result<State, String> {
        Ok(State {
            apps: quoted_values(&self.run(&["--app-list"])?, "--app-reg"),
            hidden: quoted_values(&self.run(&["--dev-list"])?, "--dev-hide"),
            cloak_on: self.run(&["--cloak-state"])?.contains("--cloak-on"),
        })
    }

    /// The instance path that HidHide expects, found from the device's symbolic link.
    fn instance_of(&self, symbolic_link: &str) -> Result<String, String> {
        let out = self.run(&["--dev-gaming"])?;
        let groups: Vec<GamingGroup> = serde_json::from_str(out.trim())
            .map_err(|e| Msg::HidHideUnreadableResponse { reason: e.to_string() }.text())?;
        groups
            .into_iter()
            .flat_map(|g| g.devices)
            .find(|d| d.symbolic_link.eq_ignore_ascii_case(symbolic_link))
            .map(|d| d.instance_path)
            .ok_or_else(|| Msg::HidHideUnknownController.text())
    }
}

impl Cloak for HidHide {
    fn hide(&self, symbolic_link: &str) -> Result<String, String> {
        let instance = self.instance_of(symbolic_link)?;
        let before = self.state()?;

        // Only ask for what is missing: no needless rewriting of HidHide's configuration.
        let mut args: Vec<&str> = Vec::new();
        if !contains_ci(&before.apps, &self.app_exe) {
            args.extend(["--app-reg", &self.app_exe]);
        }
        if !contains_ci(&before.hidden, &instance) {
            args.extend(["--dev-hide", &instance]);
        }
        if !before.cloak_on {
            args.push("--cloak-on");
        }
        if args.is_empty() {
            return Ok(instance);
        }

        self.write(&args)?;

        let after = self.state()?;
        let app_ok = contains_ci(&after.apps, &self.app_exe);
        let hidden_ok = contains_ci(&after.hidden, &instance);
        if app_ok && hidden_ok && after.cloak_on {
            return Ok(instance);
        }
        // Say precisely what did not take, along with what HidHide reports: without it, a genuine
        // rights problem could not be told apart from a format difference (path, case…).
        let detail = Msg::HidHideApplyDetail {
            app_ok,
            hidden_ok,
            cloak_on: after.cloak_on,
            expected_app: self.app_exe.clone(),
            expected_device: instance,
            apps: format!("{:?}", after.apps),
            devices: format!("{:?}", after.hidden),
        }
        .text();
        Err(not_applied(&detail))
    }

    fn unhide(&self, instance_path: &str) -> Result<(), String> {
        if !contains_ci(&self.state()?.hidden, instance_path) {
            return Ok(());
        }
        self.write(&["--dev-unhide", instance_path])?;
        let after = self.state()?;
        if contains_ci(&after.hidden, instance_path) {
            let detail = Msg::HidHideStillHidden {
                device: instance_path.to_string(),
                devices: format!("{:?}", after.hidden),
            }
            .text();
            return Err(not_applied(&detail));
        }
        Ok(())
    }

    fn unhide_link(&self, symbolic_link: &str) -> Result<(), String> {
        match self.instance_of(symbolic_link) {
            Ok(instance) => self.unhide(&instance),
            // Unknown to HidHide: it cannot have been hidden, so there is nothing to do.
            Err(_) => Ok(()),
        }
    }

    fn status(&self) -> CloakStatus {
        match self.state() {
            Ok(state) => CloakStatus {
                installed: true,
                cloak_active: Some(state.cloak_on),
                app_registered: Some(contains_ci(&state.apps, &self.app_exe)),
                error: None,
            },
            Err(error) => CloakStatus {
                installed: true,
                cloak_active: None,
                app_registered: None,
                error: Some(error),
            },
        }
    }
}

/// Used when HidHide is not installed.
pub struct NoCloak;

impl Cloak for NoCloak {
    fn hide(&self, _: &str) -> Result<String, String> {
        Err(Msg::HidHideNotInstalled.text())
    }
    fn unhide(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn unhide_link(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn status(&self) -> CloakStatus {
        CloakStatus { installed: false, cloak_active: None, app_registered: None, error: None }
    }
}

struct ProcessCli {
    exe: std::path::PathBuf,
}

impl Cli for ProcessCli {
    fn run(&self, args: &[&str]) -> Result<String, String> {
        let mut command = std::process::Command::new(&self.exe);
        command.args(args);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no flashing console
        }
        let output = command
            .output()
            .map_err(|e| Msg::HidHideCannotRun { reason: e.to_string() }.text())?;
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        if output.status.success() {
            Ok(stdout)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let detail = if stderr.trim().is_empty() { stdout.trim().to_string() } else { stderr.trim().to_string() };
            Err(Msg::HidHideCliFailed { status: output.status.to_string(), detail }.text())
        }
    }
}

/// The path of HidHideCLI on this computer, if HidHide is installed.
fn cli_path() -> Option<std::path::PathBuf> {
    let relative = r"Nefarius Software Solutions\HidHide\x64\HidHideCLI.exe";
    ["ProgramW6432", "ProgramFiles"]
        .iter()
        .filter_map(|var| std::env::var_os(var))
        .map(|dir| std::path::Path::new(&dir).join(relative))
        .find(|path| path.is_file())
}

/// Whether HidHide is installed right now (unlike `detect`, which is only evaluated at start-up).
pub fn is_installed() -> bool {
    cli_path().is_some()
}

/// Looks for HidHide on this computer; returns `NoCloak` if it is absent.
pub fn detect() -> Arc<dyn Cloak> {
    let exe = cli_path();

    match (exe, std::env::current_exe()) {
        (Some(exe), Ok(app)) => {
            Arc::new(HidHide::new(Box::new(ProcessCli { exe }), app.to_string_lossy().into_owned()))
        }
        _ => Arc::new(NoCloak),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    const LINK: &str = r"\\?\hid#{00001124-0000-1000-8000-00805f9b34fb}_vid&0002054c_pid&0ce6#8&aaaaaaaa&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}";
    const INSTANCE: &str = r"HID\{00001124-0000-1000-8000-00805f9b34fb}_VID&0002054c_PID&0ce6\8&aaaaaaaa&0&0000";
    const APP: &str = r"C:\Tools\ds5forge.exe";

    /// A real `--dev-gaming` output (identifiers altered).
    fn gaming_json() -> String {
        format!(
            r#" [ {{ "friendlyName" : "Sony Interactive Entertainment DualSense Wireless Controller" , "devices" : [
{{ "present" : true ,
"gamingDevice" : true ,
"symbolicLink" : "{}" ,
"vendor" : "Sony Interactive Entertainment" ,
"product" : "DualSense Wireless Controller" ,
"serialNumber" : "aabbccddeeff" ,
"usage" : "Gamepad" ,
"description" : "HID-compliant game controller" ,
"deviceInstancePath" : "{}" ,
"baseContainerDeviceInstancePath" : "BTHENUM\\{{00001124}}" ,
"baseContainerClassGuid" : "{{745A17A0-74D3-11D0-B6FE-00A0C90F57DA}}" ,
"baseContainerDeviceCount" : 1 }} ] }} ]
"#,
            LINK.replace('\\', "\\\\"),
            INSTANCE.replace('\\', "\\\\"),
        )
    }

    /// Simulates HidHide. With `read_only`, writes are ignored (access denied).
    #[derive(Default)]
    struct FakeState {
        apps: Vec<String>,
        hidden: Vec<String>,
        cloak_on: bool,
        read_only: bool,
        device_present: bool,
        calls: Vec<Vec<String>>,
    }

    struct FakeCli(Arc<Mutex<FakeState>>);

    impl Cli for FakeCli {
        fn run(&self, args: &[&str]) -> Result<String, String> {
            let mut s = self.0.lock().unwrap();
            s.calls.push(args.iter().map(|a| a.to_string()).collect());
            let mut i = 0;
            let mut out = String::new();
            while i < args.len() {
                match args[i] {
                    "--dev-gaming" => {
                        out = if s.device_present { gaming_json() } else { "[ ]".into() };
                    }
                    "--app-list" => {
                        out = s.apps.iter().map(|a| format!("--app-reg \"{a}\"\n")).collect();
                    }
                    "--dev-list" => {
                        out = s.hidden.iter().map(|d| format!("--dev-hide \"{d}\"\n")).collect();
                    }
                    "--cloak-state" => out = if s.cloak_on { "--cloak-on".into() } else { "--cloak-off".into() },
                    "--app-reg" => {
                        i += 1;
                        if !s.read_only { s.apps.push(args[i].to_string()); }
                    }
                    "--dev-hide" => {
                        i += 1;
                        if !s.read_only { s.hidden.push(args[i].to_string()); }
                    }
                    "--dev-unhide" => {
                        i += 1;
                        if !s.read_only { s.hidden.retain(|d| !d.eq_ignore_ascii_case(args[i])); }
                    }
                    "--cloak-on" => { if !s.read_only { s.cloak_on = true; } }
                    other => return Err(format!("unknown command: {other}")),
                }
                i += 1;
            }
            Ok(out)
        }
    }

    fn hidhide(state: FakeState) -> (HidHide, Arc<Mutex<FakeState>>) {
        let state = Arc::new(Mutex::new(FakeState { device_present: true, ..state }));
        (HidHide::new(Box::new(FakeCli(state.clone())), APP.into()), state)
    }

    fn writes(state: &Arc<Mutex<FakeState>>) -> Vec<Vec<String>> {
        state.lock().unwrap().calls.iter()
            .filter(|c| c.iter().any(|a| matches!(a.as_str(), "--app-reg" | "--dev-hide" | "--dev-unhide" | "--cloak-on")))
            .cloned().collect()
    }

    #[test]
    fn parses_the_real_cli_output_formats() {
        let apps = quoted_values("--app-reg \"C:\\a b\\x.exe\"\n--app-reg \"D:\\y.exe\"\n", "--app-reg");
        assert_eq!(apps, [r"C:\a b\x.exe", r"D:\y.exe"]);
        assert!(quoted_values("", "--dev-hide").is_empty());
        assert!(quoted_values("--dev-hide", "--dev-hide").is_empty()); // no quotes
    }

    #[test]
    fn hide_registers_the_app_hides_the_device_and_turns_the_cloak_on() {
        let (h, state) = hidhide(FakeState::default());
        assert_eq!(h.hide(LINK).unwrap(), INSTANCE);

        let s = state.lock().unwrap();
        assert!(contains_ci(&s.apps, APP));
        assert!(contains_ci(&s.hidden, INSTANCE));
        assert!(s.cloak_on);
    }

    #[test]
    fn hide_sends_all_changes_in_one_cli_call() {
        let (h, state) = hidhide(FakeState::default());
        h.hide(LINK).unwrap();
        assert_eq!(writes(&state).len(), 1, "a single run for the three changes");
    }

    #[test]
    fn hide_only_asks_for_what_is_missing() {
        let (h, state) = hidhide(FakeState { apps: vec![APP.into()], cloak_on: true, ..Default::default() });
        h.hide(LINK).unwrap();
        let calls = writes(&state);
        assert_eq!(calls, [vec!["--dev-hide".to_string(), INSTANCE.to_string()]]);
    }

    #[test]
    fn hide_is_a_no_op_when_everything_is_already_in_place() {
        let (h, state) = hidhide(FakeState {
            apps: vec![APP.to_lowercase()], // case does not matter
            hidden: vec![INSTANCE.to_uppercase()],
            cloak_on: true,
            ..Default::default()
        });
        assert_eq!(h.hide(LINK).unwrap(), INSTANCE);
        assert!(writes(&state).is_empty());
    }

    #[test]
    fn hide_reports_a_denied_write_instead_of_pretending_it_worked() {
        let (h, state) = hidhide(FakeState { read_only: true, ..Default::default() });
        let error = h.hide(LINK).unwrap_err();
        assert!(error.contains("administrator"), "{error}");
        assert!(state.lock().unwrap().hidden.is_empty());
    }

    /// The real HidHideCLI without administrator rights: reads allowed, writes failing (code 31).
    struct DeniedCli(FakeCli);

    impl Cli for DeniedCli {
        fn run(&self, args: &[&str]) -> Result<String, String> {
            if args.iter().any(|a| matches!(*a, "--app-reg" | "--dev-hide" | "--dev-unhide" | "--cloak-on")) {
                return Err("HidHideCLI failed (exit code: 31): Error code 0x0005 at \
                            src\\FilterDriverProxy.cpp(33) `anonymous-namespace'::Device: Access is denied."
                    .into());
            }
            self.0.run(args)
        }
    }

    #[test]
    fn a_cli_failure_with_access_denied_becomes_the_administrator_advice() {
        let state = Arc::new(Mutex::new(FakeState { device_present: true, ..Default::default() }));
        let h = HidHide::new(Box::new(DeniedCli(FakeCli(state.clone()))), APP.into());
        let error = h.hide(LINK).unwrap_err();
        assert!(error.contains("administrator"), "{error}");
        assert!(!error.contains("0x0005"), "no raw output: {error}");

        state.lock().unwrap().hidden.push(INSTANCE.into());
        assert!(h.unhide(INSTANCE).unwrap_err().contains("administrator"));
    }

    /// HidHide refusing even reads (its control device cannot be opened without sufficient rights).
    struct DeniedEverythingCli;

    impl Cli for DeniedEverythingCli {
        fn run(&self, _: &[&str]) -> Result<String, String> {
            Err("HidHideCLI failed (exit code: 31): Error code 0x0005 at \
                 src\\FilterDriverProxy.cpp(33) `anonymous-namespace'::Device: Access is denied."
                .into())
        }
    }

    #[test]
    fn access_denied_on_reads_also_becomes_the_administrator_advice() {
        let h = HidHide::new(Box::new(DeniedEverythingCli), APP.into());
        let error = h.hide(LINK).unwrap_err();
        assert!(error.contains("administrator"), "{error}");
        assert!(!error.contains("0x0005"), "no raw output: {error}");
        assert!(h.unhide(INSTANCE).unwrap_err().contains("administrator"));
        assert!(h.status().error.unwrap().contains("administrator"));
    }

    #[test]
    fn another_cli_failure_is_reported_as_is() {
        assert!(!is_access_denied("HidHideCLI failed (exit code: 1): driver not found"));
        assert!(is_access_denied("... Access is denied."));
    }

    #[test]
    fn hide_fails_for_a_device_hidhide_does_not_know() {
        let (h, _) = hidhide(FakeState::default());
        assert!(h.hide(r"\\?\hid#other").unwrap_err().contains("not recognized"));
    }

    #[test]
    fn hide_fails_cleanly_when_the_device_list_is_not_json() {
        let state = Arc::new(Mutex::new(FakeState { device_present: false, ..Default::default() }));
        let h = HidHide::new(Box::new(FakeCli(state)), APP.into());
        assert!(h.hide(LINK).is_err());
    }

    #[test]
    fn unhide_removes_only_our_device() {
        let other = r"HID\VID_1234&PID_5678\9&bbbb&0&0000".to_string();
        let (h, state) = hidhide(FakeState { hidden: vec![INSTANCE.into(), other.clone()], ..Default::default() });
        h.unhide(INSTANCE).unwrap();
        assert_eq!(state.lock().unwrap().hidden, [other]);
    }

    #[test]
    fn unhide_works_after_the_controller_was_unplugged() {
        let (h, state) = hidhide(FakeState { hidden: vec![INSTANCE.into()], ..Default::default() });
        state.lock().unwrap().device_present = false; // the instance comes from the cache, not from HidHide
        h.unhide(INSTANCE).unwrap();
        assert!(state.lock().unwrap().hidden.is_empty());
    }

    #[test]
    fn unhide_does_nothing_when_the_device_is_not_hidden() {
        let (h, state) = hidhide(FakeState::default());
        h.unhide(INSTANCE).unwrap();
        assert!(writes(&state).is_empty());
    }

    #[test]
    fn unhide_reports_a_denied_write() {
        let (h, _) = hidhide(FakeState { hidden: vec![INSTANCE.into()], read_only: true, ..Default::default() });
        assert!(h.unhide(INSTANCE).unwrap_err().contains("administrator"));
    }

    #[test]
    fn unhide_link_releases_a_device_hidden_by_a_previous_session() {
        let (h, state) = hidhide(FakeState { hidden: vec![INSTANCE.into()], cloak_on: true, ..Default::default() });
        h.unhide_link(LINK).unwrap();
        assert!(state.lock().unwrap().hidden.is_empty());
    }

    #[test]
    fn unhide_link_leaves_other_devices_and_unknown_ones_alone() {
        let other = r"HID\VID_1234&PID_5678\9&bbbb&0&0000".to_string();
        let (h, state) = hidhide(FakeState { hidden: vec![other.clone()], ..Default::default() });
        h.unhide_link(LINK).unwrap(); // not hidden: nothing to do
        h.unhide_link(r"\\?\hid#other").unwrap(); // unknown to HidHide: not an error
        assert_eq!(state.lock().unwrap().hidden, [other]);
        assert!(writes(&state).is_empty());
    }

    #[test]
    fn hide_never_turns_the_cloak_off() {
        let (h, state) = hidhide(FakeState::default());
        h.hide(LINK).unwrap();
        h.unhide(INSTANCE).unwrap();
        assert!(state.lock().unwrap().cloak_on, "the global cloak belongs to the user");
    }

    #[test]
    fn status_reports_cloak_and_registration() {
        let (h, _) = hidhide(FakeState { cloak_on: true, apps: vec![APP.into()], ..Default::default() });
        assert_eq!(
            h.status(),
            CloakStatus { installed: true, cloak_active: Some(true), app_registered: Some(true), error: None }
        );
        let (h, _) = hidhide(FakeState::default());
        let status = h.status();
        assert_eq!((status.cloak_active, status.app_registered), (Some(false), Some(false)));
    }

    #[test]
    fn without_hidhide_hiding_fails_but_unhiding_is_harmless() {
        assert!(NoCloak.hide(LINK).unwrap_err().contains("not installed"));
        assert!(NoCloak.unhide(INSTANCE).is_ok());
        assert!(!NoCloak.status().installed);
    }
}
