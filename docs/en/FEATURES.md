# IH feature status

This is the **honest** reference. Three statuses:

- ✅ **done**: implemented, wired to the UI, covered by automated tests.
- ⚠️ **partial**: genuinely works, but part of the need is uncovered — stated explicitly.
- ❌ **missing**: not implemented. No UI button pretends otherwise.

Last verified: 363 Rust tests + 181 frontend tests, all green.

| # | Feature | Status | Detail |
|---|---|---|---|
| 1 | Complete IH core | ✅ | `ih-core`: validated TOML config, typed IDs, events, resource policy, platform detection, redaction |
| 2 | Multi-agent manager | ✅ | Adapter registry, `PATH` detection, supervised sessions, kernel-enforced concurrency limit |
| 3 | Claude Code, Codex, Gemini CLI, OpenCode | ✅ | Documented non-interactive arguments (`claude -p`, `codex exec`, `gemini -p`, `opencode run`), overridable via `args` with `{prompt}`. **Not tested against the real binaries** (absent from this machine): supervision is tested with a fake agent |
| 4 | Cloud and local AI providers | ✅ | OpenAI-compatible (OpenAI, OpenRouter, custom), Anthropic, Gemini, Ollama; chat, SSE/NDJSON streaming, models, health. **Complete chat screen**: live streaming, cancellation, history, provider/model selection, retry |
| 5 | Model manager | ✅ | Live API discovery, default model, per-provider enable/disable, capabilities only when the API exposes them |
| 6 | Skills: install and updates | ⚠️ | Install from a folder with strict manifest validation, SHA-256 digest, integrity verification, enable, uninstall. **Update from a remote registry: missing** |
| 7 | MCP plugins | ⚠️ | **Native remote MCP servers: Streamable HTTP and legacy SSE** (real transports, tested against a local server), tool discovery, explicit per-tool enablement, calls gated by approvals bound to parameter fingerprints, keyring token, anti-SSRF, bounded sizes and timeouts, history and audit; existing stdio servers kept. **Remaining: resources/prompts/sampling, chat tool-use, automatic injection into agent CLIs** |
| 8 | Project management | ✅ | Folder picker, persistence, recent projects, list removal **without ever deleting the folder** |
| 9 | Code editor | ✅ | CodeMirror 6: Rust/TS/JS/Python/JSON/TOML/Markdown/YAML/CSS/HTML highlighting, line numbers, Ctrl+F, Ctrl+S, tabs, write-conflict detection |
| 10 | Built-in Linux terminal | ⚠️ | Real PTY (`portable-pty`), input, resize, close, ANSI/OSC filtering. **No full VT emulation** (cursor, alternate screen, IME) |
| 11 | Git and worktrees | ⚠️ | Porcelain status, diff (staged/unstaged), stage/unstage, commit, branches, log, worktrees. **Local operations only**: no fetch/push/pull/merge/rebase |
| 12 | Autonomous agents and teams | ⚠️ | Task graph, lead/worker/reviewer roles, dependencies, bounded scheduling, real step execution. **No self-restarting autonomous loop**: the plan is created then executed under user control |
| 13 | Task orchestrator | ✅ | Deterministic plan creation, persistence, state transitions, cycle detection, blocking on failed dependency |
| 14 | Per-project persistent memory | ✅ | Project/global scope, tags, search, delete, bounded injected context preview |
| 15 | Permissions and security | ✅ | One-shot category-bound approvals with TTL, risk classification, path jail, keyring-only secrets |
| 16 | Automations and workflows | ⚠️ | Creation (interval ≥ 5 min, agent+prompt steps **or MCP tool steps**), real manual execution, run history; MCP steps go through IH's guard (server/tool enabled, arguments validated, non-interactive mode limited to pre-approved reads). **No background scheduler firing on its own**, no workflow edit/delete in the UI |
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

## 0.2.1 milestone delivered — Workspace, context and diff

- **Three resizable zones** (conversation, workspace, bottom panel), layout
  persisted in the database and tabs restored on restart.
- **Explorer + editor**: tree, file search, tabs with dirty state, CodeMirror 6
  editor across 11 languages.
- **Rust-assembled project context** (`crates/ih-context`): `@` mentions, fuzzy
  search, default exclusions, secret detection, size limits, exact preview
  before sending.
- **Diff**: bounded line-by-line computation, preview, accept with digest check
  and backup, reject without writing.
- Validated in the real binary: `@` mention → chip + preview, `+1 −0` diff,
  reject (disk untouched), accept (file written), tab restoration.

## Explicitly missing

1. **Binary attachments** (images, PDF) in the chat: absent.
2. **Agent Mode** inside the conversation (running a CLI from chat, before/after
   diff, accept/reject): planned for 0.2.2.
3. **Visual Team Mode** (Lead/Worker/Reviewer): planned for 0.2.3.
4. **CLI agent sessions fed back into chat threads**.
5. **Remote skills registry** with update verification.
6. **Extended MCP**: resources, prompts, sampling and model tool use — **HTTP/Streamable and SSE transports shipped in 0.6.1**.
7. **Remote Git** (`push`, `pull`, `fetch`) and conflict resolution.
8. **Active background automation scheduler**.
9. **Audit page** exposed through IPC (storage exists).
10. ~~LICENSE file~~ → **MIT** since 0.5.2 (see `LICENSE`).
11. **Physical Arch machine**: CI validates build and tests inside an
    `archlinux:latest` container.
12. **Very large file fallback**: above 2 MiB (configurable limit) the file is
    not opened.

## Automated tests

| Suite | Count | Scope |
|---|---|---|
| `ih-core` | 38 | config, errors, events, resources, paths, redaction, platform |
| `ih-store` | 19 | migrations (incl. v4), CRUD, cascade, pagination, concurrency, search, **MCP catalog and calls** |
| `ih-security` | 41 | approvals (fingerprint-bound), expiry/replay, risk classification, **MCP network policy (SSRF)**, keyring |
| `ih-providers` | 19 | 4 providers, SSE/NDJSON, 401 errors, timeout, missing key |
| `ih-agents` | 20 | adapters, success/failure, cancel, timeout, limit, bounded output |
| `ih-extensions` | 27 | manifests, digest, install, path jail, MCP stdio, silent server |
| `ih-workspace` | 33 | Git, path jail, PTY, **LCS diff**, metadata, checked write |
| `ih-orchestrator` | 23 | cycles, readiness, teams, scheduler, memory budget |
| `ih-update` | 39 | Ed25519 signature, digest, download, staging, rollback |
| `ih-i18n` | 5 | FR/EN parity, fallback, substitution |
| `ih-chat` | 12 | streaming, cancellation, bounded history, injected context, errors, title, search + **real Ollama e2e** |
| `ih-context` | 23 | exclusions, secrets, truncation, budget, path jail, fuzzy mentions |
| `ih-agent-mode` | 8 | worktree sandbox, change collection, adapter detection, action fingerprint |
| `ih-mcp` | 42 | schemas, URL policy, unified session; **9 Streamable HTTP integration tests** and **4 SSE integration tests** against a real local MCP server |
| **Rust total** | **363** | |
| Frontend (`vitest`) | 181 | i18n, formatting, ANSI, Git, file tree, chat, layout, mentions, tabs, diff, agent, **MCP** |
| **Total** | **530** | |

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
