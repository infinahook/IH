# Système de mise à jour d'IH

## Modèle de confiance

Un manifeste est publié avec une **signature Ed25519 détachée** et chaque
artefact est accompagné de son **SHA-256**. IH refuse toute installation si
l'un des deux contrôles échoue.

```
manifest.json          octets exacts signés
manifest.json.sig      signature Ed25519 des octets de manifest.json (hex)
<artefact>             binaire, vérifié par SHA-256 après téléchargement
```

La clé publique (32 octets en hexadécimal) est configurée dans
`~/.config/ih/config.toml` :

```toml
[updates]
channel = "stable"
endpoint = "https://releases.exemple.org/ih/manifest.json"
public_key_hex = "…64 caractères hex…"
check_interval_minutes = 360
auto_download = false
```

L'URL de signature est `endpoint + ".sig"`.

## Format du manifeste

```json
{
  "schema": 1,
  "version": "0.2.0",
  "channel": "stable",
  "notes": "Notes de version",
  "released_at": "2026-11-22T10:00:00Z",
  "platforms": {
    "linux-x86_64-appimage": {
      "url": "https://releases.exemple.org/ih/IH_0.2.0.AppImage",
      "sha256": "…64 caractères hex…",
      "size": 45678901,
      "kind": "appimage"
    },
    "linux-x86_64-deb":      { "url": "…", "sha256": "…", "kind": "deb" },
    "linux-x86_64-pacman":   { "url": "…", "sha256": "…", "kind": "pacman" }
  }
}
```

Règles appliquées par `ih_update::parse_manifest` : `schema` supporté,
version semver, URL **HTTPS obligatoire**, `sha256` hexadécimal de 64
caractères, `kind` connu.

## Déroulement

1. `update_check` récupère `manifest.json` et `manifest.json.sig`, vérifie la
   signature, puis compare les versions (refus de tout downgrade).
2. `update_download` télécharge dans `staging/` en calculant le SHA-256 au fil
   de l'eau, vérifie le digest, puis renomme le `.part`.
3. `update_request_approval` crée une demande d'approbation (catégorie
   `install`).
4. `update_apply(approvalId)` consomme l'approbation puis :
   - **AppImage** : sauvegarde horodatée dans `backups/`, copie dans un fichier
     temporaire du même dossier, permissions préservées, remplacement
     atomique par `rename` ;
   - **deb / pacman** : aucune exécution. IH renvoie la commande exacte
     (`pkexec apt-get install -y <fichier>` ou
     `pkexec pacman -U --noconfirm <fichier>`) que l'utilisateur lance
     lui-même.

## Outils de publication

```bash
# 1. Générer la paire de clés (une seule fois ; la clé privée ne doit JAMAIS
#    être versionnée ni copiée dans le dépôt)
bash scripts/gen-update-key.sh .release-keys

# 2. Publier les artefacts, écrire manifest.json, puis le signer
bash scripts/sign-release.sh manifest.json .release-keys/ih-update-private.pem
# → manifest.json.sig, avec vérification immédiate par ih-update-verify
```

`ih-update-verify` permet aussi de valider une publication en CI :

```bash
cargo run --release -p ih-update --bin ih-update-verify -- \
  manifest.json manifest.json.sig <clé_publique_hex>
```

Codes de sortie : `0` valide, `1` invalide, `2` usage incorrect.

## Récupération

- `rollback_appimage(backup, cible)` restaure contenu et permissions.
- Les sauvegardes sont conservées dans `~/.config/ih/backups/` avec un nom
  horodaté et un nonce aléatoire (pas d'écrasement).
- En cas de paquet `deb`/`pacman` défectueux, le rollback est du ressort du
  gestionnaire de paquets.

## Limites

- Pas de mise à jour delta ni de reprise `Range`.
- Pas de rotation/révocation de clé : prévoyez une nouvelle clé publique dans
  une version publiée avant de changer de clé de signature.
- `released_at` est informatif ; aucune protection anti-rejeu n'est appliquée
  au-delà du refus de downgrade (binaire plus ancien).
- Les skills et plugins ne passent pas par ce système : ils ont leur propre
  vérification d'intégrité SHA-256 (voir `docs/SECURITY.md`).
