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

## Style

- Rust: `rustfmt` defaults, `clippy -D warnings`, `thiserror` for domain errors.
- TypeScript: strict mode, no `any`, no default exports for components.
- Commits: imperative subject ≤ 72 chars, body explains *why*.
- Comments explain why, not what. Delete a comment that only restates the line below it.

## Layout discipline

The top level is fixed at: `apps/ crates/ profiles/ scripts/ docs/` plus root config files.
A new top-level directory needs a reason in `docs/设计.md` first.
