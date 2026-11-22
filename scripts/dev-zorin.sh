#!/usr/bin/env bash
set -euo pipefail
sudo apt update
sudo apt install -y build-essential curl wget file libssl-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libwebkit2gtk-4.1-dev
if ! command -v node >/dev/null 2>&1; then
  echo "Install Node.js 22+ (for example with nvm), then rerun this script." >&2
  exit 1
fi
if ! command -v cargo >/dev/null 2>&1; then
  echo "Install Rust stable using rustup, then rerun this script." >&2
  exit 1
fi
npm install
echo "Dependencies installed. Run: npm run tauri:dev"
