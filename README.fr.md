# DS5Forge

Un compagnon pour la manette Sony DualSense sous Windows, développé en Rust avec
[Tauri](https://tauri.app). [English](README.md) | Français

![Aperçu : vue en direct de la manette](docs/screenshots/overview.png)

![Diagnostic : pilotes, droits et journal des événements](docs/screenshots/diagnostics.png)

## Fonctionnalités

- **Usage natif** : la manette fonctionne comme une DualSense classique, avec des réglages enregistrés par manette. Tout le reste est facultatif.
- **Couleur de la manette** : l'illustration peut prendre la couleur de votre DualSense (White, Midnight Black, Cosmic
  Red, Nova Pink…), choisie par manette sous l'image. Les valeurs sont des approximations des couleurs officielles.
- **Vue en direct** de la manette : boutons, sticks, gâchettes, pavé tactile, batterie et type de connexion.
- **Gâchettes adaptatives** (résistance, clic d'arme, vibration), **vibration**, **lightbar** (couleur fixe,
  selon la batterie ou selon le mode) et **LED joueur**.
- **Mode Xbox 360 facultatif** : une manette virtuelle (via [ViGEmBus](https://github.com/nefarius/ViGEmBus)), avec la vraie
  manette éventuellement masquée aux jeux grâce à [HidHide](https://github.com/nefarius/HidHide).
- **Règles** : réattribuer des boutons, des combinaisons et des directions de stick à des boutons Xbox, des
  touches du clavier et des actions de souris ; gestes (appui bref, maintien, double appui, tant que tenu),
  macros, turbo, verrouillage et couches.
- **Profils de jeu** : vos réglages habituels sont le profil par défaut ; un profil de jeu a ses propres réglages
  et s'applique automatiquement tant que ce jeu est au premier plan (seul le nom du programme au premier plan est
  lu ; rien n'est injecté dans les jeux).
- **Menu de la manette** : un menu dans l'overlay, piloté avec la manette elle-même (croix directionnelle et
  Croix) : passer du mode natif au mode Xbox 360, changer de profil, et voir la batterie de la manette. Il
  s'ouvre avec l'action « Ouvrir le menu de la manette » d'une règle ; les règles par défaut l'ouvrent en
  maintenant le bouton PS, et supprimer cette règle retire le raccourci. Tant que le menu est ouvert, le jeu et les
  règles ne reçoivent rien, et le menu prend le focus pour que la plupart des jeux ne réagissent plus à la manette
  (désactivable dans les réglages). L'overlay ne montre jamais que la manette qui l'a demandé.
- **Liaisons analogiques** : un stick peut piloter la souris, la molette ou des touches.
- **Gyroscope et pavé tactile** en souris ou en stick droit.
- **Overlay** avec la batterie et le mode de chaque manette, **icône dans la zone de notification**,
  alertes de batterie faible et vibrations de confirmation.
- **Interface UDP** pour piloter l'éclairage, les gâchettes et la vibration depuis d'autres programmes
  (voir [UDP.fr.md](UDP.fr.md)).
- Page **Diagnostic** pour vérifier les pilotes, les droits, chaque manette et le journal des événements.
- Interface en **anglais et en français**, modifiable à tout moment dans *Réglages → Langue*. Par défaut,
  la langue du système est utilisée.

## Installation

Téléchargez `DS5Forge_<version>_x64-setup.exe` depuis la page [Releases](../../releases) et lancez-le.
Il s'installe pour l'utilisateur courant uniquement (sans droits administrateur) et propose l'anglais ou le français.

L'installeur n'est pas signé : Windows SmartScreen peut afficher un avertissement la première fois, il faut
choisir *Informations complémentaires* → *Exécuter quand même*.

## Prérequis

- Windows 10 ou 11.
- Facultatif, pour la manette virtuelle : [ViGEmBus](https://github.com/nefarius/ViGEmBus). Les deux pilotes facultatifs s'installent depuis *Réglages → Composants* (via winget).
- Facultatif, pour masquer la vraie manette aux jeux : [HidHide](https://github.com/nefarius/HidHide).
  Le masquage demande que l'application soit le seul programme à utiliser HidHide à ce moment-là (fermez
  HidHide Configuration Client et les autres outils de manette) et peut nécessiter les droits administrateur.

## Compilation

Prérequis : [Rust](https://rustup.rs), [Node.js](https://nodejs.org) et les
[prérequis Tauri pour Windows](https://tauri.app/start/prerequisites/).

```bash
npm install
npm run tauri dev     # version de développement avec rechargement à chaud
npm run tauri build   # installeur dans src-tauri/target/release/bundle
```

Vérifications :

```bash
cd src-tauri && cargo test    # tests unitaires du backend
npx tsc --noEmit              # vérification des types de l'interface
```

Pour publier une version, poussez un tag (`git tag v0.1.0 && git push origin v0.1.0`) : le workflow *Release*
construit l'installeur et l'attache à une release en brouillon. En local, `npm run tauri build` l'écrit dans
`src-tauri/target/release/bundle/nsis/`.

## Mises à jour

L'application cherche une nouvelle version sur GitHub quelques secondes après son démarrage (une seule requête).
Quand il y en a une, un point apparaît à côté de *Réglages*, et *Réglages → Mises à jour* propose
**Installer et redémarrer**. Le paquet est vérifié avec la clé publique de `src-tauri/tauri.conf.json` avant
d'être installé.

Pour les mainteneurs : les mises à jour sont signées avec une paire de clés. Générez-la une fois avec
`npx tauri signer generate -w ~/.tauri/ds5forge.key`, mettez la clé publique dans `tauri.conf.json`
(`plugins.updater.pubkey`) et la clé privée dans les secrets du dépôt `TAURI_SIGNING_PRIVATE_KEY` (et
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` si elle a un mot de passe). **Ne commitez jamais la clé privée et gardez-en
une sauvegarde** : sans elle, les copies installées ne peuvent plus être mises à jour. Un
`npm run tauri build` local demande aussi `TAURI_SIGNING_PRIVATE_KEY`. La release doit être publiée (pas en
brouillon) pour que les copies installées la voient.

## Organisation du projet

| Chemin | Contenu |
|---|---|
| `src-tauri/src/` | Backend Rust : rapports HID, moteur de règles (`bindings.rs`), manette virtuelle, HidHide, serveur UDP |
| `src-tauri/src/i18n.rs` | Messages produits par le backend, en anglais et en français |
| `src/` | Interface TypeScript (Vite, sans framework) |
| `src/locales/` | Textes de l'interface : `en.ts` fait référence, `fr.ts` doit fournir les mêmes clés |
| `src/assets/` | Illustration de la manette (voir [son README](src/assets/README.md), en anglais) |
| `tools/` | Script qui convertit le visuel de Sony en SVG pour l'interface |
| `examples/` | Un petit client UDP |

## Ajouter une langue

1. Copiez `src/locales/en.ts` dans un nouveau fichier et traduisez les valeurs (le compilateur vérifie qu'aucune clé ne manque).
2. Déclarez le dictionnaire et le nom de la langue dans `src/i18n.ts`.
3. Ajoutez la variante correspondante à `Lang` et les traductions des messages `Msg` dans `src-tauri/src/i18n.rs`
   (les libellés des touches du clavier sont dans `src-tauri/src/keys.rs`).

## Licence

Le code est publié sous [licence MIT](LICENSE). L'illustration de la manette dans `src/assets/` dérive d'un visuel
de Sony Interactive Entertainment et n'est pas couverte par cette licence (voir [son README](src/assets/README.md)).

## Marques

DualSense et PlayStation sont des marques de Sony Interactive Entertainment. Xbox est une marque de
Microsoft. Ce projet n'est affilié à aucune de ces sociétés.
