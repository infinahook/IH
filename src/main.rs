//! IH — application Rust pour Linux (Arch, Ubuntu).
//!
//! Point d'entrée minimal : analyse des arguments, sortie propre et codes de
//! retour conventionnels. Aucune dépendance externe pour l'instant.

use std::process::ExitCode;

const USAGE: &str = "\
IH — application Rust pour Linux (Arch, Ubuntu)

USAGE:
    ih [COMMANDE]

COMMANDES:
    info        Affiche les informations de plateforme et de version
    help        Affiche cette aide
    version     Affiche la version

OPTIONS:
    -h, --help       Affiche cette aide
    -V, --version    Affiche la version
";

/// Commande reconnue par la ligne de commande.
#[derive(Debug, PartialEq, Eq)]
enum Command {
    Help,
    Version,
    Info,
}

/// Traduit les arguments en une [`Command`], ou renvoie un message d'erreur.
fn parse(args: &[String]) -> Result<Command, String> {
    match args.first().map(String::as_str) {
        None | Some("help") | Some("-h") | Some("--help") => Ok(Command::Help),
        Some("version") | Some("-V") | Some("--version") => Ok(Command::Version),
        Some("info") => Ok(Command::Info),
        Some(autre) => Err(format!("commande inconnue : {autre}")),
    }
}

fn print_info() {
    println!("ih      : {}", env!("CARGO_PKG_VERSION"));
    println!(
        "système : {} ({})",
        std::env::consts::OS,
        std::env::consts::FAMILY
    );
    println!("cible   : {}", std::env::consts::ARCH);
    println!(
        "profil  : {}",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    );
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    match parse(&args) {
        Ok(Command::Help) => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Ok(Command::Version) => {
            println!("ih {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Ok(Command::Info) => {
            print_info();
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("erreur : {message}\n\n{USAGE}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(valeurs: &[&str]) -> Vec<String> {
        valeurs.iter().map(|v| v.to_string()).collect()
    }

    #[test]
    fn sans_argument_affiche_l_aide() {
        assert_eq!(parse(&args(&[])), Ok(Command::Help));
    }

    #[test]
    fn alias_de_laide_et_de_la_version() {
        for drapeau in ["help", "-h", "--help"] {
            assert_eq!(parse(&args(&[drapeau])), Ok(Command::Help));
        }
        for drapeau in ["version", "-V", "--version"] {
            assert_eq!(parse(&args(&[drapeau])), Ok(Command::Version));
        }
    }

    #[test]
    fn info_est_reconnu() {
        assert_eq!(parse(&args(&["info"])), Ok(Command::Info));
    }

    #[test]
    fn commande_inconnue_est_rejetee() {
        let erreur = parse(&args(&["frobnicate"])).unwrap_err();
        assert!(erreur.contains("frobnicate"), "message: {erreur}");
    }
}
