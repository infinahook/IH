# Guide du workspace IH

IH 0.2.1 transforme la conversation en véritable environnement de
développement : la conversation à gauche, le code à droite, les outils en bas.

## Disposition

```
┌───────────────┬──────────────────────────────────────┐
│ Conversation  │ Explorateur │ Onglets + Éditeur       │
│ (chat 0.2.0)  │             │                         │
├───────────────┴─────────────┴─────────────────────────┤
│ Panneau bas : Terminal │ Git │ Problèmes              │
└───────────────────────────────────────────────────────┘
```

- Les séparateurs verticaux et horizontaux se déplacent à la souris.
- L'explorateur peut être masqué ; le panneau bas escamoté.
- « Réinitialiser la disposition » restaure les valeurs par défaut.
- La disposition (largeurs, hauteur, explorateur visible, onglets ouverts) est
  enregistrée dans la base locale et restaurée au redémarrage.

## Explorateur et éditeur

- L'arborescence vient de `file_tree` (côté Rust) ; les dossiers techniques
  (`.git`, `target`, `node_modules`) sont exclus.
- Un clic ouvre un onglet. Les onglets modifiés portent un point ; leur
  fermeture demande confirmation.
- L'éditeur est CodeMirror 6 : coloration Rust, TypeScript/JavaScript, Python,
  JSON, TOML, Markdown, YAML, CSS, HTML ; numéros de ligne ; recherche (Ctrl+F).
- **Ctrl+S** enregistre. L'empreinte SHA-256 lue à l'ouverture est renvoyée :
  si le fichier a changé sur le disque entre-temps, l'enregistrement est
  **refusé** et l'interface propose de recharger le fichier ou d'écraser
  explicitement.

## Contexte projet et mentions `@`

- Tapez `@` dans le compositeur : une liste de fichiers s'ouvre (recherche
  floue, les fichiers exclus sont signalés).
- Sélectionner un fichier ajoute un « chip » de contexte et insère `@chemin`.
- Retirer un chip ne modifie pas le texte du message.
- Un aperçu (`context_prepare`) affiche **exactement** ce qui sera envoyé :
  fichiers inclus, octets lus, troncatures, et fichiers écartés avec leur motif.
- Le contenu est lu et assemblé **côté Rust** : l'interface ne lit jamais un
  fichier pour le compte du modèle.

### Fichiers exclus par défaut

`.env`, `.env.*`, `id_rsa`, `id_ed25519`, `*.pem`, `*.key`, `*.p12`, `*.pfx`,
`.netrc`, `.git-credentials`, `.ssh/`, `node_modules/`, `target/`, `.git/`,
`dist/`, `build/`, `.venv/`, `__pycache__/`.

Un fichier dont le **contenu** ressemble à un secret (clé privée, `api_key =`,
jeton `sk-…`, mot de passe) est également écarté. Pour l'inclure, il faut une
confirmation explicite (« Inclure quand même »), et le fichier est alors marqué
`sensitive` dans l'aperçu.

## Diff et application des modifications

1. Modifiez un fichier dans l'éditeur.
2. « Voir le diff » prépare une modification **sans rien écrire**.
3. La vue affiche les lignes ajoutées, supprimées et le contexte.
4. « Accepter » écrit le fichier après avoir revérifié son empreinte ; une
   sauvegarde de l'ancien contenu est créée dans `~/.config/ih/backups/diffs/`.
5. « Rejeter » abandonne la modification ; le disque n'est pas touché.

Aucune modification n'est appliquée silencieusement : sans acceptation, le
fichier reste identique.

## Limites de cette version

- Pas d'Agent Mode : les modifications viennent de l'éditeur, pas encore d'un
  agent (prévu en 0.2.2).
- L'éditeur ne gère pas les très gros fichiers (limite configurable,
  2 Mio par défaut) ni l'édition binaire.
- La restauration des onglets rouvre les fichiers par leur chemin ; un fichier
  déplacé ou supprimé est ignoré sans erreur.
