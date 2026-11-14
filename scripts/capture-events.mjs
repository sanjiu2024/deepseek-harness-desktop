/**
 * Capture real `session.event` payloads into a fixture file.
 *
 * The reducer's field names (`delta`, `callId`, `content`, …) have to match what
 * the kernel actually sends. Reading the protocol docs is not proof, so this
 * records the truth: it starts the kernel, sends one prompt, and writes every
 * event to `apps/desktop/src/lib/__fixtures__/events-<stamp>.json`.
 *
 * Works without an API key: the prompt is accepted and the user event is
 * committed, then the model call fails and you get the failure events too —
 * which is exactly what the UI has to render.
 *
 * Run: node scripts/capture-events.mjs
 */
import { spawn } from 'node:child_process'
import { createInterface } from 'node:readline'
import { existsSync, mkdirSync, rmSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const entry = join(root, 'kernel', 'node_modules', '@deepseek-ai', 'dsh', 'lib', 'bin.js')
const home = join(root, '.capture-home')
const fixtureDir = join(root, 'apps', 'desktop', 'src', 'lib', '__fixtures__')
const captureMs = Number(process.env.CAPTURE_MS ?? 20_000)
const sessionId = `capture-${Date.now()}`
const prompt = process.env.CAPTURE_PROMPT ?? '用一句话说明你是什么模型。'

if (!existsSync(entry)) {
  console.error(`[capture] kernel not fetched — run \`pnpm kernel:fetch\` (looked for ${entry})`)
  process.exit(1)
}

rmSync(home, { recursive: true, force: true })
mkdirSync(home, { recursive: true })
mkdirSync(fixtureDir, { recursive: true })

const patch = join(root, 'profiles', 'sdk', 'cordis.patch.yml')

const child = spawn(process.execPath, [entry, '--profile', 'sdk', '--patch', patch], {
  cwd: root,
  env: { ...process.env, DSH_HOME: home, DSH_TELEMETRY_DISABLED: '1' },
  stdio: ['pipe', 'pipe', 'pipe'],
})

const events = []
const stderrTail = []
createInterface({ input: child.stderr }).on('line', (line) => {
  stderrTail.push(line)
  if (stderrTail.length > 60) stderrTail.shift()
})

const pending = new Map()

createInterface({ input: child.stdout }).on('line', (line) => {
  let message
  try {
    message = JSON.parse(line)
  } catch {
    console.log(`[capture] non-JSON stdout: ${line}`)
    return
  }
  if (message.method === 'session.event') {
    events.push(message.params)
    const event = message.params?.event
    console.log(`[capture] ${String(event?.type).padEnd(20)} seq=${event?.seq}`)
    return
  }
  if (message.method !== undefined) {
    console.log(`[capture] notification ${message.method}`)
    return
  }
  const resolve = pending.get(message.id)
  if (resolve !== undefined) {
    pending.delete(message.id)
    resolve(message)
  }
})

function call(method, params) {
  const id = nextId++
  return new Promise((resolve) => {
    pending.set(id, resolve)
    child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id, method, params })}\n`)
  })
}
let nextId = 1

const initialized = await call('initialize', {
  cwd: root,
  provider: process.env.CAPTURE_PROVIDER ?? 'deepseek-official',
  model: process.env.CAPTURE_MODEL ?? 'deepseek-v4.1-flash',
  approvalRequests: true,
  capabilities: { userQuestions: true },
})
if (initialized.error) {
  console.error(`[capture] initialize failed: ${JSON.stringify(initialized.error)}`)
  finish(1)
}
console.log('[capture] handshake ok, sending one prompt')

const queued = await call('session/prompt', {
  sessionId,
  contentBlocks: [{ type: 'text', text: prompt }],
})
if (queued.error) console.error(`[capture] prompt rejected: ${JSON.stringify(queued.error)}`)
else console.log(`[capture] queued as ${JSON.stringify(queued.result)}`)

await new Promise((resolve) => setTimeout(resolve, captureMs))
finish(0)

function finish(code) {
  child.kill()
  rmSync(home, { recursive: true, force: true })

  const stamp = new Date().toISOString().replaceAll(':', '-').slice(0, 19)
  const file = join(fixtureDir, `events-${stamp}.json`)
  writeFileSync(
    file,
    `${JSON.stringify({ sessionId, prompt, capturedAt: new Date().toISOString(), events }, null, 2)}\n`,
  )
  console.log(`\n[capture] ${events.length} events -> ${file}`)
  if (events.length === 0) {
    console.error('[capture] nothing captured; kernel stderr tail:')
    for (const line of stderrTail.slice(-15)) console.error(`  ${line}`)
  }
  process.exit(code === 0 && events.length === 0 ? 1 : code)
}
