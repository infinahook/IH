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
| OpenCode | `opencode run "<tâche>"` | CLI `opencode` |
| Personnalisé | `args` contenant `{prompt}`, sinon tâche ajoutée | exécutable de votre choix |

IH affiche le nom, la version (via `--version`), le chemin et le diagnostic de
chaque adaptateur, et ne propose que ceux réellement présents.

## Sécurité et limites

**Ce qu'IH garantit**
- Aucune modification du projet pendant la session (isolation par worktree).
- Aucune application de changement sans acceptation explicite.
- Détection de conflit : si le fichier a changé entre-temps, l'acceptation est
  refusée.
- Sortie bornée, timeout, annulation qui tue le groupe de processus.
- Aucun secret transmis à l'agent.

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
