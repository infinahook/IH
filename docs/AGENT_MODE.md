# Guide du mode Agent

Le mode Agent d'IH (0.2.2) exécute un CLI d'agent de développement sur une
tâche en langage naturel, dans une **copie isolée du projet**.

## Principe

1. Vous choisissez un adaptateur **réellement détecté** dans votre `PATH`.
2. IH crée un **worktree git détaché** du projet (`git worktree add --detach`).
3. L'agent travaille dans ce worktree : votre projet n'est pas modifié.
4. IH diffuse sa sortie en temps réel et conserve les événements.
5. À la fin, IH calcule les différences entre le worktree et le projet, puis
   vous les présente comme des **diffs à accepter ou rejeter** un par un.
6. L'acceptation passe par le mécanisme de la 0.2.1 : contrôle d'empreinte
   SHA-256 et sauvegarde de l'ancien contenu.

## Pourquoi une copie isolée

Les CLI d'agents écrivent directement sur le disque et n'exposent pas de
protocole d'approbation standard. IH ne prétend donc pas intercepter chaque
écriture : il **isole** la session. Rien n'entre dans votre projet sans votre
accord explicite.

## Lancement

1. Onglet **Conversation** → sélecteur de mode **Agent**.
2. Choisissez un adaptateur (les CLI absents sont affichés « introuvable » et
   non sélectionnables).
3. Décrivez la tâche.
4. **Lancer** → une demande d'approbation affiche la commande exacte, le
   répertoire et le risque ; « Autoriser une fois » n'autorise que cette action.
5. Suivez la progression, annulez si nécessaire.
6. **Voir les modifications** → examinez chaque fichier, acceptez ou rejetez.

## Détection des trois CLI supplémentaires

La détection ne se limite pas au `PATH` du lanceur graphique :

1. le nom configuré (`kilo`, `dsh`, `goose`) et ses alias connus
   (`kilocode` pour Kilo, `dsh-cli` pour DSH) ;
2. le `PATH` courant de l'application ;
3. le `PATH` du **shell de connexion** (comme pour OpenCode) ;
4. des dossiers connus : `~/.local/bin`, `~/.cargo/bin`, `~/.bun/bin`,
   `~/.npm-global/bin`, `/usr/local/bin`, `/snap/bin`,
   `/var/lib/flatpak/exports/bin` et les paquets d'application
   `/opt/apps/<app>/files/bin` (cas de DSH, absent du `PATH` graphique).

Quand plusieurs binaires portent le même nom (par exemple un `dsh` npm via nvm
et le paquet d'application), seule l'installation qui répond correctement à
`--version` est retenue : un binaire homonyme non fonctionnel est signalé comme
tel dans l'interface et **n'est pas sélectionnable**. Les versions lues sur la
machine de validation sont `kilo 7.8.32`, `dsh 0.1.0-rc.7` et `goose 1.61.0`.

## Notes d'intégration par CLI

- **Kilo** : `--auto` est requis en mode non interactif ; sans lui, kilo
  auto-rejette sa première demande de permission d'outil et la session échoue.
  L'autorisation IH (approbation liée à l'empreinte, avant lancement) et le
  worktree isolé restent les frontières de sécurité.
- **DSH CLI** : le profil `headless` répond une seule tâche et sort ; il
  n'imprime que le message final de l'assistant (pas d'interface structurée).
- **Goose** : `stream-json` fournit des événements NDJSON normalisés par IH
  (`message` assistant → texte lisible, `complete` masqué) ; `--quiet` supprime
  la bannière d'accueil qui polluait la sortie.

## Approbations

- Une approbation est **ponctuelle** : usage unique, liée à une empreinte
  (agent + commande + répertoire + tâche), expirée après 5 minutes.
- Toute modification de la commande ou de la tâche change l'empreinte : IH
  refuse alors le lancement.
- « Refuser » invalide la demande ; rien n'est exécuté.

## Adaptateurs

| Adaptateur | Commande non interactive | Prérequis |
|---|---|---|
| Claude Code | `claude -p "<tâche>"` | CLI `claude` dans le PATH |
| Codex | `codex exec "<tâche>"` | CLI `codex` |
| Gemini CLI | `gemini -p "<tâche>"` | CLI `gemini` |
| OpenCode | `opencode run --format json --archived --standalone "<tâche>"` | CLI `opencode` ≥ 2.x |
| Kilo | `kilo run --format json --auto "<tâche>"` | CLI `kilo` (Kilo Code) ; NDJSON `text`/`tool_use` |
| DSH CLI | `dsh --profile headless "<tâche>"` | CLI `dsh` (DeepSeek Harness) ; profil `headless` fourni |
| Goose | `goose run --no-session --quiet --output-format stream-json -t "<tâche>"` | CLI `goose` ≥ 1.61 ; NDJSON `message`/`complete` |
| Personnalisé | `args` contenant `{prompt}`, sinon tâche ajoutée | exécutable de votre choix |

IH affiche le nom, la version (via `--version`), le chemin et le diagnostic de
chaque adaptateur, et ne propose que ceux réellement présents.

### Détection malgré un PATH de lanceur graphique

Les lanceurs GNOME n'héritent pas des `export PATH=…` de `~/.bashrc`. Un CLI
installé dans `~/.opencode/bin` ou `~/.local/bin` serait donc invisible. Au
démarrage, IH lit le `PATH` du shell de connexion (avec un délai borné) et
complète le sien : les CLI usuels sont détectés même lancés depuis le menu.

### OpenCode : deux réglages obligatoires

- `--standalone` : sans serveur privé, OpenCode se connecte au service
  d'arrière-plan qui résout le projet depuis **son** contexte — les fichiers
  étaient écrits **hors du worktree** (constaté en v2.3.2).
- `PWD=/OLDPWD=` répertoire de travail : OpenCode résout son projet via `$PWD`
  et non via le répertoire courant réel. IH transmet donc ces variables à
  chaque processus d'agent.

Les événements JSON d'OpenCode (`step_start`, `tool_use`, `text`,
`step_finish`) sont normalisés en lignes lisibles (`[read] chemin`,
`[shell] commande`, texte de l'assistant).

## Sécurité et limites

**Ce qu'IH garantit**
- Aucune modification du projet pendant la session (isolation par worktree).
- Aucune application de changement sans acceptation explicite.
- Détection de conflit : si le fichier a changé entre-temps, l'acceptation est
  refusée.
- Sortie bornée, timeout, annulation qui tue le groupe de processus.
- Aucun secret transmis à l'agent.

**OpenCode** : ses propres permissions ne remplacent jamais l'approbation IH.
Sans TTY, OpenCode ne peut pas demander de confirmation : il exécute ses outils.
L'isolation IH (worktree + `--standalone` + `PWD`) garantit que les fichiers
restent dans la copie, mais une commande shell lancée par l'agent s'exécute avec
vos droits — configurez les permissions d'OpenCode en conséquence.

**Ce qu'IH ne peut pas garantir**
- L'agent s'exécute avec **vos droits**. S'il écrit hors de son répertoire de
  travail (chemin absolu, `/tmp`, réseau), IH ne l'empêche pas : seuls les
  garde-fous propres au CLI s'appliquent.
- Pas de bac à sable système (bubblewrap/firejail) en 0.2.2.
- L'isolation nécessite un **dépôt git** ; hors dépôt git, le mode Agent refuse
  de démarrer.
- Les fichiers binaires ou de plus de 512 Kio modifiés par l'agent ne sont pas
  proposés en diff (signalés comme tels).

## Sessions

Chaque session est persistée (identifiant, projet, adaptateur, tâche,
répertoire, état, résumé, fichiers, événements). Après redémarrage, IH
réaffiche les sessions du projet avec leurs événements ; une session interrompue
n'est jamais présentée comme réussie.

## Hors périmètre (0.2.2)

Team Mode, délégation Lead/Worker/Reviewer, MCP HTTP/SSE, Git distant,
automatisations planifiées, installation automatique des CLI, exécution
distante.
