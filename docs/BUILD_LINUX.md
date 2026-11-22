# Construction d'IH sous Linux

## Prérequis communs

- Rust stable (via [rustup](https://rustup.rs)) — `cargo`, `rustc`, `rustfmt`, `clippy`
- Node.js 22+ et npm
- Git
- Les bibliothèques de développement WebKitGTK / GTK3 / libsoup3 propres à la
  distribution (voir ci-dessous)

## Zorin OS / Ubuntu / Debian

```bash
sudo apt update
sudo apt install -y build-essential curl wget file libssl-dev \
  libgtk-3-dev libwebkit2gtk-4.1-dev libsoup-3.0-dev \
  libjavascriptcoregtk-4.1-dev libayatana-appindicator3-dev \
  librsvg2-dev patchelf
```

Zorin OS 18 est basé sur Ubuntu 24.04 (noble) : les mêmes noms de paquets
s'appliquent. Sur une version plus ancienne de Zorin/Ubuntu, `webkit2gtk-4.1`
peut ne pas exister ; il faut alors une version d'Ubuntu ≥ 24.04.

## Arch Linux

```bash
sudo pacman -Syu --needed base-devel rust git nodejs npm \
  webkit2gtk-4.1 gtk3 librsvg libsoup3 openssl pkgconf
```

## Développement

```bash
npm ci
npm run tauri:dev        # fenêtre IH avec rechargement à chaud du frontend
# ou, séparément :
npm run dev              # Vite seul (http://127.0.0.1:1420)
```

## Vérifications

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
npm run typecheck
npm test
```

## Construction des paquets

```bash
bash scripts/build-linux.sh
```

Cela exécute les vérifications, construit le frontend (`dist/`) puis les
paquets via le bundler Tauri. Le frontend doit être construit **avant** la
compilation Rust : `generate_context!` embarque `dist/` dans le binaire.

Artefacts produits :

```
target/release/bundle/deb/IH_0.1.0_amd64.deb
target/release/bundle/appimage/IH_0.1.0_amd64.AppImage
```

## Installation

### Debian / Ubuntu / Zorin

```bash
sudo apt install ./target/release/bundle/deb/IH_0.1.0_amd64.deb
```

### AppImage

```bash
chmod +x IH_0.1.0_amd64.AppImage
./IH_0.1.0_amd64.AppImage
```

### Arch Linux

```bash
cd installers/arch
makepkg -si
```

Le `PKGBUILD` construit depuis les sources : il exécute `npm ci`,
`npm run build`, puis `cargo build --release --locked`, et installe le binaire
`/usr/bin/ih` avec son entrée de bureau et son icône.

## Intégration continue

`.github/workflows/ci.yml` :

| Job | Contenu |
|---|---|
| `check` (Ubuntu 24.04) | `cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace`, `npm run build`, `npm run typecheck`, `npm test` |
| `package` (Ubuntu 24.04) | `tauri build --bundles deb,appimage` + publication des artefacts |
| `arch` (conteneur `archlinux:latest`) | installation des dépendances pacman, build frontend, `cargo test --workspace`, build release |

## Notes

- Le premier build Rust est long (WebKitGTK/GTK + ~10 crates IH).
- L'AppImage est produite par `linuxdeploy`, téléchargé par le bundler Tauri au
  moment du build : une connexion réseau est nécessaire en CI.
- Le binaire s'appelle `ih-desktop` dans `target/release/` ; les paquets
  l'installent sous le nom `ih`.
