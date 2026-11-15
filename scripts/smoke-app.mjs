/**
 * End-to-end smoke: the real app binary, no window automation.
 *
 * The window cannot be clicked from CI, but almost everything that breaks first is visible
 * without a click: the front end fails to load, the kernel never starts, the session store
 * cannot be read. So this starts the release binary with `DSH_DESKTOP_AUTOSTART=1`, watches
 * its log file, and asserts three things happened:
 *
 *   1. `kernel_paths invoked`  — the bundled front end loaded and called Rust
 *   2. `kernel_initialize ok`  — the kernel process started and completed the handshake
 *   3. `session_list count=N`  — the store was read (N ≥ 0)
 *
 * A window that stays blank never produces line 1; a kernel that cannot boot never produces
 * line 2. Both have shipped before, so both are worth a test.
 *
 *   node scripts/smoke-app.mjs                 # uses the release binary
 *   node scripts/smoke-app.mjs --binary path   # explicit binary
 *   node scripts/smoke-app.mjs --keep          # leave the app running for inspection
 */
import { spawn } from 'node:child_process'
import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const home = join(root, '.smoke-home')
const logDir = join(home, 'logs')

const args = process.argv.slice(2)
const binaryArg = args.indexOf('--binary')
const binary =
  binaryArg === -1
    ? join(root, 'target', 'release', process.platform === 'win32' ? 'dsh-desktop-app.exe' : 'dsh-desktop-app')
    : args[binaryArg + 1]
const keep = args.includes('--keep')
const budgetMs = Number(process.env.SMOKE_MS ?? 60_000)

if (!existsSync(binary)) {
  console.error(`[smoke] no binary at ${binary} — run \`cargo build --release\` (or \`pnpm build:app\`) first`)
  process.exit(1)
}

// A private home per run: a stale session list would make "the store works" unfalsifiable.
rmSync(home, { recursive: true, force: true })
mkdirSync(logDir, { recursive: true })
// The front end needs a key to complete a turn; the assertion below only needs the handshake,
// so an empty provider.env is enough. This keeps the smoke test offline and free.
writeFileSync(join(home, 'provider.env'), '')

const child = spawn(binary, [], {
  cwd: root,
  env: {
    ...process.env,
    DSH_DESKTOP_HOME: home,
    DSH_DESKTOP_AUTOSTART: '1',
    // The file log is the assertion channel: stdout is not reliably attached in CI.
    RUST_LOG: 'dsh_desktop_app=debug,info',
  },
  stdio: ['ignore', 'pipe', 'pipe'],
})

const seen = { paths: false, initialized: false, sessions: null }
let output = ''

child.stdout.on('data', (chunk) => (output += chunk.toString()))
child.stderr.on('data', (chunk) => (output += chunk.toString()))

/**
 * The newest `desktop.log*` in the log directory.
 *
 * The rolling appender appends the date (`desktop.log.2026-11-15`), so an exact filename never
 * matches — which once made this script report "no log" while the log sat right there.
 */
function newestLog() {
  if (!existsSync(logDir)) return null
  const candidates = readdirSync(logDir)
    .filter((name) => name.startsWith('desktop.log'))
    .map((name) => join(logDir, name))
  if (candidates.length === 0) return null
  return candidates.sort((left, right) => statSync(right).mtimeMs - statSync(left).mtimeMs)[0]
}
const started = Date.now()
let timer

const done = (code, message) => {
  clearInterval(timer)
  if (!keep) child.kill()
  if (code === 0) {
    console.log(`[smoke] ok — ${message}`)
  } else {
    console.error(`[smoke] FAILED — ${message}`)
    const logPath = newestLog()
    if (logPath !== null) {
      console.error(`[smoke] last lines of ${logPath}:`)
      console.error(readFileSync(logPath, 'utf8').split('\n').slice(-25).join('\n'))
    } else if (output !== '') {
      console.error('[smoke] process output:')
      console.error(output.split('\n').slice(-25).join('\n'))
    }
  }
  process.exit(code)
}

timer = setInterval(() => {
  const logPath = newestLog()
  const log = logPath === null ? '' : readFileSync(logPath, 'utf8')
  // The file log is the primary source; stdout covers the case where writing it failed.
  const haystack = log === '' ? output : log

  if (!seen.paths && haystack.includes('kernel_paths invoked')) seen.paths = true
  if (!seen.initialized && haystack.includes('kernel_initialize ok')) seen.initialized = true
  const sessions = /session_list count=(\d+)/.exec(haystack)
  if (sessions !== null) seen.sessions = Number(sessions[1])

  if (seen.paths && seen.initialized && seen.sessions !== null) {
    done(0, `front end loaded, kernel handshook, ${seen.sessions} session(s) read in ${Date.now() - started}ms`)
  }
  if (Date.now() - started > budgetMs) {
    const missing = Object.entries(seen)
      .filter(([, value]) => value === false || value === null)
      .map(([name]) => name)
    done(1, `timed out after ${budgetMs}ms; never saw: ${missing.join(', ')}`)
  }
}, 500)

child.on('exit', (code) => {
  if (!keep) done(1, `the app exited early with ${String(code)}`)
})
