# Protocole UDP

[English](UDP.md) | Français

DS5Forge écoute sur **`127.0.0.1:6970`** (local uniquement, jamais exposé au réseau).
Chaque datagramme est un objet JSON UTF-8 (4 Ko maximum). **Tous les champs sont facultatifs** :
un champ absent laisse l'état correspondant inchangé. Un paquet invalide est ignoré.

```json
{
  "controller": "Salon",
  "lightbar": [255, 0, 0],
  "player_leds": 31,
  "rumble": { "left": 0, "right": 128 },
  "triggers": {
    "left":  { "kind": "feedback", "start": 3, "strength": 6 },
    "right": { "kind": "weapon", "start": 3, "end": 6, "strength": 8 }
  }
}
```

| Champ | Valeurs |
|---|---|
| `controller` | la manette visée (voir ci-dessous). Absent = **toutes** les manettes connectées |
| `reset` | `true` : réapplique d'abord le profil enregistré de la manette (utile à la fermeture d'un jeu) |
| `lightbar` | `[r, g, b]`, chaque composante de 0 à 255 |
| `player_leds` | masque de 5 bits (bit 0 = LED de gauche), 0 à 31 |
| `rumble` | `left` (moteur fort) et `right` (moteur faible), 0 à 255 |
| `triggers.left` / `triggers.right` | un effet, voir ci-dessous |

## Choisir la manette

`controller` accepte, au choix :

| Valeur | Exemple | Désigne |
|---|---|---|
| un nom | `"Salon"` | la manette portant ce nom (insensible à la casse) |
| un identifiant | `"E8:47:3A:11:22:33"` | la manette d'adresse MAC donnée (affichée dans l'application) |
| un index | `0`, `1`, `2` | la position dans la liste des manettes **connectées**, triées par identifiant |
| `"all"` | `"all"` | toutes les manettes connectées |

Préférez le nom ou l'identifiant : un index change dès qu'une manette se connecte ou se déconnecte.
Si aucune manette connectée ne correspond, le paquet est ignoré (l'erreur s'affiche dans l'application).

Les commandes UDP sont **temporaires** : elles ne modifient pas le profil enregistré.

## Effets de gâchettes (`kind`)

| `kind` | Paramètres |
|---|---|
| `off` | aucun |
| `feedback` (résistance) | `start` 0-9, `strength` 1-8 |
| `weapon` (clic) | `start` 2-7, `end` start+1 à 8, `strength` 1-8 |
| `vibration` | `start` 0-9, `amplitude` 1-8, `frequency` 1-255 (Hz) |

La course de la gâchette est découpée en 10 zones (0 = relâchée, 9 = à fond).
Des paramètres hors plage désactivent l'effet.

## Essai rapide

```bash
node examples/udp-test.mjs
```
