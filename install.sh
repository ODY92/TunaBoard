#!/usr/bin/env bash
set -euo pipefail

case "${1:-}" in
    ""|--autostart) ;;
    --help|-h)
        printf 'Usage : ./install.sh [--autostart]\nCompile et installe Tuna Board pour votre utilisateur.\n'
        exit 0 ;;
    *) printf 'Option inconnue : %s\nUsage : ./install.sh [--autostart]\n' "$1" >&2; exit 2 ;;
esac
if (( $# > 1 )); then
    printf 'Usage : ./install.sh [--autostart]\n' >&2
    exit 2
fi
cd -- "$(dirname -- "${BASH_SOURCE[0]}")"
for command in cargo python3 pkg-config ffmpeg pactl pw-play; do
    if ! command -v "$command" >/dev/null 2>&1; then
        printf 'Dépendance manquante : %s. Voir les commandes dans README.md.\n' "$command" >&2
        exit 1
    fi
done
if ! pkg-config --exists gtk+-3.0; then
    printf 'Les fichiers de développement GTK 3 sont nécessaires. Voir README.md.\n' >&2
    exit 1
fi
cargo build --release --locked
build_dir="${CARGO_TARGET_DIR:-target}"
install -m 755 "$build_dir/release/tuna-board" ./tuna-board
python3 install-menu.py "$@"
