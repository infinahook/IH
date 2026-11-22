# Validation réelle de l'adaptateur OpenCode

Date : 2026-11-22 · Hôte : Zorin OS 18.1 · IH 0.2.3 (jalon « 0.2.2.1 »)

## CLI testé

| | |
|---|---|
| Exécutable | `/home/infina/.opencode/bin/opencode` |
| Version | **v2.3.2** (`opencode --version`) |
| Authentification | aucune intégration payante ; modèles `opencode/*-free` |

## Détection

- Lancé **sans** `~/.opencode/bin` dans le `PATH` du processus.
- IH complète son `PATH` depuis le shell de connexion : OpenCode apparaît
  « Détecté », version v2.3.2, chemin réel, badge sélectionnable.
- Les autres adaptateurs (Claude Code, Codex, Gemini, OpenCode) restent
  « introuvable » lorsqu'ils sont absents.

## Commandes réellement exécutées

```
opencode --version
opencode run --format json --archived --standalone "<tâche>"
```

La commande est affichée **telle quelle** dans la modale d'approbation
(répertoire, risque, empreinte SHA-256 de l'action).

## Protocole de sortie observé

NDJSON, un objet par ligne :

| `type` | Contenu utile |
|---|---|
| `step_start` | identifiant de session, snapshot |
| `tool_use` | `part.tool` (`read`, `shell`, `edit`, `write`, `question`), `state.input` |
| `text` | `part.text` : réponse de l'assistant |
| `step_finish` | raison, jetons, coût |

IH normalise ces trames en lignes lisibles : `[read] src/lib.rs`,
`[edit] src/lib.rs`, `[write] NOTES.md`, `[question] …`, texte de l'assistant.

## Scénario validé

1. Projet de test isolé `/tmp/ih-oc2` (dépôt git, 1 fichier + README).
2. Agent Mode → OpenCode sélectionné → tâche :
   « Ajoute une ligne `// ih-ok` à la fin de `src/lib.rs` et crée `NOTES.md` ».
3. Approbation affichée puis accordée **une fois**.
4. Exécution réelle : 8 étapes, sortie normalisée.
5. **2 fichiers détectés** : `src/lib.rs`, `NOTES.md` — le dépôt IH et le
   projet de test sont restés intacts pendant la session (isolation vérifiée).
6. Aperçu du diff `src/lib.rs` : `+2 −0`, hunk `@@ -1,1 +1,3 @@`.
7. **Accepter** → `src/lib.rs` écrit sur disque (`// ih-ok` présent,
   `git status` = `M src/lib.rs`).
8. Aperçu du diff `NOTES.md` : `+3 −0` (nouveau fichier).
9. **Rejeter** → `NOTES.md` **absent** du projet.
10. Registre : « 0 en attente » après les deux décisions.

## Bugs trouvés et corrigés pendant cette validation

1. **Écriture hors worktree** sans `--standalone` : OpenCode se connectait au
   service d'arrière-plan et écrivait dans le dépôt IH.
2. **`$PWD` ignoré** : même avec `--standalone`, OpenCode résolvait son projet
   via `$PWD` hérité du répertoire de lancement d'IH. `PWD`/`OLDPWD` sont
   maintenant transmis par le superviseur.
3. **Détection invisible en lancement graphique** : `PATH` des lanceurs GNOME.

## Non validé

- **Annulation d'une session OpenCode** : le mécanisme (groupe de processus,
  récolte) est testé au niveau superviseur avec un agent contrôlé, pas avec
  OpenCode.
- Le type `question` est affiché avec son JSON brut : lisible mais perfectible.
- Claude Code, Codex et Gemini CLI restent non exécutés (absents).

## Captures

- `docs/screenshots/opencode-review.png` — session terminée, 2 fichiers.
- `docs/screenshots/opencode-diff.png` — diff réel `+2 −0` avec accepter/rejeter.
