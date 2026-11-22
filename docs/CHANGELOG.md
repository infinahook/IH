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

## 0.1.0 — Fondation complète

- Workspace Rust de 10 crates (207 → 208 tests), interface 16 sections,
  paquets DEB/AppImage, PKGBUILD Arch, mises à jour signées Ed25519.
- Voir `docs/FEATURES.md` pour l'état détaillé par fonctionnalité.
