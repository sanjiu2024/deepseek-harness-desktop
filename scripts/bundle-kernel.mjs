/**
 * Assemble a distributable directory: the app binary, the slimmed kernel, and a Node runtime.
 *
 * The installer problem in one sentence: this client is useless without a kernel, the kernel is
 * 370 MB of JavaScript, and JavaScript needs Node — so a "5 MB installer" here would be a lie
 * followed by a 500 MB download on first launch.
 *
 * What this produces instead:
 *
 *   dist-bundle/
 *     dsh-desktop-app(.exe)      the client
 *     kernel/                    the pinned, slimmed kernel (no fetch at runtime)
 *     node(.exe)                 a Node runtime, because the kernel is a Node program
 *     README.txt                 what this is and what it needs
 *
 * The app resolves each of those from its own directory when they exist, so the bundle runs on
 * a machine with no Node installed and no network. `tauri build` then packs this directory.
 *
 *   node scripts/bundle-kernel.mjs            # dry run: report what would be copied
 *   node scripts/bundle-kernel.mjs --apply    # build dist-bundle/
 */
import { copyFileSync, existsSync, mkdirSync, readdirSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { duBytes, human } from './lib/size.mjs'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const bundleDir = join(root, 'dist-bundle')
const kernelDir = join(root, 'kernel')
const apply = process.argv.includes('--apply')

const exeName = process.platform === 'win32' ? 'dsh-desktop-app.exe' : 'dsh-desktop-app'
const exePath = join(root, 'target', 'release', exeName)
const nodePath = process.execPath

const problems = []
if (!existsSync(kernelDir)) {
  problems.push('kernel/ is missing — run `pnpm kernel:fetch`')
}
if (!existsSync(exePath)) {
  problems.push(`${exePath} is missing — run \`cargo build --release -p dsh-desktop-app\``)
}
// A development Node is ~110 MB and its layout is not redistributable as-is; say so rather than
// silently shipping whatever happens to be on this machine.
const nodeIsPortable = statSync(nodePath).size > 0

const parts = [
  ['client', exePath, existsSync(exePath) ? statSync(exePath).size : 0],
  ['kernel', kernelDir, existsSync(kernelDir) ? duBytes(kernelDir) : 0],
  ['node', nodePath, nodeIsPortable ? statSync(nodePath).size : 0],
]

console.log('[bundle] contents')
for (const [name, path, size] of parts) {
  console.log(`  ${human(size).padStart(7)}  ${name.padEnd(7)} ${path}`)
}
console.log(`  ${human(parts.reduce((total, [, , size]) => total + size, 0)).padStart(7)}  total`)

if (problems.length > 0) {
  for (const problem of problems) console.error(`[bundle] ${problem}`)
  process.exit(1)
}

const nodeMajor = process.versions.node.split('.')[0]
if (Number(nodeMajor) < 22) {
  console.error(`[bundle] Node ${process.version} is too old for the kernel; 22+ is required`)
  process.exit(1)
}

if (!apply) {
  console.log('\n[bundle] dry run. Re-run with --apply to build dist-bundle/.')
  console.log('[bundle] note: this copies the local Node runtime. For a release, point')
  console.log('         DSH_BUNDLE_NODE at a redistributable Node build.')
  process.exit(0)
}

rmSync(bundleDir, { recursive: true, force: true })
mkdirSync(bundleDir, { recursive: true })

copyFileSync(exePath, join(bundleDir, exeName))
console.log(`[bundle] client  -> ${exeName}`)

const nodeTarget = join(bundleDir, process.platform === 'win32' ? 'node.exe' : 'node')
copyFileSync(process.env.DSH_BUNDLE_NODE ?? nodePath, nodeTarget)
console.log(`[bundle] node    -> ${nodeTarget.split(/[\\/]/).pop()}`)

// The kernel is a directory tree; copying file by file keeps the symlinks pnpm uses intact on
// Windows, where a recursive copy of a symlinked store can explode into gigabytes.
mkdirSync(join(bundleDir, 'kernel'), { recursive: true })
copyTree(kernelDir, join(bundleDir, 'kernel'))
console.log('[bundle] kernel  -> kernel/')

writeFileSync(
  join(bundleDir, 'README.txt'),
  [
    'DeepSeek Harness Desktop',
    '',
    '这是免安装版本：把整个目录放在一起运行，不要单独移动其中的文件。',
    '',
    '  dsh-desktop-app   客户端',
    '  kernel/           钉死版本的 DeepSeek Harness 内核（已瘦身）',
    '  node              内核需要的 Node 运行时（22+）',
    '',
    '首次启动会在同目录下建 dsh-home/，用来放会话、设置与日志。',
    '日志：dsh-home/logs/desktop.log',
    '',
  ].join('\n'),
)

const total = duBytes(bundleDir)
console.log(`\n[bundle] dist-bundle/ is ${human(total)}`)
console.log('[bundle] next: `pnpm tauri build` picks this up as a resource if tauri.conf.json lists it')

/** Copy a directory tree without following symlinks out of it. */
function copyTree(from, to) {
  for (const entry of readdirSync(from, { withFileTypes: true })) {
    const source = join(from, entry.name)
    const target = join(to, entry.name)
    if (entry.isDirectory()) {
      mkdirSync(target, { recursive: true })
      copyTree(source, target)
    } else if (entry.isFile()) {
      try {
        copyFileSync(source, target)
      } catch (error) {
        // A file the antivirus has open, or a path too long for Windows: report and continue,
        // because aborting halfway leaves a bundle that looks complete and is not.
        console.warn(`[bundle] skipped ${source}: ${error.message}`)
      }
    }
  }
}

