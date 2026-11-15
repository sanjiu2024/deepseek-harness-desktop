/**
 * Development launcher.
 *
 * Node rather than PowerShell: a `.ps1` is blocked by the default execution
 * policy on a fresh Windows install, and asking users to run
 * `-ExecutionPolicy Bypass` is not a project's business.
 *
 * What it adds over `pnpm tauri dev`, each for a measured reason:
 *
 * 1. **PATH** — the DeepSeek Harness Desktop installer puts command shims ahead
 *    of the npm global bin and its `pnpm.cmd` refuses to run until the managed
 *    pnpm exists.
 * 2. **A settle wait** — on this machine, a freshly written executable cannot
 *    create its WebView2 environment: the security driver is still scanning it,
 *    and the failure surfaces as `0x8000FFFF` / `0x800700AA`. The same binary
 *    started seconds later works every time, so the launcher waits for the file
 *    to stop being held before running it.
 * 3. **One retry** — the scan window is not deterministic; a second attempt
 *    after the wait always succeeded in testing.
 *
 * Usage:
 *   node scripts/dev.mjs              # Vite + build + run
 *   node scripts/dev.mjs build        # production bundle
 *   node scripts/dev.mjs --check      # print the resolved environment, exit
 *   node scripts/dev.mjs --no-run     # build only
 */
import { spawn, spawnSync } from 'node:child_process'
import { existsSync, readFileSync, statSync } from 'node:fs'
import net from 'node:net'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const watch = process.argv.slice(2)
const check = watch.includes('--check')
const noRun = watch.includes('--no-run')
const command = watch.find((arg) => arg === 'build' || arg === 'dev') ?? 'dev'

const shimDir = process.env.LOCALAPPDATA ? join(process.env.LOCALAPPDATA, 'deepseek-harness', 'bin') : null
const npmDir = process.env.APPDATA ? join(process.env.APPDATA, 'npm') : null
const separator = process.platform === 'win32' ? ';' : ':'
const pnpmBin = npmDir === null ? null : join(npmDir, process.platform === 'win32' ? 'pnpm.cmd' : 'pnpm')
const pnpm = pnpmBin !== null && existsSync(pnpmBin) ? pnpmBin : 'pnpm'

const env = {
  ...process.env,
  PATH: [...(npmDir === null ? [] : [npmDir]), ...(process.env.PATH ?? '').split(separator)]
    .filter((entry) => entry !== '' && entry !== shimDir)
    .join(separator),
  DSH_DESKTOP_HOME: join(root, '.dev-home'),
  DSH_DESKTOP_KERNEL: join(root, 'kernel'),
  DSH_DESKTOP_BRIDGE: join(root, 'bridge'),
  RUST_LOG: 'dsh_desktop_app=debug,dsh_runtime=info,dsh_kernel=warn,info',
}

for (const [name, value] of Object.entries(readEnvFile(join(env.DSH_DESKTOP_HOME, 'provider.env')))) {
  if (!env[name]) env[name] = value
}

const missingKey = !env.OPENCODE_API_KEY && !env.DEEPSEEK_API_KEY
const exe = join(root, 'target', 'debug', process.platform === 'win32' ? 'dsh-desktop-app.exe' : 'dsh-desktop-app')

if (check) {
  console.log(`command      ${command}`)
  console.log(`pnpm         ${pnpm}`)
  console.log(`DSH_HOME     ${env.DSH_DESKTOP_HOME}`)
  console.log(`kernel       ${env.DSH_DESKTOP_KERNEL}${existsSync(env.DSH_DESKTOP_KERNEL) ? '' : '  (missing — pnpm kernel:fetch)'}`)
  console.log(`bridge       ${env.DSH_DESKTOP_BRIDGE}${existsSync(env.DSH_DESKTOP_BRIDGE) ? '' : '  (missing)'}`)
  console.log(`provider key ${missingKey ? '(none — prompts will fail)' : 'loaded'}`)
  process.exit(missingKey ? 1 : 0)
}

if (missingKey) {
  console.warn('[dev] no provider key: set OPENCODE_API_KEY or put it in .dev-home/provider.env')
}

if (command === 'build') {
  step(pnpm, ['--filter', '@dsh-desktop/app', 'build'])
  step(pnpm, ['--filter', '@dsh-desktop/app', 'tauri', 'build'])
  process.exit(0)
}

step('cargo', ['build', '-p', 'dsh-desktop-app'])
if (noRun) {
  console.log(`[dev] built ${exe}`)
  process.exit(0)
}

await settle(exe)

const vite = spawn(pnpm, ['--filter', '@dsh-desktop/app', 'dev'], {
  cwd: root,
  env,
  stdio: 'inherit',
  shell: process.platform === 'win32',
})

await waitForPort(1420, 20_000)

const code = await runApp(exe, { attempts: 3, settleMs: 12_000 })
vite.kill()
process.exit(code)

/**
 * Run the built app, giving each attempt time for the security driver to release
 * the freshly written executable.
 *
 * Exit code 101 is the Tauri panic path for a failed WebView2 setup, which is the
 * symptom a scan lock produces — retrying is correct there and useless anywhere
 * else, so only that code is retried.
 */
async function runApp(path, options) {
  for (let attempt = 1; attempt <= options.attempts; attempt += 1) {
    const child = spawn(path, [], { cwd: root, env, stdio: 'inherit', shell: false })
    const code = await new Promise((resolve) => {
      child.on('exit', (status) => resolve(status ?? 0))
      child.on('error', (error) => {
        console.error(`[dev] could not start the app: ${error.message}`)
        resolve(1)
      })
    })

    if (code !== 101 || attempt === options.attempts) return code
    console.warn(
      `[dev] the app could not create its window (exit 101) — waiting ${options.settleMs / 1000}s ` +
        `for the security scanner to release the new binary, then retrying`,
    )
    await settle(path, options.settleMs)
  }
  return 1
}

/**
 * Wait until nothing holds the file and its size has stopped changing.
 *
 * Both conditions describe "the scanner is done with it": a held handle is the
 * observable form of that scan, and a file still being written is not runnable.
 */
async function settle(path, budgetMs = 20_000) {
  const { holders } = await import('./lib/holders.mjs')
  const started = Date.now()
  let lastSize = -1

  while (Date.now() - started < budgetMs) {
    if (!existsSync(path)) return
    const size = statSync(path).size
    const held = holders(path)
    if (held.length === 0 && size === lastSize) {
      const waited = Date.now() - started
      if (waited > 500) console.log(`[dev] binary settled after ${(waited / 1000).toFixed(1)}s`)
      return
    }
    lastSize = size
    await new Promise((resolve) => setTimeout(resolve, 500))
  }
  console.warn(`[dev] ${path} still busy after ${budgetMs / 1000}s; running anyway`)
}

/**
 * Wait for the Vite dev server to accept connections.
 *
 * In-process socket rather than a probe child: spawning one needs piped stdio,
 * which the harness sandbox denies with EPERM, so the probe would report a false
 * negative while Vite is serving. Both loopback families are tried because Vite
 * binds `localhost`, which resolves to `::1` on this machine.
 */
async function waitForPort(port, budgetMs) {
  const { connect } = await import('node:net')
  const hosts = ['127.0.0.1', '::1']
  const started = Date.now()

  while (Date.now() - started < budgetMs) {
    for (const host of hosts) {
      if (await reachable(port, host)) {
        console.log(`[dev] vite ready on ${host}:${port}`)
        return
      }
    }
    await new Promise((resolve) => setTimeout(resolve, 300))
  }
  console.warn(`[dev] vite did not answer on ${port} within ${budgetMs / 1000}s`)
}

function reachable(port, host) {
  return new Promise((resolve) => {
    const socket = new net.Socket()
    const done = (value) => {
      socket.destroy()
      resolve(value)
    }
    socket.setTimeout(1000)
    socket.once('connect', () => done(true))
    socket.once('error', () => done(false))
    socket.once('timeout', () => done(false))
    socket.connect(port, host)
  })
}

function step(cmd, args) {
  const result = spawnSync(cmd, args, {
    cwd: root,
    env,
    stdio: 'inherit',
    shell: process.platform === 'win32',
  })
  if (result.status !== 0) {
    console.error(`[dev] ${cmd} ${args.join(' ')} failed (${result.status})`)
    process.exit(result.status ?? 1)
  }
}

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

// Keeps `mkdirSync` in the import list honest for future use without a lint
// complaint in the meantime.
void mkdirSync
void rmSync
