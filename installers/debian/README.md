# Zorin OS / Ubuntu / Debian

Build the Tauri `.deb` artifact:
```bash
npm ci
npm run tauri:build
```
Tauri places the generated package under `target/release/bundle/deb/` on a successful build. Install:
```bash
sudo apt install ./target/release/bundle/deb/IH_*.deb
```
Check the actual generated filename before installing. The dependency list in `tauri.conf.json` is a baseline; validate it against the Zorin release you support.
