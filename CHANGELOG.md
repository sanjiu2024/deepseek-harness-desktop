# Changelog

All notable changes to this project are documented here. The format loosely follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.6.2] — 2026-11-15

Added a tool that names the culprit, because the error text never does.

### Added

- `scripts/handles.mjs` — asks Windows' **Restart Manager** which process holds a path, and
  optionally kills it. No Sysinternals download, no administrator rights.
  `node scripts/handles.mjs <path> [--kill] [--only name]`.
- `docs/句柄占用.md` — the three misleading error shapes ("access denied", `0x800700AA`,
  "file in use") that all mean one thing, and the order to handle them in.

### Found

- The exe that would not start was held by **`tailscale-ipn`** (the Tailscale GUI client), not
  by a build problem: `node scripts/handles.mjs` named it in one call, after `Get-Process`,
  `icacls`, an exclusive-open sweep and Restart Manager against the *directory* had all come up
  empty.
- **Correction to 0.6.1's diagnosis.** I attributed the WebView2 `E_UNEXPECTED` to a missing
  `%APPDATA%\<identifier>` parent directory. That was wrong: with the directory deleted, a copy
  of the same binary starts fine. Creating the directory before the window is kept as hygiene,
  but the comments no longer claim a cause that does not exist.

### Changed

- `tauri-plugin-single-instance` focuses the running window instead of racing it for the
  WebView2 profile.
- The app deliberately does **not** kill other processes at startup: it cannot know whether the
  holder is the user's VPN, backup or debugger. Killing stays a human decision in `scripts/`.

### Verified

- `node scripts/handles.mjs` identified `tailscale-ipn` as the holder in one call.
- A copy of the binary under `%TEMP%` starts and stays up on a machine where the in-place exe
  cannot be launched.
- `pnpm --filter @dsh-desktop/app test` → 43 passed; `cargo clippy` with `-D warnings` clean.

## [0.6.1] — 2026-11-15

### Fixed

- **`scripts/dev.ps1` could not run at all.** PowerShell's default execution policy rejects an
  unsigned `.ps1`, so the launcher the previous release added died on
  `UnauthorizedAccess` before doing anything — and "run it with `-ExecutionPolicy Bypass`" is
  not something a project should ask of its users. Replaced by `scripts/dev.mjs`: Node is
  already required, and no execution policy applies to it. Same job — drop the DeepSeek
  Harness shim directory from PATH for this process, set the three `DSH_DESKTOP_*` variables,
  read the key from `provider.env` — plus `--check` to print the resolved environment without
  launching anything.
- `docs/工程.md` carried a stray backspace character inside the shim path.

## [0.6.0] — 2026-11-15

M2: approvals, questions, and tool cards — with the shapes captured from a real run.

### Added

- `src/lib/interaction.ts` — the two blocking kernel requests as first-class values:
  `approval/request` and `user-question/ask`. They arrive as JSON-RPC **requests**, not
  notifications; a client that only listens for events hangs the turn with no error anywhere.
  10 tests.
- `InteractionHost` — approval dialog (allow-once / reject) and a question dialog with
  single/multi select plus free text. One decision on screen at a time, the rest queue: two
  stacked modals is how the wrong button gets clicked.
- `ToolCard` — collapsed tool invocation: name, status, one-line summary, command and output
  on expand.

### Corrected against a capture

The tool shapes were guessed wrong twice. A real run (`pwsh`, plus an escalation to
`danger-full-access`) produced:

| Guessed | Actually |
| --- | --- |
| `tool/call.arguments` is an object | a **JSON string**; re-encoding doubled the quoting |
| `tool/result` carries flat `output` / `callId` / `isError` | `{ message: { toolCallId, content: [{type:'text',text}], isError } }` |

Two synthetic tests that encoded the wrong guess were deleted rather than patched — they
asserted a shape the kernel does not send.

### Found — when approval actually happens

A plain `pwsh ls` inside `workspace-write` runs with **no prompt**. The prompt appears only on
escalation: the same write outside the workspace was first denied by the sandbox, then retried
with `sandbox_permissions: danger-full-access` + `justification`, and only then did the kernel
ask. `approval/request` params: `{ sessionId, toolName, callId, reason }`.

### Verified

- `pnpm --filter @dsh-desktop/app test` → 43 passed (2 files).
- `pnpm --filter @dsh-desktop/app build` → 475.39 kB JS (145.87 kB gzip).
- `cargo clippy --workspace --all-targets -- -D warnings` → clean.

## [0.5.2] — 2026-11-15

### Fixed

- The thinking trace stayed open after the answer finished. 0.5.1 opened it while streaming but
  never closed it. It now closes when text starts, unless the user toggled it — a manual
  decision outranks the streaming state.

## [0.5.1] — 2026-11-15

### Fixed

- Reasoning is expanded while the model is thinking. A collapsed disclosure on a live turn looks
  exactly like no thinking at all, which is how the missing trace got reported as a bug. Measured
  first: `你好` produces 0 reasoning deltas even at `reasoningEffort: high`; a word problem
  produces 15. The greeting case was never a client bug — only the invisible presentation was.

## [0.5.0] — 2026-11-15

Reasoning is visible, and answers are rendered as markdown.

### Fixed

- **Streamed reasoning was erased when the answer settled.** The durable
  `assistant/message` replaced every part with its own content; when a route omits
  reasoning from that message, a thought the user had already watched stream in vanished
  at the end of the turn. Parts are now reconciled: the durable message wins for what it
  carries, and streamed reasoning is preserved when the durable one has none.
  Measured on a prompt that forces thinking: 35 reasoning deltas in, 375 characters of
  durable reasoning out — so the data was always there, the render was dropping it.
- **Markdown was shown as raw text.** `**bold**`, backticks, and lists appeared literally,
  which makes an answer read like source code. Assistant text now renders through
  `react-markdown` + `remark-gfm`; raw HTML stays disabled because the text comes from a
  model.

### Changed

- Reasoning moved out of the message body into a collapsed `思考过程 · N 字` disclosure.
  A trace you can open is not the same thing as a paragraph of the reply.
- `MessageRow` split into its own file; the marker list of unrendered kernel events moved
  from a permanent line under the composer into a collapsed footer entry.

### Verified

- `pnpm --filter @dsh-desktop/app test` → 32 passed (2 new: reasoning survives a durable
  message that omits it, and durable reasoning wins when present).
- `pnpm --filter @dsh-desktop/app build` → 468.19 kB JS (143.88 kB gzip) — markdown support
  costs ~170 kB raw, ~52 kB gzipped.

## [0.4.1] — 2026-11-15

First run in a real window. Two problems surfaced immediately.

### Fixed

- **A failed turn rendered as nothing at all.** The reducer recorded `failure`; the view never
  read it. The user saw a prompt, a blank panel, and the input box still there. There is now a
  failure banner quoting the kernel's own message (`MISSING_CREDENTIAL` names the variable that
  is missing). A new turn clears the previous failure.
- **`pnpm tauri dev` died on `[pnpm] pnpm not found`.** The DeepSeek Harness Desktop installer
  puts its command shims in `%LOCALAPPDATA%\deepseek-harness\bin` ahead of the npm global bin,
  and its `pnpm.cmd` refuses to run until the managed pnpm exists. Added `scripts/dev.ps1`:
  it moves the real pnpm ahead **for its own process only**, sets the three `DSH_DESKTOP_*`
  variables, and starts tauri. Nothing machine-wide changes.

### Added

- `provider.env` in the kernel home (git-ignored, one `KEY=value` per line, process environment
  wins): no need to export the key in every shell.

### Verified

- Real window screenshot: title, user message, input box, and the unrendered-event list all
  render; `seq 18` means the stream reached `turn/end`.
- `cargo test -p dsh-desktop-app` → 2 passed (env-file parsing).
- `pnpm --filter @dsh-desktop/app test` → 30 passed.

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
