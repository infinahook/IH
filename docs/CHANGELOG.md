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

### Non validé (honnêtement)
- **Revue de diff et acceptation/rejet avec OpenCode** : les deux exécutions
  réelles ont eu lieu avant la correction `$PWD` ; la troisième tentative n'a
  pas démarré (automatisation d'interface). Le chemin diff→acceptation est testé
  avec un agent contrôlé, pas avec OpenCode.
- **Annulation d'une session OpenCode** : non testée.
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
