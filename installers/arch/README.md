# Arch Linux package

The included PKGBUILD is a packaging template, not a release-ready package: it intentionally contains no fake source checksum or binary URL. For a real release, build the Tauri AppImage or Linux binary in CI, stage it into `$pkgdir`, add verified source assets and checksums, then test in a clean Arch VM. Do not publish this template to AUR unchanged.
