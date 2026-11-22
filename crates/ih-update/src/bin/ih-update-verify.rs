//! Vérifie la signature Ed25519 d'un manifeste de mise à jour IH.
//!
//! Usage : `ih-update-verify <manifest.json> <manifest.json.sig> <clé_publique_hex>`
//!
//! Codes de sortie : 0 signature valide, 1 signature invalide ou entrée
//! illisible, 2 usage incorrect. Utilisé par `scripts/sign-release.sh` et
//! utilisable en CI pour valider les publication avant diffusion.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 3 {
        eprintln!("Usage: ih-update-verify <manifest.json> <signature.sig> <clé_publique_hex>");
        return ExitCode::from(2);
    }

    let manifest_path = &args[0];
    let signature_path = &args[1];
    let public_key = &args[2];

    let manifest = match std::fs::read(manifest_path) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("manifeste illisible ({manifest_path}) : {e}");
            return ExitCode::from(1);
        }
    };
    let signature_raw = match std::fs::read_to_string(signature_path) {
        Ok(text) => text,
        Err(e) => {
            eprintln!("signature illisible ({signature_path}) : {e}");
            return ExitCode::from(1);
        }
    };

    match ih_update::verify_signature(&manifest, signature_raw.trim(), public_key) {
        Ok(()) => {
            println!("signature valide");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("signature invalide : {e}");
            ExitCode::from(1)
        }
    }
}
