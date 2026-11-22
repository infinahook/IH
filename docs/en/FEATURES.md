# IH feature status

This is the **honest** reference. Three statuses:

- ✅ **done**: implemented, wired to the UI, covered by automated tests.
- ⚠️ **partial**: genuinely works, but part of the need is uncovered — stated explicitly.
- ❌ **missing**: not implemented. No UI button pretends otherwise.

Last verified: 218 Rust tests + 49 frontend tests, all green.

| # | Feature | Status | Detail |
|---|---|---|---|
| 1 | Complete IH core | ✅ | `ih-core`: validated TOML config, typed IDs, events, resource policy, platform detection, redaction |
| 2 | Multi-agent manager | ✅ | Adapter registry, `PATH` detection, supervised sessions, kernel-enforced concurrency limit |
| 3 | Claude Code, Codex, Gemini CLI, OpenCode | ✅ | Documented non-interactive arguments (`claude -p`, `codex exec`, `gemini -p`, `opencode run`), overridable via `args` with `{prompt}`. **Not tested against the real binaries** (absent from this machine): supervision is tested with a fake agent |
| 4 | Cloud and local AI providers | ✅ | OpenAI-compatible (OpenAI, OpenRouter, custom), Anthropic, Gemini, Ollama; chat, SSE/NDJSON streaming, models, health. **Complete chat screen**: live streaming, cancellation, history, provider/model selection, retry |
| 5 | Model manager | ✅ | Live API discovery, default model, per-provider enable/disable, capabilities only when the API exposes them |
| 6 | Skills: install and updates | ⚠️ | Install from a folder with strict manifest validation, SHA-256 digest, integrity verification, enable, uninstall. **Update from a remote registry: missing** |
| 7 | MCP plugins | ⚠️ | stdio servers, `initialize`, `tools/list`, `tools/call`, one-shot approval gating, no zombies. **No resources/prompts/sampling, no HTTP/SSE transport** |
| 8 | Project management | ✅ | Folder picker, persistence, recent projects, list removal **without ever deleting the folder** |
| 9 | Code editor | ⚠️ | File tree, confined read/write, line numbers, search, Ctrl+S, dirty state. **No syntax highlighting or multi-cursor** |
| 10 | Built-in Linux terminal | ⚠️ | Real PTY (`portable-pty`), input, resize, close, ANSI/OSC filtering. **No full VT emulation** (cursor, alternate screen, IME) |
| 11 | Git and worktrees | ⚠️ | Porcelain status, diff (staged/unstaged), stage/unstage, commit, branches, log, worktrees. **Local operations only**: no fetch/push/pull/merge/rebase |
| 12 | Autonomous agents and teams | ⚠️ | Task graph, lead/worker/reviewer roles, dependencies, bounded scheduling, real step execution. **No self-restarting autonomous loop**: the plan is created then executed under user control |
| 13 | Task orchestrator | ✅ | Deterministic plan creation, persistence, state transitions, cycle detection, blocking on failed dependency |
| 14 | Per-project persistent memory | ✅ | Project/global scope, tags, search, delete, bounded injected context preview |
| 15 | Permissions and security | ✅ | One-shot category-bound approvals with TTL, risk classification, path jail, keyring-only secrets |
| 16 | Automations and workflows | ⚠️ | Creation (interval ≥ 5 min, agent+prompt steps), real manual execution, run history. **No background scheduler firing on its own**, no workflow edit/delete in the UI |
| 17 | Conversation and task history | ✅ | Threads, paginated messages, delete, downloadable JSON export |
| 18 | French and English UI | ✅ | 800 keys FR and EN, parity enforced by a Rust test and a frontend test, immediate switching |
| 19 | Complete settings | ✅ | Theme, language, resource limits, log level, update channel and interval, auto-download. Validated on the Rust side |
| 20 | Logging and diagnostics | ✅ | Daily rotating log + console, redaction, log tail, exportable diagnostics report, open log folder |
| 21 | Update system | ⚠️ | Ed25519-signed manifest, strict verification, SHA-256, `.part` staging, mandatory approval, atomic AppImage replacement + backup/rollback, `deb`/`pacman` plan never executed by IH. **No delta updates, no percentage progress** |
| 22 | DEB, AppImage, Arch install | ✅ | Tauri `deb` + `appimage` bundles, source-based Arch `PKGBUILD`, CI producing artifacts and building on Arch |

## 0.2.0 milestone delivered — Conversation

- Conversations persisted in SQLite: create, rename, delete, search by title
  **and** message content.
- Real fragment-by-fragment streaming, including through the actual Ollama
  client (end-to-end test `crates/ih-chat/tests/ollama_e2e.rs`).
- Effective cancellation: the stream is cut, partial text is kept and marked
  `cancelled`.
- Provider and model selection, API model discovery, remembered default model.
- Network errors shown with their code + Retry button.
- Indicators: generation in progress, tokens consumed, latency.

## Explicitly missing

1. **Project context in chat** (adding files, `@file` mentions, attachments):
   planned for 0.2.1.
2. **Resizable panel workspace** and syntax-highlighted editor: planned for 0.2.1.
3. **Agent Mode** inside the conversation (running a CLI from chat, before/after
   diff, accept/reject): planned for 0.2.2.
4. **Visual Team Mode** (Lead/Worker/Reviewer): planned for 0.2.3.
5. **CLI agent sessions fed back into chat threads**.
6. **Remote skills registry** with update verification.
7. **Non-stdio MCP** (HTTP/SSE, resources, prompts, sampling).
8. **Remote Git** (`push`, `pull`, `fetch`) and conflict resolution.
9. **Active background automation scheduler**.
10. **Audit page** exposed through IPC (storage exists).
11. **LICENSE file**: license not chosen yet.
12. **Physical Arch machine**: CI validates build and tests inside an
    `archlinux:latest` container.

## Automated tests

| Suite | Count | Scope |
|---|---|---|
| `ih-core` | 30 | config, errors, events, resources, paths, redaction, platform |
| `ih-store` | 15 | migrations, CRUD, cascade, pagination, concurrency, search |
| `ih-security` | 19 | approvals, expiry/replay, risk classification, keyring |
| `ih-providers` | 15 | 4 providers, SSE/NDJSON, 401 errors, timeout, missing key |
| `ih-agents` | 13 | adapters, success/failure, cancel, timeout, limit, bounded output |
| `ih-extensions` | 27 | manifests, digest, install, path jail, MCP stdio, silent server |
| `ih-workspace` | 22 | Git (status, commit, worktrees), path jail, zombie-free PTY |
| `ih-orchestrator` | 23 | cycles, readiness, teams, scheduler, memory budget |
| `ih-update` | 39 | Ed25519 signature, digest, download, staging, rollback |
| `ih-i18n` | 5 | FR/EN parity, fallback, substitution |
| `ih-chat` | 10 | streaming, cancellation, bounded history, errors, title, search + **real Ollama e2e** |
| **Rust total** | **218** | |
| Frontend (`vitest`) | 49 | i18n, formatting, ANSI, Git, file tree, chat stream reducer, errors |
| **Total** | **267** | |

## Manual verifications performed

- **End-to-end chat in the real application** (Xvfb + local NDJSON Ollama
  server): project opened, Ollama provider and model selected, message sent,
  answer received **streaming fragment by fragment**, final text persisted with
  tokens and latency, then **cancellation** verified ("Cancelled" badge,
  1,359 ms, partial text kept in the database with `cancelled: true`).
  Screenshots: [chat-stream.png](../screenshots/chat-stream.png),
  [chat-cancelled.png](../screenshots/chat-cancelled.png).

- `cargo clippy --workspace --all-targets -- -D warnings`: no warnings.
- `cargo fmt --all -- --check`: clean.
- `npm run typecheck`, `npm test`, `npm run build`: OK.
- Tauri release build + `deb` and `AppImage` packages.
- End-to-end update chain: Ed25519 key generated with `openssl`, manifest signed,
  verified by `ih-update-verify` (success), then tampered and rejected (exit 1).
