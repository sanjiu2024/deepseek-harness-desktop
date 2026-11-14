/**
 * Development launcher.
 *
 * Written in Node rather than PowerShell on purpose: a `.ps1` is blocked by the
 * default execution policy on a fresh Windows install, and "run this with
 * -ExecutionPolicy Bypass" is not a thing a project should ask of its users.
 * Node is already a hard requirement here.
 *
 * Two things this fixes over calling `pnpm tauri dev` directly:
 *
 * 1. The DeepSeek Harness Desktop installer puts its command shims in
 *    %LOCALAPPDATA%\deepseek-harness\bin ahead of the npm global bin, and its
 *    `pnpm.cmd` refuses to run until the managed pnpm exists. The shim directory
 *    is dropped from PATH for this process only.
 * 2. The three DSH_DESKTOP_* variables are set so a dev run never mixes with an
 *    installed build, and the provider key is read from provider.env.
 *
 * Usage:
 *   node scripts/dev.mjs              # tauri dev
 *   node scripts/dev.mjs build        # production build
 *   node scripts/dev.mjs --check      # print the resolved environment and exit
 */
import { spawn } from 'node:child_process'
import { existsSync, readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const args = process.argv.slice(2)
const check = args.includes('--check')
const command = args.find((arg) => arg === 'build' || arg === 'dev') ?? 'dev'

const shimDir = process.env.LOCALAPPDATA
  ? join(process.env.LOCALAPPDATA, 'deepseek-harness', 'bin')
  : null
const npmDir = process.env.APPDATA ? join(process.env.APPDATA, 'npm') : null

// Drop the shim directory, then put the real package manager first.
const pathEntries = (process.env.PATH ?? '')
  .split(process.platform === 'win32' ? ';' : ':')
  .filter((entry) => entry !== '' && entry !== shimDir)
const pnpmBin = npmDir === null ? null : join(npmDir, process.platform === 'win32' ? 'pnpm.cmd' : 'pnpm')

const env = {
  ...process.env,
  PATH: [...(npmDir === null ? [] : [npmDir]), ...pathEntries].join(
    process.platform === 'win32' ? ';' : ':',
  ),
  DSH_DESKTOP_HOME: join(root, '.dev-home'),
  DSH_DESKTOP_KERNEL: join(root, 'kernel'),
  DSH_DESKTOP_BRIDGE: join(root, 'bridge'),
  RUST_LOG: 'dsh_desktop_app=debug,dsh_runtime=info,dsh_kernel=warn,info',
}

// provider.env lives with the kernel home and is git-ignored; process env wins.
const providerFile = join(env.DSH_DESKTOP_HOME, 'provider.env')
const fromFile = readEnvFile(providerFile)
for (const [name, value] of Object.entries(fromFile)) {
  if (!env[name]) env[name] = value
}

const missingKey = !env.OPENCODE_API_KEY && !env.DEEPSEEK_API_KEY

if (check) {
  console.log(`command      ${command}`)
  console.log(`pnpm         ${pnpmBin !== null && existsSync(pnpmBin) ? pnpmBin : '(not found — npm i -g pnpm)'}`)
  console.log(`DSH_HOME     ${env.DSH_DESKTOP_HOME}`)
  console.log(`kernel       ${env.DSH_DESKTOP_KERNEL} ${existsSync(env.DSH_DESKTOP_KERNEL) ? '' : '(missing — run pnpm kernel:fetch)'}`)
  console.log(`bridge       ${env.DSH_DESKTOP_BRIDGE} ${existsSync(env.DSH_DESKTOP_BRIDGE) ? '' : '(missing)'}`)
  console.log(`provider key ${missingKey ? '(none — prompts will fail)' : 'loaded'}`)
  process.exit(missingKey ? 1 : 0)
}

if (missingKey) {
  console.warn('[dev] no provider key: set OPENCODE_API_KEY or put it in .dev-home/provider.env')
}

const pnpm = pnpmBin !== null && existsSync(pnpmBin) ? pnpmBin : 'pnpm'
const tauriArgs = ['--filter', '@dsh-desktop/app', 'tauri', command]

const child = spawn(pnpm, tauriArgs, {
  cwd: root,
  env,
  stdio: 'inherit',
  shell: process.platform === 'win32',
})

child.on('exit', (code, signal) => {
  if (signal !== null) process.exit(1)
  process.exit(code ?? 0)
})

/** Minimal `KEY=value` reader; unreadable is normal, not an error. */
function readEnvFile(path) {
  if (!existsSync(path)) return {}
  const out = {}
  for (const line of readFileSync(path, 'utf8').split('\n')) {
    const trimmed = line.trim()
    if (trimmed === '' || trimmed.startsWith('#')) continue
    const at = trimmed.indexOf('=')
    if (at <= 0) continue
    const name = trimmed.slice(0, at).trim()
    const value = trimmed.slice(at + 1).trim().replace(/^["']|["']$/g, '')
    if (name !== '' && value !== '') out[name] = value
  }
  return out
}
