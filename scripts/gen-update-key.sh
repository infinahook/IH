#!/usr/bin/env bash
# Génère la paire de clés Ed25519 qui signe les manifestes de mise à jour IH.
# La clé PRIVÉE ne doit jamais être versionnée : garde-la dans un coffre ou un
# secret CI (jamais dans le dépôt).
set -euo pipefail

OUT="${1:-.release-keys}"
mkdir -p "$OUT"
chmod 700 "$OUT"

PRIVATE="$OUT/ih-update-private.pem"
PUBLIC="$OUT/ih-update-public.pem"

if [ -f "$PRIVATE" ]; then
  echo "Refus : $PRIVATE existe déjà (supprime-le explicitement pour régénérer)." >&2
  exit 1
fi

openssl genpkey -algorithm ED25519 -out "$PRIVATE"
chmod 600 "$PRIVATE"
openssl pkey -in "$PRIVATE" -pubout -out "$PUBLIC"
chmod 644 "$PUBLIC"

PUB_HEX=$(openssl pkey -in "$PRIVATE" -pubout -outform DER | tail -c 32 | xxd -p -c 64)

echo "Clé privée : $PRIVATE (à protéger)"
echo "Clé publique: $PUBLIC"
echo
echo "Clé publique hexadécimale à mettre dans la configuration IH (updates.public_key_hex) :"
echo "$PUB_HEX"
