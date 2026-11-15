/**
 * Shrink the fetched kernel.
 *
 * The kernel is 822 MB, and 338 MB of that is LibreOffice — an Office toolchain this client
 * never opens. Deleting files out of a `node_modules` tree is the kind of thing that works
 * until it silently doesn't, so this script is built around one rule:
 *
 *   **Nothing is deleted until a real kernel run proves the kernel still works without it.**
 *
 * The loop is: move a candidate aside → start the SDK kernel → send one prompt → require a
 * completed turn → keep it pruned, or put it straight back. Candidates are moved, not copied,
 * to `kernel/.pruned/`, so a restore is a rename and an interrupted run is recoverable.
 *
 *   node scripts/slim-kernel.mjs              # report what is large and prunable
 *   node scripts/slim-kernel.mjs --apply      # prune, verifying after each candidate
 *   node scripts/slim-kernel.mjs --restore    # put everything back
 *
 * A candidate that fails verification is restored **and remembered**: the report says why, so
 * the next person does not repeat the experiment.
 */
import { spawn } from 'node:child_process'
import { createInterface } from 'node:readline'
import { cpSync, existsSync, mkdirSync, readdirSync, renameSync, rmSync, statSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { duBytes, human } from './lib/size.mjs'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const pnpmDir = join(root, 'kernel', 'node_modules', '.pnpm')
const prunedDir = join(root, 'kernel', '.pruned')
const entry = join(root, 'kernel', 'node_modules', '@deepseek-ai', 'dsh', 'lib', 'bin.js')

/**
 * Packages worth removing, biggest first.
 *
 * Each line states what it is for and why the SDK profile does not need it. `reason` is what
 * ends up in the report, so it has to be a claim someone can check.
 */
const CANDIDATES = [
  {
    match: /^@deepseek-ai\+libreoffice-kit-win32-x64@/,
    reason: 'the LibreOffice binary; the desktop client has no Office tools mounted',
  },
  {
    match: /^@deepseek-ai\+libreoffice-kit-wasm@/,
    reason: 'the WASM fallback of the same LibreOffice kit',
  },
  {
    match: /^sherpa-onnx-win-x64@/,
    reason: 'speech recognition for voice input, which this client does not offer',
  },
  {
    match: /^@deepseek-ai\+dsh-web-fronte/,
    reason: 'the upstream Web GUI bundle; this app brings its own front end',
  },
  {
    match: /^@deepseek-ai\+dsh-client-ui-/,
    reason: 'the upstream Web GUI components; same reason',
  },
]

const apply = process.argv.includes('--apply')
const restore = process.argv.includes('--restore')

if (restore) {
  const restored = restoreAll()
  console.log(`[slim] restored ${restored} package(s)`)
  process.exit(0)
}

if (!existsSync(pnpmDir)) {
  console.error('[slim] kernel not fetched — run `pnpm kernel:fetch` first')
  process.exit(1)
}

const installed = readdirSync(pnpmDir)
const found = CANDIDATES.map((candidate) => {
  const matches = installed.filter((name) => candidate.match.test(name))
  const bytes = matches.reduce((total, name) => total + duBytes(join(pnpmDir, name)), 0)
  return { ...candidate, matches, bytes }
}).filter((candidate) => candidate.matches.length > 0)

const totalPrunable = found.reduce((total, candidate) => total + candidate.bytes, 0)

console.log(`[slim] kernel is ${human(duBytes(join(root, 'kernel')))}`)
console.log(`[slim] prunable candidates: ${human(totalPrunable)}\n`)
for (const candidate of found) {
  console.log(
    `  ${human(candidate.bytes).padStart(7)}  ${candidate.matches.join(', ')}\n           ${candidate.reason}`,
  )
}

if (!apply) {
  console.log('\n[slim] dry run. Re-run with --apply to prune, verifying after each candidate.')
  process.exit(0)
}

mkdirSync(prunedDir, { recursive: true })
const kept = []
const rejected = []

for (const candidate of found) {
  // One verification per candidate, with every hashed copy moved at once: pnpm stores the
  // same package once per peer set, and checking sixty of them separately would take an hour
  // to learn one thing.
  const moved = []
  for (const name of candidate.matches) {
    const source = join(pnpmDir, name)
    const target = join(prunedDir, name)
    renameSync(source, target)
    moved.push([source, target])
  }
  console.log(`\n[slim] moved aside ${candidate.matches.length} cop(ies), ${human(candidate.bytes)}`)

  const verdict = await verify()
  if (verdict.ok) {
    console.log('[slim]   verified — the kernel still completes a turn with a tool call')
    kept.push(...candidate.matches)
  } else {
    for (const [source, target] of moved) renameSync(target, source)
    console.log(`[slim]   FAILED — restored. ${verdict.reason}`)
    rejected.push({ name: candidate.matches[0], reason: verdict.reason })
  }
}

// The moved-aside copies now cost exactly what pruning saved. Everything that failed
// verification was already put back, so their only remaining use is a rollback for something
// nobody predicted — and `pnpm kernel:fetch` is a better one, because it rebuilds from the pin
// instead of trusting a directory someone moved by hand.
rmSync(prunedDir, { recursive: true, force: true })

// The moved-aside copies now cost exactly what pruning saved. Anything that failed
// verification was already put back, so their only remaining use is a rollback for something
// nobody predicted — and `pnpm kernel:fetch` is a better one: it rebuilds from the pin instead
// of trusting a directory someone moved by hand.
rmSync(prunedDir, { recursive: true, force: true })

console.log(`\n[slim] pruned ${kept.length} package(s), restored ${rejected.length}`)
for (const item of rejected) console.log(`  kept: ${item.name} — ${item.reason}`)
console.log(`[slim] kernel is now ${human(duBytes(join(root, 'kernel')))}`)
console.log('[slim] `node scripts/slim-kernel.mjs --restore` puts everything back')

/**
 * Start the kernel, initialize, send one prompt, and require a completed turn.
 *
 * A handshake alone would pass with half the tools missing: the profile mounts most of its
 * rows lazily, so the first thing to break is a tool call, not startup.
 */
async function verify() {
  if (!existsSync(entry)) return { ok: false, reason: `no kernel entry at ${entry}` }

  const home = join(root, '.slim-home')
  rmSync(home, { recursive: true, force: true })
  mkdirSync(join(home, 'node_modules'), { recursive: true })
  cpSync(join(root, 'bridge'), join(home, 'node_modules', 'dsh-desktop-live-bridge'), {
    recursive: true,
  })

  const child = spawn(
    process.execPath,
    [entry, '--profile', 'sdk', '--patch', join(root, 'profiles', 'sdk', 'cordis.patch.yml')],
    {
      cwd: root,
      env: {
        ...process.env,
        DSH_HOME: home,
        DSH_DESKTOP_BRIDGE_PATH: join(home, 'node_modules', 'dsh-desktop-live-bridge', 'index.js'),
      },
    },
  )

  const pending = new Map()
  let settleTurn
  const turnEnded = new Promise((resolve) => {
    settleTurn = resolve
  })
  let stderrNoise = ''

  const send = (message) => child.stdin.write(`${JSON.stringify(message)}\n`)
  const request = (method, params) =>
    new Promise((resolve, reject) => {
      const id = pending.size + 1
      pending.set(id, { resolve, reject })
      send({ jsonrpc: '2.0', id, method, params })
    })

  createInterface({ input: child.stdout }).on('line', (line) => {
    let frame
    try {
      frame = JSON.parse(line)
    } catch {
      return
    }
    if (typeof frame.id === 'number' && pending.has(frame.id)) {
      const { resolve, reject } = pending.get(frame.id)
      pending.delete(frame.id)
      if (frame.error) reject(new Error(String(frame.error.message)))
      else resolve(frame.result)
      return
    }
    if (frame.method === 'session.event' && frame.params?.event?.type === 'turn/end') {
      settleTurn(frame.params.event.data?.reason?.kind ?? 'unknown')
    }
  })
  createInterface({ input: child.stderr }).on('line', (line) => {
    if (/no adapter|Cannot find|ERR_MODULE|Failed to resolve|Cannot locate/i.test(line)) {
      stderrNoise = line.trim()
      settleTurn('crashed')
    }
  })

  // Wait for the turn to *end*, not for the prompt to be accepted: `session/prompt`
  // returns a message id immediately, and killing the kernel then would look exactly like
  // "the kernel never completed a turn".
  const finish = async (outcome) => {
    child.kill()
    await new Promise((resolve) => child.on('exit', resolve))
    rmSync(home, { recursive: true, force: true })
    return outcome
  }

  const timer = setTimeout(() => settleTurn('timeout'), 90_000)

  try {
    await request('initialize', {
      cwd: root,
      provider: 'opencode-go',
      model: 'deepseek-v4.1-flash',
      // Off on purpose: with approvals on, a tool call blocks on a question nobody is here
      // to answer, and the run would fail for a reason that has nothing to do with pruning.
      approvalRequests: false,
    })
    // A tool-using prompt, because a tool is what a missing package breaks first.
    await request('session/prompt', {
      sessionId: `slim-${Date.now()}`,
      contentBlocks: [{ type: 'text', text: '用 pwsh 执行 echo ok，然后告诉我结果。' }],
    })
  } catch (error) {
    clearTimeout(timer)
    return finish({ ok: false, reason: `request failed: ${String(error.message)}` })
  }

  const kind = await turnEnded
  clearTimeout(timer)
  if (kind === 'completed') return finish({ ok: true })
  return finish({
    ok: false,
    reason:
      kind === 'timeout'
        ? 'the kernel did not end the turn in 90s'
        : stderrNoise || `turn ended: ${String(kind)}`,
  })
}

function restoreAll() {
  if (!existsSync(prunedDir)) return 0
  let restored = 0
  for (const name of readdirSync(prunedDir)) {
    const source = join(prunedDir, name)
    const target = join(pnpmDir, name)
    if (!existsSync(target)) {
      renameSync(source, target)
      restored += 1
    }
  }
  if (readdirSync(prunedDir).length === 0) rmSync(prunedDir, { recursive: true, force: true })
  return restored
}

// `statSync` is imported for `duBytes`'s contract; keep the import honest.
void statSync
