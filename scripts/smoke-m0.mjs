/**
 * M0 smoke test: does the pinned kernel answer a real SDK handshake?
 *
 * Spawns `dsh --profile sdk`, sends `initialize`, prints the reply, then
 * shuts the runtime down. No model request is made, so it needs no API key.
 *
 * Run: node scripts/smoke-m0.mjs
 * Exit 0 = the wire works; non-zero = read the kernel's stderr below.
 */
import { spawn } from 'node:child_process'
import { createInterface } from 'node:readline'
import { existsSync, mkdirSync, rmSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const entry = join(root, 'kernel', 'node_modules', '@deepseek-ai', 'dsh', 'lib', 'bin.js')
const home = join(root, '.smoke-home')
const timeoutMs = 30_000

if (!existsSync(entry)) {
  fail(`kernel not fetched — run \`pnpm kernel:fetch\` first (looked for ${entry})`)
}

// A throwaway home keeps the smoke test away from any real installation.
rmSync(home, { recursive: true, force: true })
mkdirSync(home, { recursive: true })

const child = spawn(process.execPath, [entry, '--profile', 'sdk'], {
  cwd: root,
  env: { ...process.env, DSH_HOME: home, DSH_TELEMETRY_DISABLED: '1' },
  stdio: ['pipe', 'pipe', 'pipe'],
})

const stderrLines = []
createInterface({ input: child.stderr }).on('line', (line) => stderrLines.push(line))

let settled = false
const timer = setTimeout(() => fail(`no initialize response within ${timeoutMs}ms`), timeoutMs)

const stdout = createInterface({ input: child.stdout })
stdout.on('line', (line) => {
  let message
  try {
    message = JSON.parse(line)
  } catch {
    // A non-JSON line on stdout is a protocol violation; surface it.
    console.error(`[smoke] non-JSON stdout: ${line}`)
    return
  }
  if (message.id !== 1) return

  clearTimeout(timer)
  if (message.error) {
    fail(`initialize rejected: [${message.error.code}] ${message.error.message}`)
  }
  const server = message.result?.serverInfo
  console.log(`[smoke] ok — serverInfo: ${JSON.stringify(server)}`)
  send({ jsonrpc: '2.0', id: 2, method: 'shutdown' })
  settled = true
  // Give the runtime a moment to flush the shutdown reply and exit.
  setTimeout(() => {
    child.kill()
    rmSync(home, { recursive: true, force: true })
    console.log('[smoke] M0 handshake verified')
    process.exit(0)
  }, 500)
})

child.on('exit', (code) => {
  if (settled) return
  clearTimeout(timer)
  if (code !== 0) {
    console.error('[smoke] kernel exited early, stderr tail:')
    for (const line of stderrLines.slice(-15)) console.error(`  ${line}`)
    fail(`kernel exited with code ${code}`)
  }
})

send({
  jsonrpc: '2.0',
  id: 1,
  method: 'initialize',
  params: {
    cwd: root,
    provider: 'deepseek-official',
    model: 'deepseek-v4.1-flash',
    approvalRequests: true,
    capabilities: { userQuestions: true },
  },
})

function send(message) {
  child.stdin.write(`${JSON.stringify(message)}\n`)
}

function fail(message) {
  clearTimeout(timer)
  child.kill()
  console.error(`[smoke] ${message}`)
  process.exit(1)
}
