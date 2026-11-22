# Architecture d'IH

## Principes

- **Un seul noyau Rust** ; l'interface React ne fait que présenter et déclencher.
- **Aucune autorité implicite** : le renderer n'a ni accès disque, ni shell, ni
  réseau, ni secret. Tout passe par une commande Tauri validée.
- **Contrats typés** : les événements et les erreurs sont normalisés dans
  `ih-core`, quel que soit le fournisseur ou l'agent d'origine.

## Frontières de processus

```
┌─────────────────────────────┐
│ React (WebView)             │  présentation, i18n, état d'UI
│  src/                       │  aucun droit direct
└──────────────┬──────────────┘
               │ IPC typé (commandes) + événements ih://…
┌──────────────▼──────────────┐
│ ih-desktop (Tauri 2)        │  validation, état partagé, orchestration des appels
│  src-tauri/src/commands/*   │
└──────────────┬──────────────┘
               │ appels de bibliothèque
┌──────────────▼──────────────────────────────────────────────┐
│ Crates du noyau                                             │
│  ih-core · ih-store · ih-security · ih-providers            │
│  ih-agents · ih-extensions · ih-workspace · ih-orchestrator │
│  ih-update · ih-i18n                                        │
└──────────────┬──────────────────────────────────────────────┘
               │ processus enfants supervisés
┌──────────────▼──────────────┐
│ CLI d'agents (claude, codex,│  un processus par session,
│ gemini, opencode)           │  groupe de processus, sortie bornée
│ Serveurs MCP (stdio)        │
│ PTY (shell de l'utilisateur)│
│ git                         │
└─────────────────────────────┘
```

## Rôles des crates

| Crate | Responsabilité | Points durs testés |
|---|---|---|
| `ih-core` | configuration TOML, identifiants typés, événements, politique de ressources, détection de plateforme, redaction | schémas d'erreur stables, clamp des limites, redaction des secrets |
| `ih-store` | persistance SQLite (projets, threads, messages, sessions, tâches, mémoire, extensions, workflows, audit) | migrations `user_version`, cascade, accès concurrent |
| `ih-security` | classification du risque, registre d'approbations à usage unique, trousseau | expiration, rejeu refusé, absence de repli en clair |
| `ih-providers` | OpenAI-compatible, Anthropic, Google, Ollama ; streaming SSE/NDJSON ; découverte de modèles | parsing incrémental, erreurs HTTP, clé absente |
| `ih-agents` | adaptateurs CLI et supervision de processus | annulation, timeout, limite de parallélisme, sortie bornée |
| `ih-extensions` | installation vérifiée des skills, client MCP stdio | empreinte SHA-256, framing NDJSON, serveur muet |
| `ih-workspace` | Git, édition confinée, terminal PTY | prison de chemins, worktrees, absence de zombie |
| `ih-orchestrator` | graphe de tâches, équipes, ordonnanceur, mémoire injectée | cycles, disponibilité, budget de contexte |
| `ih-update` | manifeste signé Ed25519, SHA-256, staging, plan d'installation, rollback AppImage | signature, digest, remplacement atomique |
| `ih-chat` | moteur de conversation : streaming, annulation, contexte injecté | annulation, historique borné, e2e Ollama réel |
| `ih-context` | assemblage du contexte projet, exclusions, mentions `@` | exclusions, secrets, troncature, budget, prison de chemins |
| `ih-i18n` | catalogues FR/EN partagés avec l'interface | parité stricte des clés |

## Flux d'une session d'agent

1. L'interface appelle `agent_start(projectId, agentId, prompt)`.
2. `ih-desktop` vérifie l'agent, le projet, puis délègue à `ih-agents`.
3. `ih-agents` vérifie la limite `max_concurrent_agents`, crée un groupe de
   processus et lance le CLI dans le dossier du projet.
4. stdout/stderr sont lus ligne à ligne et émis en `ih://agent-output`
   (numéro de séquence, flux, données), dans la limite
   `max_output_bytes_per_session`.
5. Un thread de veille applique le timeout, collecte le code de sortie et tue
   le groupe de processus ; l'état final est conservé pour que
   `agent_state`/`wait` ne dépendent pas d'une course.
6. L'état est persisté dans `sessions` par `ih-desktop`.

## Flux du contexte projet

1. L'interface appelle `files_match` pour la complétion `@` (recherche floue,
   fichiers exclus signalés).
2. L'utilisateur sélectionne des fichiers : ils deviennent des « chips ».
3. `context_prepare` assemble l'aperçu : fichiers inclus, octets réellement lus,
   troncatures, fichiers écartés avec motif.
4. À l'envoi, `chat_send` reçoit **les chemins**, pas le contenu. Le noyau
   assemble le bloc et l'injecte dans le message système de `ih-chat`.
5. Le diff (`diff_prepare`) calcule les lignes modifiées sans écrire ; l'écriture
   n'a lieu qu'après `diff_accept`, avec revérification de l'empreinte SHA-256.

## Politique de ressources (16 Go / GTX 1660 Ti)

- `max_concurrent_agents` par défaut **2**, plafond absolu **4** appliqué par
  le noyau, non contournable par la configuration.
- Aucun modèle local chargé au démarrage (`preload_local_models = false`).
- Sortie d'agent plafonnée (4 Mio par défaut), diff Git plafonné (2 Mio),
  lecture de fichier plafonnée, pagination de l'historique.
- Ollama est traité comme un service externe : IH ne le démarre pas.

## Stockage

Base unique `~/.config/ih/data/ih.sqlite3` (WAL, `foreign_keys=ON`,
`busy_timeout=5s`), répertoires en `0700`, journal rotatif dans
`~/.config/ih/logs/`. Les secrets ne sont **jamais** dans la base : seul le
trousseau du système les contient.
