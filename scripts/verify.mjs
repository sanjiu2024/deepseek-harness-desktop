/**
 * Everything that has to be true before this repository is worth handing over, in one command.
 *
 * The point is not convenience. Each of these checks has caught a real bug that a person would
 * not have noticed — a release build with no front end inside, a test that depended on the wall
 * clock, a clippy warning that only appears on Linux, a kernel that refused data we wrote. A check
 * that lives in a chat message is a check nobody runs; a check behind one command is a habit.
 *
 *   node scripts/verify.mjs             # everything that can run here
 *   node scripts/verify.mjs --fast       # skip the slow ones (build, smoke)
 *   node scripts/verify.mjs --only=rust  # one group
 *
 * Exit code is 0 only when nothing failed. Skips are reported as skips, never as passes: a
 * verification that quietly passes because it did not run is worse than no verification.
 */
import { spawn } from 'node:child_process'
import { existsSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const fast = process.argv.includes('--fast')
const only = process.argv.find((arg) => arg.startsWith('--only='))?.slice('--only='.length)

/**
 * Quote one argument so the single command string survives a shell.
 *
 * A string plus `shell: true` is the portable way to run `pnpm` and `cargo` on Windows (pnpm is a
 * `.cmd` shim there), but Node deprecated passing an args array alongside a shell for exactly the
 * reason it looks wrong: nothing is escaped. Doing it here, deliberately, keeps the deprecation
 * out and the quoting visible.
 */
function quote(argument) {
  return /[\s"']/.test(argument) ? `"${argument.replaceAll('"', '\\"')}"` : argument
}

/** Run one command and capture its tail. Never throws: a failure is a result here. */
function run(command, args, options = {}) {
  return new Promise((resolve) => {
    const started = Date.now()
    // Windows consoles default to a legacy code page, so a Chinese error message from a child
    // arrives as mojibake. Switching the console to UTF-8 first keeps the reason readable — the
    // whole point of printing it.
    const prefix = process.platform === 'win32' ? 'chcp 65001 >nul & ' : ''
    const line = prefix + [command, ...args].map(quote).join(' ')
    const child = spawn(line, {
      cwd: root,
      // The kernel tests read these; passing them through is what makes a local run mean anything.
      env: process.env,
      stdio: ['ignore', 'pipe', 'pipe'],
      shell: true,
      ...options,
    })
    let output = ''
    const collect = (chunk) => {
      output += chunk.toString()
      // Keep the tail only: a full cargo build log is megabytes and the head is never the part
      // that explains a failure.
      if (output.length > 40_000) output = output.slice(-20_000)
    }
    child.stdout.on('data', collect)
    child.stderr.on('data', collect)
    child.on('error', (error) => {
      resolve({ code: -1, output: `${output}\n${error.message}`, durationMs: Date.now() - started })
    })
    child.on('close', (code) => {
      resolve({ code: code ?? -1, output, durationMs: Date.now() - started })
    })
  })
}

/** `CARGO` lets a machine point at a toolchain that is not on PATH — and lets the failure path
 * of this script be tested without breaking anything. */
const cargo = process.env.CARGO ?? 'cargo'

const kernelPresent = existsSync(join(root, 'kernel', 'node_modules', '@deepseek-ai', 'dsh'))
const homePresent = existsSync(join(root, '.dev-home'))

/** Each step: what it proves, how to run it by hand, and when it can be skipped. */
const steps = [
  {
    group: 'rust',
    name: 'rustfmt',
    why: 'formatting is enforced in CI; a clean tree here means the diff is code, not whitespace',
    command: [cargo, ['fmt', '--all', '--check']],
  },
  {
    group: 'rust',
    name: 'clippy',
    why: 'warnings are errors; this is the check that caught the Linux-only unused import',
    command: [cargo, ['clippy', '--workspace', '--all-targets', '--release', '--', '-D', 'warnings']],
  },
  {
    group: 'rust',
    name: 'cargo test',
    why: 'the unit and integration suites across all four crates',
    command: [cargo, ['test', '--workspace', '--release']],
  },
  {
    group: 'kernel',
    name: 'kernel round trips',
    why: 'fork, image blocks and restart are only true if the kernel says so',
    command: [
      'cargo',
      [
        'test',
        '-p',
        'dsh-runtime',
        '--release',
        '--test',
        'fork_resume',
        '--test',
        'prompt_blocks',
        '--test',
        'restart',
      ],
    ],
    skip: kernelPresent && homePresent ? null : 'needs kernel/ and .dev-home/ (run the app once)',
  },
  {
    group: 'front',
    name: 'typecheck',
    why: 'strict TypeScript, no `any`',
    command: ['pnpm', ['--filter', '@dsh-desktop/app', 'check']],
  },
  {
    group: 'front',
    name: 'front-end tests',
    why: 'the reducer against real captured events, plus every panel',
    command: ['pnpm', ['--filter', '@dsh-desktop/app', 'test']],
  },
  {
    group: 'front',
    name: 'production build',
    why: 'a build that fails here fails an installer later, when it is much more expensive',
    command: ['pnpm', ['--filter', '@dsh-desktop/app', 'build']],
    slow: true,
  },
  {
    group: 'smoke',
    name: 'headless app smoke',
    why: 'the real binary: front end embedded, kernel handshook, store read — asserted from the log',
    command: ['node', ['scripts/smoke-app.mjs']],
    slow: true,
    skip: kernelPresent ? null : 'needs kernel/ (pnpm kernel:fetch)',
  },
]

const selected = steps.filter((step) => (only ? step.group === only : true))
const results = []

console.log(`[verify] ${selected.length} check(s)${fast ? ', fast mode (slow ones skipped)' : ''}`)
for (const step of selected) {
  const label = `${step.group}/${step.name}`
  if (step.skip) {
    results.push({ label, status: 'skip', detail: step.skip, durationMs: 0 })
    console.log(`[verify] ${label.padEnd(28)} skip — ${step.skip}`)
    continue
  }
  if (fast && step.slow) {
    results.push({ label, status: 'skip', detail: 'fast mode', durationMs: 0 })
    console.log(`[verify] ${label.padEnd(28)} skip — fast mode`)
    continue
  }
  process.stdout.write(`[verify] ${label.padEnd(28)} …`)
  const [command, args] = step.command
  const { code, output, durationMs } = await run(command, args)
  const status = code === 0 ? 'pass' : 'fail'
  console.log(`\r[verify] ${label.padEnd(28)} ${status}  ${(durationMs / 1000).toFixed(1)}s`)
  results.push({ label, status, output, detail: step.why, durationMs, command: `${command} ${args.join(' ')}` })
}

const failed = results.filter((result) => result.status === 'fail')
const skipped = results.filter((result) => result.status === 'skip')
const passed = results.filter((result) => result.status === 'pass')

if (failed.length > 0) {
  console.log('\n[verify] failures — with the command that reproduces each:\n')
  for (const result of failed) {
    console.log(`  ${result.label}`)
    console.log(`    ${result.command}`)
    const tail = result.output.trim().split('\n').slice(-15).join('\n    ')
    console.log(`    ${tail}\n`)
  }
}

console.log(
  `\n[verify] ${passed.length} passed · ${failed.length} failed · ${skipped.length} skipped`,
)
if (skipped.length > 0) {
  console.log('[verify] skipped, not verified:')
  for (const result of skipped) console.log(`  - ${result.label}: ${result.detail}`)
}
process.exit(failed.length === 0 ? 0 : 1)
