# État des fonctionnalités d'IH

Ce document est la **référence honnête** de l'avancement. Trois statuts :

- ✅ **terminé** : implémenté, connecté à l'interface, couvert par des tests automatisés.
- ⚠️ **partiel** : fonctionne réellement, mais une partie du besoin n'est pas couverte — c'est écrit explicitement.
- ❌ **absent** : non implémenté. Aucun bouton de l'interface ne prétend le contraire.

Dernière vérification : 367 tests Rust + 181 tests frontend, tous verts.

## Les 22 fonctionnalités demandées

| # | Fonctionnalité | Statut | Détail |
|---|---|---|---|
| 1 | Noyau IH complet | ✅ | `ih-core` : configuration TOML validée, identifiants typés, événements, politique de ressources, détection de plateforme, redaction |
| 2 | Gestionnaire multi-agents | ✅ | Registre d'adaptateurs, détection dans le `PATH`, sessions supervisées, limite de parallélisme appliquée par le noyau |
| 3 | Claude Code, Codex, Gemini CLI, OpenCode | ✅ | Arguments non interactifs documentés (`claude -p`, `codex exec`, `gemini -p`, `opencode run`), surchargeables par `args` avec `{prompt}`. **Non testés contre les binaires réels** (absents de la machine) : la supervision est testée sur un faux agent |
| 4 | Fournisseurs IA cloud et locaux | ✅ | OpenAI-compatible (OpenAI, OpenRouter, custom), Anthropic, Gemini, Ollama ; chat, streaming SSE/NDJSON, modèles, santé. **Écran de conversation complet** : streaming temps réel, annulation, historique, sélection provider/modèle, réessai |
| 5 | Gestionnaire de modèles | ✅ | Découverte réelle par API, modèle par défaut, activation par fournisseur, capacités seulement si exposées par l'API |
| 6 | Skills : installation et mises à jour | ⚠️ | Installation depuis un dossier avec validation stricte du manifeste, empreinte SHA-256, vérification d'intégrité, activation, désinstallation. **Mise à jour depuis un registre distant : absente** |
| 7 | Plugins MCP | ⚠️ | **Serveurs MCP natifs distants : Streamable HTTP et SSE historique** (transports réels, testés contre un serveur local), découverte d'outils, catalogue avec activation explicite, appels contrôlés par approbation liée à l'empreinte des paramètres, jeton au trousseau, anti-SSRF, tailles et délais bornés, historique et audit ; serveurs stdio existants conservés. **Reste : resources/prompts/sampling, tool use côté chat, injection automatique dans les CLI d'agents** |
| 8 | Gestion des projets | ✅ | Ouverture par sélecteur de dossier, persistance, projets récents, retrait de liste **sans jamais supprimer le dossier** |
| 9 | Éditeur de code | ✅ | CodeMirror 6 : coloration Rust/TS/JS/Python/JSON/TOML/Markdown/YAML/CSS/HTML, numéros de ligne, Ctrl+F, Ctrl+S, onglets, détection de conflit d'écriture |
| 10 | Terminal Linux intégré | ⚠️ | PTY réel (`portable-pty`), saisie, redimensionnement, fermeture, filtrage ANSI/OSC. **Pas d'émulation VT complète** (curseur, écran alterné, IME) |
| 11 | Git et worktrees | ⚠️ | Statut porcelain, diff (indexé/non indexé), index/désindex, commit, branches, log, worktrees (création/suppression). **Opérations locales uniquement** : pas de fetch/push/pull/merge/rebase |
| 12 | Agents autonomes et équipes | ⚠️ | Graphe de tâches, rôles lead/worker/reviewer, dépendances, ordonnancement borné, exécution réelle des étapes. **Pas de boucle autonome auto-relancée** : le plan est créé puis exécuté sous contrôle utilisateur |
| 13 | Orchestrateur de tâches | ✅ | Création de plan déterministe, persistance, transitions d'état, détection de cycle, blocage en cas d'échec d'une dépendance |
| 14 | Mémoire persistante par projet | ✅ | Portée projet/globale, tags, recherche, suppression, aperçu du contexte injecté borné en octets |
| 15 | Permissions et sécurité | ✅ | Approbations à usage unique liées à une catégorie, TTL, classification du risque, prison de chemins, secrets dans le trousseau uniquement |
| 16 | Automatisations et workflows | ⚠️ | Création (intervalle ≥ 5 min, étapes agent+prompt **ou outil MCP**), exécution manuelle réelle, historique des exécutions ; les étapes MCP passent par le contrôle d'IH (serveur/outil activés, arguments validés, mode sans interaction limité aux lectures pré-autorisées). **Pas de planificateur en arrière-plan qui déclenche seul**, pas d'édition/suppression de workflow dans l'UI |
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

## Jalon 0.2.1 livré — Workspace, contexte et diff

- **Disposition à trois zones** redimensionnables (conversation, workspace,
  panneau inférieur), disposition persistée en base et onglets restaurés.
- **Explorateur + éditeur** : arborescence, recherche de fichiers, onglets avec
  état modifié, éditeur CodeMirror 6 sur 11 langages.
- **Contexte projet assemblé en Rust** (`crates/ih-context`) : mentions `@`,
  recherche floue, exclusions par défaut, détection de secrets, limites de
  taille, aperçu exact avant envoi.
- **Diff** : calcul ligne à ligne borné, aperçu, acceptation avec contrôle
  d'empreinte et sauvegarde, rejet sans écriture.
- Validation dans le binaire réel : mention `@` → chip + aperçu, diff `+1 −0`,
  rejet (disque intact), acceptation (fichier écrit), restauration d'onglet.

## Jalon 0.2.2 livré — Agent Mode

- **Mode Agent** : sélection d'un adaptateur réellement détecté, tâche libre,
  approbation ponctuelle liée à une empreinte, exécution isolée, événements en
  temps réel, annulation, revue des modifications.
- **Isolation** : worktree git détaché ; le projet n'est jamais modifié pendant
  la session. Hors dépôt git, le mode refuse de démarrer.
- **Sessions persistées** (migration v3) avec événements bornés et reprise après
  redémarrage.
- **Diffs** : chaque fichier touché devient un diff en attente, prévisualisable
  ligne à ligne, accepté (contrôle d'empreinte + sauvegarde) ou rejeté (aucune
  écriture). Suppression de fichier prise en charge.
- **Annulation** : groupe de processus tué, enfant récolté, session `cancelled`.
- **Aucun CLI d'agent réel n'étant installé sur la machine de validation**, les
  intégrations Claude Code / Codex / Gemini CLI / OpenCode sont implémentées
  mais **non exécutées** ; la validation réelle a utilisé un agent contrôlé.

## Jalon 0.3.0 livré — Fournisseurs personnalisés

- Providers personnalisés persistés dans la configuration : protocole, base URL,
  chemin d'endpoint, en-têtes, timeout, streaming, paramètres de génération,
  modèle par défaut, dates.
- Quatre protocoles (OpenAI-compatible, Anthropic, Google, Ollama) inchangés
  pour les providers intégrés.
- Test de connexion structuré (7 statuts) et gestion des modèles (découverte,
  ajout manuel, suppression, défaut).
- Sécurité : clés uniquement au trousseau, en-têtes réservés refusés, anti-CRLF,
  redirections non suivies, HTTP local autorisé.

## Ce qui est explicitement absent

1. **Pièces jointes binaires** (images, PDF) dans le chat : absentes.
2. **Agent Mode** dans la conversation (exécution d'un CLI depuis le chat,
   diff avant/après, acceptation) : prévu pour 0.2.2.
3. **Team Mode** visuel (Lead/Worker/Reviewer) : prévu pour 0.2.3.
4. **Sessions d'agents CLI réinjectées dans les threads de conversation**.
5. **Registre de skills distant** avec vérification de mise à jour.
6. **MCP étendu** : ressources, prompts, sampling et tool use des modèles — les transports **HTTP/Streamable et SSE sont livrés en 0.6.1**.
7. **Git distant** (`push`, `pull`, `fetch`) et résolution de conflits.
8. **Planificateur d'automatisations** actif en arrière-plan.
9. **Page d'audit** exposée par l'IPC (le stockage existe en base).
10. ~~Fichier LICENSE~~ → **MIT** depuis 0.5.2 (voir `LICENSE`).
11. **Arch sur machine physique** : la CI valide la compilation et les tests
    dans un conteneur `archlinux:latest`.
12. **Repli de l'éditeur sur très gros fichiers** : au-delà de 2 Mio (limite
    configurable), le fichier n'est pas ouvert.

## Tests automatisés

| Suite | Nombre | Portée |
|---|---|---|
| `ih-core` | 38 | configuration, erreurs, événements, ressources, chemins, redaction, plateforme |
| `ih-store` | 19 | migrations (v4 incluse), CRUD, cascade, pagination, concurrence, recherche, **MCP : catalogue et appels** |
| `ih-security` | 41 | approbations (dont empreintes), expiration/rejeu, classification de risque, **politique réseau MCP (SSRF)**, trousseau |
| `ih-providers` | 19 | 4 fournisseurs, SSE/NDJSON, erreurs 401, timeout, clé absente |
| `ih-agents` | 20 | adaptateurs, succès/échec, annulation, timeout, limite, sortie bornée |
| `ih-extensions` | 27 | manifestes, empreinte, installation, prison, MCP stdio, serveur muet |
| `ih-workspace` | 33 | Git, prison de chemins, PTY, **diff LCS**, métadonnées, écriture vérifiée |
| `ih-orchestrator` | 23 | cycles, disponibilité, équipes, ordonnanceur, budget mémoire |
| `ih-update` | 39 | signature Ed25519, digest, téléchargement, staging, rollback |
| `ih-i18n` | 5 | parité FR/EN, repli, substitution |
| `ih-chat` | 12 | streaming, annulation, historique borné, contexte injecté, erreurs, titre, recherche + **e2e Ollama réel** |
| `ih-context` | 23 | exclusions, secrets, troncature, budget, path jail, mentions floues |
| `ih-agent-mode` | 8 | sandbox worktree, collecte des changements, détection d'adaptateurs, empreinte d'action |
| `ih-mcp` | 42 | schémas, politique URL, session unifiée ; **9 tests d'intégration Streamable HTTP** et **4 tests SSE** contre un vrai serveur MCP local |
| **Rust total** | **367** | |
| Frontend (`vitest`) | 181 | i18n, formatage, ANSI, Git, arborescence, chat, layout, mentions, onglets, diff, agent, **MCP** |
| **Total** | **530** | |

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
