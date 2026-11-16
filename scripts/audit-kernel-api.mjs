/**
 * What the kernel offers, and what this client actually uses.
 *
 * The table in `docs/接口.md` used to be maintained by hand, which is exactly the kind of table
 * that silently goes wrong: a method is added upstream, or a client-side handler is deleted, and
 * nobody notices because nothing recomputes the claim. This script recomputes it from source:
 *
 *   kernel SDK        → method names from the JSON-RPC dispatch switch, plus the notifications and
 *                       requests it sends back to us
 *   our Rust          → command names, and method strings the client sends
 *   our front end     → reducer event cases, and typed helpers in `ipc.ts`
 *
 *   node scripts/audit-kernel-api.mjs             # human table
 *   node scripts/audit-kernel-api.mjs --markdown  # paste-able section for docs/接口.md
 *
 * Exit code is always 0: this is an inventory, not a gate. The gate for behaviour is
 * `pnpm verify`.
 */
import { execFileSync } from 'node:child_process'
import { existsSync, readFileSync, readdirSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const markdown = process.argv.includes('--markdown')

/** The SDK server package inside the fetched kernel, whatever its pnpm hash is. */
function sdkDir() {
  const store = join(root, 'kernel', 'node_modules', '.pnpm')
  if (!existsSync(store)) return null
  const entry = readdirSync(store).find((name) => name.startsWith('@deepseek-ai+dsh-sdk-app@'))
  if (!entry) return null
  const lib = join(store, entry, 'node_modules', '@deepseek-ai', 'dsh-sdk-jsonrpc-server', 'lib')
  return existsSync(lib) ? lib : null
}

function read(path) {
  return existsSync(path) ? readFileSync(path, 'utf8') : ''
}

const unique = (items) => [...new Set(items)].sort()

/** Method names the kernel dispatches, from `switch (method)` cases. */
function kernelMethods(source) {
  return unique([...source.matchAll(/case "([a-z][a-z\/.-]+)":/g)].map((match) => match[1]))
}

/** What the kernel sends to us without being asked. */
function kernelInbound(source) {
  const notifications = [...source.matchAll(/transport\.notify\("([a-z\/.-]+)"/g)].map((m) => m[1])
  const requests = [...source.matchAll(/transport\.request\("([a-z\/.-]+)"/g)].map((m) => m[1])
  return { notifications: unique(notifications), requests: unique(requests) }
}

/** Find a file by name anywhere under `kernel/node_modules/.pnpm`, without hard-coding the hash. */
function findInStore(relative) {
  const store = join(root, 'kernel', 'node_modules', '.pnpm')
  if (!existsSync(store)) return null
  for (const entry of readdirSync(store)) {
    const candidate = join(store, entry, 'node_modules', relative)
    if (existsSync(candidate)) return candidate
  }
  return null
}

/** The kernel's own list of durable session event types. */
function durableEvents() {
  const file = findInStore(join('@deepseek-ai', 'dsh-session', 'lib', 'types', 'known-event-types.js'))
  if (!file) return []
  const source = read(file)
  // The catalog uses single quotes — matching only double quotes reported "0 event types"
  // while the file was right there.
  return unique([...source.matchAll(/['"]([a-z][a-z/-]+)['"]/g)].map((match) => match[1]))
}

// ── what we have ────────────────────────────────────────────────────────────

const commandsRs = read(join(root, 'apps', 'desktop', 'src-tauri', 'src', 'commands.rs'))
const workspaceRs = read(join(root, 'apps', 'desktop', 'src-tauri', 'src', 'workspace.rs'))
const stateRs = read(join(root, 'apps', 'desktop', 'src-tauri', 'src', 'state.rs'))
const ipcTs = read(join(root, 'apps', 'desktop', 'src', 'lib', 'ipc.ts'))
const reducerTs = read(join(root, 'apps', 'desktop', 'src', 'lib', 'reducer.ts'))
const docsTs = read(join(root, 'docs', '接口.md'));

const runtimeRs = read(join(root, 'crates', 'dsh-runtime', 'src', 'process.rs'))
const poolRs = read(join(root, 'crates', 'dsh-runtime', 'src', 'pool.rs'))
// Every place a kernel method string or a typed client call can appear. Leaving the runtime crate
// out made `.shutdown(` invisible and reported `shutdown` as "not connected" — while the pool was
// shutting kernels down every time a route was evicted.
const rustAll = [commandsRs, workspaceRs, stateRs, runtimeRs, poolRs].join('\n')

/**
 * Does our Rust deliberately use this kernel method?
 *
 * Two shapes count: a literal method string (`"session/resume"`) for the generic call path, and a
 * typed client method (`.initialize(`, `.shutdown(`) for the ones the runtime wraps. Missing the
 * second shape made the first version of this audit report `initialize` as "not connected" while
 * every session in the app was initialising a kernel.
 */
function sentByRust(method) {
  if (rustAll.includes(`"${method}"`)) return true
  const tail = method.includes('/') ? null : method
  return tail ? new RegExp(`\\.${tail}\\s*\\(`).test(rustAll) : false
}

/** Is it wrapped by a typed helper in the front end? */
function typedInIpc(method) {
  return ipcTs.includes(`'${method}'`)
}

/** Does the reducer have an explicit case for this event type? */
function handledByReducer(kind) {
  return reducerTs.includes(`'${kind}'`)
}

/** Does the doc mention it at all? */
function inDocs(token) {
  return docsTs.includes(token)
}

const sdk = sdkDir()
if (!sdk) {
  console.error('[audit] kernel is not fetched — run `pnpm kernel:fetch`')
  process.exit(0)
}
const sdkSource = read(join(sdk, 'index.js'))
const methods = kernelMethods(sdkSource)
const inbound = kernelInbound(sdkSource)
const events = durableEvents()

const rows = []
for (const method of methods) {
  const rust = sentByRust(method)
  const typed = typedInIpc(method)
  const status = typed ? '✅ 有类型封装' : rust ? '🔶 Rust 里在用（前端只能 kernel_call 直通）' : '⬜ 没接'
  rows.push({ group: 'SDK 方法', name: method, status, note: typed ? '`ipc.ts` 有具名函数' : rust ? '命令内部使用' : '可通过 `kernel_call` 直通，但没有封装' })
}
for (const method of inbound.requests) {
  const answered = rustAll.includes(method) || commandsRs.includes('kernel_answer')
  rows.push({
    group: '内核→客户端 请求',
    name: method,
    state: answered ? 'done' : 'todo',
    status: answered ? '✅ 必须应答，已接' : '⬜ 没接',
    note: answered ? '`kernel_answer` 路径' : '内核会阻塞等人回答',
  })
}
for (const method of inbound.notifications) {
  const forwarded = rustAll.includes('forward_frames') || commandsRs.includes('KERNEL_EVENT')
  rows.push({
    group: '内核→客户端 通知',
    name: method,
    state: forwarded ? 'partial' : 'todo',
    status: forwarded ? '🔶 已转发到前端' : '⬜ 没接',
    note: forwarded ? '按帧转发，前端自行解读' : '',
  })
}

const eventRows = events.map((kind) => ({
  group: '会话事件（durable）',
  name: kind,
  state: handledByReducer(kind) ? 'done' : 'todo',
  status: handledByReducer(kind) ? '✅ reducer 有分支' : inDocs(kind) ? '⬜ 文档列出，未处理' : '⬜ 未处理且未记录',
  note: '',
}))

const all = [...rows, ...eventRows]
const counts = all.reduce((acc, row) => {
  acc[row.state] = (acc[row.state] ?? 0) + 1
  return acc
}, {})

if (!markdown) {
  let group = null
  for (const row of all) {
    if (row.group !== group) {
      group = row.group
      console.log(`\n## ${group}`)
    }
    console.log(`  ${row.status.padEnd(34)} ${row.name}${row.note ? `  — ${row.note}` : ''}`)
  }
  console.log(
    `\n合计：✅ ${counts.done ?? 0} · 🔶 ${counts.partial ?? 0} · ⬜ ${counts.todo ?? 0}`,
  )
  console.log(`内核 ${methods.length} 个方法 · ${inbound.requests.length} 个请求 · ${inbound.notifications.length} 个通知 · ${events.length} 种事件`)
} else {
  const lines = []
  lines.push('| 层 | 名字 | 状态 | 说明 |')
  lines.push('| --- | --- | --- | --- |')
  for (const row of all) lines.push(`| ${row.group} | \`${row.name}\` | ${row.status} | ${row.note} |`)
  lines.push('')
  lines.push(
    `合计：✅ ${counts.done ?? 0} · 🔶 ${counts.partial ?? 0} · ⬜ ${counts.todo ?? 0}；` +
      `内核 ${methods.length} 个 SDK 方法、${inbound.requests.length} 个请求、${inbound.notifications.length} 个通知、${events.length} 种会话事件。`,
  )
  console.log(lines.join('\n'))
}
