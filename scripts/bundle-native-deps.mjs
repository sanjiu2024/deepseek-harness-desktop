#!/usr/bin/env node
/**
 * 离线包的目标平台原生可选依赖补齐（CI 用，`prepare-bundle-resources` 调用）。
 *
 * 背景（事故）：`deepseek-harness-pkg` 的 Linux 发行包只有一份
 * `deepseek-harness-pkg-linux.zip`（与架构无关），里面带的是 x64 的原生可选依赖。
 * aarch64 上桌面端启动时 `prepare_active_runtime` 会先探测 sharp/koffi：导入失败
 * 就按核心清单里的 `optionalDependencies` 现场 `npm install` 目标平台包
 * （见 `src-tauri/src/service/core/runtime.rs` 的 `native_package_plan` /
 * `install_native_packages`）。离线机器上这次安装必然失败，实测落到
 * `CORE_NATIVE_DEPENDENCY_REPAIR_TIMEOUT`（120s 超时）→ 启动直接放弃，
 * 也就是「装了离线包也起不来」。
 *
 * 因此随包资源必须在**构建期**就把目标平台的原生包放进 `resources/dsh`：
 *   1. 用随包 Node 跑一次与运行时同源的导入探测（sharp / koffi）；
 *   2. 探测通过即什么都不做（幂等；x64 等已带原生包的组合不受影响）；
 *   3. 失败时按同一套规则取 `optionalDependencies` 里的目标平台包版本，
 *      在临时目录 `npm install` 后把新增包拷进随包内核的 `node_modules`
 *      （只拷「计划内的包」与「随包内核里还没有的包」，绝不覆盖既有版本）；
 *   4. 再用随包 Node 复探一次，仍失败就让构建失败 —— 不能把起不来的包发出去。
 *
 * 平台/架构到包名的映射与 Rust 侧一致：`@img/sharp-<platform>-<arch>`、
 * `@img/sharp-libvips-<platform>-<arch>`、`@koromix/koffi-<platform>-<arch>`，
 * 其中 platform 用 Node 的取值（`win32` / `darwin` / `linux`）。
 */
import { spawnSync } from 'node:child_process'
import * as fs from 'node:fs'
import * as os from 'node:os'
import * as path from 'node:path'
import process from 'node:process'
import { pathToFileURL } from 'node:url'

const ERROR_PREFIX = 'BUNDLE_NATIVE'

function bundleError(message, error) {
  const detail = error instanceof Error ? `: ${error.message}` : ''
  return new Error(`${ERROR_PREFIX}: ${message}${detail}`)
}

/** 清单平台 → Node 平台标识（包名与 npm 的 `--os` 都用它） */
export function platformToken(platform) {
  if (platform === 'windows')
    return 'win32'
  if (platform === 'macos')
    return 'darwin'
  if (platform === 'linux')
    return 'linux'
  throw bundleError(`platform must be windows|macos|linux, got ${JSON.stringify(platform)}`)
}

function readOptionalDependencies(manifestPath) {
  try {
    const raw = fs.readFileSync(manifestPath, 'utf8')
    const parsed = JSON.parse(raw)
    const optional = parsed?.optionalDependencies
    if (!optional || typeof optional !== 'object')
      return {}
    return Object.fromEntries(
      Object.entries(optional).filter(([, version]) => typeof version === 'string'),
    )
  }
  catch {
    return {}
  }
}

/**
 * 需要补齐的原生包安装参数，逐字复刻 `runtime::native_package_plan`：
 * 优先从随包内核里已安装的 `sharp` / `koffi` 取目标平台包的精确版本；
 * 根包整个缺失（核心闭包被删）时退化为不带版本的根包名。
 */
export function nativePackagePlan({ dshDir, platform, arch }) {
  const token = platformToken(platform)
  const nodeModules = path.join(dshDir, 'node_modules')
  const sharpDir = path.join(nodeModules, 'sharp')
  const koffiDir = path.join(nodeModules, 'koffi')
  const sharpOptional = readOptionalDependencies(path.join(sharpDir, 'package.json'))
  const koffiOptional = readOptionalDependencies(path.join(koffiDir, 'package.json'))

  const names = [
    [`@img/sharp-${token}-${arch}`, sharpOptional],
    [`@img/sharp-libvips-${token}-${arch}`, sharpOptional],
    [`@koromix/koffi-${token}-${arch}`, koffiOptional],
  ]

  const packages = []
  for (const [name, optional] of names) {
    if (typeof optional[name] === 'string')
      packages.push(`${name}@${optional[name]}`)
  }
  if (!fs.existsSync(sharpDir) && !packages.includes('sharp'))
    packages.push('sharp')
  if (!fs.existsSync(koffiDir) && !packages.includes('koffi'))
    packages.push('koffi')
  return packages
}

/** 与 `NATIVE_PROBE_SCRIPT` 同源的最小探测：只关心 sharp / koffi 的导入。 */
const PROBE_SOURCE = `
const { createRequire } = await import('node:module');
const { join } = await import('node:path');
const loader = createRequire(join(process.cwd(), 'dsh-native-probe.cjs'));
const failures = [];
for (const name of ['sharp', 'koffi']) {
  try { loader(name); } catch (error) { failures.push({ name, message: String((error && error.message) || error) }); }
}
process.stdout.write('DSH_NATIVE_PROBE:' + JSON.stringify(failures) + '\\n');
`

/**
 * 用随包 Node 在随包内核根跑一次导入探测。
 *
 * 探测结论无法解析（Node 起不来、脚本被截断）时抛错，而不是当成「探测通过」——
 * 那会把一个起不来的包放行。
 */
export function probeNativeModules({ node, dshDir }) {
  if (!fs.existsSync(node))
    throw bundleError(`bundled Node.js runtime is missing: ${node}`)
  const result = spawnSync(node, ['--input-type=module', '-e', PROBE_SOURCE], {
    cwd: dshDir,
    encoding: 'utf8',
    timeout: 120_000,
  })
  if (result.error)
    throw bundleError(`native probe failed to run (${node})`, result.error)
  const line = (result.stdout ?? '').split('\n').find(entry => entry.startsWith('DSH_NATIVE_PROBE:'))
  if (!line) {
    throw bundleError(
      `native probe produced no verdict (exit ${result.status}); stderr: ${(result.stderr ?? '').trim().slice(0, 400)}`,
    )
  }
  return JSON.parse(line.slice('DSH_NATIVE_PROBE:'.length))
}

function npmCommand() {
  return process.platform === 'win32' ? 'npm.cmd' : 'npm'
}

/** 在临时目录装上目标平台包，返回 stage 的 node_modules 路径。 */
function installPlanIntoStage(packages, { platform, arch }) {
  const stage = fs.mkdtempSync(path.join(os.tmpdir(), 'dsh-native-stage-'))
  fs.writeFileSync(
    path.join(stage, 'package.json'),
    `${JSON.stringify({ name: 'dsh-native-stage', private: true }, null, 2)}\n`,
  )
  // 与运行时同一组参数：--os/--cpu 让可选依赖按目标平台解析，
  // `--include=optional` 保证平台包不被 optional 规则跳过。
  const args = [
    'install',
    '--no-save',
    '--package-lock=false',
    '--include=optional',
    `--os=${platformToken(platform)}`,
    `--cpu=${arch}`,
    ...packages,
  ]
  // 不用 `--prefix <临时路径>`：临时目录在 Windows 上可能带空格，shell 转发会拆参数。
  // 直接以 cwd 指定安装位置，路径不过命令行。
  const result = spawnSync(npmCommand(), args, {
    cwd: stage,
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
    timeout: 15 * 60_000,
    shell: process.platform === 'win32',
  })
  if (result.error) {
    fs.rmSync(stage, { recursive: true, force: true })
    throw bundleError('npm install failed to start', result.error)
  }
  if (result.status !== 0) {
    const tail = `${result.stdout ?? ''}\n${result.stderr ?? ''}`.trim().slice(-600)
    fs.rmSync(stage, { recursive: true, force: true })
    throw bundleError(`npm install exited with ${result.status}: ${tail}`)
  }
  return stage
}

/** 列出 node_modules 里的包目录（含 scope），跳过 `.bin` / `.pnpm`。 */
function listInstalledPackages(nodeModules) {
  const found = []
  if (!fs.existsSync(nodeModules))
    return found
  const isPackageDir = entry => entry.isDirectory() || entry.isSymbolicLink()
  for (const entry of fs.readdirSync(nodeModules, { withFileTypes: true })) {
    if (entry.name === '.bin' || entry.name === '.pnpm' || entry.name === '.package-lock.json')
      continue
    if (entry.name.startsWith('@')) {
      const scope = path.join(nodeModules, entry.name)
      if (!isPackageDir(entry))
        continue
      for (const inner of fs.readdirSync(scope, { withFileTypes: true })) {
        if (isPackageDir(inner))
          found.push(`${entry.name}/${inner.name}`)
      }
      continue
    }
    if (isPackageDir(entry))
      found.push(entry.name)
  }
  return found
}

/** `@scope/name@1.2.3` / `name@1.2.3` / `name` → 包名 */
export function packageSpecName(spec) {
  const separator = spec.startsWith('@') ? spec.indexOf('@', 1) : spec.indexOf('@')
  return separator < 0 ? spec : spec.slice(0, separator)
}

/**
 * 按运行时同源的规则补齐随包内核的原生可选依赖。
 *
 * 只在探测失败时动手；拷贝策略是「计划内的包 + 随包内核里还没有的包」，
 * 既有版本一律不覆盖 —— 内核自带的模块不能被构建期的解析结果换掉。
 */
export function seedNativePackages({ dshDir, node, platform, arch }) {
  if (!fs.existsSync(path.join(dshDir, 'node_modules')))
    throw bundleError(`bundled core node_modules is missing: ${path.join(dshDir, 'node_modules')}`)

  const before = probeNativeModules({ node, dshDir })
  if (before.length === 0)
    return { ok: true, seeded: [], probe: before }

  const plan = nativePackagePlan({ dshDir, platform, arch })
  if (plan.length === 0)
    throw bundleError(`no optional package version matches ${platformToken(platform)}:${arch} in ${dshDir}`)

  const stage = installPlanIntoStage(plan, { platform, arch })
  const seeded = []
  try {
    const stageModules = path.join(stage, 'node_modules')
    const targetModules = path.join(dshDir, 'node_modules')
    const planned = new Set(plan.map(packageSpecName))
    for (const name of listInstalledPackages(stageModules)) {
      const target = path.join(targetModules, name)
      if (!planned.has(name) && fs.existsSync(target))
        continue
      fs.rmSync(target, { recursive: true, force: true })
      fs.mkdirSync(path.dirname(target), { recursive: true })
      // dereference：npm 的包可能是软链，拷内容而不是拷链
      fs.cpSync(path.join(stageModules, name), target, { recursive: true, dereference: true })
      seeded.push(name)
    }
  }
  finally {
    fs.rmSync(stage, { recursive: true, force: true })
  }

  const after = probeNativeModules({ node, dshDir })
  if (after.length > 0) {
    throw bundleError(
      `native modules still cannot load after seeding ${plan.join(', ')}: ${after.map(f => `${f.name}: ${f.message}`).join(' | ')}`,
    )
  }
  return { ok: true, seeded, probe: after }
}

function parseArgs(argv) {
  const args = { probeOnly: false }
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i]
    if (arg === '--probe-only') {
      args.probeOnly = true
      continue
    }
    if (arg.startsWith('--')) {
      const key = arg.slice(2)
      const value = argv[++i]
      if (value === undefined)
        throw bundleError(`missing value for --${key}`)
      args[key] = value
      continue
    }
    throw bundleError(`unknown argument ${JSON.stringify(arg)}`)
  }
  return args
}

function requireString(value, label) {
  if (typeof value !== 'string' || value === '')
    throw bundleError(`${label} is required`)
  return value
}

async function main() {
  const args = parseArgs(process.argv.slice(2))
  const repo = path.resolve(args.repo ?? process.cwd())
  const platform = requireString(args.platform, '--platform (windows|macos|linux)')
  const arch = requireString(args.arch, '--arch (x64|arm64)')
  const nodeName = platform === 'windows' ? 'node.exe' : path.join('bin', 'node')
  const node = path.resolve(args.node ?? path.join(repo, 'src-tauri', 'resources', 'node', nodeName))
  const dshDir = path.resolve(args.dsh ?? path.join(repo, 'src-tauri', 'resources', 'dsh'))

  if (args.probeOnly) {
    const failures = probeNativeModules({ node, dshDir })
    if (failures.length > 0)
      throw bundleError(`native modules cannot load: ${failures.map(f => `${f.name}: ${f.message}`).join(' | ')}`)
    process.stdout.write(`${ERROR_PREFIX}: probe ok (${platform}/${arch})\n`)
    return
  }

  const { seeded } = seedNativePackages({ dshDir, node, platform, arch })
  if (seeded.length === 0)
    process.stdout.write(`${ERROR_PREFIX}: probe ok, nothing to seed (${platform}/${arch})\n`)
  else
    process.stdout.write(`${ERROR_PREFIX}: seeded ${seeded.join(', ')} for ${platform}/${arch}\n`)
}

const entryPoint = process.argv[1]
if (entryPoint && import.meta.url === pathToFileURL(path.resolve(entryPoint)).href) {
  try {
    await main()
  }
  catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    console.error(message.startsWith(`${ERROR_PREFIX}:`) ? message : `${ERROR_PREFIX}: ${message}`)
    process.exitCode = 1
  }
}
