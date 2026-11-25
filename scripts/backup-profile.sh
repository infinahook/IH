#!/usr/bin/env bash
# Sauvegarde cohérente d'un profil IH (base SQLite + configuration).
#
# La base est copiée avec l'API backup de SQLite (`sqlite3 .backup`), qui est
# cohérente même en mode WAL. Une simple copie du fichier principal perdrait
# les transactions encore présentes dans `ih.sqlite3-wal` — c'est exactement
# l'erreur qui a provoqué la perte d'un profil pendant la recette 0.6.2.
#
# Sécurité : refuse de toucher au profil utilisateur réel
# (`$XDG_CONFIG_HOME/ih` ou `$HOME/.config/ih`) sauf `--allow-real-profile`
# explicite. Aucune suppression n'est effectuée à aucun moment.
set -euo pipefail

usage() {
  cat <<'EOF'
Usage : scripts/backup-profile.sh [--profile DIR] [--out DIR] [--allow-real-profile]

  --profile DIR           racine du profil IH (par défaut : $IH_HOME ou $XDG_CONFIG_HOME/ih)
  --out DIR               dossier de destination de la sauvegarde
  --allow-real-profile    autorise explicitement le profil utilisateur réel

La sauvegarde est une copie cohérente (WAL inclus) via `sqlite3 .backup`.
Aucun fichier du profil n'est supprimé ni déplacé.
EOF
}

PROFILE="${IH_HOME:-${XDG_CONFIG_HOME:-$HOME/.config}/ih}"
OUT=""
ALLOW_REAL=0
while [ $# -gt 0 ]; do
  case "$1" in
    --profile) PROFILE="${2:?}"; shift 2 ;;
    --out) OUT="${2:?}"; shift 2 ;;
    --allow-real-profile) ALLOW_REAL=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Argument inconnu : $1" >&2; usage; exit 2 ;;
  esac
done

command -v sqlite3 >/dev/null 2>&1 || { echo "sqlite3 requis" >&2; exit 1; }

REAL_PROFILE="${XDG_CONFIG_HOME:-$HOME/.config}/ih"
PROFILE="$(realpath --canonicalize-missing "$PROFILE")"
REAL_PROFILE="$(realpath --canonicalize-missing "$REAL_PROFILE")"

if [ "$PROFILE" = "$REAL_PROFILE" ] && [ "$ALLOW_REAL" -ne 1 ]; then
  cat >&2 <<EOF
Refus : le profil réel ($REAL_PROFILE) est protégé.
Relancez avec --allow-real-profile après avoir vérifié la commande.
Pour une recette, utilisez un profil isolé :
  IH_HOME=/tmp/ih-recette ./target/release/ih
EOF
  exit 3
fi

DB="$PROFILE/data/ih.sqlite3"
[ -f "$DB" ] || { echo "Base absente : $DB" >&2; exit 1; }

STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
OUT="${OUT:-$PROFILE/backups}"
mkdir -p "$OUT"
TARGET="$OUT/ih-$STAMP.sqlite3"
[ "$(realpath --canonicalize-missing "$TARGET")" != "$(realpath --canonicalize-missing "$DB")" ] \
  || { echo "Destination identique à la source" >&2; exit 1; }

# Copie cohérente WAL incluse, sans jamais toucher la base source.
sqlite3 "$DB" ".backup '$TARGET'"

CHECK="$(sqlite3 "$TARGET" 'PRAGMA integrity_check;')"
if [ "$CHECK" != "ok" ]; then
  echo "Sauvegarde invalide ($CHECK) : $TARGET" >&2
  exit 1
fi

if [ -f "$PROFILE/config.toml" ]; then
  cp -p "$PROFILE/config.toml" "$OUT/config-$STAMP.toml"
fi

echo "Sauvegarde cohérente : $TARGET"
sha256sum "$TARGET" | sed 's/^/  sha256 /'
echo "Source intacte : $DB"
