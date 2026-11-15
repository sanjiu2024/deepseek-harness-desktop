# Changelog

All notable changes to this project are documented here. The format loosely follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.10.0] — 2026-11-15

The last untested hop has a test, and the surface got the work it was owed.

### Added — the click is covered

- `ChatPanel.test.tsx` renders the real component against a mocked Tauri API and asserts what
  a finger does: an `approval/request` frame opens the dialog, **允许这一次** invokes
  `kernel_answer` with the kernel's **string** id and `{ outcome: 'allowed-once' }`, **拒绝**
  sends `rejected`, the dialog closes after answering, and a blocked call reads
  `等待你批准` instead of `执行中`.
- Two vitest projects: `logic` (node) for the reducer and protocol tests, `ui` (jsdom) for
  components. A single environment would charge jsdom's cost to every test in the repo.

### Changed — the surface

- **Layout.** Header shows the route the kernel reported (`provider / model`) rather than a
  session id nobody reads. Messages sit in a scrollable body; the composer is a bordered field
  with the send action inside it, so the input reads as one control instead of a box next to a
  button.
- **Blocks render in arrival order**, and a reasoning part that resumes after a tool call is its
  own block — thinking next to the call it preceded, not a wall of thought on top.
- **Tool cards** are collapsible with an animated height, carry a status that means something
  (pulsing while running, `✓`/`✗`, amber pulse while waiting for approval), and hide the command
  until opened — `{"command":"ls"}` is noise next to `ls`.
- **Reasoning** is a real disclosure: rotating chevron, animated open/close, a breathing dot and
  `思考中…` while it streams, and a character count.
- **Dialogs** fade their backdrop and pop their card in; destructive and primary buttons have
  distinct weight.
- **Streaming** shows a blinking caret after the last arriving block.
- Scrollbars, selection colour and focus rings are styled; the launch screen got a lit card
  instead of a blank rectangle.

### Verified

- `pnpm --filter @dsh-desktop/app test` → 53 passed (4 new UI tests).
- `pnpm --filter @dsh-desktop/app build` → 479.74 kB JS / 25.81 kB CSS.
- `cargo clippy -D warnings` clean; runtime suite unchanged.

## [0.9.2] — 2026-11-15

The approval round trip is now proven without a model, a sandbox, or a click.

### Added

- `crates/dsh-runtime/tests/fixtures/fake-kernel.mjs` — a minimal stdio peer that
  handshakes, then sends an `approval/request` with a **string** id exactly like the real
  kernel (`req_<hex>`), and exits 0 only if the answer comes back with that same id.
- `tests/approval_roundtrip.rs` — spawns it through `KernelClient`, asserts the frame is
  classified as a **request**, answers `allowed-once`, and asserts the fixture accepted it.

Why this shape: the previous attempt at this test went through the real kernel and the model,
so it passed or failed depending on whether the model chose to escalate — and I deleted it as
"flaky" while it was actually reporting this bug. A fake peer removes both sources of
non-determinism and tests only the seam that broke.

### Verified

- `cargo test -p dsh-runtime --test approval_roundtrip` → 2 passed (0.15s).
- Full suite: 16 unit + 2 real-kernel integration + 2 round-trip tests.

### Still unproven

The React click → `invoke` hop. The Rust half is now covered end to end; if a restart still
leaves a card at "执行中" after allowing, the `answer` line in `frames.log` says whether the
button actually sent anything.

## [0.9.1] — 2026-11-15

The frame log now records answers too, and the tool itself is confirmed healthy.

### Added

- `kernel_answer` appends `answer<TAB><id><TAB><outcome>` to the same `frames.log` when
  `DSH_DESKTOP_LOG_FRAMES` is set. The log previously showed only what was forwarded to the
  view, so "did we send the decision?" was unanswerable — the other half of the conversation was
  invisible.

### Confirmed

- `approval/request` now arrives as a **request** in the log (it used to be logged as a
  notification — that was 0.9.0's bug), so the dialog has something to open.
- The `pwsh` tool is healthy on this machine: a fresh headless session ran
  `$PSVersionTable.PSVersion.ToString()` and got `5.1.22621.2506` back with no approval needed.
  So the ✗ rows in a turn are **sandbox denials that need an approval**, not a broken tool — and
  a card still sitting at "执行中" after clicking allow is a separate question from the tool
  being able to run at all.

### Not yet established

Whether the answer reaches the kernel after the button is clicked. That is exactly what the new
log line will show, and it needs one more run from the UI.

## [0.9.0] — 2026-11-15

The approval dialog never appeared because the client could not read the kernel's ids.

### The bug

The frame log settled it in one line:

```
notification	approval/request
```

An approval is a JSON-RPC **request**, but the client logged it as a notification. The kernel
issues **string** ids for the requests it sends (`req_<uuid>` in
`packages/sdk/protocol/src/transport.ts`); this client read the id with
`Value::as_u64`, got `None` for a string, and classified the frame as an id-less
notification. The view only opens its dialog for `kind === "request"`, so it never did — and the
agent stayed blocked forever with nothing on screen.

Both sides of the seam were typed `number`: Rust's `Incoming::Request.id: u64` and the front
end's `KernelFrame.id: number`. Neither type check could see the wire.

### Fixed

- `FrameId` in `dsh-runtime` accepts a number **or** a string, carries it on requests and
  responses, and `response_frame` echoes a string id back as a JSON string. `answer()` and
  `answer_approval()` take `&FrameId`; only numeric ids can match a pending call.
- `kernel_answer` accepts the id as JSON and converts both forms.
- The front end's `FrameId` is `number | string` throughout, including `parseInteraction`.

### Tests that would have caught it

- A frame with a string id and a method is a **request**, not a notification.
- Answering a string id echoes it quoted, and the reply parses back with the same id.
- A numeric id still works.
- The front end parses an approval whose id is `req_9f3c`.

### Corrected

The Rust integration test removed in 0.8.0 as "dependent on model behaviour" was not flaky: it
asserted that an escalating prompt produces `Incoming::Request`, and it was failing because it
was right. It is replaced by the deterministic classification tests above, which do not need a
model at all.

### Verified

- `cargo test -p dsh-runtime` → 16 unit + 2 integration (real kernel, 14.94s) pass.
- `pnpm --filter @dsh-desktop/app test` → 49 passed. `cargo clippy -D warnings` clean.

## [0.8.1] — 2026-11-15

A frame log, because "the dialog never appeared" has two opposite causes.

### Added

- `DSH_DESKTOP_LOG_FRAMES=1` writes one line per forwarded frame to
  `<dsh_home>/frames.log` (`kind<TAB>method`). The two explanations for a missing approval
  dialog — the frame never reached the client, or it reached it and the view did not draw it —
  need opposite fixes, and a screenshot cannot tell them apart. This does.

### Found

- The kernel is not the problem, and neither is the Rust client: with the app's own
  `.dev-home`, a headless run receives `approval/asked` **and** the `approval/request` that
  follows it, answers `allowed-once`, and the turn completes. So the break is downstream of
  `KernelClient::subscribe`.
- Correction to the previous entry: an earlier version of that probe reported zero approvals on
  `.dev-home` and I read it as a reproduction. The probe sent `session/prompt` before
  `initialize` returned, so the runtime correctly rejected it and nothing ever started. Fixed,
  and the same home then behaved.

### Verified

- `pnpm --filter @dsh-desktop/app test` → 47 passed. `cargo clippy` with `-D warnings` clean.
- The frame log itself is not yet exercised end to end: it needs the app's kernel started from
  the UI, which cannot be driven from here.

## [0.8.0] — 2026-11-15

Thinking is rendered where it happened, and a stuck approval can no longer hide.

### Changed

- **Blocks render in arrival order.** The view grouped every reasoning part into one block at the
  top and every tool card after it, so a turn that thinks, calls a tool, then thinks again read as
  one wall of thought followed by unrelated cards. Each part and each call now carries the
  sequence it opened at, and the row sorts by it.
- A reasoning part that resumes **after** a tool call starts a new block instead of continuing the
  paragraph above it — the interruption is the point.

### Added

- A count of received blocking requests in the footer (`收到的请求帧：approval/request ×2`).
  Without it a missing dialog is indistinguishable from a slow tool: if the count stays at zero
  the frame never reached the client, and if it counts up the dialog has a rendering problem.
  Those two need opposite fixes, and a screenshot could not tell them apart before.

### Removed

- An integration test that asserted an approval request arrives for an escalating prompt. It
  passed or failed depending on what the model chose to do — it measured the model, not this
  crate. The wire shape it was really guarding is covered by the serialisation tests in
  `protocol.rs`.

### Verified

- `pnpm --filter @dsh-desktop/app test` → 47 passed (2 new: reasoning splits around a call, and
  consecutive reasoning still merges while nothing interrupts it).

## [0.7.2] — 2026-11-15

A failed model call no longer leaves the view claiming it is still generating.

### Fixed

- A terminal failure in `assistant/attempt` now clears the running state. Waiting for
  `turn/end` was the assumption and it was wrong for exactly the case that matters: when the
  model request itself fails (a connect timeout), the kernel posted the attempt error and did not
  close the turn, so the window showed the failure banner **and** "generating" underneath it —
  two statements that contradict each other. A retry reopens the turn through its own
  `turn/start`.

### Not a bug

The failure itself was real network trouble (`UND_ERR_CONNECT_TIMEOUT` to the provider's
Cloudflare addresses) at the moment the proxy client was being restarted. Verified afterwards:
the same prompt completes both through the proxy and directly.

The kernel does **not** read `HTTPS_PROXY` — a bogus proxy value changes nothing, so the
machine routes through a TUN device rather than an HTTP proxy. Worth knowing before debugging
"the client ignores my proxy": it is not the client.

### Verified

- `pnpm --filter @dsh-desktop/app test` → 45 passed (2 new: the failure clears the running
  state, and a retry reopens the turn).

## [0.7.1] — 2026-11-15

A blocked tool no longer looks like a slow one.

### Fixed

- A tool card waiting on approval now says **等待你批准**, marks its status `?`, and explains
  itself (`内核在等你的决定 —— 见上方弹窗`). Before, it read `执行中…` exactly like a call that
  was simply taking a while — one of those needs the user and the other does not, which is the
  entire reason the card is on screen.
- A `turn/end` drops any queued decision. The kernel cannot still be waiting on a turn it has
  closed, and leaving the queue populated would block the next modal forever.

### Verified

- Headless repro of the screenshot's prompt: the kernel **does** send `approval/request`
  (`{sessionId, toolName, callId, reason}`) and answering `allowed-once` completes the turn —
  2 calls, 2 results, `turn/end completed`. So the request reaches the client; the missing part
  was the card saying so.
- New Rust tests pin the wire shape the front end switches on: `kind` is `request` /
  `notification` / `response`. If that tag ever disappears the approval dialog silently stops
  appearing, and no type check would catch it.

## [0.7.0] — 2026-11-15

Root cause found, and it was neither a handle nor a security driver.

### Found

The WebView2 profile directory could not be created — the app's own error said so once it
printed the reason:

```
cannot create C:\Users\Administrator\AppData\Roaming\com.sanjiu2024.dsh-desktop\EBWebView:
拒绝访问。 (os error 5)
```

A process started from inside the harness sandbox may write its workspace and the temp
directory but **not** `%APPDATA%` or `%LOCALAPPDATA%`. WebView2 turns that `EACCES` into
`0x800700AA` ("resource in use") or `0x8000FFFF` ("catastrophic failure"), which is why every
handle query came back empty and why copies in `%TEMP%` worked while the in-place binary did
not.

Corrections to earlier conclusions in this file:

- 0.6.4 claimed a security driver keyed to the project path. Wrong: the variable is **where the
  profile is created**, not where the binary lives. Removing 火绒 changed nothing.
- 0.6.1 claimed a missing `%APPDATA%\<identifier>` parent. Also wrong, for the same reason.

Both were plausible from the observations and both were falsified by a later measurement. The
lesson is recorded in `docs/句柄占用.md`: when a program fails to create a file, make it **print
the operating system's own error** instead of guessing from the resulting status code.

### Changed

- The WebView2 profile lives beside the kernel home (`<home>/webview2`), which is inside the
  workspace in development and under the app data directory when packaged. Same rule as the
  session store: our data lives where we are allowed to write.
- `scripts/dev.mjs` now orchestrates the loop itself instead of delegating to `tauri dev`:
  build, wait for the binary to settle, start Vite, wait for the port, run the app, stop Vite.
  The port probe is an in-process socket over both loopback families, because Vite binds
  `localhost` → `::1` here and a spawned probe child is denied piped stdio by the sandbox.
- `AppState::dsh_home` extracted so the startup path and the managed state agree on one answer.

### Verified

- Five consecutive launches from `target\debug\` inside the project directory: 5/5.
- `node scripts/dev.mjs`: `[dev] binary settled after 3.6s` → `[dev] vite ready on ::1:1420` →
  app window titled `DeepSeek Harness Desktop`, profile created at `.dev-home/webview2/EBWebView`.

## [0.6.4] — 2026-11-15

The blocking cause is a driver, not a handle — and no handle query can see it.

### Found

After the user installed 火绒, the failure persisted and the investigation reached a firm
conclusion:

- Every path with **zero holders**: exe, `target\debug`, and both WebView2 profile candidates
  report `nothing is holding`.
- The **same binary** runs from `%TEMP%`, `D:\cargo-test`, `D:\space test dir` (spaces are
  irrelevant), `D:\deepseek harness desktop2` and `D:\dsh repo` — and fails from **every**
  subdirectory of `D:\deepseek harness desktop`.
- `--no-sandbox` for the Chromium renderer changes nothing.
- 火绒's processes are running (`HipsDaemon`, `HipsMain`, `HipsTray`).

A **filter driver** refuses the operation without holding a handle, which is why every handle
tool came back empty. It keys on path, which is why a copy elsewhere works.

### Changed

- The WebView2 profile is now created and pinned by the app itself instead of inherited
  (`WEBVIEW2_USER_DATA_FOLDER`), so its location is ours and documented. It is **not** a fix
  for the driver-level block: verified failing after the change.
- `docs/句柄占用.md` gained the second class of problem with the discriminating rule: **when
  `handles.mjs` says nobody holds it and the program still will not start, stop querying
  handles** — add a trust-zone exclusion (`D:\deepseek harness desktop`,
  `%LOCALAPPDATA%\com.sanjiu2024.dsh-desktop`, `%APPDATA%\com.sanjiu2024.dsh-desktop`,
  `%TEMP%`) and retry.

### Not fixed

The app still cannot start from inside this project directory on this machine. Nothing in the
repository can change that: it is an exclusion-list configuration outside the code.

## [0.6.3] — 2026-11-15

The handle tool grew a shared library, and the project directory turned out to be the poisoned
part.

### Added

- `scripts/lib/holders.mjs` — the Restart Manager query as a module, so the `handles` CLI and a
  launcher can share it instead of shelling out to each other.

### Found — measured, not inferred

- The same executable starts fine from `%TEMP%`, from `D:\cargo-test`, and from
  `D:\space test dir` (spaces are not the problem), and fails from **every** path inside
  `D:\deepseek harness desktop` — `target\debug\`, `target\run\`, `plain\`. Same binary, same
  environment, same profile directory. Whatever breaks WebView2 creation is keyed to the project
  path.
- Lockers observed on this machine, in order: `tailscale-ipn` (holds the built exe),
  `clash-verge` (holds it persistently, 30s+). Killing the first frees the exe; the second is
  the user's proxy, so killing it costs network access.
- `esbuild` also fails on this machine with `remove ...temp\esbuild-...: Access is denied` —
  its own cleanup file is taken by something else. Intermittent; a rerun usually works.

### Not done

- A launcher that builds into `target/` and runs a copy from outside the repository was written
  and **reverted**: it was not verified end to end, and shipping an unverified dev loop is worse
  than shipping none. The blocker is the esbuild temp-file failure above, not the design.
- `scripts/dev.mjs` is unchanged from 0.6.2 — the version that was actually run.

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
