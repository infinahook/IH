//! Mises à jour vérifiées d'IH.
//!
//! Ce crate est la seule porte d'entrée pour vérifier et préparer une mise à
//! jour. Règle non négociable : **aucune installation sans signature Ed25519
//! valide et sans empreinte SHA-256 conforme**.
//!
//! # Décisions de conception
//!
//! * **Signature détachée hexadécimale.** `manifest.json.sig` contient la
//!   signature Ed25519, en hexadécimal, des octets exacts de `manifest.json`
//!   (aucune re-sérialisation, aucun JSON canonique). La vérification utilise
//!   `verify_strict`, qui rejette les clés/signatures de faible ordre.
//! * **Transport.** Les URL d'artefacts doivent être en HTTPS, sans exception.
//!   L'authenticité ne dépend toutefois pas de TLS : elle dépend de la
//!   signature Ed25519 et de l'empreinte SHA-256, vérifiées avant toute
//!   installation.
//! * **Aucun chemin du manifeste.** Le nom du fichier téléchargé est un nom
//!   constant choisi par le crate, écrit dans le dossier de staging fourni par
//!   l'appelant. Une URL de manifeste n'est jamais convertie en chemin local.
//! * **Plans d'installation.** `deb` et `pacman` produisent une commande
//!   `pkexec` (respectivement `apt-get install -y` et `pacman -U --noconfirm`)
//!   qui n'est **pas exécutée** par ce crate : l'interface doit demander une
//!   confirmation explicite. Seul l'AppImage est remplacé directement, avec une
//!   copie de sauvegarde préalable.
//!
//! # Limites assumées
//!
//! * pas de mise à jour différentielle (delta) ;
//! * pas de notarisation/signature de code macOS ou Windows ici ;
//! * pas de vérification de révocation de clé ni de rotation ;
//! * le rollback n'est fourni que pour les AppImage ; pour `deb`/`pacman`, le
//!   gestionnaire de paquets fait foi.

pub mod client;
pub mod crypto;
pub mod install;
pub mod manifest;

pub use client::{UpdateClient, MAX_DOWNLOAD_BYTES, MAX_MANIFEST_BYTES, MAX_SIGNATURE_BYTES};
pub use crypto::{verify_digest, verify_signature};
pub use install::{apply_appimage, plan_install, rollback_appimage, InstallPlan, StagedUpdate};
pub use manifest::{
    is_newer, parse_manifest, target_key, Artifact, ArtifactKind, UpdateManifest, SUPPORTED_SCHEMA,
};

#[cfg(test)]
mod test_support;
