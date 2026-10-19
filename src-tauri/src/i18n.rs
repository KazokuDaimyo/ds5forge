//! Interface language of the messages produced by the backend.
//!
//! Every user-facing sentence that the backend generates (overlay messages, errors returned to the
//! interface, tray menu, diagnostics log lines) is a variant of [`Msg`], rendered in English or
//! French. The language is process-wide: the interface sets it at start-up and whenever the user
//! changes it. Strings that only exist on the front-end live in `src/i18n.ts`.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    #[default]
    En,
    Fr,
}

impl Lang {
    fn to_u8(self) -> u8 {
        match self {
            Self::En => 0,
            Self::Fr => 1,
        }
    }

    fn from_u8(value: u8) -> Self {
        if value == 1 { Self::Fr } else { Self::En }
    }
}

#[cfg(not(test))]
static CURRENT: AtomicU8 = AtomicU8::new(0);

// Tests run in parallel threads: each one gets its own language so that they cannot interfere.
#[cfg(test)]
thread_local! {
    static CURRENT: AtomicU8 = const { AtomicU8::new(0) };
}

/// The language messages are currently rendered in.
pub fn language() -> Lang {
    #[cfg(not(test))]
    return Lang::from_u8(CURRENT.load(Ordering::Relaxed));
    #[cfg(test)]
    return CURRENT.with(|c| Lang::from_u8(c.load(Ordering::Relaxed)));
}

/// Changes the language of every message rendered from now on.
pub fn set_language(lang: Lang) {
    #[cfg(not(test))]
    CURRENT.store(lang.to_u8(), Ordering::Relaxed);
    #[cfg(test)]
    CURRENT.with(|c| c.store(lang.to_u8(), Ordering::Relaxed));
}

/// A user-facing message of the backend.
#[derive(Debug, Clone, PartialEq)]
pub enum Msg {
    // Tray menu
    TrayOpen,
    TrayQuit,

    // Overlay
    ModeXbox360,
    ModeNative,
    PoweringOff,
    MotionEnabled,
    MotionDisabled,
    BatteryLow { from: u16, to: u16 },
    BatteryCritical { from: u16, to: u16 },
    /// `base` followed by a notice that hiding the physical controller failed.
    WithHidHideError { base: String },

    // Controllers
    /// Default name of the `n`-th controller ever seen.
    ControllerName { n: usize },
    NotConnected,
    PowerOffUsb,
    UnknownBluetoothAddress,
    ControllerMissing,
    NoBluetoothRadio,
    BluetoothDisconnectRefused { reason: String },
    // Only produced on other platforms than Windows.
    #[cfg_attr(windows, allow(dead_code))]
    BluetoothPowerOffWindowsOnly,
    #[cfg_attr(windows, allow(dead_code))]
    AutostartWindowsOnly,

    // Virtual controller
    ViGEmUnavailable { reason: String },
    VirtualPadPlugFailed { reason: String },
    VirtualPadNotReady { reason: String },
    VirtualPadRumbleUnavailable { reason: String },

    // HidHide
    HidHideAccessDenied { elevated: bool },
    HidHideNotApplied { elevated: bool, detail: String },
    HidHideUnreadableResponse { reason: String },
    HidHideUnknownController,
    HidHideApplyDetail {
        app_ok: bool,
        hidden_ok: bool,
        cloak_on: bool,
        expected_app: String,
        expected_device: String,
        apps: String,
        devices: String,
    },
    HidHideStillHidden { device: String, devices: String },
    HidHideNotInstalled,
    HidHideCannotRun { reason: String },
    HidHideCliFailed { status: String, detail: String },

    // UDP API
    UdpNotUtf8,
    UdpInvalidJson { reason: String },
    UdpNoMatchingController,
    UdpPortUnavailable { port: u16, reason: String },
    UdpListening { port: u16 },
    UdpPacketRejected { reason: String },

    // Diagnostics log
    LogConnected { id: String, link: &'static str },
    LogDisconnected { id: String },
    LogHidden { id: String },
    LogUnhidden { id: String },
    LogVirtualPadPlugged { id: String },
    LogVirtualPadUnplugged { id: String },
    LogWriteFailed { reason: String },
    LogSettingsUnreadable { reason: String },
    LogProfilesUnreadable { reason: String, backup: String },

    // Component installation
    InstallFailed { code: String, detail: String },
    UnknownComponent,

    // Updates
    NoUpdateAvailable,
}

fn pick(lang: Lang, en: String, fr: String) -> String {
    match lang {
        Lang::En => en,
        Lang::Fr => fr,
    }
}

impl Msg {
    /// The message in the current language.
    pub fn text(&self) -> String {
        self.text_in(language())
    }

    /// The message in `lang`.
    pub fn text_in(&self, lang: Lang) -> String {
        use Msg::*;
        let p = |en: String, fr: String| pick(lang, en, fr);
        match self {
            TrayOpen => p("Open DS5Forge".into(), "Ouvrir DS5Forge".into()),
            TrayQuit => p("Quit".into(), "Quitter".into()),

            ModeXbox360 => p("Xbox 360 mode".into(), "Mode Xbox 360".into()),
            ModeNative => p("Native mode".into(), "Mode natif".into()),
            PoweringOff => p("Powering off the controller…".into(), "Extinction de la manette…".into()),
            MotionEnabled => p("Gyroscope and touchpad: enabled".into(), "Gyroscope et pavé tactile : activés".into()),
            MotionDisabled => p("Gyroscope and touchpad: disabled".into(), "Gyroscope et pavé tactile : désactivés".into()),
            BatteryLow { from, to } => p(format!("Low battery ({from}–{to}%)"), format!("Batterie faible ({from}–{to} %)")),
            BatteryCritical { from, to } => {
                p(format!("Critical battery ({from}–{to}%)"), format!("Batterie critique ({from}–{to} %)"))
            }
            WithHidHideError { base } => p(format!("{base} · HidHide error"), format!("{base} · erreur HidHide")),

            ControllerName { n } => p(format!("Controller {n}"), format!("Manette {n}")),
            NotConnected => p("This controller is not connected".into(), "Cette manette n'est pas connectée".into()),
            PowerOffUsb => p(
                "Not possible over USB: the cable powers the controller.".into(),
                "Impossible en USB : la manette est alimentée par le câble.".into(),
            ),
            UnknownBluetoothAddress => p(
                "The controller's Bluetooth address is unknown, so it cannot be powered off".into(),
                "L'adresse Bluetooth de la manette est inconnue, extinction impossible".into(),
            ),
            ControllerMissing => p("Controller not found".into(), "Manette introuvable".into()),
            NoBluetoothRadio => p("No Bluetooth radio found on this PC".into(), "Aucune radio Bluetooth trouvée sur ce PC".into()),
            BluetoothDisconnectRefused { reason } => {
                p(format!("Windows refused the disconnection ({reason})"), format!("Windows a refusé la déconnexion ({reason})"))
            }
            BluetoothPowerOffWindowsOnly => p(
                "Bluetooth power-off is only available on Windows".into(),
                "L'extinction Bluetooth n'est disponible que sous Windows".into(),
            ),
            AutostartWindowsOnly => p(
                "Launching at startup is only supported on Windows".into(),
                "Le lancement au démarrage n'est géré que sous Windows".into(),
            ),

            ViGEmUnavailable { reason } => p(
                format!("ViGEmBus driver not found or not accessible ({reason}). It can be installed from Settings → Components."),
                format!("Pilote ViGEmBus introuvable ou inaccessible ({reason}). Il peut être installé depuis Réglages → Composants."),
            ),
            VirtualPadPlugFailed { reason } => {
                p(format!("Could not plug in the virtual controller ({reason})"), format!("Branchement impossible ({reason})"))
            }
            VirtualPadNotReady { reason } => {
                p(format!("Virtual controller not ready ({reason})"), format!("Manette virtuelle non prête ({reason})"))
            }
            VirtualPadRumbleUnavailable { reason } => {
                p(format!("Rumble feedback unavailable ({reason})"), format!("Retour de vibration indisponible ({reason})"))
            }

            HidHideAccessDenied { elevated: false } => p(
                "HidHide did not apply the change: DS5Forge probably needs to be run as administrator.".into(),
                "HidHide n'a pas appliqué le changement : il faut sans doute lancer DS5Forge en administrateur.".into(),
            ),
            HidHideAccessDenied { elevated: true } => p(
                "HidHide denied access although the app already runs as administrator: another program is probably \
                 using HidHide right now (HidHide Configuration Client, DSX…). Close it and try again."
                    .into(),
                "HidHide refuse l'accès alors que l'application est déjà en administrateur : un autre programme utilise \
                 sans doute HidHide en ce moment (HidHide Configuration Client, DSX…). Fermez-le puis réessayez."
                    .into(),
            ),
            HidHideNotApplied { elevated, detail } => {
                if *elevated {
                    p(
                        format!("HidHide did not apply the change although the app already runs as administrator. {detail}"),
                        format!("HidHide n'a pas appliqué le changement, alors que l'application est déjà en administrateur. {detail}"),
                    )
                } else {
                    format!("{} {detail}", HidHideAccessDenied { elevated: false }.text_in(lang))
                }
            }
            HidHideUnreadableResponse { reason } => {
                p(format!("Unreadable response from HidHide ({reason})"), format!("Réponse de HidHide illisible ({reason})"))
            }
            HidHideUnknownController => p(
                "This controller is not recognized by HidHide".into(),
                "Cette manette n'est pas reconnue par HidHide".into(),
            ),
            HidHideApplyDetail { app_ok, hidden_ok, cloak_on, expected_app, expected_device, apps, devices } => p(
                format!(
                    "app registered: {app_ok}, controller hidden: {hidden_ok}, cloak active: {cloak_on} | \
                     expected: app “{expected_app}”, device “{expected_device}” | \
                     HidHide reports: apps {apps}, hidden devices {devices}"
                ),
                format!(
                    "application déclarée : {app_ok}, manette masquée : {hidden_ok}, masquage actif : {cloak_on} | \
                     attendu : application « {expected_app} », appareil « {expected_device} » | \
                     HidHide rapporte : applications {apps}, appareils masqués {devices}"
                ),
            ),
            HidHideStillHidden { device, devices } => p(
                format!("controller “{device}” is still hidden | HidHide reports: hidden devices {devices}"),
                format!("la manette « {device} » est toujours masquée | HidHide rapporte : appareils masqués {devices}"),
            ),
            HidHideNotInstalled => p("HidHide is not installed".into(), "HidHide n'est pas installé".into()),
            HidHideCannotRun { reason } => {
                p(format!("Could not run HidHideCLI ({reason})"), format!("Impossible de lancer HidHideCLI ({reason})"))
            }
            HidHideCliFailed { status, detail } => {
                p(format!("HidHideCLI failed ({status}): {detail}"), format!("HidHideCLI a échoué ({status}) : {detail}"))
            }

            UdpNotUtf8 => p("packet is not valid UTF-8".into(), "le paquet n'est pas de l'UTF-8 valide".into()),
            UdpInvalidJson { reason } => p(format!("invalid JSON: {reason}"), format!("JSON invalide : {reason}")),
            UdpNoMatchingController => {
                p("no connected controller matches".into(), "aucune manette connectée ne correspond".into())
            }
            UdpPortUnavailable { port, reason } => {
                p(format!("Port {port} unavailable ({reason})"), format!("Port {port} indisponible ({reason})"))
            }
            UdpListening { port } => p(format!("listening on 127.0.0.1:{port}"), format!("à l'écoute sur 127.0.0.1:{port}")),
            UdpPacketRejected { reason } => p(format!("packet rejected: {reason}"), format!("paquet refusé : {reason}")),

            LogConnected { id, link } => p(format!("{id} connected ({link})"), format!("{id} connectée ({link})")),
            LogDisconnected { id } => p(format!("{id} disconnected"), format!("{id} déconnectée")),
            LogHidden { id } => p(format!("{id} hidden from games"), format!("{id} masquée aux jeux")),
            LogUnhidden { id } => p(format!("{id} visible to games again"), format!("{id} démasquée")),
            LogVirtualPadPlugged { id } => {
                p(format!("virtual controller plugged in for {id}"), format!("manette virtuelle de {id} branchée"))
            }
            LogVirtualPadUnplugged { id } => {
                p(format!("virtual controller of {id} unplugged"), format!("manette virtuelle de {id} débranchée"))
            }
            LogWriteFailed { reason } => p(format!("write failed ({reason})"), format!("écriture impossible ({reason})")),
            LogSettingsUnreadable { reason } => p(
                format!("file unreadable ({reason}); default values used"),
                format!("fichier illisible ({reason}) : valeurs par défaut utilisées"),
            ),
            LogProfilesUnreadable { reason, backup } => p(
                format!("file unreadable ({reason}); a copy was kept at {backup}"),
                format!("fichier illisible ({reason}) ; copie conservée dans {backup}"),
            ),

            InstallFailed { code, detail } => p(
                format!("Installation failed (code {code}): {detail}"),
                format!("L'installation a échoué (code {code}) : {detail}"),
            ),
            UnknownComponent => p("Unknown component".into(), "Composant inconnu".into()),
            NoUpdateAvailable => p("No update is available".into(), "Aucune mise à jour disponible".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_language_is_english() {
        assert_eq!(language(), Lang::En);
        assert_eq!(Msg::TrayQuit.text(), "Quit");
    }

    #[test]
    fn changing_the_language_changes_every_message() {
        set_language(Lang::Fr);
        assert_eq!(language(), Lang::Fr);
        assert_eq!(Msg::TrayQuit.text(), "Quitter");
        set_language(Lang::En);
        assert_eq!(Msg::TrayQuit.text(), "Quit");
    }

    #[test]
    fn languages_serialize_as_lowercase_codes() {
        assert_eq!(serde_json::to_string(&Lang::Fr).unwrap(), "\"fr\"");
        assert_eq!(serde_json::from_str::<Lang>("\"en\"").unwrap(), Lang::En);
    }

    #[test]
    fn hidhide_advice_depends_on_elevation() {
        let plain = Msg::HidHideAccessDenied { elevated: false }.text_in(Lang::En);
        let busy = Msg::HidHideAccessDenied { elevated: true }.text_in(Lang::En);
        assert!(plain.contains("administrator") && !plain.contains("another program"));
        assert!(busy.contains("another program"));
    }

    #[test]
    fn a_message_with_parameters_differs_between_languages_but_keeps_them() {
        let msg = Msg::UdpPortUnavailable { port: 26760, reason: "busy".into() };
        let (en, fr) = (msg.text_in(Lang::En), msg.text_in(Lang::Fr));
        assert_ne!(en, fr);
        for text in [en, fr] {
            assert!(text.contains("26760") && text.contains("busy"));
        }
    }
}
