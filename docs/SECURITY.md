# Modèle de sécurité d'IH

## Contrôles implémentés

### Approbation explicite à usage unique
`ih-security::approval` gère un registre `Pending → Approved → Consumed`
(durée de vie 5 min par défaut). Une approbation :
- ne peut être consommée qu'**une seule fois** ;
- est liée à une **catégorie** (`shell`, `fs_write`, `network`, `mcp_tool`,
  `install`, `secrets`) : une approbation « shell » ne permet pas un appel MCP ;
- expire et devient inutilisable passé le délai ;
- est refusée si l'utilisateur a dit non.

Les appels d'outils MCP renvoient d'abord `{ status: "approval_required",
approval_id }` : aucune exécution n'a lieu avant décision.

### Classification du risque
`classify_command` analyse la ligne de commande et renvoie un niveau
`low → critical` avec les raisons détectées. Toute commande shell exige une
approbation, quelle que soit la catégorie ; les cas `critical` (suppression
récursive de la racine, `mkfs`, `dd of=/dev/`, `curl | bash`, fork bomb…)
imposent une confirmation supplémentaire côté interface.

### Prison de chemins
`ih-workspace::fs::ProjectFs` canonicalise la racine et refuse avant toute
écriture :
- les chemins absolus,
- les segments `..`,
- les liens symboliques qui sortent de la racine,
- les fichiers binaires et fichiers trop volumineux pour l'éditeur.

Les tests couvrent `../etc/passwd`, `/etc/passwd`, un lien symbolique sortant
et un lien interne légitime.

### Secrets
- Stockage **exclusivement** dans le trousseau du système (Secret Service) via
  `keyring` ; aucun repli en clair n'existe dans le code.
- La configuration ne contient qu'un booléen `has_secret`.
- Si le trousseau est verrouillé, l'enregistrement échoue explicitement plutôt
  que d'écrire un secret sur disque.
- Les journaux passent par `ih_core::redaction` (préfixes de clés connus,
  `Bearer`, `api_key=`, paramètres d'URL, clés AWS, champs JSON sensibles).

### Processus enfants
- Un processus par session, groupe de processus dédié (`process_group(0)`).
- Sortie bornée, troncature signalée, `ETXTBSY` géré par une nouvelle tentative.
- Timeout par agent, annulation qui tue le groupe complet, récolte du code de
  sortie, aucun zombie (vérifié par test `/proc/<pid>`).
- Les CLI héritent de `PATH`/`HOME` mais **aucun secret** ne leur est injecté
  par IH.

### Contexte projet et écriture de fichiers
- Le contexte est assemblé **côté Rust** (`ih-context`) : l'interface ne lit
  jamais un fichier pour le compte du modèle.
- Toute lecture passe par `ProjectFs` (prison de chemins, canonicalisation,
  refus des symlinks sortants).
- Exclusions par défaut : `.env`, `.env.*`, `id_rsa`, `id_ed25519`, `*.pem`,
  `*.key`, `*.p12`, `.ssh/`, `node_modules/`, `target/`, `.git/`, `dist/`,
  `build/`, `.venv/`, `__pycache__/`. Liste complétable dans la configuration.
- Détection de secrets **par contenu** (clé privée, `api_key =`, `sk-…`,
  mot de passe) : inclusion seulement après confirmation explicite, et le
  fichier est marqué `sensitive` dans l'aperçu.
- Limites : 12 fichiers, 64 Kio par fichier, 256 Kio au total (configurables).
- **Écriture** : `workspace_save` et `diff_accept` exigent l'empreinte SHA-256
  lue à l'ouverture ; une divergence produit `conflict` et rien n'est écrit.
  Une sauvegarde de l'ancien contenu est créée dans `~/.config/ih/backups/`.

### Mises à jour
- Signature Ed25519 détachée obligatoire + SHA-256 obligatoire.
- Téléchargement dans un fichier `.part`, renommé seulement après vérification.
- Plafond de taille (500 Mio) et de métadonnées.
- Aucune installation sans approbation utilisateur explicite ; `deb`/`pacman`
  ne sont jamais exécutés par IH, seulement proposés sous forme de commande.

### Frontière WebView
- Capacités Tauri minimales : `core:default` + `dialog:default`.
- CSP restrictive (`default-src 'self'`, `connect-src ipc:`).
- Aucun `dangerouslySetInnerHTML` dans l'interface.
- Les URL externes ne peuvent être ouvertes que si elles commencent par
  `https://` ou `http://` (`open_external`).

## Limites connues (assumées, non contournées)

1. **HTTPS obligatoire** pour les artefacts de mise à jour : même un serveur
   local en HTTP est refusé par le parseur de production.
2. Pas de sandbox OS (bubblewrap/firejail) autour des CLI d'agents : ils
   s'exécutent avec les droits de l'utilisateur. C'est un choix documenté, pas
   une garantie.
3. La vérification de chemin est faite avant ouverture (risque TOCTOU résiduel
   sur un système de fichiers partagé).
4. Les skills ne sont pas signés : le SHA-256 garantit l'intégrité, pas
   l'origine. N'installez que des skills dont vous connaissez la source.
5. Le rollback n'est implémenté que pour l'AppImage ; pour `deb`/`pacman`, le
   gestionnaire de paquets fait foi.

## Ce qu'IH ne fait pas

- Il n'exécute jamais une commande shell de sa propre initiative.
- Il n'installe jamais de plugin ou de skill sans action utilisateur.
- Il n'envoie jamais de clé d'API dans un journal, une erreur ou un fichier de
  configuration.
- Il ne supprime jamais les fichiers d'un projet retiré de la liste.
