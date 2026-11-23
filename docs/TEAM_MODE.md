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


## Validation graphique réelle (0.5.2)

Exécutée sur la session Zorin OS réelle (DISPLAY `:1`), binaire 0.5.2, projet
`/tmp/tm2`.

| Étape | Résultat observé | Capture |
|---|---|---|
| Conversation → Agent → **Équipe** | bascule sans rechargement, formulaire affiché | `team-form.png` |
| Agents proposés | **seuls les détectés** (Claude Code, Codex, Gemini CLI, OpenCode) | `team-form.png` |
| Même CLI sur deux rôles | OpenCode Lead **et** Worker accepté, sans erreur | `team-form.png` |
| Approbation | plan prévu dans l'ordre **Lead → Workers → Reviewer**, empreinte, répertoire isolé | `team-approval.png` |
| Aucune exécution avant approbation | aucune session avant le clic « Autoriser une fois » | — |
| Progression | 4 tâches, rôles, `1 session(s)`, états « Terminée », interface **réactive** | `team-progress.png` |
| Résumé | « course terminée — 2 fichier(s) en attente de validation » + fichiers | `team-progress.png` |
| Revue | 2 diffs, `Voir le diff`, Accepter, Rejeter | `team-review.png` |
| **Rejet** | `NOTES.md` inchangé sur disque, `git status` vide | `docs/screenshots/team-review.png` |
| **Acceptation** | `src/lib.rs` écrit (`soustraction` présent), sauvegarde `lib.rs.<horodatage>.bak`, toast « modification en attente acceptée » | `team-small-window.png` |
| Petite fenêtre 1280×800 | pas de débordement, explorateur replié, commandes accessibles | `team-small-window.png` |

### Bugs corrigés en 0.5.2

1. **Ordre d'exécution non topologique** : `TaskGraph::iter()` suivait les
   identifiants aléatoires — le Lead pouvait passer en dernier et le Reviewer
   avant un Worker. `topological_order()` (dépendances, puis Lead → Workers →
   Reviewer) est utilisé pour l'exécution, la persistance et le plan affiché.
2. **Interface figée pendant la course** : « IH ne répond pas » (dialogue GTK)
   car `team_run` bloquait le thread principal. La commande est désormais
   déportée (`#[tauri::command(async)]`) ; vérifié : fenêtre réactive pendant
   toute la course.
3. **Texte d'aide obsolète** (« exécutants distincts du lead et du relecteur »)
   alors que le même agent peut tenir plusieurs rôles.
4. **Rappel de mode comprimé** en fenêtre étroite : masqué sous 1400 px.

### Persistance constatée

Les tâches (rôle, agent, état) sont en base : 8 tâches retrouvées **à
l'identique après redémarrage**. En revanche, une course **ne peut pas être
reprise** : le worktree est supprimé en fin de course et rien ne conserve
l'état d'avancement d'une course interrompue.
