/**
 * Who is holding this path, and (optionally) kill them.
 *
 * Usage:
 *   node scripts/handles.mjs <path>                    # report
 *   node scripts/handles.mjs <path> --kill             # terminate the holders
 *   node scripts/handles.mjs <path> --kill --only tailscale-ipn,node
 *
 * See docs/句柄占用.md for why the three different Windows error texts
 * ("access denied", `0x800700AA`, "file in use") all mean this one thing.
 */
import { existsSync } from 'node:fs'
import { holders, killHolders } from './lib/holders.mjs'

const args = process.argv.slice(2)
const target = args.find((arg) => !arg.startsWith('--'))
const kill = args.includes('--kill')
const onlyAt = args.indexOf('--only')
const only = onlyAt === -1
  ? null
  : (args[onlyAt + 1] ?? '').split(',').map((name) => name.trim().toLowerCase()).filter(Boolean)

if (target === undefined) {
  console.error('usage: node scripts/handles.mjs <path> [--kill] [--only name,name]')
  process.exit(2)
}
if (!existsSync(target)) {
  console.error(`[handles] path does not exist: ${target}`)
  process.exit(2)
}

const found = holders(target)

if (found.length === 0) {
  console.log(`[handles] nothing is holding ${target}`)
  process.exit(0)
}

console.log(`[handles] ${found.length} process(es) hold ${target}`)
for (const holder of found) {
  console.log(`  pid ${holder.pid}  ${holder.name}  ${holder.app}`)
}

if (!kill) {
  console.log('\nre-run with --kill to terminate them')
  process.exit(0)
}

const targets = only === null
  ? found
  : found.filter((holder) => only.includes(holder.name.toLowerCase()))

const skipped = found.length - targets.length
if (skipped > 0) console.log(`[handles] skipping ${skipped} not in --only`)

const killed = await killHolders(targets)
for (const holder of killed) console.log(`  killed ${holder.pid} ${holder.name}`)
for (const holder of targets.filter((entry) => !killed.includes(entry))) {
  console.error(`  could not kill ${holder.pid} ${holder.name}`)
}

console.log('[handles] done')
