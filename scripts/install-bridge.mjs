/**
 * Install `bridge/` where the kernel's module resolution can find it.
 *
 * Node resolves a profile's plugin names by walking up from
 * `$DSH_HOME/profiles/sdk/`, so the kernel home's own `node_modules` is the one
 * location that works — not the fetched kernel's `node_modules`, and not a
 * machine-specific path baked into the committed profile patch.
 *
 * The desktop app does this copy itself at start-up (`AppState::install_bridge`);
 * this script is the same operation for scripted runs and for a quick check.
 *
 * Run: node scripts/install-bridge.mjs [--home <dir>]
 * Default home: ./.dev-home (override with --home or $DSH_HOME)
 */
import { cpSync, existsSync, mkdirSync, readFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const from = join(root, 'bridge')

const args = process.argv.slice(2)
const homeFlag = args.indexOf('--home')
const home = resolve(
  homeFlag === -1
    ? (process.env.DSH_HOME ?? join(root, '.dev-home'))
    : (args[homeFlag + 1] ?? join(root, '.dev-home')),
)

const manifest = readJson(join(from, 'package.json'))
if (typeof manifest.name !== 'string' || manifest.name === '') {
  console.error('[bridge] bridge/package.json has no "name"')
  process.exit(1)
}

if (!existsSync(from)) {
  console.error(`[bridge] missing source directory: ${from}`)
  process.exit(1)
}

const target = join(home, 'node_modules', ...manifest.name.split('/'))
mkdirSync(target, { recursive: true })
cpSync(from, target, { recursive: true, force: true })

const patch = join(root, 'profiles', 'sdk', 'cordis.patch.yml')
if (!readFileSync(patch, 'utf8').includes(`name: '${manifest.name}'`)) {
  console.warn(`[bridge] ${manifest.name} is installed but ${patch} never names it`)
}

console.log(`[bridge] ${manifest.name} -> ${target}`)

function readJson(path) {
  try {
    return JSON.parse(readFileSync(path, 'utf8'))
  } catch (error) {
    console.error(`[bridge] cannot read ${path}: ${error.message}`)
    process.exit(1)
  }
}
