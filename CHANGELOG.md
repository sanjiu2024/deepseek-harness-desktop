# Changelog

All notable changes to this project are documented here. The format loosely follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
