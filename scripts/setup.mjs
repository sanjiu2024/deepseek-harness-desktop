/**
 * One-shot developer setup: verify the toolchain, install JS dependencies,
 * fetch the kernel, and report what is still missing.
 *
 * Idempotent. Safe to run repeatedly.
 */
import { execFileSync } from 'node:child_process'
import { existsSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const problems = []

step('toolchain', () => {
  requireVersion('node', 'node', '--version', 24, 9)
  requireVersion('pnpm', 'pnpm', '--version', 12, 0)
  requireVersion('rustc', 'rustc', '--version', 1, 100)
  requireVersion('cargo', 'cargo', '--version', 1, 0)
})

step('dependencies', () => {
  run('pnpm', ['install'])
})

step('kernel', () => {
  if (existsSync(join(root, 'kernel', 'node_modules'))) {
    console.log('  kernel already present, skipping (delete kernel/ to refetch)')
    return
  }
  run('node', ['scripts/fetch-kernel.mjs'])
})

if (problems.length > 0) {
  console.error('\nsetup finished with problems:')
  for (const problem of problems) console.error(`  - ${problem}`)
  process.exit(1)
}

console.log('\nsetup complete — run `pnpm tauri dev`')

function step(name, body) {
  console.log(`\n=== ${name} ===`)
  try {
    body()
  } catch (error) {
    problems.push(`${name}: ${error.message}`)
    console.error(`  failed: ${error.message}`)
  }
}

function run(command, args) {
  execFileSync(command, args, { cwd: root, stdio: 'inherit', shell: process.platform === 'win32' })
}

function requireVersion(label, command, flag, major, minor) {
  let raw
  try {
    raw = execFileSync(command, [flag], { encoding: 'utf8', shell: process.platform === 'win32' }).trim()
  } catch {
    throw new Error(`${label} not found on PATH`)
  }
  const match = /(\d+)\.(\d+)/.exec(raw)
  if (match === null) throw new Error(`${label}: unrecognized version "${raw}"`)
  const [, gotMajor, gotMinor] = match.map(Number)
  if (gotMajor < major || (gotMajor === major && gotMinor < minor)) {
    throw new Error(`${label} ${gotMajor}.${gotMinor} is older than required ${major}.${minor} (${raw})`)
  }
  console.log(`  ${label} ok (${raw})`)
}
