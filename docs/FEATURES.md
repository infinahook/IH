# État des fonctionnalités d'IH

Ce document est la **référence honnête** de l'avancement. Trois statuts :

- ✅ **terminé** : implémenté, connecté à l'interface, couvert par des tests automatisés.
- ⚠️ **partiel** : fonctionne réellement, mais une partie du besoin n'est pas couverte — c'est écrit explicitement.
- ❌ **absent** : non implémenté. Aucun bouton de l'interface ne prétend le contraire.

Dernière vérification : 218 tests Rust + 49 tests frontend, tous verts.

## Les 22 fonctionnalités demandées

| # | Fonctionnalité | Statut | Détail |
|---|---|---|---|
| 1 | Noyau IH complet | ✅ | `ih-core` : configuration TOML validée, identifiants typés, événements, politique de ressources, détection de plateforme, redaction |
| 2 | Gestionnaire multi-agents | ✅ | Registre d'adaptateurs, détection dans le `PATH`, sessions supervisées, limite de parallélisme appliquée par le noyau |
| 3 | Claude Code, Codex, Gemini CLI, OpenCode | ✅ | Arguments non interactifs documentés (`claude -p`, `codex exec`, `gemini -p`, `opencode run`), surchargeables par `args` avec `{prompt}`. **Non testés contre les binaires réels** (absents de la machine) : la supervision est testée sur un faux agent |
| 4 | Fournisseurs IA cloud et locaux | ✅ | OpenAI-compatible (OpenAI, OpenRouter, custom), Anthropic, Gemini, Ollama ; chat, streaming SSE/NDJSON, modèles, santé. **Écran de conversation complet** : streaming temps réel, annulation, historique, sélection provider/modèle, réessai |
| 5 | Gestionnaire de modèles | ✅ | Découverte réelle par API, modèle par défaut, activation par fournisseur, capacités seulement si exposées par l'API |
| 6 | Skills : installation et mises à jour | ⚠️ | Installation depuis un dossier avec validation stricte du manifeste, empreinte SHA-256, vérification d'intégrité, activation, désinstallation. **Mise à jour depuis un registre distant : absente** |
| 7 | Plugins MCP | ⚠️ | Serveurs stdio, `initialize`, `tools/list`, `tools/call`, appels soumis à approbation à usage unique, pas de zombie. **Pas de resources/prompts/sampling, pas de transport HTTP/SSE** |
| 8 | Gestion des projets | ✅ | Ouverture par sélecteur de dossier, persistance, projets récents, retrait de liste **sans jamais supprimer le dossier** |
| 9 | Éditeur de code | ⚠️ | Arbre de fichiers, lecture/écriture confinée, numéros de ligne, recherche, Ctrl+S, détection de modification. **Pas de coloration syntaxique ni de multi-curseurs** |
| 10 | Terminal Linux intégré | ⚠️ | PTY réel (`portable-pty`), saisie, redimensionnement, fermeture, filtrage ANSI/OSC. **Pas d'émulation VT complète** (curseur, écran alterné, IME) |
| 11 | Git et worktrees | ⚠️ | Statut porcelain, diff (indexé/non indexé), index/désindex, commit, branches, log, worktrees (création/suppression). **Opérations locales uniquement** : pas de fetch/push/pull/merge/rebase |
| 12 | Agents autonomes et équipes | ⚠️ | Graphe de tâches, rôles lead/worker/reviewer, dépendances, ordonnancement borné, exécution réelle des étapes. **Pas de boucle autonome auto-relancée** : le plan est créé puis exécuté sous contrôle utilisateur |
| 13 | Orchestrateur de tâches | ✅ | Création de plan déterministe, persistance, transitions d'état, détection de cycle, blocage en cas d'échec d'une dépendance |
| 14 | Mémoire persistante par projet | ✅ | Portée projet/globale, tags, recherche, suppression, aperçu du contexte injecté borné en octets |
| 15 | Permissions et sécurité | ✅ | Approbations à usage unique liées à une catégorie, TTL, classification du risque, prison de chemins, secrets dans le trousseau uniquement |
| 16 | Automatisations et workflows | ⚠️ | Création (intervalle ≥ 5 min, étapes agent+prompt), exécution manuelle réelle, historique des exécutions. **Pas de planificateur en arrière-plan qui déclenche seul**, pas d'édition/suppression de workflow dans l'UI |
| 17 | Historique conversations et tâches | ✅ | Threads, messages paginés, suppression, export JSON téléchargeable |
| 18 | Interface française et anglaise | ✅ | 800 clés FR et EN, parité vérifiée par test Rust et par test frontend, changement immédiat |
| 19 | Paramètres complets | ✅ | Thème, langue, limites de ressources, niveau de logs, canal et intervalle de mise à jour, téléchargement automatique. Enregistrement validé côté Rust |
| 20 | Journalisation et diagnostic | ✅ | Journal rotatif quotidien + console, redaction, fin de journal consultable, rapport de diagnostic exportable, ouverture du dossier de logs |
| 21 | Système de mises à jour | ⚠️ | Manifeste signé Ed25519, vérification stricte, SHA-256, staging `.part`, approbation obligatoire, remplacement atomique AppImage + sauvegarde/rollback, plan `deb`/`pacman` jamais exécuté par IH. **Pas de mise à jour delta ni de progression en pourcentage** |
| 22 | Installation DEB, AppImage, Arch | ✅ | Bundles Tauri `deb` + `appimage`, `PKGBUILD` Arch depuis les sources, CI qui produit les artefacts et compile sous Arch |

## Jalon 0.2.0 livré — Conversation

- Conversations persistées en SQLite, créées, renommées, supprimées, recherchées
  par titre **et** par contenu.
- Streaming réel fragment par fragment, y compris via le vrai client Ollama
  (test de bout en bout `crates/ih-chat/tests/ollama_e2e.rs`).
- Annulation effective : le flux est coupé, le texte partiel est conservé et
  marqué `cancelled`.
- Sélection du fournisseur et du modèle, découverte des modèles par API,
  modèle par défaut mémorisé.
- Erreurs réseau affichées avec leur code + bouton Réessayer.
- Indicateurs : génération en cours, jetons consommés, latence.

## Ce qui est explicitement absent

1. **Contexte projet dans le chat** (ajout de fichiers, mentions `@fichier`,
   pièces jointes) : prévu pour 0.2.1.
2. **Workspace à panneaux redimensionnables** et éditeur avec coloration
   syntaxique : prévu pour 0.2.1.
3. **Agent Mode** dans la conversation (exécution d'un CLI depuis le chat,
   diff avant/après, acceptation) : prévu pour 0.2.2.
4. **Team Mode** visuel (Lead/Worker/Reviewer) : prévu pour 0.2.3.
5. **Sessions d'agents CLI réinjectées dans les threads de conversation**.
6. **Registre de skills distant** avec vérification de mise à jour.
7. **MCP hors stdio** (HTTP/SSE, ressources, prompts, sampling).
8. **Git distant** (`push`, `pull`, `fetch`) et résolution de conflits.
9. **Planificateur d'automatisations** actif en arrière-plan.
10. **Page d'audit** exposée par l'IPC (le stockage existe en base).
11. **Fichier LICENSE** : licence pas encore choisie.
12. **Arch sur machine physique** : la CI valide la compilation et les tests
    dans un conteneur `archlinux:latest`.

## Tests automatisés

| Suite | Nombre | Portée |
|---|---|---|
| `ih-core` | 30 | configuration, erreurs, événements, ressources, chemins, redaction, plateforme |
| `ih-store` | 15 | migrations, CRUD, cascade, pagination, concurrence, recherche |
| `ih-security` | 19 | approbations, expiration/rejeu, classification de risque, trousseau |
| `ih-providers` | 15 | 4 fournisseurs, SSE/NDJSON, erreurs 401, timeout, clé absente |
| `ih-agents` | 13 | adaptateurs, succès/échec, annulation, timeout, limite, sortie bornée |
| `ih-extensions` | 27 | manifestes, empreinte, installation, prison, MCP stdio, serveur muet |
| `ih-workspace` | 22 | Git (statut, commit, worktrees), prison de chemins, PTY sans zombie |
| `ih-orchestrator` | 23 | cycles, disponibilité, équipes, ordonnanceur, budget mémoire |
| `ih-update` | 39 | signature Ed25519, digest, téléchargement, staging, rollback |
| `ih-i18n` | 5 | parité FR/EN, repli, substitution |
| `ih-chat` | 10 | streaming, annulation, historique borné, erreurs, titre, recherche + **e2e Ollama réel** |
| **Rust total** | **218** | |
| Frontend (`vitest`) | 49 | i18n, formatage, ANSI, Git, arborescence, réduction du flux de chat, erreurs |
| **Total** | **267** | |

## Vérifications manuelles effectuées

- **Chat de bout en bout dans l'application réelle** (Xvfb + serveur Ollama NDJSON
  local) : projet ouvert, provider Ollama et modèle sélectionnés, message envoyé,
  réponse reçue **en streaming fragment par fragment**, texte final persisté avec
  jetons et latence, puis **annulation** vérifiée (badge « Annulé »,
  1 359 ms, texte partiel conservé en base avec `cancelled: true`).
  Captures : [chat-stream.png](screenshots/chat-stream.png),
  [chat-cancelled.png](screenshots/chat-cancelled.png).

- `cargo clippy --workspace --all-targets -- -D warnings` : aucun avertissement.
- `cargo fmt --all -- --check` : conforme.
- `npm run typecheck`, `npm test`, `npm run build` : OK.
- Build release Tauri + paquets `deb` et `AppImage`.
- Chaîne de mise à jour de bout en bout : clé Ed25519 générée par `openssl`,
  manifeste signé, vérifié par `ih-update-verify` (succès), puis falsifié et
  rejeté (code 1).
