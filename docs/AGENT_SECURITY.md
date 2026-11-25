# Sécurité des agents CLI (sandbox système)

Le mode Agent exécute des CLI d'agents (Kilo, DSH CLI, Goose, OpenCode, pi…) qui
écrivent directement sur le disque. Deux isolations distinctes s'appliquent :

| Isolation | Rôle | Limite |
|---|---|---|
| **Worktree git** | Le dépôt du projet n'est pas modifié pendant la session ; les changements sont relus en diff. | Ne protège **pas** le reste du système. |
| **Sandbox système (bubblewrap)** | Fichiers, environnement, réseau et processus de l'agent sont confinés. | L'agent garde les droits de l'utilisateur **à l'intérieur** de la sandbox. |

Une sandbox n'est **pas** un compte séparé : un agent isolé ne peut pas lire le
dossier personnel de l'utilisateur, mais tout ce qui est monté dans la sandbox
reste accessible avec les droits du compte.

## Trois niveaux

| Niveau | Clé | Sandbox | Réseau | Usage |
|---|---|---|---|---|
| **Isolé** | `isolated` | bubblewrap | **désactivé** (`--unshare-net`) | tâches hors ligne ; un provider distant échoue explicitement |
| **Isolé + réseau** | `isolated_network` | bubblewrap | réseau de l'hôte | **défaut** : les agents joignent leur provider |
| **Non isolé** | `none` | aucune | hôte | confirmation explicite obligatoire |

Le niveau est configuré **par agent** (`security` dans `[[agents]]`) et inscrit
dans l'empreinte d'approbation : le changer invalide l'approbation existante.

> Pourquoi le défaut n'est-il pas « Isolé » ? Les trois agents intégrés joignent
> un provider distant ; sans réseau ils échouent par conception. Le défaut
> `isolated_network` est le niveau le plus strict **compatible** avec leur
> fonctionnement. `isolated` reste disponible et testé (échec réseau vérifié).

## Ce que la sandbox donne et refuse

Montages de la sandbox bubblewrap :

- `/usr`, `/bin`, `/sbin`, `/lib*`, `/etc` : **lecture seule** ;
- `/tmp` : **tmpfs privé**, vidé à chaque session ;
- **worktree** : seul répertoire **inscriptible** du système de fichiers ;
- `/home/agent` : dossier personnel **privé, vide au départ**, alimenté par des
  copies minimales (voir ci-dessous) ; le vrai `$HOME` n'est **jamais** monté ;
- `--proc`, `--dev`, `--unshare-pid`, `--unshare-ipc`, `--unshare-uts`,
  `--die-with-parent` : pas de processus hôtes visibles, enfants tués avec IH ;
- réseau : selon le niveau (`--unshare-net` pour `isolated`).

Refusé par construction : lecture de `~`, `~/.ssh`, `~/.gnupg`, `~/.config`,
fichiers de secrets, sockets de session (keyring), écriture/suppression hors
worktree, accès aux services de l'utilisateur.

## Secrets par agent (méthode contrôlée)

L'agent ne reçoit **que** ses propres identifiants, copiés (0600) dans son
dossier privé, et jamais le profil complet :

- **Kilo** : `~/.local/share/kilo/auth.json` + `~/.config/kilo/*.json` ;
  binaires npm et runtime `node` montés en lecture seule.
- **DSH CLI** : `.credentials.yaml`, `settings.yaml`, `cordis.patch.yml` du
  `DSH_HOME`, profils montés en lecture seule ; `DSH_HOME` interne à la sandbox.
- **Goose** : `~/.config/goose/*` (config, secrets, providers personnalisés) ;
  état/sessions dans le dossier privé.

Ces copies sont supprimées à la fin de la session. Les identifiants des autres
outils restent invisibles.

L'environnement est **vidé** (`--clearenv`) puis reconstruit : `HOME`, `PATH`
(contrôlé), `TMPDIR`, `LANG`, `TERM`, plus les variables explicitement déclarées
dans la configuration de l'agent (jamais héritées du processus IH).

## Réseau

- `isolated` : interface réseau isolée, aucune résolution DNS — vérifié en réel
  (les trois agents échouent avec une erreur réseau explicite).
- `isolated_network` : réseau de l'hôte. **Aucun filtrage par destination**
  n'est appliqué (pas de proxy ni de pare-feu applicatif) : un agent peut
  joindre n'importe quelle destination accessible à l'utilisateur. C'est une
  limite assumée, documentée, pas une prétention d'isolation réseau fine.

## Prérequis et compatibilité

- **bubblewrap** : Zorin OS / Ubuntu/Debian `apt install bubblewrap` ;
  Arch Linux `pacman -S bubblewrap`. Vérifié sur la machine de validation :
  `bubblewrap 0.11.0`.
- Sans bubblewrap **fonctionnel**, les niveaux isolés **refusent** le lancement
  avec un message d'installation : IH ne retombe **jamais** silencieusement sur
  une exécution non isolée (`tools/`… voir `sandbox::check_engine`).
- Le niveau non isolé reste possible mais exige une confirmation explicite
  (case à cocher) et apparaît en rouge dans l'approbation.

## Goût du risque : Kilo `--auto`

`kilo run --format json --auto` est **exécuté dans la sandbox** (niveau
configuré). `--auto` reste nécessaire au mode non interactif, mais l'agent ne
peut plus sortir de son worktree : audit réel (voir `docs/AGENT_MODE.md`).

## Tests réels effectués (25/11/2026)

Avec les trois CLI installés, sandbox `isolated_network`, canaris dans le
`$HOME` de l'utilisateur :

| Vérification | Kilo | DSH | Goose |
|---|---|---|---|
| réponse « FAIT » / création de fichier dans le worktree | ✅ | ✅ | ✅ |
| canari `$HOME` illisible (secret jamais dans la sortie) | ✅ | ✅ | ✅ |
| écriture `/tmp` hors sandbox impossible | ✅ | ✅ | ✅ |
| suppression hors worktree impossible (canari intact) | ✅ | ✅ | ✅ |
| accès `~/.ssh` refusé | ✅ | ✅ | ✅ |
| socket de session/keyring invisible | ✅ | ✅ | ✅ |
| environnement hôte non hérité | ✅ | ✅ | ✅ |
| réseau coupé au niveau `isolated` (échec explicite) | ✅ | ✅ | ✅ |
| réseau disponible au niveau `isolated_network` | ✅ | ✅ | ✅ |
| annulation (état `Cancelled`, bwrap et enfants arrêtés) | ✅ | ✅ | ✅ |
| diff limité au fichier créé dans le worktree | ✅ | ✅ | ✅ |

Machine sans sandbox (CI GitHub, conteneur sans espace de noms non
privilégié) : la variable **`IH_SANDBOX_ENGINE`** force le chemin du moteur.
Les tests s'en servent pour simuler une machine sans bubblewrap ; quand le
moteur est absent ou inopérant, les tests qui portent sur la supervision
demandent explicitement le niveau non isolé et les tests d'isolation sont
ignorés — la CI reste donc verte sans jamais faire croire que la sandbox a
été exercée.

Détail déterministe (shell dans la sandbox, sans LLM) :
`crates/ih-agents/tests/sandbox_runtime.rs`.
Tests réels agents : `crates/ih-agent-mode/tests/real_agents_sandbox.rs`
(gated `IH_REAL_AGENTS=1`).

## Limitations restantes

- Isolation système **par sandbox**, pas par compte séparé : un noyau
  vulnérable ou un montage supplémentaire volontaire reste hors de portée.
- Pas de filtrage réseau fin (allowlist) : hormis la coupure totale du niveau
  `isolated`, le réseau reste ouvert.
- Agents de familles non couvertes (Claude Code, Codex, Gemini, OpenCode, pi) :
  binaire monté en lecture seule, **aucun** de leurs identifiants n'est copié
  automatiquement — configurez `env`/`args` ou le niveau non isolé si leur
  authentification l'exige. Non testés en réel (absents de la machine).
- Team Mode : l'isolation sandbox s'applique à chaque session d'équipe, mais la
  course complète n'a pas été rejouée en réel.
- Résolution des exécutables : le chemin **résolu** (liens symboliques et paquet
  npm entier) est monté dans la sandbox. Pour DSH, le CLI du paquet
  d'application (`/opt/apps/.../bin/dsh`) est préféré à une installation npm
  homonyme : une copie nvm récente (`0.2.2-rc.5`) se termine sans sortie en
  profil `headless`, ce qu'IH refuse de considérer comme un succès.
