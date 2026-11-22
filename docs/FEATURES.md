# État des fonctionnalités d'IH

Ce document est la **référence honnête** de l'avancement. Trois statuts :

- ✅ **terminé** : implémenté, connecté à l'interface, couvert par des tests automatisés.
- ⚠️ **partiel** : fonctionne réellement, mais une partie du besoin n'est pas couverte — c'est écrit explicitement.
- ❌ **absent** : non implémenté. Aucun bouton de l'interface ne prétend le contraire.

Dernière vérification : 208 tests Rust + 36 tests frontend, tous verts.

## Les 22 fonctionnalités demandées

| # | Fonctionnalité | Statut | Détail |
|---|---|---|---|
| 1 | Noyau IH complet | ✅ | `ih-core` : configuration TOML validée, identifiants typés, événements, politique de ressources, détection de plateforme, redaction |
| 2 | Gestionnaire multi-agents | ✅ | Registre d'adaptateurs, détection dans le `PATH`, sessions supervisées, limite de parallélisme appliquée par le noyau |
| 3 | Claude Code, Codex, Gemini CLI, OpenCode | ✅ | Arguments non interactifs documentés (`claude -p`, `codex exec`, `gemini -p`, `opencode run`), surchargeables par `args` avec `{prompt}`. **Non testés contre les binaires réels** (absents de la machine) : la supervision est testée sur un faux agent |
| 4 | Fournisseurs IA cloud et locaux | ✅ | OpenAI-compatible (OpenAI, OpenRouter, custom), Anthropic, Google Gemini, Ollama ; chat, streaming SSE/NDJSON, modèles, santé. **Aucun écran de chat cloud** dans l'interface (voir #17/gaps) |
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

## Ce qui est explicitement absent

1. **Écran de conversation avec les fournisseurs cloud.** Le noyau sait dialoguer
   (chat + streaming) mais l'interface n'expose pas de chat : l'interaction passe
   aujourd'hui par les CLI d'agents. C'est le manque le plus visible.
2. **Chat multi-tours persistant** pour un agent : les sorties de session ne sont
   pas réinjectées dans un thread persistant.
3. **Registre de skills distant** avec vérification de mise à jour.
4. **MCP hors stdio** (HTTP/SSE, ressources, prompts, sampling).
5. **Git distant** (`push`, `pull`, `fetch`) et résolution de conflits.
6. **Planificateur d'automatisations** actif en arrière-plan.
7. **Coloration syntaxique** dans l'éditeur.
8. **Page d'audit** : le stockage d'audit existe côté base, mais aucune commande
   IPC ne l'expose encore à l'interface ; la page Sécurité l'indique au lieu
   d'inventer une liste.
9. **Fichier LICENSE** : licence pas encore choisie (voir README).
10. **Test sur machine Arch réelle** : la compilation et les tests tournent dans
    un conteneur `archlinux:latest` en CI, pas sur une installation Arch physique.

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
| **Rust total** | **208** | |
| Frontend (`vitest`) | 36 | i18n, formatage, ANSI, Git, arborescence |
| **Total** | **244** | |

## Vérifications manuelles effectuées

- `cargo clippy --workspace --all-targets -- -D warnings` : aucun avertissement.
- `cargo fmt --all -- --check` : conforme.
- `npm run typecheck`, `npm test`, `npm run build` : OK.
- Build release Tauri + paquets `deb` et `AppImage`.
- Chaîne de mise à jour de bout en bout : clé Ed25519 générée par `openssl`,
  manifeste signé, vérifié par `ih-update-verify` (succès), puis falsifié et
  rejeté (code 1).
