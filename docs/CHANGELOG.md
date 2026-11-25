# Journal des versions

## 0.6.5 — Nouveau logo (25/11/2026)

### Modifié
- **Logo de l'application** : icônes système et marque de l'interface
  remplacées par le nouveau logo (emblème sur tuile blanche), recadrées en
  carré depuis le fichier fourni (1024×1024).

## 0.6.4 — Agents CLI (Kilo, DSH CLI, Goose), sandbox système et corrections d'audit (25/11/2026)

### Ajouté
- **Trois nouveaux adaptateurs d'agents CLI**, au même niveau que les agents
  existants : Kilo (`kilo run --format json --auto "<tâche>"`), DSH CLI
  (`dsh --profile headless "<tâche>"`) et Goose
  (`goose run --no-session --quiet --output-format stream-json -t "<tâche>"`).
- **Détection renforcée** : nom configuré et alias connus, `PATH` de
  l'application, `PATH` du shell de connexion (comme OpenCode) et dossiers
  connus (`~/.local/bin`, `~/.cargo/bin`, `~/.bun/bin`, `~/.npm-global/bin`,
  `/usr/local/bin`, `/snap/bin`, `flatpak`, paquets `/opt/apps/*/files/bin`).
  En cas d'homonymes, seule l'installation qui répond correctement à
  `--version` est retenue ; un faux positif est signalé et non sélectionnable.
- **Normalisation** : Kilo réutilise le parseur JSON d'OpenCode (même famille
  d'événements) ; Goose a son parseur `stream-json` ; DSH affiche son message
  final (interface texte officielle du profil `headless`).
- **Versions et chemins** affichés dans la page Agents ; l'état du moteur
  signale les exécutables trouvés mais non fonctionnels.
- **Avertissement « mode autonome »** : quand la commande contient un drapeau
  d'autonomie (`--auto` pour Kilo, `--yolo`, …), le détail d'approbation précise
  que l'agent peut écrire, exécuter, supprimer et accéder au réseau hors du
  worktree avec les droits de l'utilisateur, et que ces actions ne sont pas
  couvertes par le diff. Ces drapeaux apparaissent aussi dans les capacités de
  l'adaptateur (audit du 25/11 : portée réelle vérifiée sur kilo 7.8.32).

### Modifié
- Le superviseur lance l'**exécutable résolu** (chemin absolu) et transmet les
  variables d'environnement déclarées dans la configuration de l'agent.
- Sélection d'adaptateur : un binaire seulement détecté mais non fonctionnel
  n'est plus sélectionnable. Pour DSH, le CLI livré avec le paquet
  d'application (`/opt/apps/.../bin/dsh`, Node embarqué) est préféré à une
  autre installation npm homonyme : une copie nvm 0.2.2-rc.5 se termine sans
  sortie en profil `headless`, ce qu'IH ne peut pas accepter comme succès.
- Un exécutable utilisateur est monté dans la sandbox par sa **cible résolue**
  (liens symboliques npm résolus, paquet `node_modules` entier) : un script
  Node a besoin de son paquet, pas du seul fichier de lancement.

### Sécurité — sandbox système des agents
- **Isolation réelle par bubblewrap** (Zorin OS / Ubuntu : `apt install bubblewrap`, Arch : `pacman -S bubblewrap`) : `/usr`, `/etc` en lecture seule, `/tmp` privé, worktree seul répertoire inscriptible, dossier personnel privé, `--proc`/`--dev` dédiés, PID/IPC/UTS isolés, `--die-with-parent`.
- **Trois niveaux par agent** (`security` dans `[[agents]]`) : `isolated` (réseau coupé), `isolated_network` (**défaut**, provider distant), `none` (non isolé, confirmation explicite obligatoire, jamais de bascule silencieuse). Le niveau fait partie de l'empreinte d'approbation.
- **Environnement reconstruit** (`--clearenv` puis HOME/PATH/TMPDIR/LANG/TERM + variables déclarées) : aucun héritage des secrets de l'utilisateur.
- **Secrets contrôlés** : seuls les identifiants propres à l'agent sont copiés (0600) dans son dossier privé (Kilo auth.json/config, DSH credentials/profils, Goose config/secrets), puis supprimés en fin de session.
- **Refus explicite** si bubblewrap est indisponible ou inopérant : aucun repli vers une exécution non sandboxée.
- Interface : niveau, moteur, état du réseau et avertissements affichés dans la page Agents, le mode Agent, l'approbation et le résumé d'équipe.
- Tests réels (agents installés, canaris) : lecture/écriture dans le worktree autorisées ; canari `$HOME`, `~/.ssh`, sockets, écriture/suppression hors worktree refusés ; réseau coupé en `isolated` ; annulation effective ; diff limité au worktree. Détails : `docs/AGENT_SECURITY.md`.

### Corrigé (audit indépendant de sécurité)
- **Protection du dossier personnel** : un exécutable placé directement dans
  `$HOME` (ou dont la racine de montage contiendrait `$HOME`, `~/.ssh`,
  `~/.aws`, `~/.config`, `~/.local/share`) est **refusé** au lieu de faire
  entrer tout le home en lecture seule dans la sandbox.
- **Moteur de sandbox authentifié** : `IH_SANDBOX_ENGINE` n'est honorée qu'en
  build de test/débogage ; le moteur doit être un binaire absolu, non
  inscriptible par le groupe ou le monde (et possédé par root en production).
  Une **sonde négative** vérifie qu'il refuse réellement une écriture hors
  sandbox : un faux `bwrap` qui ignore les drapeaux est refusé.
- **Acquittement explicite en Mode Équipe** : si un membre s'exécute au niveau
  `none`, la course exige une confirmation dédiée, transmise par l'interface,
  inscrite dans l'empreinte d'approbation et revérifiée au lancement.
- **Sondes sans environnement hérité** : `--version` des agents et le shell de
  connexion sont exécutés avec un environnement minimal (`env_clear`) : plus
  aucune variable de l'application n'est transmise avant la sandbox.
- **Nettoyage garanti des dossiers privés** : garde RAII (succès, erreur,
  échec de lancement, timeout, annulation), permissions `0700`, marqueur de
  propriétaire et purge des dossiers orphelins au démarrage — sans toucher aux
  instances IH encore actives.
- **Identifiants validés** : tout identifiant utilisé comme composant de chemin
  est restreint à `[A-Za-z0-9._-]` (sans `..`, sans `/`, 64 caractères max)
  avant toute opération de fichier, notamment `remove_dir_all`.
- **Git dans le worktree sandboxé** : `git status` et `git diff` fonctionnent
  dans la sandbox (métadonnées du worktree en écriture, dépôt principal monté
  en **lecture seule** : le `.git/config` du dépôt reste intangible).
- **Commande IPC obsolète supprimée** : `agent_start` (lancement sans
  approbation ni worktree) est retirée ; la page Agents renvoie vers le Mode
  Agent, seul chemin avec approbation, worktree et revue de diff.
- **CI** : nouveau job « Sandbox réelle (bubblewrap) » qui installe
  bubblewrap, vérifie les user namespaces, exige que la sandbox refuse une
  écriture hors worktree, puis exécute les tests d'isolation. Les jobs Ubuntu
  et Arch sont conservés.
- **Tests honnêtes** : les tests exigeant bubblewrap ou des agents réels sont
  désormais `#[ignore]` (visibles comme ignorés, jamais « réussis » sans
  exécution) ; les canaris des tests sont créés dans un dossier temporaire et
  ne touchent plus le `$HOME` réel.

### Corrigé (audit final indépendant — NV1 à NV9)
- **NV1 (critique) — évasion de sandbox via `.git`** : le contenu du fichier
  `.git` d'un worktree est inscriptible par l'agent et **ne fait plus
  autorité**. Les montages Git sont calculés par IH à la création du worktree
  (`GitMounts`, depuis le dépôt découvert puis canonicalisé et confiné),
  conservés côté Rust et transmis au superviseur. Un `.git` absent, symbolique,
  malformé, relatif ou pointant ailleurs **refuse le lancement** : aucun montage
  alternatif n'est créé.
- **NV2** : `commondir` est vérifié à chaque lancement ; il doit désigner
  exactement le répertoire Git commun attendu, sinon refus explicite.
- **NV3/NV4 (architecture Git)** : le répertoire Git commun **et** les
  métadonnées du worktree sont montés en **lecture seule** ; l'index de session
  vit dans le home privé (`GIT_INDEX_FILE=/home/agent/.git-index`). Le dépôt
  n'est donc jamais inscriptible et rien de contrôlé par l'agent ne devient un
  montage.
- **NV4 (hôte)** : les inspections Git d'IH sur un worktree d'agent sont
  exécutées avec un environnement minimal et une configuration explicite
  (`core.fsmonitor=false`, `core.hooksPath=/dev/null`, `diff.external=`,
  `protocol.allow=never`, …) : fsmonitor/hooks/textconv/`GIT_*` du dépôt ne
  peuvent plus faire exécuter un programme sur l'hôte (test canari).
- **NV3 (dossier privé)** : le marqueur `.ih-owner` est **obligatoire** (échec
  d'écriture ⇒ création refusée et dossier supprimé) et un **verrou `flock`**
  est tenu pendant toute la session : le nettoyage des orphelins ne supprime
  jamais un dossier vivant, même avec un PID réutilisé (NV8 : PID + instant de
  démarrage de `/proc/<pid>/stat`).
- **NV5** : la racine de montage d'un exécutable est canonicalisée et un
  répertoire symbolique est refusé ; la fenêtre TOCTOU résiduelle est
  documentée.
- **NV6** : le moteur de sandbox est **revalidé à chaque lancement**
  (identité dev/ino/taille/mtime/uid puis nouvelle sonde si changement).
- **NV9** : les approbations d'agent et d'équipe utilisent un **champ
  d'empreinte dédié** (`ask_with_fingerprint` / `consume_with_fingerprint`)
  au lieu d'une recherche dans le texte du détail.
- Tests de régression : canaris `.git`/`commondir` (HOME factice, `.ssh`
  factice, `..`, lien symbolique, chemin inexistant, chemin valide), Team Mode
  (empoisonnement refusé à la session suivante), Git normal (`status`/`diff`,
  index dans le home privé), exécution contrôlée par le dépôt (canari
  fsmonitor/textconv), dossier privé (marqueur obligatoire, verrou).

### Vérifié
- Tests réels exécutés sur la machine de validation (`IH_REAL_AGENTS=1`) :
  détection et version (`kilo 7.8.32`, `dsh 0.1.0-rc.7`, `goose 1.61.0`),
  lancement et réponse, annulation effective du groupe de processus, erreur
  volontairement provoquée, isolation par worktree git et collecte des fichiers
  modifiés — pour les trois CLI.
- Sandbox réelle : les trois agents écrivent `sbox-ok.txt` dans le worktree
  avec un accès hôte refusé (canaris `$HOME`, `~/.ssh`, sockets), un prompt
  hostile ne fait fuiter aucun canari, et le niveau `isolated` échoue
  explicitement chez les trois (DNS/transport coupés).
- 442 tests Rust (+17 tests d'isolation/agents ignorés explicitement), 188 tests frontend,
  `tsc --noEmit` et build de production verts.

## 0.6.3 — Cohérence visuelle de l'interface

### Corrigé
- **Sélecteurs Fournisseur / Modèle** : les menus déroulants étaient rendus par
  le thème GTK natif (fond `#343434`, hauteur et padding imposés), donc
  désalignés des champs, du badge « Prêt » et du bouton d'options. Ils sont
  désormais dessinés par le design system (`appearance: none`, chevron CSS,
  32 px, rayon 8), avec les mêmes états de focus que les autres champs.
- **Grille de mise en page** : jetons d'espacement 8 px (`--space-*`), hauteurs
  de contrôles (32 / 24 px) et rayons (8 px contrôles, 12 px cartes) pour un
  rendu homogène d'une page à l'autre.
- **Menu latéral** : largeur 240 px (208/192 px sous 1040/960), espacements
  verticaux réguliers, entrées de navigation de 36 px.
- **Barre supérieure** : hauteur 64 px, fil d'Ariane lisible, sélecteur « Projet
  courant » de 32 px, bascule FR/EN de 32 px, icônes d'action 32×32 avec zone
  cliquable cohérente.
- **En-tête de page** : marges et espacements du titre, du sous-titre et des
  actions alignés sur la grille.
- **Onglets Chat / Agent et « Nouvelle conversation »** : hauteurs et
  espacements identiques (32 px, 8 px d'écart).
- **Barre Fournisseur / Modèle** : une rangée de 48 px, contrôles de 32 px
  alignés, badge « Prêt » et bouton de réglages alignés, gap de 8 px, plus
  aucun chevauchement.
- **Zone centrale et composeur** : messages espacés de 12 px, texte de 13 px /
  1.6, composeur de 12/16 px de padding, saisie de 72 px, rangée
  Contexte · Outils MCP · compteur · Envoyer alignée sur 32 px.
- **Composants partagés** (boutons, champs, badges, rayons, focus) harmonisés,
  y compris aux largeurs réduites.

### Vérifié
- 182 tests frontend, `tsc --noEmit` et build de production verts ; captures aux
  résolutions 1366×768, 1440×920 et 1920×1080 ; saisie testée dans le
  composeur ; aucun débordement horizontal.
- Aucune modification de fonctionnalité, de logique métier, d'IPC ni de données.

## 0.6.2 — Tool calling MCP natif dans Chat

### Ajouté
- **Modèle commun de tool calling** (`ih-providers`) : `ToolDefinition`,
  `ToolCall`, `ToolCallResult` ; `ChatRequest.tools`, `ChatMessage.tool_calls` /
  `tool_call_id`, `ChatResponse.tool_calls`. La capacité est **déclarée**, jamais
  devinée : `Provider::supports_native_tools()` (défaut `false`).
- **Fournisseur OpenAI-compatible** : outils sérialisés au format
  `{"type":"function",…}`, messages assistant à `tool_calls` et messages
  `tool`, parsing non-streaming (arguments chaîne JSON ou objet) et streaming
  SSE avec appels **fragmentés** (index, identifiant, nom, arguments) ;
  arguments invalides remontés en erreur explicite ; flux sans outils inchangé.
- **Découverte et conversion** (`src-tauri/commands/mcp_tools.rs`) : seuls les
  serveurs **activés et connectés** et les outils **activés par l'utilisateur**
  sont exposés, sous un nom sûr `{serveur}__{outil}` (≤ 64 caractères,
  caractères normalisés, hachage déterministe des noms longs, suffixe en cas de
  collision). Table inverse nom exposé → (serveur, outil) gardée côté noyau :
  aucune URL, aucun jeton, aucun en-tête ne part au modèle. Schémas non-objet
  écartés, catalogue borné (24 outils, 8 Kio par schéma, 32 Kio au total).
  IPC `chat_tools_list` sans secret.
- **Cycle sécurisé dans Chat** : une réponse du modèle avec `tool_calls` ne
  déclenche **aucune** exécution. Chaque demande devient une proposition
  persistée `pending` (arguments redactés pour l'audit et l'interface) et une
  carte `ih://chat-tool-call`. La décision humaine (`chat_tool_decide`)
  matérialise une **approbation à usage unique liée à l'empreinte** des
  paramètres, puis appelle `invoke_mcp_tool` — seul chemin d'exécution. Le
  résultat est renvoyé au modèle comme message `tool` et le tour reprend
  (bornes **testées** : 3 cycles de continuation, 6 appels par tour, les
  propositions invalides ne consommant pas de budget). Aucun chemin parallèle,
  aucune reprise automatique après redémarrage.
- **Persistance** (migration SQLite **v6**) : table `chat_tool_calls`
  (tour, cycle, identifiant d'appel fournisseur, nom exposé, serveur/outil
  réels, risque, catégorie, empreinte SHA-256, arguments redactés et argents
  bruts d'exécution, statut, erreur, résultat, approbation, horodatages) avec
  transitions verrouillées. Une proposition `pending` n'est jamais rejouée.
- **Interface** : cartes d'appels d'outils dans la conversation (serveur/outil,
  risque, statut, paramètres dépliables **valeurs sensibles masquées**,
  résultat, erreur), boutons Approuver/Refuser, rafraîchissement par événements
  et depuis la base, y compris pendant les cycles de continuation.
- **Tests réels** : cycle complet de bout en bout contre un **serveur MCP local
  réel** (Streamable HTTP) et un **fournisseur OpenAI simulé** (serveur HTTP
  local) : requête d'outil → proposition → approbation liée à l'empreinte →
  exécution MCP → message `tool` → continuation → réponse finale, avec
  vérification de l'empreinte (rejeu refusé) et de l'unicité du message
  utilisateur.

### Modifié
- `ProviderConfig.supports_tools` (défaut `false`) : opt-in explicite du tool
  calling natif, exposé dans l'interface de configuration des fournisseurs
  (famille OpenAI-compatible ; les autres familles annoncent honnêtement
  l'absence de support).
- Chat : si la capacité est déclarée, les outils partent en natif et le
  protocole texte `ih-mcp-call` n'est pas injecté ; sinon le comportement 0.6.1
  est conservé à l'identique (aucune régression).
- Anthropic, Gemini et Ollama : `tool_calls` vide, capacité fausse, fallback
  texte préservé.
- Version **0.6.2**.

### Sécurité
- Invariants 0.6.1 conservés : point d'entrée unique `invoke_mcp_tool`,
  anti-SSRF (URL validée, DNS revalidé, redirections non suivies, proxys
  ignorés), approbations à usage unique liées à l'empreinte, redaction des
  arguments et des erreurs, tailles et délais bornés, aucun secret au modèle.
- Nouveau : le modèle ne peut jamais exécuter un outil sans décision humaine
  explicite ; une carte ne s'exécute qu'une fois (réclamation atomique en base
  `claim_chat_tool_call`, une décision `deny` est refusée après réclamation) ;
  un événement de continuation ne peut pas contourner la décision ; toute
  issue d'exécution autre que `completed` est traitée comme un échec explicite.
- **MCP stdio historique** : les appels de la page Plugins (`mcp_call_tool`)
  appliquent désormais les mêmes contrôles que le garde-fou distant — serveur
  activé (nouvelle commande `mcp_stdio_set_enabled`, session fermée à la
  désactivation) et connecté, outil réellement annoncé par `tools/list`,
  arguments validés contre son `inputSchema`, classification locale du risque,
  approbation interactive à usage unique liée à l'empreinte et trace d'audit
  redactée. Le chemin reste isolé du chat, du modèle, des agents, de l'équipe
  et des automatisations (catalogue distant uniquement), ce qui est
  désormais couvert par un test de régression.
- **Protection des données locales** : sauvegarde cohérente (API backup SQLite,
  WAL inclus) écrite automatiquement avant toute migration de schéma, un échec
  interrompant l'ouverture plutôt que de migrer sans filet ; script
  `scripts/backup-profile.sh` qui refuse le profil utilisateur réel sauf
  autorisation explicite et vérifie l'intégrité de la copie ; post-mortem et
  procédures dans [DATA_SAFETY.md](DATA_SAFETY.md).

## 0.6.1 — MCP natif : Streamable HTTP et SSE

### Ajouté
- **Crate `ih-mcp`** : client MCP natif.
  - Transport **Streamable HTTP** via `rmcp` 3.11 (SDK officiel,
    Apache-2.0) : négociation, sessions `Mcp-Session-Id`, flux SSE de réponse,
    reconnexions, expiration de session.
  - Transport **SSE historique** (HTTP+SSE 2024-11-05) implémenté localement :
    le SDK officiel l'a retiré. Flux borné, délais en envoi et en attente,
    reconnexions contrôlées (repli exponentiel, abandon explicite).
  - URL **épinglée** après résolution DNS (anti-rebinding), redirections non
    suivies, proxys ignorés, en-têtes réservés refusés.
  - Validation structurelle des schémas d'outils et des arguments.
- **Sécurité** (`ih-security`) : politique réseau anti-SSRF (blocage métadonnées
  cloud/link-local/CGNAT/réservées, revalidation des adresses résolues) ;
  classification MCP (lecture/écriture/exécution/destructif/réseau) ;
  décision d'appel (auto-approbation limitée aux lectures pré-autorisées) ;
  empreinte canonique SHA-256 des paramètres ; approbations **liées à
  l'empreinte** (`ask_with_fingerprint`, `consume_with_fingerprint`).
- **Persistance** (migration SQLite v4) : `mcp_servers`, `mcp_tools`,
  `mcp_calls` ; catalogue avec préservation des activations ; historique pour
  l'observabilité ; cascade à la suppression. Aucun secret en base.
- **IPC** : `mcp_call` (point d'entrée unique : serveur, outil, arguments,
  permission, approbation, exécution, historique, audit), gestion complète des
  serveurs (ajout/édition/suppression/activation/connexion/déconnexion/test/
  rafraîchissement), catalogue d'outils, historique, export de configuration
  sans secrets. Événements `ih://mcp-status`, `ih://mcp-discovered`,
  `ih://mcp-call`.
- **Conversation** : bouton « Outils MCP » dans le composeur, panneau d'appel
  contrôlé (approbation liée aux paramètres) et insertion du résultat dans le
  message ; partagé avec la page MCP.
- **Proposition d'outil par le modèle** : les outils MCP activés sont décrits
  dans le prompt système ; une proposition `ih-mcp-call` est filtrée (couples
  serveur/outil autorisés), affichée avec son risque, puis exécutée seulement
  après approbation ; le résultat est ajouté au fil comme message `tool`.
  (Pas de tool use natif des fournisseurs ; résultat non réinjecté au modèle.)
- **Automatisations** : étapes « outil MCP » (`server_id`, `mcp_tool`,
  `arguments`) exécutées via le même point d'entrée, avec la règle de mode sans
  interaction (seules les lectures pré-autorisées s'exécutent).
- **Agents / Team** : `mcp_agent_config` génère la configuration MCP des CLI
  (Claude Code, Gemini CLI, OpenCode) sans secrets ; fichier non injecté
  automatiquement (documenté).
- **Permissions MCP par rôle d'équipe** : `mcp_role_permissions_get/set`
  (Lead/Worker/Reviewer), édition dans la page MCP, vérification dans
  `invoke_mcp_tool` avant approbation ; rôle absent = défaut global, liste
  vide = aucun outil, appel Team sans rôle refusé.
- **Interface** : section **Serveurs MCP** (transport, URL, délai, jeton
  trousseau, en-têtes JSON, pré-autorisation), statuts lisibles, catalogue avec
  risque et activation individuelle, appel avec approbation affichant serveur,
  outil, paramètres, effet, risque et empreinte ; historique compact ; export.

### Tests
- `ih-mcp` : 29 tests unitaires + **9 tests d'intégration Streamable HTTP** et
  **4 tests d'intégration SSE** contre un vrai serveur MCP local (JSON-RPC et
  flux SSE réels, 127.0.0.1) : connexion, découverte, appels, erreurs serveur,
  délai, réponse surdimensionnée, JSON invalide, schémas invalides exclus,
  jeton exigé, redirection non suivie, reconnexion après coupure.
- `ih-security` : URL/SSRF, classification, décision, empreintes, approbations
  liées (42 tests).
- `ih-store` : migration v4, CRUD serveurs, catalogue (préservation
  d'activation, purge), historique et cascade (19 tests).
- Frontend : helpers MCP (schéma, risques, troncature) ; i18n FR/EN en parité.

### Corrigé
- **Revue de sécurité (avant merge 0.6.1)** — durcissements :
  - arguments d'appel **redactés** avant persistance dans `mcp_calls` et dans
    le détail d'audit/approbation (un argument nommé `api_key`, `token`,
    `secret`, `password`… n'atteint plus SQLite en clair) ;
  - borne de taille des arguments exécutés (256 Kio) avant toute exécution ;
  - **serveurs MCP stdio (page Plugins)** : approbation désormais liée à
    l'empreinte des paramètres (comme les serveurs distants), risque classé
    localement au lieu d'un niveau figé, arguments obligatoirement un objet.
- Le trousseau système n'est plus sollicité au démarrage : le statut ne lit
  plus d'entrée sonde ; la liste des fournisseurs n'effectue plus de lecture
  (drapeau `has_secret` persisté), et l'existence d'un jeton MCP est désormais
  un drapeau en base (migration v5, `mcp_servers.has_secret`). Le chat local
  (Ollama) ne lit plus le trousseau. Le déverrouillage n'est demandé qu'au
  premier usage réel d'un secret (conversation avec clé, test de fournisseur,
  connexion/appel MCP authentifié).

### Limites assumées
OAuth 2.1 non intégré ; appel d'outil proposé par le modèle (tool use) non
câblé ; injection automatique dans les CLI d'agents et étapes MCP des
automatisations non implémentées ; ressources/prompts/sampling MCP non exposés.
Détail : [docs/MCP.md](MCP.md).


## 0.2.0 — AI Workspace · Chat

### Ajouté
- Crate `ih-chat` : moteur de conversation indépendant de Tauri.
  - Persistance du message utilisateur **avant** l'appel réseau.
  - Streaming fragment par fragment vers l'interface (`ih://chat-delta`).
  - **Annulation réelle** : le flux est interrompu, le texte partiel est
    conservé en base et marqué `cancelled`.
  - Aucun message assistant vide en cas d'erreur fournisseur.
  - Historique réinjecté dans l'ordre, borné (40 par défaut, 200 max).
  - Réponse plafonnée à 2 Mio, troncature signalée.
  - Titre de conversation dérivé du premier message.
  - Recherche de conversations par titre **et** par contenu des messages.
- Commandes IPC : `chat_send`, `chat_cancel`, `chat_active`, `thread_rename`,
  `threads_search`.
- Événements : `ih://chat-delta`, `ih://chat-done`, `ih://chat-error`.
- Interface : section **Conversation** (liste des conversations, recherche,
  création, renommage, suppression, streaming, arrêt, réessai, sélection du
  provider et du modèle, indicateur de ressources).

### Tests
- `ih-chat` : 9 tests unitaires + 1 test de bout en bout avec le **vrai** client
  Ollama (serveur NDJSON local) = 10 tests.
- Frontend : 13 tests ajoutés (réduction du flux, erreurs) → 49 au total.
- Rust : 218 tests au total.
- Validation manuelle dans l'application réelle : streaming visible et
  annulation persistée (`cancelled: true`, texte partiel conservé).

### Corrigé
- Le registre des générations n'était jamais vidé en fin de tour : `chat_active`
  comptait des tours terminés. L'entrée est maintenant libérée en fin de tour,
  succès comme erreur.
- La zone de saisie défilait avec les messages : la liste défile désormais dans
  son propre conteneur et le compositeur reste ancré en bas.

## 0.5.2 — Team Mode : validation graphique et fiabilisation

### Corrigé (découvert dans l'application réelle)
- **Ordre d'exécution non topologique** : `TaskGraph::iter()` suivait les
  identifiants aléatoires (Lead parfois en dernier, Reviewer avant un Worker).
  Nouvel ordre `Lead → Workers → Reviewer` respectant les dépendances, utilisé
  pour l'exécution, la persistance et le plan affiché ; test dédié.
- **Interface figée pendant une course** (« IH ne répond pas ») : `team_run`
  bloquait le thread principal. Commande déportée sur un thread de travail.
- **Texte d'aide obsolète** sur les exécutants (le même agent peut tenir
  plusieurs rôles).
- **Rappel de mode comprimé** en fenêtre étroite (masqué sous 1400 px).

### Ajouté
- `team_fingerprint` / `approval_covers` partagés entre la commande et les
  tests ; tests d'invalidation : objectif, Lead, Workers (ordre inclus),
  Reviewer, répertoire, empreinte d'une autre course.
- `run_team_tasks_with_events` : les événements de tâche sont émis depuis le
  chemin d'exécution réel ; test d'intégration vérifiant ordre, états et
  absence de doublon.
- Validation graphique réelle documentée (captures `team-form`,
  `team-approval`, `team-progress`, `team-review`, `team-small-window`).

### Validé en conditions réelles
Parcours complet dans l'application sur Zorin OS : bascule Équipe, sélection
des agents détectés, même CLI multi-rôles, approbation avec plan et empreinte,
progression live réactive, résumé, revue des diffs, rejet sans écriture,
acceptation avec sauvegarde, fenêtre 1280×800.

### Non validé
Expiration du TTL d'approbation en conditions réelles, réutilisation d'une
approbation consommée via l'interface, worktree indisponible, fichier modifié
extérieurement avant acceptation. Reprise, annulation et parallélisme ne sont
pas implémentés.

## 0.5.1 — Team Mode : validation réelle et correctifs

### Corrigé
- **Course impossible avec un seul CLI** : `Team::validate` et le formulaire
  refusaient le même agent dans deux rôles. Aucune équipe n'était lançable avec
  OpenCode seul. Un agent peut désormais tenir plusieurs rôles (une session par
  rôle) ; test de régression Rust + frontend.
- **Exécution dédupliquée** : la boucle d'exécution de la commande Tauri est
  extraite dans `ih-agent-mode::run_team_tasks`, seule implémentation, testable.

### Ajouté
- **Adaptateur `pi`** (v1.6.0) : détection, commande `pi --print --no-session`,
  agent activé par défaut, test de construction de commande.
- **Test d'intégration réel** `crates/ih-agent-mode/tests/team_real.rs`
  (`#[ignore]`, exécution manuelle) : 4 sessions réelles OpenCode + pi, worktree
  isolé, dépôt principal intact, acceptation d'un diff (sauvegarde + écriture),
  rejet d'un autre.

### Validé en conditions réelles
Voir `docs/TEAM_MODE.md` : 4 tâches (`lead`, 2 `worker`, `reviewer`) terminées en
40,5 s, 2 fichiers produits, acceptation écrite (102 octets + sauvegarde), rejet
sans écriture, worktree supprimé.

### Non validé
Interface graphique du mode Équipe (rendu Xvfb indisponible dans
l'environnement), invalidation d'approbation après changement d'objectif ou
d'agents, événements `ih://team-event` observés dans un client, persistance
SQLite vérifiée en base, annulation et reprise de course.

## 0.5.0 — Team Mode (Lead · Workers · Reviewer)

### Ajouté
- **Course d'équipe** : un objectif, un coordinateur (Lead), 1 à 3 exécutants
  (Workers) et un relecteur (Reviewer). Le plan vient de `ih-orchestrator`
  (graphe de tâches et rôles) et s'exécute en **sessions d'agent réelles**.
- **Isolation** : toutes les tâches s'exécutent dans **un seul worktree git**
  détaché ; le projet n'est modifié pendant la course.
- **Une approbation par course**, liée à une empreinte (objectif + agents +
  répertoire) ; toute modification invalide l'approbation.
- **Progression** : événement `ih://team-event` par tâche (rôle, titre, état) et
  événement final (résumé, fichiers).
- **Modifications** : les fichiers produits deviennent des diffs en attente
  (`origin = "team:<run_id>"`), acceptés ou rejetés via le système 0.2.1.
- **Persistance** : chaque tâche du plan est enregistrée dans la table `tasks`
  avec son rôle et son état.
- Interface : sous-mode **Équipe** dans le panneau Agent (objectif, rôles,
  modale d'approbation avec plan prévu, liste des tâches en direct, revue des
  modifications).

### Limites
- Exécution **séquentielle** (la relecture dépend des exécutions) ; pas de
  parallélisme entre workers.
- Une course = un worktree ; pas de fusion inter-worktrees.
- Pas de reprise d'une course interrompue.

## 0.4.0 — Refonte UX/UI : la conversation d'abord

### Problème corrigé
Dans la colonne de conversation, les contrôles Fournisseur/Modèle, un bandeau
d'alerte permanent et le bloc contexte occupaient ~300 px : **la liste des
messages était écrasée à zéro** (aucun message visible malgré une conversation
existante).

### Changements
- **Barre compacte 46 px** dans l'en-tête : `[Provider ▼] [Model ▼] [pastille] [⚙]`.
  Plus de libellés empilés, plus de bouton « Découvrir » dans la colonne.
- **Plus aucun bandeau permanent** : clé manquante, modèle absent, provider
  désactivé deviennent une pastille avec infobulle ; le blocage s'affiche au
  moment de l'envoi.
- **Contexte repliable** (`Contexte · N`) en overlay dans le composer, replié
  par défaut.
- **Drawer de configuration** (420 px, ✕/Échap/clic extérieur) : Providers,
  Models, API Endpoints, API Keys, Avancé — la conversation reste montée et un
  streaming continue pendant la configuration.
- **Settings → Providers en maître/détail** : liste compacte à gauche, détail et
  modèles à droite ; plus de tableau géant.
- **Layout conversation-first** : 66 % par défaut, explorateur replié, bornes
  420/320 px, migration des anciennes dispositions, mode compact < 1200 px,
  `overflow-x: hidden`.
- Messages pleine largeur, liste en `flex: 1` avec scroll interne, composer
  ancré et borné.

### Tests
- Frontend : **156 tests** (13 nouveaux : répartition, bornes, mode compact,
  pastille de disponibilité, drawer, conservation de la conversation au
  changement de provider).
- Vérification visuelle réelle : captures avant/après dans
  `docs/screenshots/ux-before.png`, `ux-after.png`, `ux-drawer.png`.

## 0.3.0 — Fournisseurs personnalisés et endpoints API

### Ajouté
- **Providers personnalisés** : identifiant, nom, protocole, `base_url`,
  `endpoint_path` optionnel, en-têtes HTTP, timeout, streaming, température,
  `max_tokens`, modèle par défaut, modèles, dates de création/mise à jour.
- **`endpoint_path`** : chemin complet de chat sur l'origine de `base_url`
  (passerelles privées, préfixes exotiques).
- **En-têtes personnalisés** transmis à toutes les requêtes (chat, modèles,
  santé) des quatre protocoles.
- **Streaming désactivable** : `streaming = false` → réponse complète en un
  seul delta, sans casser l'interface.
- **Test de connexion** structuré : `ok`, `auth_failed`, `unreachable`,
  `unsupported_protocol`, `model_unavailable`, `timeout`, `error`, avec latence
  et nombre de modèles.
- **Gestion des modèles** : découverte, ajout manuel, suppression, modèle par
  défaut.
- **Interface** : page Fournisseurs (liste, ajout, édition, suppression avec
  confirmation, activation, test, modèles), cartes « Intégré / Personnalisé »
  et « Actif / Inactif ».
- **HTTP local autorisé** : localhost, boucle locale, réseaux privés, `*.local`
  (HTTPS reste exigé ailleurs).

### Sécurité
- Clés uniquement dans le trousseau ; jamais en configuration, logs ou IPC.
- En-têtes `Authorization`, `Host`, `Content-Length`, `Transfer-Encoding`,
  `Connection` refusés ; noms/valeurs validés (anti-injection CRLF).
- **Redirections HTTP non suivies** (protection de la clé d'API).
- Aucun blocage des IP privées ni de localhost.

### Tests
- Rust : validation de configuration (hôtes locaux, chemins, en-têtes,
  paramètres), tests d'intégration sur serveur HTTP local (endpoint personnalisé,
  en-têtes transmis, streaming désactivé, 302 non suivi), classification des
  erreurs de connexion.
- Frontend : validation du formulaire, parsing des en-têtes, mapping des statuts.
- Validation réelle avec un serveur **compatible OpenAI local** (voir
  `docs/CUSTOM_PROVIDERS.md`).

## 0.2.3 — Validation réelle OpenCode (jalon « 0.2.2.1 »)

Version interne `0.2.3` : semver n'accepte pas quatre nombres, mais le jalon
correspond à « IH 0.2.2.1 ».

### Validé avec le vrai CLI OpenCode v2.3.2
- Détection : `/home/infina/.opencode/bin/opencode`, version **v2.3.2**,
  diagnostic « exécutable trouvé », sélectionnable dans Agent Mode.
- Protocole réel : `opencode run --format json` produit du NDJSON
  (`step_start`, `tool_use`, `text`, `step_finish`) — capture réelle, parseur
  ajouté et testé sur ces trames.
- Exécution via IH : commande exacte
  `opencode run --format json --archived --standalone <tâche>`, sessions
  exécutées (23 et 19 étapes), événements normalisés (`[write] <chemin>`,
  `[read] <chemin>`, texte de l'assistant), fin de session détectée.

### Corrigé à cause de la validation réelle
- **Fuite hors worktree** : sans `--standalone`, OpenCode se connectait au
  service d'arrière-plan qui résolvait le projet depuis **son** contexte ; les
  fichiers étaient écrits dans le dépôt IH. `--standalone` est désormais
  obligatoire.
- **`$PWD` ignoré par le noyau** : même avec `--standalone`, OpenCode résolvait
  son projet via `$PWD` (hérité du répertoire de lancement d'IH) et non via le
  répertoire courant réel. Le superviseur transmet maintenant `PWD`/`OLDPWD`
  égaux au répertoire de travail de l'agent (test dédié).
- **Détection en lancement graphique** : les lanceurs GNOME n'héritent pas des
  `export PATH` de `~/.bashrc` ; `~/.opencode/bin` était donc invisible. IH
  complète son `PATH` avec celui du shell de connexion au démarrage.
- Normalisation des événements JSON d'OpenCode (les autres CLI restent inchangés).

### Validé après correction (voir docs/VALIDATION_OPENCODE.md)
- Revue de diff avec OpenCode : 2 fichiers détectés (`src/lib.rs`, `NOTES.md`),
  aperçus `+2 −0` et `+3 −0`, **acception** de `src/lib.rs` (écrit sur disque)
  et **rejet** de `NOTES.md` (absent du disque).

### Non validé (honnêtement)
- **Annulation d'une session OpenCode** : non testée ; le mécanisme est couvert
  par les tests du superviseur avec un agent contrôlé.
- Les adaptateurs Claude Code / Codex / Gemini restent non exécutés.

## 0.2.2 — Agent Mode · AI Coding Execution

### Ajouté
- **Mode Agent** dans la conversation : sélection d'un CLI détecté, tâche en
  langage naturel, approbation ponctuelle, exécution isolée, revue des diffs.
- **Isolation par worktree git** (`crates/ih-agent-mode`) : l'agent travaille
  dans un worktree détaché ; le projet n'est pas modifié pendant la session.
  Le mode refuse de démarrer hors dépôt git.
- **Détection d'adaptateurs** : nom, chemin, version (sondée prudemment, jamais
  pour un adaptateur personnalisé), capacités contractuelles, diagnostic.
- **Sessions persistées** (migration SQLite v3) : identifiant, projet,
  adaptateur, tâche, répertoire, état, résumé, fichiers, événements bornés
  (4 000 max) ; rechargement après redémarrage.
- **Événements typés** `ih://agent-event` : started, output, state, completed,
  cancelled, error — liés au `session_id`, diffusés en temps réel.
- **Approbation liée à une empreinte** (agent + commande + répertoire + tâche) :
  usage unique, expiration, refus explicite ; toute modification de la commande
  invalide l'approbation.
- **Revue des modifications** : les changements de l'agent deviennent des diffs
  en attente, avec aperçu ligne à ligne (`diff_view`), acceptation vérifiée par
  empreinte et rejet sans écriture ; suppression de fichier prise en charge.
- **Annulation réelle** : groupe de processus tué et enfant récolté (plus de
  zombie), session marquée `cancelled`.

### Corrigé pendant la validation réelle
- Le sondage `--version` d'un adaptateur personnalisé exécutait un script dans
  le répertoire du projet (effets de bord) : il est désormais limité aux CLI
  connus, dans un répertoire neutre.
- Les processus annulés restaient zombies : le superviseur récolte maintenant
  l'enfant.
- `diff_view` manquait pour prévisualiser un diff produit par un agent.

### Tests
- Rust : **262 tests** (dont `ih-agent-mode` 5, `ih-store` 16 avec migration v3).
- Frontend : **122 tests** (30 nouveaux : réducteur d'événements, approbations,
  sélection d'adaptateur, bornes, fusion historique/live).
- Validation dans le binaire réel avec un **agent contrôlé** (script local) :
  approbation affichée (commande exacte, répertoire, empreinte, risque),
  exécution, 2 fichiers modifiés détectés, aperçu de diff, acceptation d'un
  fichier (écrit sur disque) et rejet de l'autre (disque intact), persistance
  après redémarrage, annulation (état `cancelled`, processus récolté).
- **Aucun CLI d'agent réel n'est installé sur la machine de validation** : les
  adaptateurs Claude Code / Codex / Gemini CLI / OpenCode n'ont donc pas pu être
  exécutés. Les tests réels ont été faits avec un agent contrôlé.

## 0.2.1 — Workspace · Project Context · Editor

### Ajouté
- **Workspace à trois zones** : conversation à gauche, explorateur + onglets +
  éditeur à droite, panneau inférieur (terminal, Git, problèmes) escamotable.
  Séparateurs redimensionnables à la souris et au clavier, disposition persistée
  (`layout_save`/`layout_get`) et onglets restaurés au redémarrage.
- **Éditeur CodeMirror 6** : coloration Rust, TypeScript/JavaScript, Python,
  JSON, TOML, Markdown, YAML, CSS, HTML ; numéros de ligne ; Ctrl+F ; Ctrl+S.
- **Sauvegarde vérifiée** : l'empreinte SHA-256 lue à l'ouverture est renvoyée à
  `workspace_save` ; un fichier modifié entre-temps n'est jamais écrasé
  silencieusement (`conflict`).
- **Contexte projet** (`crates/ih-context`, 23 tests) : mentions `@` avec
  recherche floue, aperçu exact de ce qui sera envoyé, exclusions par défaut
  (`.env`, `id_rsa`, `*.pem`, `*.key`, `.ssh/`, `node_modules/`, `target/`,
  `.git/`…), détection de secrets par contenu, confirmation explicite pour
  inclure un fichier sensible.
- **Diff** (`ih_workspace::diff`) : calcul ligne à ligne LCS borné, hunks avec
  contexte, aperçu avant écriture, acceptation avec revérification d'empreinte
  et sauvegarde, rejet sans toucher au disque.
- Commandes IPC : `context_settings`, `workspace_meta`, `workspace_save`,
  `files_match`, `context_prepare`, `diff_prepare`, `diff_pending`,
  `diff_accept`, `diff_reject`, `layout_get`, `layout_save` ; `chat_send`
  accepte `contextPaths` et `allowSensitiveContext` (assemblage côté Rust).

### Corrigé en cours de validation
- La zone de saisie écrasait la liste des messages quand l'aperçu de contexte
  grandissait : la liste défile désormais seule et le compositeur reste borné.
- Le popup de mentions `@` était rogné par l'`overflow` du compositeur.
- Libellé dupliqué « Aucun fichier ouvert » entre l'onglet vide et l'éditeur.

### Tests
- Rust : **256 tests** (dont `ih-context` 23, `ih-workspace` 33, `ih-chat` 11).
- Frontend : **92 tests** (layout, mentions, onglets, diff, chat, i18n…).
- Validation manuelle dans le binaire réel : ouverture de projet, explorateur,
  ouverture d'un fichier, mention `@` → chip + aperçu, diff `+1 −0`, rejet
  (disque intact) puis acceptation (fichier écrit), restauration de l'onglet
  après redémarrage.

## 0.1.0 — Fondation complète

- Workspace Rust de 10 crates (207 → 208 tests), interface 16 sections,
  paquets DEB/AppImage, PKGBUILD Arch, mises à jour signées Ed25519.
- Voir `docs/FEATURES.md` pour l'état détaillé par fonctionnalité.
