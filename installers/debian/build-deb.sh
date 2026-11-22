#!/usr/bin/env bash
# Construit les paquets Debian/AppImage d'IH via le bundler Tauri.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

command -v cargo >/dev/null || { echo "cargo introuvable (rustup)" >&2; exit 1; }
command -v npm >/dev/null || { echo "npm introuvable" >&2; exit 1; }

echo "== Dépendances frontend =="
npm ci --no-audit --no-fund

echo "== Vérifications =="
npm run typecheck
cargo test --workspace --quiet
cargo clippy --workspace --all-targets -- -D warnings

echo "== Construction des paquets =="
npm run tauri build -- --bundles deb,appimage

echo
echo "Paquets produits :"
find target/release/bundle -maxdepth 2 -type f \
  \( -name '*.deb' -o -name '*.AppImage' \) -printf '  %p (%s octets)\n'
