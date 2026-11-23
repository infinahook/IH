# Journal des versions

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
