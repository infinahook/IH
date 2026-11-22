#!/usr/bin/env bash
set -euo pipefail
sudo pacman -Syu --needed base-devel curl wget file openssl gtk3 libayatana-appindicator webkit2gtk-4.1 librsvg npm
command -v node >/dev/null || { echo "Install Node.js 22+." >&2; exit 1; }
command -v cargo >/dev/null || { echo "Install Rust stable via rustup." >&2; exit 1; }
npm install
echo "Dependencies installed. Run: npm run tauri:dev"
