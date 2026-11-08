#!/usr/bin/env python3
"""Ajouter l’application à son menu Linux, sans installation système."""
from pathlib import Path
import argparse
import os
import shutil
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--autostart", action="store_true", help="Démarrer en arrière-plan avec les bureaux compatibles XDG")
args = parser.parse_args()

root = Path(__file__).resolve().parent
binary = root / "tuna-board"
if not binary.is_file():
    raise SystemExit("Compiler ou copier le binaire tuna-board dans ce dossier d’abord.")
data = Path(os.environ.get("XDG_DATA_HOME") or Path.home() / ".local/share")
apps = data / "applications"
icons = data / "icons/hicolor/scalable/apps"
apps.mkdir(parents=True, exist_ok=True)
icons.mkdir(parents=True, exist_ok=True)
target = apps / "tuna-board.desktop"
if target.exists() and "soundboard native Linux" not in target.read_text():
    raise SystemExit(f"Une autre entrée existe déjà : {target}. Rien n’a été remplacé.")
app_dir = data / "tuna-board/bin"
app_dir.mkdir(parents=True, exist_ok=True)
installed_binary = app_dir / "tuna-board"
temporary = app_dir / "tuna-board.new"
shutil.copyfile(binary, temporary)
temporary.chmod(0o755)
temporary.replace(installed_binary)
shutil.copyfile(root / "assets/tuna-board.svg", icons / "tuna-board.svg")
escaped = str(installed_binary).replace("\\", "\\\\").replace('"', '\\"').replace("`", "\\`").replace("$", "\\$").replace("%", "%%")
entry = f'''[Desktop Entry]
Type=Application
Name=Tuna Board
GenericName=Soundboard
GenericName[fr]=Boîte à sons
Comment=Sons communautaires Voicemod, soundboard native Linux
Exec="{escaped}"
TryExec={installed_binary}
Icon={icons / "tuna-board.svg"}
Terminal=false
Categories=AudioVideo;Audio;
Keywords=soundboard;tuna;voicemod;sons;audio;memes;
StartupWMClass=tuna-board
'''
target.write_text(entry)
if shutil.which("update-desktop-database"):
    subprocess.run(["update-desktop-database", str(apps)], check=True)
print(f"Raccourci créé : {target}")
print(f"Application installée : {installed_binary}")
if args.autostart:
    config = Path(os.environ.get("XDG_CONFIG_HOME") or Path.home() / ".config")
    autostart = config / "autostart/tuna-board.desktop"
    if autostart.exists() and "soundboard native Linux" not in autostart.read_text():
        raise SystemExit(f"Une autre entrée existe déjà : {autostart}. Rien n’a été remplacé.")
    autostart.parent.mkdir(parents=True, exist_ok=True)
    autostart.write_text(entry.replace(f'Exec="{escaped}"\n', f'Exec="{escaped}" --background\n', 1))
    print(f"Démarrage automatique XDG : {autostart}")
