//! Installation of the optional components: the ViGEmBus driver (Xbox 360 mode) and HidHide
//! (hiding the physical controller from games).
//!
//! Nothing is downloaded or run by this application itself. The installation is delegated to
//! `winget`, Windows' package manager, which fetches the packages from its own repository and makes
//! Windows ask for administrator permission. Where `winget` is missing, the official download
//! page is opened in the browser instead.

use crate::i18n::Msg;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Component {
    ViGEmBus,
    HidHide,
}

impl Component {
    /// The component for an identifier sent by the interface.
    pub fn parse(id: &str) -> Option<Self> {
        match id {
            "vigembus" => Some(Self::ViGEmBus),
            "hidhide" => Some(Self::HidHide),
            _ => None,
        }
    }

    fn winget_id(self) -> &'static str {
        match self {
            Self::ViGEmBus => "ViGEm.ViGEmBus",
            Self::HidHide => "Nefarius.HidHide",
        }
    }

    fn releases_url(self) -> &'static str {
        match self {
            Self::ViGEmBus => "https://github.com/nefarius/ViGEmBus/releases",
            Self::HidHide => "https://github.com/nefarius/HidHide/releases",
        }
    }
}

/// What happened, for the interface.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    /// The installer ran to the end (or the component was already installed).
    Installed,
    /// `winget` is not available: the download page was opened in the browser.
    OpenedPage,
}

fn winget_args(component: Component) -> Vec<&'static str> {
    vec![
        "install",
        "--id",
        component.winget_id(),
        "--exact",
        "--silent",
        "--accept-package-agreements",
        "--accept-source-agreements",
    ]
}

/// `APPINSTALLER_CLI_ERROR_PACKAGE_ALREADY_INSTALLED`.
const ALREADY_INSTALLED: u32 = 0x8A15_0061;

/// Turns the exit code of `winget` into a result.
fn interpret(code: Option<i32>, output: &str) -> Result<(), String> {
    match code {
        Some(0) => Ok(()),
        Some(c) if c as u32 == ALREADY_INSTALLED => Ok(()),
        other => {
            let code = other.map_or_else(|| "?".to_string(), |c| format!("0x{:08X}", c as u32));
            let detail = output.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string();
            Err(Msg::InstallFailed { code, detail }.text())
        }
    }
}

/// Runs `winget`. `NotFound` means that it is not installed.
fn run_winget(component: Component) -> std::io::Result<(Option<i32>, String)> {
    let mut command = std::process::Command::new("winget");
    command.args(winget_args(component));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no flashing console
    }
    let output = command.output()?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok((output.status.code(), text))
}

/// Installs `component`. `open_page` opens a URL in the browser (the fallback without `winget`).
/// Blocking: the installer takes a while.
pub fn install(component: Component, open_page: impl Fn(&str) -> Result<(), String>) -> Result<Outcome, String> {
    match run_winget(component) {
        Ok((code, output)) => interpret(code, &output).map(|()| Outcome::Installed),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            open_page(component.releases_url())?;
            Ok(Outcome::OpenedPage)
        }
        Err(e) => Err(Msg::InstallFailed { code: "?".into(), detail: e.to_string() }.text()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_from_the_interface_are_checked() {
        assert_eq!(Component::parse("vigembus"), Some(Component::ViGEmBus));
        assert_eq!(Component::parse("hidhide"), Some(Component::HidHide));
        assert_eq!(Component::parse("calc.exe"), None);
    }

    #[test]
    fn only_known_packages_are_ever_requested() {
        let args = winget_args(Component::HidHide);
        assert!(args.contains(&"Nefarius.HidHide") && args.contains(&"--exact"));
        assert!(winget_args(Component::ViGEmBus).contains(&"ViGEm.ViGEmBus"));
    }

    #[test]
    fn success_and_already_installed_both_count_as_installed() {
        assert!(interpret(Some(0), "").is_ok());
        assert!(interpret(Some(0x8A15_0061_u32 as i32), "").is_ok());
    }

    #[test]
    fn a_failure_reports_the_code_and_the_last_line_of_output() {
        let error = interpret(Some(0x8A15_0010_u32 as i32), "Installing…\nThe installer failed.\n\n").unwrap_err();
        assert!(error.contains("0x8A150010") && error.contains("The installer failed."), "{error}");
        assert!(interpret(None, "").is_err());
    }
}
