# Team Mode — Lead · Workers · Reviewer

Une course d'équipe transforme un objectif en un **plan** (orchestrateur), puis
exécute chaque tâche comme une **vraie session d'agent**, dans **un seul
worktree git isolé**. Le projet n'est jamais modifié pendant la course.

## Rôles

| Rôle | Rôle réel |
|---|---|
| `lead` | analyse l'objectif et découpe le travail |
| `worker` | exécute une partie (1 à 3) |
| `reviewer` | relit le résultat |

Un **même CLI peut tenir plusieurs rôles** : chaque rôle lance ses propres
sessions. Avec un seul agent installé (cas fréquent), `opencode` peut être Lead,
Worker et Reviewer — trois sessions distinctes.

## Déroulement

1. `team_request_approval` construit le plan et demande **une** approbation,
   liée à une empreinte (objectif + agents + répertoire).
2. `team_run` vérifie l'empreinte, consomme l'approbation, crée le worktree.
3. Chaque tâche démarre une session (`ih-agent-mode::run_team_tasks`).
4. Les fichiers produits sont collectés et publiés en diffs
   (`origin = team:<run_id>`), puis le worktree est supprimé.
5. L'utilisateur accepte ou rejette chaque fichier (contrôle SHA-256 + sauvegarde).

## Validation réelle (2026-11-23)

Exécutée avec **OpenCode v2.3.2** et **pi v1.6.0**, dépôt de test
`/tmp/team-validate` (`src/lib.rs`, `README.md`, `NOTES.md`).

```bash
cargo test -p ih-agent-mode --test team_real -- --ignored --nocapture
```

Résultat (40,5 s) :

```
tâche Analyser l'objectif   | rôle lead     | agent opencode | état completed
tâche Exécuter la partie 1  | rôle worker   | agent pi       | état completed
tâche Exécuter la partie 2  | rôle worker   | agent opencode | état completed
tâche Relire et valider     | rôle reviewer | agent pi       | état completed
fichiers modifiés : ["NOTES.md", "src/lib.rs"]
accepté : 102 octets, sauvegarde lib.rs.<horodatage>.bak
```

Vérifications observées :

- 4 sessions distinctes (`ses-…`), aucune simulée ;
- worktree `<tmp>/worktrees/run-reelle`, supprimé après la course ;
- dépôt principal **intact** pendant l'exécution ;
- après acceptation : `git status` = `M src/lib.rs` seulement ;
- rejet de `NOTES.md` → contenu inchangé sur disque ;
- sauvegarde créée avant écriture.

## Limites

- Exécution **séquentielle** (le relecteur dépend des exécutions) : pas de
  parallélisme entre workers.
- Pas d'historique des courses, pas d'annulation, pas de reprise après
  redémarrage (le worktree est supprimé en fin de course).
- Le parcours graphique (formulaire, modale, progression live) n'a pas pu être
  joué dans l'environnement de validation (rendu Xvfb indisponible) : voir le
  rapport de validation pour la distinction réel / testé / non validé.
