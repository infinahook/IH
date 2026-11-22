# IH workspace guide

IH 0.2.1 turns the conversation into a real development environment:
conversation on the left, code on the right, tools at the bottom.

## Layout

```
┌───────────────┬──────────────────────────────────────┐
│ Conversation  │ Explorer │ Tabs + Editor              │
│ (chat 0.2.0)  │          │                            │
├───────────────┴──────────┴────────────────────────────┤
│ Bottom panel: Terminal │ Git │ Problems               │
└───────────────────────────────────────────────────────┘
```

- Vertical and horizontal splitters are draggable.
- The explorer can be hidden; the bottom panel collapsed.
- "Reset layout" restores defaults.
- Layout (widths, height, explorer visibility, open tabs) is stored in the
  local database and restored on restart.

## Explorer and editor

- The tree comes from `file_tree` (Rust side); technical folders (`.git`,
  `target`, `node_modules`) are excluded.
- A click opens a tab. Modified tabs show a dot; closing one asks for
  confirmation.
- The editor is CodeMirror 6: Rust, TypeScript/JavaScript, Python, JSON, TOML,
  Markdown, YAML, CSS, HTML highlighting; line numbers; Ctrl+F search.
- **Ctrl+S** saves. The SHA-256 recorded at open time is sent back: if the file
  changed on disk meanwhile, saving is **refused** and the UI offers to reload
  or to overwrite explicitly.

## Project context and `@` mentions

- Type `@` in the composer: a file list opens (fuzzy search, excluded files
  flagged).
- Selecting a file adds a context chip and inserts `@path`.
- Removing a chip does not change the message text.
- A preview (`context_prepare`) shows **exactly** what will be sent: included
  files, bytes read, truncations, and skipped files with their reason.
- Contents are read and assembled **on the Rust side**: the UI never reads a
  file on behalf of the model.

### Default exclusions

`.env`, `.env.*`, `id_rsa`, `id_ed25519`, `*.pem`, `*.key`, `*.p12`, `*.pfx`,
`.netrc`, `.git-credentials`, `.ssh/`, `node_modules/`, `target/`, `.git/`,
`dist/`, `build/`, `.venv/`, `__pycache__/`.

A file whose **content** looks like a secret (private key, `api_key =`, `sk-…`
token, password) is also skipped. Including it requires an explicit
confirmation ("Include anyway"); it is then marked `sensitive` in the preview.

## Diff and applying changes

1. Edit a file in the editor.
2. "View diff" prepares a modification **without writing anything**.
3. The view shows added, removed and context lines.
4. "Accept" writes the file after re-checking its digest; a backup of the
   previous content is created in `~/.config/ih/backups/diffs/`.
5. "Reject" drops the modification; the disk is untouched.

Nothing is ever applied silently: without acceptance, the file stays identical.

## Limits of this version

- No Agent Mode yet: changes come from the editor, not from an agent (0.2.2).
- Very large files (configurable limit, 2 MiB by default) and binary files are
  not editable.
- Tab restoration reopens files by path; a moved or deleted file is silently
  ignored.
