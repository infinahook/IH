# AppImage

Tauri can produce an AppImage when the Linux build environment has the required bundling dependencies. Build with `npm run tauri:build`, then inspect `target/release/bundle/appimage/`. Test on a clean Zorin and Arch installation. AppImage runtime compatibility is not a substitute for native package testing.
