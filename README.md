# DeepSeek Harness Desktop

A desktop client for the [DeepSeek Harness](https://github.com/deepseek-ai/deepseek-harness) agent
kernel. We keep **only the kernel** — the agent loop, LLM adapters, tool execution, session
persistence — and put our own front end on top. The shell is [Tauri 2](https://tauri.app), so the
binary is a fraction of the Electron build it replaces.

> **Status: works, ships on Windows, not packaged yet.** Real conversations, streaming, tool
> cards, approvals, a file tree and a terminal. 131 Rust tests, 84 front-end tests, CI green on
> three jobs. What is missing is written down in [docs/状态.md](docs/状态.md) — including one gap
> that is *not* fixed rather than quietly omitted.

```
┌──────────────────────────────────────────────┐
│  React 19 + Tailwind 4                       │  front end
│  sessions · streamed answers · tool cards     │  (someone else's job — see docs/接口.md)
│  approvals · settings · file tree · terminal  │
└───────────────────┬──────────────────────────┘
                    │ invoke() / listen()  — 37 commands, 6 events
┌───────────────────┴──────────────────────────┐
│  Rust core                                   │
│  kernel pool · NDJSON wire · session store    │
│  PTY · workspace tree · settings · logs       │
└───────────────────┬──────────────────────────┘
                    │ stdio, JSON-RPC 2.0 over newline-delimited JSON
┌───────────────────┴──────────────────────────┐
│  dsh kernel — `dsh --profile sdk`            │  upstream, unmodified
└──────────────────────────────────────────────┘
```

The bridge is the kernel's **SDK surface**, not its web GUI protocol. That choice is explained in
[docs/设计.md](docs/设计.md); the short version is that the Remote protocol spans 34 generated
namespaces and would cost more than everything else here combined.

## Layout

| Path | What lives there |
| --- | --- |
| `apps/desktop` | The application: React front end + Tauri assembly layer |
| `crates/dsh-runtime` | Kernel processes: spawn, NDJSON framing, one kernel per route, exit watching |
| `crates/dsh-store` | Everything read from and written to `$DSH_HOME/sessions` — logs, renames, trash, search |
| `crates/dsh-native` | Host facilities: PTY, workspace tree, text decoding, preview gate |
| `bridge/` | A kernel-side plugin, mounted through the profile patch, that turns streamed deltas into frames |
| `profiles/sdk` | Kernel profile patches (compression, phrasing, home directory) |
| `kernel-patches/` | Patches applied to the fetched kernel when a plugin cannot reach far enough |
| `scripts/` | Setup, kernel fetch, slimming, bundling, smoke test, clean |
| `docs/` | [设计](docs/设计.md) · [工程](docs/工程.md) · [状态](docs/状态.md) · [接口](docs/接口.md) · [需求](docs/requirements/) |

Two rules the code follows:

- **`apps/desktop/src-tauri` holds no business logic.** It assembles crates and exposes commands;
  anything longer than a dozen lines belongs in a crate.
- **The kernel is fetched, never vendored.** `kernel/` is git-ignored; changing kernel behaviour
  goes through the plugin, then a patch, then an upstream PR — in that order.

## Getting started

```bash
# toolchain: rust 1.100, node 24, pnpm 12
pnpm install
pnpm kernel:fetch          # pins @deepseek-ai/dsh into kernel/, verifies the version
pnpm kernel:slim --apply   # 753 MB → 370 MB, each removal verified by a real turn
pnpm kernel:smoke          # prove the wire works before touching anything else
pnpm dev                   # the app (vite + tauri); see scripts/dev.mjs
pnpm verify                # everything: fmt, clippy, tests, kernel round trips, front end, smoke
```

A model needs a key: either `OPENCODE_API_KEY` / `DEEPSEEK_API_KEY` in the environment, or
`<dsh_home>/provider.env`. The app can also write it — `settings_set_secret` — and never puts it
in its own environment, so it does not leak into child processes.

Builds on this machine sometimes collide with an external process holding `target/`:

```bash
CARGO_TARGET_DIR=D:/dsh-build-target cargo build --release -p dsh-desktop-app
node scripts/handles.mjs <path>    # names whoever holds it, before you touch anything
```

`pnpm clean` reports what is reclaimable; `pnpm clean:apply` removes it (`target/debug` alone
reaches 18 GB here).

## What it does

| | |
| --- | --- |
| Conversation | sessions on disk, history loaded from the kernel's own logs, session restore on launch |
| Streaming | answers and reasoning arrive token by token (a kernel-side plugin, not a protocol feature) |
| Tools | collapsible cards, real inputs and outputs, per-tool status |
| Approvals | the kernel blocks on a decision; the dialog is what unblocks it |
| Models | one kernel per route; switching a model starts a second process and leaves the first running |
| Images | `session_prompt_blocks` accepts base64 image blocks; validation happens in Rust before the kernel sees them |
| Workspace | file tree with preview and insert-path, a real PTY, and a change watcher — all in one side panel |
| Storage | search across sessions, per-session export, a trash that never empties itself |
| Cleanup | image attachments are collected when nothing references them (6 h grace) |
| Recovery | a crashed kernel is restarted on the next message, with a bounded retry policy |
| Diagnostics | logs to `<dsh_home>/logs/desktop.log.<date>`, panics and kernel crash output included, 14-day retention |

## Size, honestly

The kernel is **370 MB** after `kernel:slim` (753 MB as published): the `sdk` profile does not load
the LibreOffice kit (335 MB), speech recognition or the official web front end, and each removal is
verified by running a real turn. What remains cannot be dropped — it is the agent runtime itself.

An installer would be roughly 500 MB, because the client is nothing without a kernel and the
kernel is JavaScript that needs a Node runtime. `pnpm bundle:apply` assembles `dist-bundle/` with
all three; the app resolves `kernel/` and `node` beside its own executable. **No installer has been
built or signed yet** — that is the largest remaining piece of work.

## Tests

```bash
cargo test --workspace --release     # 126: runtime 35+2+2+5+1+1+1 · store 48+1 · native 21+4 · app 6
pnpm --filter @dsh-desktop/app test  # 84
pnpm smoke:app                       # the real binary, headless: front end loaded? kernel? store?
```

`pnpm verify` runs all of the above in one go, including the three checks that ask a live kernel.
It prints skips explicitly — a check that did not run is never reported as a pass.

`smoke:app` is the one that catches what unit tests cannot: a release build with no front end
inside, or a kernel that never completes its handshake. It asserts on the **log file**, because a
blank window produces no other evidence.

## License

MIT. The kernel is MIT too — see its own repository for third-party notices.
