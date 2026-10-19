//! Powering off a Bluetooth controller.
//!
//! HID has no command to switch a DualSense off. Like DS4Windows, this module asks the Windows
//! Bluetooth radio to drop the link, and the controller then powers itself off.

use crate::i18n::Msg;

/// Converts an identifier such as "E8:47:3A:11:22:33" into a 48-bit Bluetooth address, most
/// significant byte first (the layout of the Windows `BLUETOOTH_ADDRESS`).
pub fn parse_mac(id: &str) -> Option<u64> {
    let parts: Vec<&str> = id.split(':').collect();
    if parts.len() != 6 {
        return None;
    }
    parts.iter().try_fold(0u64, |acc, part| {
        if part.len() != 2 {
            return None;
        }
        Some(acc << 8 | u64::from(u8::from_str_radix(part, 16).ok()?))
    })
}

#[cfg(windows)]
pub fn disconnect(mac: u64) -> Result<(), String> {
    use std::ffi::c_void;
    use std::ptr::null_mut;
    use windows_sys::Win32::Devices::Bluetooth::{
        BluetoothFindFirstRadio, BluetoothFindNextRadio, BluetoothFindRadioClose,
        BLUETOOTH_FIND_RADIO_PARAMS,
    };
    use windows_sys::Win32::Foundation::{CloseHandle, FALSE};
    use windows_sys::Win32::System::IO::DeviceIoControl;

    // IOCTL_BTH_DISCONNECT_DEVICE (bthioctl.h) =
    // CTL_CODE(FILE_DEVICE_BLUETOOTH = 0x41, 3, METHOD_BUFFERED, FILE_ANY_ACCESS).
    const IOCTL_BTH_DISCONNECT_DEVICE: u32 = 0x0041_000C;

    // SAFETY: Win32 calls with pointers that are valid for the duration of each call. Every radio
    // handle obtained is closed exactly once, and the search handle is always closed.
    unsafe {
        let params = BLUETOOTH_FIND_RADIO_PARAMS {
            dwSize: size_of::<BLUETOOTH_FIND_RADIO_PARAMS>() as u32,
        };
        let mut radio = null_mut();
        let find = BluetoothFindFirstRadio(&params, &mut radio);
        if find.is_null() {
            return Err(Msg::NoBluetoothRadio.text());
        }

        let mut last_error = String::new();
        let mut done = false;
        loop {
            let mut returned = 0u32;
            let ok = DeviceIoControl(
                radio,
                IOCTL_BTH_DISCONNECT_DEVICE,
                &mac as *const u64 as *const c_void,
                size_of::<u64>() as u32,
                null_mut(),
                0,
                &mut returned,
                null_mut(),
            ) != FALSE;
            if !ok {
                last_error = std::io::Error::last_os_error().to_string();
            }
            CloseHandle(radio);

            if ok {
                done = true;
                break;
            }
            if BluetoothFindNextRadio(find, &mut radio) == FALSE {
                break;
            }
        }
        BluetoothFindRadioClose(find);

        if done {
            Ok(())
        } else {
            Err(Msg::BluetoothDisconnectRefused { reason: last_error }.text())
        }
    }
}

#[cfg(not(windows))]
pub fn disconnect(_mac: u64) -> Result<(), String> {
    Err(Msg::BluetoothPowerOffWindowsOnly.text())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_is_parsed_most_significant_byte_first() {
        assert_eq!(parse_mac("E8:47:3A:11:22:33"), Some(0xE8_47_3A_11_22_33));
        assert_eq!(parse_mac("00:00:00:00:00:01"), Some(1));
        assert_eq!(parse_mac("e8:47:3a:11:22:33"), Some(0xE8_47_3A_11_22_33));
    }

    #[test]
    fn malformed_identifiers_are_rejected() {
        assert_eq!(parse_mac(""), None);
        assert_eq!(parse_mac("E8:47:3A:11:22"), None); // too short
        assert_eq!(parse_mac("E8:47:3A:11:22:33:44"), None); // too long
        assert_eq!(parse_mac("E8:47:3A:11:22:GG"), None); // not hexadecimal
        assert_eq!(parse_mac("E8:47:3A:11:22:3"), None); // incomplete byte
        assert_eq!(parse_mac("\\\\?\\hid#vid_054c"), None); // fallback to the device path
    }
}
