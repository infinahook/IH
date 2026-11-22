#!/usr/bin/env bash
# Construction complète d'IH (vérifications + paquets Linux).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

export PATH="${HOME}/.cargo/bin:${PATH}"

command -v cargo >/dev/null || { echo "cargo introuvable : installe Rust via rustup" >&2; exit 1; }
command -v npm >/dev/null || { echo "npm introuvable : installe Node.js 22+" >&2; exit 1; }

echo "== Versions =="
cargo --version
rustc --version
node --version
npm --version

echo "== Dépendances frontend =="
npm ci --no-audit --no-fund

echo "== Formatage =="
cargo fmt --all -- --check

echo "== Lint =="
cargo clippy --workspace --all-targets -- -D warnings

echo "== Tests Rust =="
cargo test --workspace --quiet

echo "== Tests + build frontend =="
npm run typecheck
npm test --silent
npm run build

echo "== Paquets =="
npm run tauri build -- --bundles deb,appimage

echo
echo "Artefacts :"
find target/release/bundle -maxdepth 2 -type f \
  \( -name '*.deb' -o -name '*.AppImage' \) -printf '  %p\n'
