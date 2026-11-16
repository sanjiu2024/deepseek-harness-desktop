# AGENTS.md

Conventions for working in this repository. Confirm before trusting any command here — if a
command fails, fix this file in the same change.

## Commands

| What | Command | Notes |
| --- | --- | --- |
| Install JS deps | `pnpm install` | pnpm 12, workspace root |
| Fetch kernel | `pnpm kernel:fetch` | downloads the pinned `@deepseek-ai/dsh` into `kernel/` |
| Dev app | `pnpm tauri dev` | Vite + Tauri hot reload |
| Frontend only | `pnpm --filter @dsh-desktop/app dev` | no Rust, no kernel |
| Build | `pnpm build` | Vite production bundle |
| Rust check | `cargo check --workspace` | first run compiles Tauri, ~5 min |
| Rust test | `cargo test --workspace` | |
| Rust lint | `cargo clippy --workspace -- -D warnings` | |
| TS typecheck | `pnpm --filter @dsh-desktop/app check` | |
| **Verify everything** | `pnpm verify` | format · clippy · tests · kernel round trips · front end · build · smoke |
| Verify, fast | `pnpm verify:fast` | skips the build and the smoke (the two slow ones) |

Node ≥ 24.9, Rust 1.100, pnpm 12. The version pins live in `.nvmrc`,
`rust-toolchain.toml`, and `package.json#packageManager`.

## Where code goes

- **UI, rendering, user interaction** → `apps/desktop/src`. Split by feature under
  `src/features/<name>/`, shared primitives under `src/components/`.
- **Talking to the kernel** → `crates/dsh-runtime`. Nothing else spawns `dsh`.
- **Reading session logs** → `crates/dsh-store`. Nothing else parses JSONL.
- **PTY, file tree, file watching** → `crates/dsh-native`.
- **`apps/desktop/src-tauri`** — assembly only: `#[tauri::command]` wrappers, app state,
  window/tray setup. If a command body exceeds ~10 lines, the logic belongs in a crate.

## Versions

Semantic versioning describes **changes a consumer has to know about**. Nothing consumes this
build yet, so the version is bumped only when a build is actually handed out:

- **Bump on**: a tagged release, an installer someone else runs, a rollback point.
- **Do not bump on**: an ordinary commit. Those accumulate under `Unreleased` in
  `CHANGELOG.md`, which is the honest place for work nobody has consumed.

A day of development that goes 0.1 → 0.12 is not progress, it is a commit counter. When a
release is cut, the accumulated `Unreleased` section becomes one version with one rationale.

## Changing the kernel

The kernel is fetched, not vendored (`kernel/` is git-ignored, 819 MB). It is still ours to
change — three layers, cheapest first:

1. **Plugin** — `bridge/` mounted through `profiles/sdk/cordis.patch.yml`. Adds behaviour
   (event subscriptions, services) and survives upgrades. This is how the client already
   modifies kernel behaviour.
2. **Patch** — `kernel-patches/*.patch`, applied by `scripts/fetch-kernel.mjs` on every fetch,
   in filename order. For a change that must land *inside* a kernel module. A patch that fails
   to apply fails the fetch by design. Rules live in `kernel-patches/README.md`.
3. **Upstream PR** — anything still patched after one kernel upgrade.

Never copy kernel source into this repository, and never edit `kernel/` by hand: the next
`kernel:fetch` replaces it and the change vanishes silently.

## Building when `target/` is locked

Some processes on a Windows machine take a handle on freshly written executables — Tailscale,
Wallpaper Engine, MIXLINE, and ComfyUI's Python have all done it here. The symptom is
`LNK1104: cannot open file …/build/dsh-desktop-app/<hash>/out/dsh_desktop_app.exe`, and it is
not a code error.

Two rules:

1. **Find out who holds it** before changing anything: `node scripts/handles.mjs <path>`
   (`--kill` if it is ours; **never** kill a stranger's process — work around it).
2. **Build somewhere else** rather than fighting the lock:

   ```bash
   CARGO_TARGET_DIR=D:/dsh-build-target cargo build --release -p dsh-desktop-app
   CARGO_TARGET_DIR=D:/dsh-build-target pnpm smoke:app      # finds it automatically
   ```

   `scripts/smoke-app.mjs` looks in `$CARGO_TARGET_DIR/release` first, then `target/release`.

Cleanup after a build experiment: `pnpm clean` (report) → `pnpm clean:apply` (remove).
`target/debug` alone reaches 18 GB in this project; only `release` is ever shipped.

## Hard rules

1. **Never vendor the kernel.** It is fetched by `scripts/fetch-kernel.mjs` into `kernel/`,
   which is git-ignored. Do not copy upstream source into this repository.
2. **Never commit secrets or user data.** `.env`, `dsh-home/`, `profiles/*/cordis.patch.local.yml`
   are ignored on purpose. The DeepSeek API key lives in the app's settings store.
3. **Never point `DSH_HOME` at `~/.dsh`.** The app uses its own private home so it does not
   fight the user's CLI install for credentials or sessions.
4. **Kernel stdout is protocol data.** JSON-RPC frames only. Log lines go to stderr; do not
   merge the two streams.
5. **Generated bindings are read-only.** `src/lib/ipc.ts` comes from `tauri-specta`; edit the
   Rust command, not the generated file.

## 提交前跑 pnpm verify

一条命令跑完全部检查，退出码 0 才算数：

```
rust/rustfmt · rust/clippy · rust/cargo test · kernel round trips
front/typecheck · front tests · front build · headless smoke
```

**跳过的项会明确打印成 skip，并说明为什么**（「需要 kernel/」「fast 模式」）。一个因为没跑而
悄悄通过的检查，比没有检查更糟——所以 `0 passed · 0 failed · 3 skipped` 这种结果要自己看一眼
skip 的原因。

失败时会打出**能直接复现的命令**和输出的最后十几行。要单跑一组：`--only=rust`、
`--only=kernel`、`--only=front`、`--only=smoke`。

## 对内核的行为要问内核

有三件事读文档读不出来，只能对真内核发一次请求：分叉出来的会话认不认、图片块形状对不对、
内核死了池能不能重来。它们各自有一个测试，靠 `DSH_DESKTOP_HOME` / `DSH_DESKTOP_KERNEL`
决定跑还是跳过：

```
crates/dsh-runtime/tests/fork_resume.rs     resume 一个复制出来的会话
crates/dsh-runtime/tests/prompt_blocks.rs   发一个带图片块的 prompt
crates/dsh-runtime/tests/restart.rs         杀掉内核，看池能不能再起一个
```

新增"我们造了内核本该自己写的数据"这类功能时，配一个这样的测试。它们抓到的都是猜不出来的
东西：`SessionHeader` 少 `type`、可选字段序列化成 `null` 被判非法、`session/prompt` 其实是
创建语义。

## Style

- Rust: `rustfmt` defaults, `clippy -D warnings`, `thiserror` for domain errors.
- TypeScript: strict mode, no `any`, no default exports for components.
- Commits: imperative subject ≤ 72 chars, body explains *why*.
- Comments explain why, not what. Delete a comment that only restates the line below it.

## Layout discipline

The top level is fixed at: `apps/ crates/ profiles/ scripts/ docs/ kernel-patches/` plus root
config files. A new top-level directory needs a reason in `docs/设计.md` first.

`kernel-patches/` is a directory rather than a `scripts/` subfolder because its contents are
*data* that outlives the script that applies them: a patch is read, rebased and reviewed on its
own, and burying it next to the tool that consumes it hides that it exists.
