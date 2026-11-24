# Protection des données locales d'IH

Ce document décrit l'incident de profil survenu pendant la recette 0.6.2, ce
qui est récupérable (et ce qui ne l'est pas), et les protections désormais en
place.

## Incident du 24 novembre 2026 — perte du profil utilisateur

### Ce qui a été fait (erreur)

1. Un test graphique a été lancé sur le profil réel `~/.config/ih`, qui a migré
   la base en v6 (schéma) ; l'application a été arrêtée brutalement.
2. Une « sauvegarde » a été prise par simple `cp` du **fichier principal**
   `ih.sqlite3` (4 Kio) **en ignorant `ih.sqlite3-wal`** (313 Kio), où se
   trouvaient toutes les transactions non encore checkpointées.
3. Le fichier principal a ensuite été **écrasé par cette copie** puis les
   fichiers `-wal` et `-shm` ont été **supprimés**.

### Cause exacte

Deux fautes distinctes, cumulatives :

- **copie non cohérente** : en mode WAL, le fichier principal peut être vide
  (en-tête seul, `user_version = 0`) alors que l'essentiel des données vit dans
  `-wal` ; un `cp` du seul fichier principal ne sauvegarde rien ;
- **suppression manuelle du WAL** : après remplacement du fichier principal,
  supprimer `-wal`/`-shm` détruit définitivement les transactions non
  checkpointées au lieu de laisser SQLite les rejouer.

### État constaté après l'incident

- `~/.config/ih/data/ih.sqlite3` : 4 096 octets, `user_version = 0`,
  `sqlite_master` vide, `PRAGMA integrity_check` = `ok` — base techniquement
  valide mais **sans aucune donnée**.
- Aucun `-wal`, aucun `-shm`, aucun fichier `.bak`, aucun `.part`.
- `~/.config/ih/backups/` et `staging/` vides ; `logs/` ne contiennent que les
  lignes de migration du dernier lancement.
- Aucun autre exemplaire du profil réel (recherche sur `$HOME` et `/tmp` :
  seuls des profils de recette isolés existent).
- `config.toml` intact (restauré) — fournisseurs, agents, langue, projets
  récents. Keyring du système jamais touché (aucun secret en base).
- Constats consignés : `Documents/ih-incident-2026-11-24/evidence/`
  (empreintes SHA-256 des fichiers d'origine et de la copie de sécurité).

### Récupération : verdict honnête

**Aucune récupération n'a été réussie ni n'est réalisable avec les accès
disponibles.** Raisons :

1. le fichier supprimé (WAL) ne peut être retrouvé par les outils normaux ;
2. un accès au périphérique brut est nécessaire pour une éventuelle
   récupération ext4 (`debugfs`/`extundelete`), or `sudo` exige un mot de passe
   non disponible dans la session ;
3. aucun instantané système n'existe (pas de Timeshift/Snapper/Btrfs) ;
4. depuis la suppression, plusieurs gigaoctets ont été écrits sur la même
   partition (compilations, tests, artefacts) : les blocs de l'ancien WAL ont
   très probablement été réattribués, ce qui réduit une tentative forensique à
   une probabilité quasi nulle.

Données perdues : projets enregistrés, conversations et messages, réglages
stockés en base (disposition, permissions MCP par rôle), serveurs/outils MCP
distants. Données conservées : `config.toml` (fournisseurs, agents, locale,
thème, projets récents) et secrets du trousseau.

### Reprise recommandée

1. Faire d'abord une copie de sécurité du profil actuel (même vide) :
   `scripts/backup-profile.sh --allow-real-profile`
2. Lancer IH : le schéma est recréé par migration ; rouvrir le projet
   `~/Documents/IH` via l'interface.
3. L'historique de conversations n'est pas récupérable.

## Protections en place

### 1. Sauvegarde automatique avant chaque migration (noyau)

`ih-store` écrit une copie **cohérente** de la base avant toute migration de
schéma, via l'API `backup` de SQLite (compatible WAL) :
`<base>.pre-v<ancienne version>-<horodatage>.bak`, à côté de la base.
Un échec d'écriture **interrompt l'ouverture** plutôt que de migrer sans
filet. Sans migration requise, aucun fichier n'est créé.
Tests : `sauvegarde_avant_migration_preserve_les_donnees`,
`aucune_sauvegarde_quand_le_schema_est_a_jour` (`crates/ih-store`).

### 2. Script de sauvegarde protégé (`scripts/backup-profile.sh`)

- refuse le profil utilisateur réel sauf `--allow-real-profile` explicite ;
- utilise `sqlite3 .backup` (cohérent WAL, contrairement à `cp`) ;
- vérifie la sauvegarde par `PRAGMA integrity_check` et affiche son SHA-256 ;
- ne supprime ni ne déplace jamais la source ;
- sort en erreur si la base est absente ou si la destination est la source.

### 3. Règles de recette (obligatoires)

- Toute recette graphique utilise un profil isolé :
  `IH_HOME=/tmp/ih-recette-<nom> ./target/release/ih` — jamais le profil réel.
- Les clés d'API de recette vivent dans un trousseau de session isolé
  (`XDG_DATA_HOME` temporaire + `gnome-keyring-daemon --unlock`), jamais dans
  le profil réel.
- Ne jamais copier une base SQLite par `cp` : utiliser `scripts/backup-profile.sh`
  ou `sqlite3 "$db" ".backup ..."`.
- Ne jamais supprimer `-wal`/`-shm` à la main. Pour restaurer : arrêter
  l'application, sauvegarder l'état courant, remplacer le fichier principal,
  laisser SQLite rejouer le WAL ; un checkpoint propre se fait avec
  `sqlite3 "$db" "PRAGMA wal_checkpoint(TRUNCATE);"`.
- Vérifier `PRAGMA integrity_check` et `PRAGMA user_version` après toute
  manipulation, avant de relancer l'application.

### 4. Tests et profils temporaires

Tous les tests automatisés utilisent une base en mémoire ou un répertoire
temporaire (`TempDb`), jamais `~/.config/ih`. Les recettes graphiques
documentées ci-dessus utilisent `IH_HOME`.
