//! Keyboard keys that rules can send to Windows.
//!
//! Every key has a stable identifier (the one stored in profiles), a label in each supported
//! language, its virtual-key code and an "extended" flag.

use crate::i18n::{self, Lang};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyDef {
    pub id: &'static str,
    /// The English label. Keys whose French label differs are listed in `french_label`.
    pub label: &'static str,
    /// Windows virtual-key code (VK_*).
    pub vk: u16,
    /// "Extended" keyboard key (arrows, Insert/Delete block…): changes the scan code.
    pub extended: bool,
    /// Media key: sent by virtual-key code only, without a scan code.
    pub media: bool,
}

const fn key(id: &'static str, label: &'static str, vk: u16) -> KeyDef {
    KeyDef { id, label, vk, extended: false, media: false }
}
const fn ext(id: &'static str, label: &'static str, vk: u16) -> KeyDef {
    KeyDef { id, label, vk, extended: true, media: false }
}
const fn media(id: &'static str, label: &'static str, vk: u16) -> KeyDef {
    KeyDef { id, label, vk, extended: true, media: true }
}

/// Every key offered, in display order.
pub const KEYS: &[KeyDef] = &[
    key("A", "A", 0x41), key("B", "B", 0x42), key("C", "C", 0x43), key("D", "D", 0x44),
    key("E", "E", 0x45), key("F", "F", 0x46), key("G", "G", 0x47), key("H", "H", 0x48),
    key("I", "I", 0x49), key("J", "J", 0x4A), key("K", "K", 0x4B), key("L", "L", 0x4C),
    key("M", "M", 0x4D), key("N", "N", 0x4E), key("O", "O", 0x4F), key("P", "P", 0x50),
    key("Q", "Q", 0x51), key("R", "R", 0x52), key("S", "S", 0x53), key("T", "T", 0x54),
    key("U", "U", 0x55), key("V", "V", 0x56), key("W", "W", 0x57), key("X", "X", 0x58),
    key("Y", "Y", 0x59), key("Z", "Z", 0x5A),
    key("0", "0", 0x30), key("1", "1", 0x31), key("2", "2", 0x32), key("3", "3", 0x33),
    key("4", "4", 0x34), key("5", "5", 0x35), key("6", "6", 0x36), key("7", "7", 0x37),
    key("8", "8", 0x38), key("9", "9", 0x39),
    key("F1", "F1", 0x70), key("F2", "F2", 0x71), key("F3", "F3", 0x72), key("F4", "F4", 0x73),
    key("F5", "F5", 0x74), key("F6", "F6", 0x75), key("F7", "F7", 0x76), key("F8", "F8", 0x77),
    key("F9", "F9", 0x78), key("F10", "F10", 0x79), key("F11", "F11", 0x7A), key("F12", "F12", 0x7B),
    key("Space", "Space", 0x20), key("Enter", "Enter", 0x0D), key("Escape", "Escape", 0x1B),
    key("Tab", "Tab", 0x09), key("Backspace", "Backspace", 0x08),
    ext("Delete", "Delete", 0x2E), ext("Insert", "Insert", 0x2D),
    ext("Home", "Home", 0x24), ext("End", "End", 0x23),
    ext("PageUp", "Page Up", 0x21), ext("PageDown", "Page Down", 0x22),
    ext("Up", "Up arrow", 0x26), ext("Down", "Down arrow", 0x28),
    ext("Left", "Left arrow", 0x25), ext("Right", "Right arrow", 0x27),
    key("Shift", "Shift", 0xA0), key("Ctrl", "Ctrl", 0xA2), key("Alt", "Alt", 0xA4),
    ext("Win", "Windows", 0x5B), key("CapsLock", "Caps Lock", 0x14),
    key("Semicolon", ";", 0xBA), key("Equals", "=", 0xBB), key("Comma", ",", 0xBC),
    key("Minus", "-", 0xBD), key("Period", ".", 0xBE), key("Slash", "/", 0xBF),
    key("Backtick", "`", 0xC0), key("LeftBracket", "[", 0xDB), key("Backslash", "\\", 0xDC),
    key("RightBracket", "]", 0xDD), key("Quote", "'", 0xDE),
    key("Num0", "Numpad 0", 0x60), key("Num1", "Numpad 1", 0x61), key("Num2", "Numpad 2", 0x62),
    key("Num3", "Numpad 3", 0x63), key("Num4", "Numpad 4", 0x64), key("Num5", "Numpad 5", 0x65),
    key("Num6", "Numpad 6", 0x66), key("Num7", "Numpad 7", 0x67), key("Num8", "Numpad 8", 0x68),
    key("Num9", "Numpad 9", 0x69), key("NumMultiply", "Numpad *", 0x6A),
    key("NumAdd", "Numpad +", 0x6B), key("NumSubtract", "Numpad -", 0x6D),
    key("NumDecimal", "Numpad .", 0x6E), ext("NumDivide", "Numpad /", 0x6F),
    ext("PrintScreen", "Print Screen", 0x2C),
    media("VolumeMute", "Volume mute", 0xAD), media("VolumeDown", "Volume down", 0xAE),
    media("VolumeUp", "Volume up", 0xAF), media("MediaNext", "Media next", 0xB0),
    media("MediaPrevious", "Media previous", 0xB1), media("MediaStop", "Media stop", 0xB2),
    media("MediaPlayPause", "Media play/pause", 0xB3),
];

/// The key with this identifier.
pub fn lookup(id: &str) -> Option<&'static KeyDef> {
    KEYS.iter().find(|k| k.id == id)
}

/// The French label of the keys whose label differs from the English one.
fn french_label(id: &str) -> Option<&'static str> {
    Some(match id {
        "Space" => "Espace",
        "Enter" => "Entrée",
        "Escape" => "Échap",
        "Tab" => "Tabulation",
        "Backspace" => "Retour arrière",
        "Delete" => "Suppr",
        "Insert" => "Inser",
        "Home" => "Début",
        "End" => "Fin",
        "PageUp" => "Page précédente",
        "PageDown" => "Page suivante",
        "Up" => "Flèche haut",
        "Down" => "Flèche bas",
        "Left" => "Flèche gauche",
        "Right" => "Flèche droite",
        "Shift" => "Maj",
        "CapsLock" => "Verr. maj",
        "Num0" => "Pavé num. 0",
        "Num1" => "Pavé num. 1",
        "Num2" => "Pavé num. 2",
        "Num3" => "Pavé num. 3",
        "Num4" => "Pavé num. 4",
        "Num5" => "Pavé num. 5",
        "Num6" => "Pavé num. 6",
        "Num7" => "Pavé num. 7",
        "Num8" => "Pavé num. 8",
        "Num9" => "Pavé num. 9",
        "NumMultiply" => "Pavé num. *",
        "NumAdd" => "Pavé num. +",
        "NumSubtract" => "Pavé num. -",
        "NumDecimal" => "Pavé num. ,",
        "NumDivide" => "Pavé num. /",
        "PrintScreen" => "Impr. écran",
        "VolumeMute" => "Volume : muet",
        "VolumeDown" => "Volume : moins",
        "VolumeUp" => "Volume : plus",
        "MediaNext" => "Média : suivant",
        "MediaPrevious" => "Média : précédent",
        "MediaStop" => "Média : stop",
        "MediaPlayPause" => "Média : lecture/pause",
        _ => return None,
    })
}

impl KeyDef {
    /// The label of the key in `lang`.
    pub fn label_in(&self, lang: Lang) -> &'static str {
        match lang {
            Lang::En => self.label,
            Lang::Fr => french_label(self.id).unwrap_or(self.label),
        }
    }
}

/// A key as the interface presents it.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct KeyInfo {
    pub id: &'static str,
    pub label: &'static str,
}

/// Every key, labelled in `lang`.
pub fn list_in(lang: Lang) -> Vec<KeyInfo> {
    KEYS.iter().map(|k| KeyInfo { id: k.id, label: k.label_in(lang) }).collect()
}

/// Every key, labelled in the current language.
pub fn list() -> Vec<KeyInfo> {
    list_in(i18n::language())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn identifiers_are_unique() {
        let ids: HashSet<_> = KEYS.iter().map(|k| k.id).collect();
        assert_eq!(ids.len(), KEYS.len());
    }

    #[test]
    fn virtual_key_codes_are_unique() {
        let codes: HashSet<_> = KEYS.iter().map(|k| k.vk).collect();
        assert_eq!(codes.len(), KEYS.len(), "two keys must not share a code");
    }

    #[test]
    fn letters_and_digits_use_their_ascii_code() {
        assert_eq!(lookup("W").unwrap().vk, b'W' as u16);
        assert_eq!(lookup("5").unwrap().vk, b'5' as u16);
    }

    #[test]
    fn arrows_are_extended_keys_and_letters_are_not() {
        assert!(lookup("Left").unwrap().extended);
        assert!(!lookup("A").unwrap().extended);
    }

    #[test]
    fn media_keys_are_flagged() {
        assert!(lookup("VolumeUp").unwrap().media);
        assert!(!lookup("Enter").unwrap().media);
    }

    #[test]
    fn unknown_keys_are_not_found() {
        assert!(lookup("Hyper").is_none());
        assert!(lookup("a").is_none(), "identifiers are case-sensitive");
    }

    #[test]
    fn the_list_keeps_the_declaration_order() {
        let list = list_in(Lang::En);
        assert_eq!(list.first().unwrap().id, "A");
        assert_eq!(list.len(), KEYS.len());
    }

    #[test]
    fn labels_follow_the_language() {
        let enter = lookup("Enter").unwrap();
        assert_eq!(enter.label_in(Lang::En), "Enter");
        assert_eq!(enter.label_in(Lang::Fr), "Entrée");
        let letter = lookup("A").unwrap();
        assert_eq!(letter.label_in(Lang::Fr), "A", "keys without a translation keep their label");
    }

    #[test]
    fn every_french_translation_belongs_to_a_known_key() {
        let translated = KEYS.iter().filter(|k| french_label(k.id).is_some()).count();
        // 15 navigation/editing keys, Shift, Caps Lock, 15 numpad keys, Print Screen and 7 media keys.
        assert_eq!(translated, 40);
    }
}
