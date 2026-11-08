# Tuna Board

Une soundboard légère pour Linux, écrite en Rust avec GTK 3. Recherche des sons sur **Voicemod Tuna**, garde tes favoris hors connexion et diffuse-les dans Discord ou tes jeux grâce au micro virtuel **Tuna**.

Petites tuiles carrées, grille adaptée à la fenêtre, préécoute dans le casque et fonctionnement en arrière-plan. Aucun changement de voix. Projet indépendant de Voicemod.

## Installation

Il faut Linux avec **PipeWire**, sa compatibilité PulseAudio et **Rust 1.88 ou plus récent**. Voicemod et Wine ne sont pas nécessaires.

### 1. Installer les dépendances

**Arch Linux / CachyOS :**

```bash
sudo pacman -S --needed base-devel git rust gtk3 librsvg pipewire-audio pipewire-pulse libpulse wireplumber ffmpeg python desktop-file-utils
```

**Ubuntu / Debian :**

```bash
sudo apt update
sudo apt install build-essential pkg-config libgtk-3-dev librsvg2-common git curl python3 ffmpeg pipewire-bin pipewire-pulse pulseaudio-utils wireplumber desktop-file-utils
```

Sur Ubuntu/Debian, installer un Rust récent avec [l’installateur officiel](https://rust-lang.org/tools/install/) si nécessaire :

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
. "$HOME/.cargo/env"
```

Si tu viens de passer de PulseAudio à PipeWire, ferme puis rouvre ta session avant de lancer l’application.

### 2. Télécharger et installer Tuna Board

```bash
git clone https://github.com/ODY92/TunaBoard.git
cd TunaBoard
./install.sh
```

Le script compile le projet et l’installe pour ton utilisateur, sans `sudo`. La première compilation prend quelques minutes. Ensuite, ouvre **Tuna Board** dans le launcher, ou lance :

```bash
~/.local/share/tuna-board/bin/tuna-board
```

## Utilisation

1. Dans **Communauté**, recherche un son ou colle son lien Tuna, puis clique sur **+** pour le garder.
2. Dans **Ma soundboard**, clique sur une **tuile** pour diffuser le son. Le petit **casque** sert uniquement à la préécoute locale.
3. Dans Discord ou ton jeu, sélectionne **Tuna** comme périphérique d’entrée. Le micro physique est mélangé aux sons ; les voix ne sont pas modifiées.

Le micro virtuel s’active au lancement. **F1 à F8** jouent les huit premiers sons lorsque la fenêtre est active ; **Échap** arrête la lecture. Un nouveau son remplace le précédent.

Fermer la fenêtre garde l’application en arrière-plan, y compris avec **Super + Q** lorsqu’il correspond à une fermeture normale. Le launcher retrouve la même instance. Pour tout arrêter, utilise **Quitter Tuna Board**.

## Démarrage automatique

Pour les bureaux compatibles avec le démarrage automatique XDG, comme GNOME ou KDE :

```bash
./install.sh --autostart
```

Sur Hyprland, ajoute la commande suivante à ton démarrage de session :

```bash
~/.local/share/tuna-board/bin/tuna-board --background
```

## Mise à jour

Depuis le dossier du dépôt :

```bash
git pull --ff-only
./install.sh
```

Quitte complètement Tuna Board puis rouvre-le pour utiliser la nouvelle version.

## En cas de problème

- **Les autres n’entendent pas les sons :** sélectionner le micro **Tuna**, cliquer sur la tuile plutôt que sur le casque et vérifier que la réduction de bruit de Discord ne filtre pas les clips.
- **Le micro virtuel ne s’active pas :** vérifier que `pactl info` et `pw-play --version` fonctionnent dans la session.
- La recherche dépend du site public Tuna. Les sons déjà ajoutés restent utilisables hors connexion.

Les données sont dans `~/.local/share/tuna-board/` et le cache dans `~/.cache/tuna-board/` (ou leurs équivalents XDG). Les miniatures sont limitées à 192 × 192 pixels et chargées à la demande. Aucun son ni compte utilisateur n’est livré avec le projet.

## Désinstallation

Quitte Tuna Board, puis :

```bash
rm -f ~/.local/share/applications/tuna-board.desktop
rm -f ~/.config/autostart/tuna-board.desktop
rm -f ~/.local/share/icons/hicolor/scalable/apps/tuna-board.svg
rm -rf ~/.local/share/tuna-board/bin
```

Si tu as ajouté la commande à Hyprland, retire aussi cette ligne. La bibliothèque reste conservée.

## Développement

```bash
cargo build --release --locked
cargo test --locked
```

Licence [MIT](LICENSE).
