//! Local UDP server: lets a game, a mod or a script drive the controllers.
//!
//! Every datagram is a JSON object whose fields are all optional:
//! ```json
//! {
//!   "controller": "Living room",
//!   "lightbar": [255, 0, 0],
//!   "player_leds": 31,
//!   "rumble": { "left": 0, "right": 128 },
//!   "triggers": {
//!     "left":  { "kind": "feedback", "start": 3, "strength": 6 },
//!     "right": { "kind": "weapon", "start": 3, "end": 6, "strength": 8 }
//!   }
//! }
//! ```
//! See `UDP.md` for the full description.

use crate::controller::Command;
use crate::events::Events;
use crate::i18n::Msg;
use crate::manager::{Controllers, Selector};
use crate::triggers::TriggerEffect;
use serde::{Deserialize, Serialize};
use std::net::UdpSocket;
use std::time::{Duration, Instant};

pub const PORT: u16 = 6970;
const MAX_DATAGRAM: usize = 4096;
/// Minimum interval between two notifications sent to the interface.
const NOTIFY_INTERVAL: Duration = Duration::from_millis(100);
/// A game that keeps sending invalid packets must not flood the log.
const LOG_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Deserialize, Default)]
struct Message {
    /// The targeted controller: index, identifier or name. Absent means every connected controller.
    controller: Option<Selector>,
    /// `true`: first re-apply the saved profile (useful when the game exits).
    reset: Option<bool>,
    lightbar: Option<[u8; 3]>,
    player_leds: Option<u8>,
    rumble: Option<RumbleMessage>,
    triggers: Option<TriggersMessage>,
}

#[derive(Deserialize)]
struct RumbleMessage {
    left: u8,
    right: u8,
}

#[derive(Deserialize)]
struct TriggersMessage {
    left: Option<TriggerEffect>,
    right: Option<TriggerEffect>,
}

impl Message {
    fn commands(&self) -> Vec<Command> {
        let mut cmds = Vec::new();
        if let Some([r, g, b]) = self.lightbar {
            cmds.push(Command::Lightbar(r, g, b));
        }
        if let Some(mask) = self.player_leds {
            cmds.push(Command::PlayerLeds(mask));
        }
        if let Some(RumbleMessage { left, right }) = self.rumble {
            cmds.push(Command::Rumble { left, right });
        }
        if let Some(t) = &self.triggers {
            if let Some(effect) = t.left {
                cmds.push(Command::Trigger { right: false, effect: effect.encode() });
            }
            if let Some(effect) = t.right {
                cmds.push(Command::Trigger { right: true, effect: effect.encode() });
            }
        }
        cmds
    }
}

/// Decodes a datagram and sends the commands to the targeted controllers.
fn handle_datagram(bytes: &[u8], controllers: &Controllers) -> Result<(), String> {
    let text = std::str::from_utf8(bytes).map_err(|_| Msg::UdpNotUtf8.text())?;
    let message: Message =
        serde_json::from_str(text).map_err(|e| Msg::UdpInvalidJson { reason: e.to_string() }.text())?;

    let targets = controllers.resolve(message.controller.as_ref());
    if targets.is_empty() {
        return Err(Msg::UdpNoMatchingController.text());
    }
    let commands = message.commands();
    for id in &targets {
        if message.reset == Some(true) {
            controllers.reset_to_profile(id);
        }
        for cmd in &commands {
            controllers.send(id, *cmd);
        }
    }
    Ok(())
}

/// The receive loop. `report` is called for every packet (raw text and the error, if any).
fn serve(socket: UdpSocket, controllers: Controllers, mut report: impl FnMut(&[u8], Option<&str>)) {
    let mut buf = [0u8; MAX_DATAGRAM];
    loop {
        let n = match socket.recv(&mut buf) {
            Ok(n) => n,
            Err(_) => {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
        };
        let bytes = &buf[..n];
        let result = handle_datagram(bytes, &controllers);
        report(bytes, result.as_ref().err().map(String::as_str));
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct UdpStatus {
    pub listening: bool,
    pub port: u16,
    pub error: Option<String>,
}

#[derive(Serialize, Clone)]
struct PacketEvent {
    text: String,
    error: Option<String>,
}

/// Starts the server. The returned status is kept by Tauri (see `get_udp_status`), because the
/// server starts before the interface listens for events.
pub fn start(events: Events, controllers: Controllers) -> UdpStatus {
    let socket = match UdpSocket::bind(("127.0.0.1", PORT)) {
        Ok(s) => s,
        Err(e) => {
            let message = Msg::UdpPortUnavailable { port: PORT, reason: e.to_string() }.text();
            controllers.log().error("udp", message.clone());
            return UdpStatus { listening: false, port: PORT, error: Some(message) };
        }
    };

    let log = controllers.log();
    log.info("udp", Msg::UdpListening { port: PORT }.text());
    let mut last_notify = Instant::now() - NOTIFY_INTERVAL;
    let mut last_log = Instant::now() - LOG_INTERVAL;
    std::thread::Builder::new()
        .name("udp-server".into())
        .spawn(move || {
            serve(socket, controllers, move |bytes, error| {
                if let Some(error) = error {
                    if last_log.elapsed() >= LOG_INTERVAL {
                        last_log = Instant::now();
                        log.warn("udp", Msg::UdpPacketRejected { reason: error.to_string() }.text());
                    }
                }
                // A game can send hundreds of packets per second: the display is throttled.
                if last_notify.elapsed() >= NOTIFY_INTERVAL {
                    last_notify = Instant::now();
                    let text = String::from_utf8_lossy(bytes).into_owned();
                    events.emit("udp-packet", &PacketEvent { text, error: error.map(String::from) });
                }
            });
        })
        .expect("failed to spawn the UDP server thread");

    UdpStatus { listening: true, port: PORT, error: None }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{self, Receiver};

    /// Two connected controllers: "AA" (named "Living room") and "BB".
    fn setup() -> (Controllers, Receiver<Command>, Receiver<Command>) {
        let c = Controllers::new(Events::none(), None);
        let (tx_a, rx_a) = mpsc::channel();
        let (tx_b, rx_b) = mpsc::channel();
        c.register("AA", "p-aa", false, tx_a);
        c.register("BB", "p-bb", false, tx_b);
        c.edit("AA", |p| p.name = "Living room".into());
        (c, rx_a, rx_b)
    }

    fn send(c: &Controllers, json: &str) -> Result<(), String> {
        handle_datagram(json.as_bytes(), c)
    }

    #[test]
    fn no_selector_targets_every_controller() {
        let (c, a, b) = setup();
        send(&c, r#"{"lightbar":[1,2,3]}"#).unwrap();
        assert_eq!(a.try_recv().unwrap(), Command::Lightbar(1, 2, 3));
        assert_eq!(b.try_recv().unwrap(), Command::Lightbar(1, 2, 3));
    }

    #[test]
    fn selector_by_name_targets_one_controller() {
        let (c, a, b) = setup();
        send(&c, r#"{"controller":"living room","lightbar":[1,2,3]}"#).unwrap();
        assert!(a.try_recv().is_ok());
        assert!(b.try_recv().is_err());
    }

    #[test]
    fn selector_by_index_targets_one_controller() {
        let (c, a, b) = setup();
        send(&c, r#"{"controller":1,"lightbar":[1,2,3]}"#).unwrap();
        assert!(a.try_recv().is_err());
        assert!(b.try_recv().is_ok());
    }

    #[test]
    fn selector_by_id_targets_one_controller() {
        let (c, a, b) = setup();
        send(&c, r#"{"controller":"BB","rumble":{"left":1,"right":2}}"#).unwrap();
        assert!(a.try_recv().is_err());
        assert_eq!(b.try_recv().unwrap(), Command::Rumble { left: 1, right: 2 });
    }

    #[test]
    fn unknown_controller_is_an_error() {
        let (c, a, b) = setup();
        assert!(send(&c, r#"{"controller":"unknown","lightbar":[1,2,3]}"#).is_err());
        assert!(send(&c, r#"{"controller":7,"lightbar":[1,2,3]}"#).is_err());
        assert!(a.try_recv().is_err() && b.try_recv().is_err());
    }

    #[test]
    fn full_message_produces_every_command() {
        let (c, a, _b) = setup();
        send(
            &c,
            r#"{"controller":0,"lightbar":[0,255,0],"player_leds":4,"rumble":{"left":10,"right":20},
                "triggers":{"left":{"kind":"off"},
                            "right":{"kind":"feedback","start":0,"strength":8}}}"#,
        )
        .unwrap();
        let cmds: Vec<_> = a.try_iter().collect();
        assert_eq!(cmds.len(), 5);
        assert_eq!(cmds[0], Command::Lightbar(0, 255, 0));
        assert_eq!(cmds[1], Command::PlayerLeds(4));
        assert_eq!(cmds[2], Command::Rumble { left: 10, right: 20 });
        assert!(matches!(cmds[3], Command::Trigger { right: false, effect } if effect[0] == 0x05));
        assert!(matches!(cmds[4], Command::Trigger { right: true, effect } if effect[0] == 0x21));
    }

    #[test]
    fn a_single_trigger_leaves_the_other_untouched() {
        let (c, a, _b) = setup();
        send(&c, r#"{"controller":0,"triggers":{"right":{"kind":"off"}}}"#).unwrap();
        let cmds: Vec<_> = a.try_iter().collect();
        assert_eq!(cmds.len(), 1);
        assert!(matches!(cmds[0], Command::Trigger { right: true, .. }));
    }

    #[test]
    fn reset_reapplies_the_saved_profile_before_other_changes() {
        let (c, a, _b) = setup();
        c.edit("AA", |p| p.lightbar = [5, 6, 7]);
        send(&c, r#"{"controller":0,"reset":true,"player_leds":3}"#).unwrap();
        let cmds: Vec<_> = a.try_iter().collect();
        assert_eq!(cmds[0], Command::Lightbar(5, 6, 7)); // the profile first
        assert_eq!(*cmds.last().unwrap(), Command::PlayerLeds(3)); // then the rest of the packet
    }

    #[test]
    fn invalid_packets_are_rejected_without_commands() {
        let (c, a, b) = setup();
        assert!(send(&c, "not json").is_err());
        assert!(send(&c, r#"{"lightbar":[300,0,0]}"#).is_err()); // out of u8 range
        assert!(send(&c, r#"{"lightbar":[1,2]}"#).is_err()); // too short
        assert!(send(&c, r#"{"triggers":{"left":{"kind":"unknown"}}}"#).is_err());
        assert!(handle_datagram(&[0xFF, 0xFE], &c).is_err()); // not UTF-8
        assert!(a.try_recv().is_err() && b.try_recv().is_err());
    }

    #[test]
    fn real_socket_round_trip() {
        let (c, a, _b) = setup();
        let server = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = server.local_addr().unwrap();
        std::thread::spawn(move || serve(server, c, |_, _| {}));

        let client = UdpSocket::bind("127.0.0.1:0").unwrap();
        client.send_to(br#"{"controller":"Living room","lightbar":[9,8,7]}"#, addr).unwrap();

        let cmd = a.recv_timeout(Duration::from_secs(2)).expect("command received");
        assert_eq!(cmd, Command::Lightbar(9, 8, 7));
    }
}
