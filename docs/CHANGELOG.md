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
