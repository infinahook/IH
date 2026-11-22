#!/usr/bin/env bash
# Signe un manifeste de mise à jour IH (signature Ed25519 détachée, en hex).
# Usage : sign-release.sh manifest.json private.pem [sortie.sig]
set -euo pipefail

MANIFEST="${1:?Usage: sign-release.sh manifest.json private.pem [sortie.sig]}"
PRIVATE="${2:?clé privée PEM manquante}"
OUT="${3:-${MANIFEST}.sig}"

[ -f "$MANIFEST" ] || { echo "manifeste introuvable : $MANIFEST" >&2; exit 1; }
[ -f "$PRIVATE" ] || { echo "clé privée introuvable : $PRIVATE" >&2; exit 1; }

TMP_DER="$(mktemp)"
trap 'rm -f "$TMP_DER"' EXIT

# Signature brute des octets exacts du manifeste.
openssl pkeyutl -sign -inkey "$PRIVATE" -rawin -in "$MANIFEST" -out "$TMP_DER"
xxd -p -c 64 "$TMP_DER" > "$OUT"

echo "Signature écrite : $OUT"
echo "Vérification :"
cargo run --quiet --release --manifest-path "$(dirname "${BASH_SOURCE[0]}")/../crates/ih-update/Cargo.toml" \
  --bin ih-update-verify -- "$MANIFEST" "$OUT" "$(openssl pkey -in "$PRIVATE" -pubout -outform DER | tail -c 32 | xxd -p -c 64)" \
  && echo "OK : signature valide"
