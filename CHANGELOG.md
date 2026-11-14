# Changelog

All notable changes to this project are documented here. The format loosely follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.4.0] — 2026-11-15

Streaming works. The kernel's live assistant stream now reaches the client token by token.

### Added

- `bridge/` — `dsh-desktop-live-bridge`, a **kernel-side plugin** subscribed to the global
  `agent/assistant-stream` event. It republishes each frame on a tagged stderr line
  (`\u0001DSH-LIVE <json>`). stderr, not stdout: stdout belongs to the JSON-RPC transport, and a
  second writer there can interleave half-frames.
- `scripts/install-bridge.mjs` — copies the plugin to `$DSH_HOME/node_modules`, which is where
  Node's resolution from `$DSH_HOME/profiles/sdk/` actually finds it. `kernel/node_modules` does
  not work, and neither does a machine path committed into the profile patch.
- `AppState::install_bridge` — the app performs the same copy before spawning the kernel.
- `dsh_runtime::LIVE_PREFIX` / `LIVE_CHUNK_METHOD` — the stderr reader turns tagged lines into
  `dsh-desktop/live-chunk` notifications on the same broadcast as stdout frames, so the UI keeps
  one path for everything.
- `applyLiveChunk` in the reducer — folds a `StreamChunk` without touching `lastSeq`. Live frames
  have no durable sequence; giving them a synthetic one would either collide with the next durable
  event or force the gate to learn an exception.
- Tests: 28 frontend tests (fixture now includes 10 captured live frames) and a Rust end-to-end
  test that spawns the kernel, prompts it, and asserts real `text-delta` frames arrive.

### Fixed

- `session-persistence-jsonl` config was invalid: a patch replaces a row's whole `config`, so
  setting `compression` alone dropped the required `root`. The kernel logged a ValidationError and
  fell back to defaults — the patch had been doing nothing.

### Verified

- `CAPTURE_PROVIDER=opencode-go node scripts/capture-events.mjs` → 19 durable events + **10 live
  frames**, whose deltas concatenate to the answer text.
- `cargo test -p dsh-runtime --test kernel_smoke live_bridge` → passes in 15.43s with a real key.
- `pnpm --filter @dsh-desktop/app test` → 28 passed.

## [0.3.0] — 2026-11-15

First real model answer. And: the SDK protocol has no token-level streaming.

### Added

- `profiles/sdk/cordis.patch.yml` declares the `opencode-go` route (`llm-pi-ai` adapter; the
  key is referenced by variable name and reaches the process through its environment).
- `scripts/capture-events.mjs` honours `CAPTURE_PROVIDER` / `CAPTURE_MODEL` and passes our
  profile patch, so a capture exercises the same composition the app runs.
- The reducer handles `assistant/message`: content is a block array, and `reasoning` and `text`
  are two blocks, not a prefix of the answer.
- Tauri forwards `DEEPSEEK_API_KEY` and `OPENCODE_API_KEY`, but only when the app process
  actually has them.

### Found — the SDK wire has no streaming

`assistant/live-chunk` is a **Web-client presentation event** (`AssistantLiveChunkEvent` in
`dsh-api-session-controller`), produced from a separate follow connection. The SDK server
forwards durable `session/event` rows only (`packages/sdk/server/src/server.ts:164`), and
`assistant/chunk` is retired in format v4 — it survives only in historical migration code.

Observed consequence: the whole `assistant/message` arrives when the turn ends; nothing
incremental reaches the client in between.

Three ways out, none taken yet:

1. Accept whole-message arrival and show a "generating" state;
2. Write a kernel-side plugin (mounted by profile patch, no source change) that re-emits the
   assistant stream as notifications;
3. Poll the session log — but the answer is not on disk before it completes either, so this
   buys nothing.

### Verified

- `node scripts/capture-events.mjs` with `CAPTURE_PROVIDER=opencode-go` → 19 events, including
  an `assistant/message` with a real answer and its reasoning.
- `pnpm --filter @dsh-desktop/app test` → 22 passed, 8 of them against the captured fixture.

## [0.2.0] — 2026-11-15

The session event reducer, built against a **captured** kernel stream rather than the docs.

### Added

- `scripts/capture-events.mjs` — starts the kernel, sends one prompt, and writes every
  `session.event` to a fixture. It runs without an API key: the prompt is accepted and the user
  side is committed, then the model call fails and the failure events come through too.
- `src/lib/reducer.ts` — the event stream folded into messages, tool calls, route, and failure
  state. Pure, no React, tested directly.
- `src/lib/reducer.test.ts` — 20 tests, 8 of them against the captured fixture.
- `src/features/chat/ChatPanel.tsx` — message list, streamed text, tool rows, input box.
- `kernel_call` Tauri command — one generic door for the SDK methods instead of eight wrappers
  that each drift on their own.

### Corrected against the capture

Reading the protocol types was not enough; the first reducer had four wrong assumptions:

| Guessed | Actually |
| --- | --- |
| user message id field is `messageId` | it is `id` |
| one prompt yields one `user/message` | **five** — the prompt plus four `<system-reminder>` context blocks, distinguished only by `source.kind` |
| streaming arrives as `assistant/chunk` | `assistant/live-chunk`, carrying a `StreamChunk` (`text-delta`, `reasoning-delta`, `tool-call-delta`, …) |
| route lives at `data.header.{provider,model}` | `data.header.config.{provider,model}` |

The first two would have shipped as visible bugs: duplicated user bubbles, and an answer rendered
as hundreds of one-token messages.

### Known limits

- `assistant/live-chunk` is exercised only through synthetic events: a real stream needs an API
  key, which the capture environment did not have. The durable `assistant/attempt` path *is*
  covered by the fixture.
- `tool/call` and `tool/result` have not been observed at all; their handling is written from the
  upstream event list and is marked unverified in the source.
- Still no history, no approvals UI, no tool cards, no multiple sessions.

## [0.1.0] — 2026-11-15

The kernel can be started, handshaken, and driven from the app. No conversation UI yet.

### Added

- `dsh-runtime::protocol` — the SDK wire vocabulary (9 methods, 2 kernel-initiated requests,
  6 notifications), frame classification, and frame builders. 8 unit tests pin the rules that
  matter: an `id`+`method` frame is a request, `id` alone is a reply, `method` alone is a
  notification, and `allowed-once` is spelled the way upstream spells it.
- `dsh-runtime::process` — `KernelClient`: spawns the kernel with `kill_on_drop`, owns stdin
  through a single write channel, reads stdout in one task that never blocks on a caller, and
  keeps stderr separate for diagnostics. Kernel-initiated requests are broadcast, not handled
  inline, because answering an approval from inside a blocked `initialize` deadlocks the agent.
- An end-to-end test that spawns a real kernel, handshakes, asserts the server identity, and
  shuts down. Skips cleanly when the kernel has not been fetched.
- Tauri commands: `kernel_start`, `kernel_initialize`, `kernel_answer`, `kernel_stop`,
  `kernel_diagnostics`, `kernel_paths`, `app_info`. Kernel frames are forwarded as Tauri events;
  a subscriber that falls behind gets a `kernel://lagged` count instead of silent loss.
- `BootPanel` — the M0 probe screen: resolved paths, start/stop, and a live frame log. It exists
  to prove React → invoke → Rust → kernel → event → React on one screen, and it says so.
- `scripts/smoke-m0.mjs` — the same handshake without the GUI, for CI and for a quick check.

### Changed

- Workspace dependencies now list only what a crate uses today; crate path dependencies are
  declared by their consumer. An unused entry is a warning, and warnings get ignored.
- Version pins moved from `0.0.1` to `0.1.0` across Cargo, the Tauri manifest, and both
  `package.json` files.

### Known limits

- The Tauri IPC hop and the event forwarder are compiled and the app launches, but the window
  path has not been clicked through — the honest claim is "the Rust ↔ kernel link is tested".
- The kernel install is 819 MB untrimmed; see `docs/状态.md`.
