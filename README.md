# IH

Application Rust pour Linux, testée sur **Arch Linux** et **Ubuntu**.

[![CI](https://github.com/infinahook/IH/actions/workflows/ci.yml/badge.svg)](https://github.com/infinahook/IH/actions/workflows/ci.yml)

## Prérequis

| Distribution | Installation de la toolchain |
|---|---|
| **Arch Linux** | `sudo pacman -S --needed base-devel rust` |
| **Ubuntu / Debian** | `sudo apt install build-essential curl` puis [rustup](https://rustup.rs) : `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh -s -- -y` |

> Sur Ubuntu, préfère **rustup** au paquet `rustc` des dépôts : celui de la 24.04 est nettement plus ancien que la toolchain stable.

Version minimale supportée (MSRV) : **Rust 1.70**, édition 2021.

## Compilation

```bash
git clone https://github.com/infinahook/IH.git
cd IH
cargo build --release
```

Le binaire est produit dans `target/release/ih`.

### Installation locale

```bash
install -Dm755 target/release/ih ~/.local/bin/ih
```

Assure-toi que `~/.local/bin` est dans ton `PATH`.

## Utilisation

```bash
ih            # aide
ih version    # version
ih info       # informations de plateforme
```

Codes de retour : `0` succès, `2` argument invalide.

## Développement

```bash
cargo fmt --all                       # formatage
cargo clippy --all-targets -- -D warnings   # lint, sans avertissement toléré
cargo test --all-targets              # tests
cargo run -- info                     # exécution
```

## Intégration continue

`.github/workflows/ci.yml` exécute `fmt`, `clippy`, `test` et un build release dans deux environnements :

- `ubuntu-24.04` avec la toolchain stable installée par rustup
- un conteneur `archlinux:latest` avec `rust` issu de `pacman`

## Structure

```
.
├── Cargo.toml
├── rust-toolchain.toml
├── src/main.rs
└── .github/workflows/ci.yml
```

## Licence

À définir.
