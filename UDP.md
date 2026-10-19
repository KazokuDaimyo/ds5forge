# UDP protocol

English | [Français](UDP.fr.md)

DS5Forge listens on **`127.0.0.1:6970`** (local only, never exposed to the network).
Each datagram is a UTF-8 JSON object (4 KB maximum). **All fields are optional**: a missing field
leaves the corresponding state unchanged. An invalid packet is ignored.

```json
{
  "controller": "Living room",
  "lightbar": [255, 0, 0],
  "player_leds": 31,
  "rumble": { "left": 0, "right": 128 },
  "triggers": {
    "left":  { "kind": "feedback", "start": 3, "strength": 6 },
    "right": { "kind": "weapon", "start": 3, "end": 6, "strength": 8 }
  }
}
```

| Field | Values |
|---|---|
| `controller` | the targeted controller (see below). Missing = **all** connected controllers |
| `reset` | `true`: first reapplies the saved profile of the controller (useful when a game exits) |
| `lightbar` | `[r, g, b]`, each component from 0 to 255 |
| `player_leds` | 5-bit mask (bit 0 = leftmost LED), 0 to 31 |
| `rumble` | `left` (strong motor) and `right` (weak motor), 0 to 255 |
| `triggers.left` / `triggers.right` | an effect, see below |

## Choosing the controller

`controller` accepts any of:

| Value | Example | Designates |
|---|---|---|
| a name | `"Living room"` | the controller with that name (case-insensitive) |
| an identifier | `"E8:47:3A:11:22:33"` | the controller with that MAC address (shown in the application) |
| an index | `0`, `1`, `2` | the position in the list of **connected** controllers, sorted by identifier |
| `"all"` | `"all"` | every connected controller |

Prefer the name or the identifier: an index changes as soon as a controller connects or disconnects.
If no connected controller matches, the packet is ignored (the error is shown in the application).

UDP commands are **temporary**: they do not modify the saved profile.

## Trigger effects (`kind`)

| `kind` | Parameters |
|---|---|
| `off` | none |
| `feedback` (resistance) | `start` 0-9, `strength` 1-8 |
| `weapon` (click) | `start` 2-7, `end` start+1 to 8, `strength` 1-8 |
| `vibration` | `start` 0-9, `amplitude` 1-8, `frequency` 1-255 (Hz) |

The trigger travel is divided into 10 zones (0 = released, 9 = fully pressed).
Parameters out of range disable the effect.

## Quick test

```bash
node examples/udp-test.mjs
```
