// @vitest-environment node
import { spawnSync } from 'node:child_process'
import { existsSync, mkdirSync, mkdtempSync, rmSync, statSync } from 'node:fs'
import { userInfo } from 'node:os'
import { join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'
import { readSource } from './setup/read-source'

/**
 * Linux 离线包（随包内核）的装配契约。
 *
 * 离线包把 dsh 内核托管到 `$Resources/dsh`，在 deb 里落在
 * `/usr/lib/<product>/resources` —— root 所有、普通用户只读。而桌面端启动时必须往
 * 内核根写内置插件入口与 JS 补丁，`prepare_active_runtime` 写不进去就直接放弃启动。
 * 所以这个包是否可用由三件事共同决定，任何一件退化都会产出「装了也起不来」的包，
 * 且只会在用户机器上暴露：
 *
 *   1. deb 带 `postinst`（`bundle.linux.deb.postInstallScript` → `debian/postinst.sh`），
 *      把随包资源树的属主交给安装用户；
 *   2. `postinst` 自己的行为契约：只在 configure 阶段动手、没有随包内核时静默跳过、
 *      任何失败都不让安装失败（dpkg 会因 postinst 非 0 报「配置失败」）；
 *   3. arm64 真的随包构建（runner、prepare 的 arch、产物架构校验），且 build 期校验
 *      postinst 与内核入口确实进了 deb。
 */

const CONFIG = JSON.parse(readSource('src-tauri/tauri.conf.json')) as {
  bundle: { linux?: { deb?: { postInstallScript?: string } } }
}
const BUNDLE_LINUX = readSource('.github/workflows/build-bundle-linux.yml')
const RELEASE_BUNDLE = readSource('.github/workflows/release-bundle.yml')

const postinstPath = fileURLToPath(new URL('../src-tauri/debian/postinst.sh', import.meta.url))
const scratchRoot = fileURLToPath(new URL('../.temp/', import.meta.url))
/** 安装者身份：用真实存在的用户，脚本才会走「按 SUDO_USER 交属主」那条路。 */
const currentUser = userInfo().username

interface PostinstRun {
  status: number | null
  stderr: string
}

/**
 * 在隔离的假资源树里跑一次 postinst。`withCore: false` 模拟普通（非离线）安装。
 */
function runPostinst(
  { withCore = true, args = ['configure'], env = {} }:
  { withCore?: boolean, args?: string[], env?: Record<string, string> } = {},
): PostinstRun {
  mkdirSync(scratchRoot, { recursive: true })
  const root = mkdtempSync(join(scratchRoot, 'postinst-'))
  try {
    const resources = join(root, 'resources')
    if (withCore) {
      mkdirSync(join(resources, 'dsh', 'node_modules'), { recursive: true })
      mkdirSync(join(resources, 'node', 'bin'), { recursive: true })
    }
    else {
      mkdirSync(resources, { recursive: true })
    }
    const result = spawnSync('sh', [postinstPath, ...args], {
      encoding: 'utf8',
      timeout: 10_000,
      env: { ...process.env, DSH_DESKTOP_RESOURCES_DIR: resources, ...env },
    })
    expect(result.error).toBeUndefined()
    return { status: result.status, stderr: result.stderr }
  }
  finally {
    expect(resolve(root).startsWith(resolve(scratchRoot) + (process.platform === 'win32' ? '\\' : '/'))).toBe(true)
    rmSync(root, { recursive: true, force: true })
  }
}

describe('linux offline bundle packaging', () => {
  it('ships the deb post-install script that makes the bundled core writable', () => {
    expect(CONFIG.bundle.linux?.deb?.postInstallScript).toBe('./debian/postinst.sh')
    expect(existsSync(postinstPath)).toBe(true)
    // tauri-cli 打包前会 `set_current_dir(tauri_dir)`，所以配置里的路径是相对
    // `src-tauri/` 解析的（与 windows.fragments 同一条约定）。
    expect(readSource('src-tauri/debian/postinst.sh')).toContain('DSH_DESKTOP_RESOURCES_DIR')
    if (process.platform === 'win32')
      return
    expect(statSync(postinstPath).mode & 0o111).toBeGreaterThan(0)
  })

  it('builds both linux architectures with the matching runner and bundled arch', () => {
    // 矩阵必须由表达式展开：`matrix` 上下文在 jobs.<id>.if 里不可用，静态矩阵加 job 级
    // `if` 过滤条目是行不通的（那种写法会被 GitHub 直接判为未知上下文）。
    expect(BUNDLE_LINUX).toContain(`fromJSON(inputs.arch == 'all' && '["x64","arm64"]'`)
    expect(BUNDLE_LINUX).toContain(`runs-on: \${{ matrix.arch == 'arm64' && 'ubuntu-22.04-arm' || 'ubuntu-22.04' }}`)
    expect(BUNDLE_LINUX).toContain(`arch: \${{ matrix.arch }}`)
    expect(BUNDLE_LINUX).toContain(`name: release-bundle-linux-\${{ matrix.arch }}`)
  })

  it('fails the build when the deb lacks the postinst or the bundled core', () => {
    // 少了 postinst 的离线 deb 在非 root 用户下必然起不来：这种包必须在构建期失败，
    // 不能留到用户机器上才发现。
    expect(BUNDLE_LINUX).toContain('dpkg-deb -e "$deb"')
    expect(BUNDLE_LINUX).toContain('[ -x "$control/postinst" ]')
    expect(BUNDLE_LINUX).toContain('dsh/node_modules/@deepseek-ai/dsh/lib/bin.js')
    expect(BUNDLE_LINUX).toContain('dpkg-deb -f "$deb" Architecture')
  })

  it('publishes the arm64 deb under an architecture suffix without renaming x64', () => {
    expect(RELEASE_BUNDLE).toContain('*linux-arm64*) suffix="_aarch64"')
    expect(RELEASE_BUNDLE).not.toContain('*linux-x64*) suffix=')
    expect(RELEASE_BUNDLE).toContain('inputs.linux_arch')
    expect(RELEASE_BUNDLE).toContain(`arch: \${{ github.event_name == 'workflow_dispatch' && inputs.linux_arch || 'all' }}`)
  })

  it.each(['configure', 'abort-upgrade'])('only touches ownership during configure (%s)', (phase) => {
    const result = runPostinst({ args: [phase], env: { SUDO_USER: currentUser, DRY_RUN: '1' } })
    expect(result.status).toBe(0)
    if (process.platform === 'win32')
      return
    if (phase === 'configure')
      expect(result.stderr).toContain('dry run: chown -R ')
    else
      expect(result.stderr).toBe('')
  })

  it('stays silent when the install has no bundled core', () => {
    const result = runPostinst({ withCore: false, env: { SUDO_USER: currentUser, DRY_RUN: '1' } })
    expect(result).toEqual({ status: 0, stderr: '' })
  })

  it('hands the whole bundled resource tree to the installing user', () => {
    const result = runPostinst({ env: { SUDO_USER: currentUser, DRY_RUN: '1' } })
    expect(result.status).toBe(0)
    if (process.platform === 'win32')
      return
    expect(result.stderr).toMatch(new RegExp(`dry run: chown -R ${currentUser}:[^ ]+ .*\\/resources\\n$`))
  })

  // 安装者身份靠 SUDO_USER / DOAS_USER / PKEXEC_UID，认不出时按应用数据目录兜底。
  // 兜底也认不出（干净的 CI 机器）时只能打印 chown 指引 —— 无论走哪条路，退出码都必须是
  // 0，postinst 非 0 会让 dpkg 把包报成「配置失败」。
  it('degrades to guidance instead of failing when the installing user is unknown', () => {
    const result = runPostinst({ env: { SUDO_USER: 'postinst-probe-no-such-user', DOAS_USER: '', PKEXEC_UID: '', DRY_RUN: '1' } })
    expect(result.status).toBe(0)
    if (process.platform === 'win32')
      return
    expect(result.stderr).toMatch(/deepseek-harness-desktop: (dry run: chown -R|cannot tell which user)/)
  })

  // 非 root 下把属主改给别人必然失败（CI 是普通用户）：这时只能告警，退出码必须仍是 0，
  // 否则 dpkg 会把这个包报成「配置失败」。
  it.skipIf(process.platform === 'win32' || process.getuid?.() === 0)('never fails the install when chown is denied', () => {
    const result = runPostinst({ env: { SUDO_USER: 'nobody' } })
    expect(result.status).toBe(0)
    expect(result.stderr).toContain('warning: chown -R nobody:')
  })
})
