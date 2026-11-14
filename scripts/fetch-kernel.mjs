/**
 * Fetch the pinned DeepSeek Harness kernel into `kernel/`.
 *
 * The kernel is not vendored: this script installs an exact `@deepseek-ai/dsh`
 * version into a staging directory, verifies the resolved manifest version,
 * then renames it into place. A failure leaves the previous kernel untouched.
 */
import { execFileSync } from 'node:child_process'
import { existsSync, mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const pinPath = join(root, 'profiles', 'sdk', 'kernel.json')
const stagingDir = join(root, '.kernel-staging')
const kernelDir = join(root, 'kernel')

const pin = readJson(pinPath)
if (typeof pin.version !== 'string' || pin.version === '') {
  fail(`kernel.json has no "version": ${pinPath}`)
}
if (typeof pin.package !== 'string' || pin.package === '') {
  fail(`kernel.json has no "package": ${pinPath}`)
}

const kernelPackageDir = join(stagingDir, 'node_modules', ...pin.package.split('/'))

console.log(`[kernel] pin: ${pin.package}@${pin.version}`)

rmSync(stagingDir, { recursive: true, force: true })
mkdirSync(stagingDir, { recursive: true })
writeJson(join(stagingDir, 'package.json'), {
  name: 'dsh-kernel-staging',
  private: true,
  type: 'module',
})

try {
  execFileSync('pnpm', ['add', `${pin.package}@${pin.version}`, '--ignore-scripts'], {
    cwd: stagingDir,
    stdio: 'inherit',
    shell: process.platform === 'win32',
  })
} catch {
  rmSync(stagingDir, { recursive: true, force: true })
  fail('pnpm add failed — check network and registry access')
}

const installedManifest = join(kernelPackageDir, 'package.json')
if (!existsSync(installedManifest)) {
  fail(`installed manifest missing: ${installedManifest}`)
}

const installed = readJson(installedManifest)
if (installed.version !== pin.version) {
  fail(`version mismatch: pinned ${pin.version}, installed ${String(installed.version)}`)
}

rmSync(kernelDir, { recursive: true, force: true })
renameSync(stagingDir, kernelDir)

const bin = typeof installed.bin === 'object' && installed.bin !== null
  ? installed.bin.dsh
  : installed.bin

console.log(`[kernel] ready at ${kernelDir}`)
console.log(`[kernel] entry: ${join(kernelDir, 'node_modules', ...pin.package.split('/'), typeof bin === 'string' ? bin : 'lib/bin.js')}`)

function readJson(path) {
  let raw
  try {
    raw = readFileSync(path, 'utf8')
  } catch (error) {
    fail(`cannot read ${path}: ${error.message}`)
  }
  try {
    return JSON.parse(raw)
  } catch (error) {
    fail(`invalid JSON in ${path}: ${error.message}`)
  }
}

function writeJson(path, value) {
  mkdirSync(dirname(path), { recursive: true })
  writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`)
}

function fail(message) {
  console.error(`[kernel] ${message}`)
  process.exit(1)
}
