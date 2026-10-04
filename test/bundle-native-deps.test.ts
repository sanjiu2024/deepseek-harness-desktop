// @vitest-environment node
import { spawnSync } from 'node:child_process'
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'
import * as nativeDeps from '../scripts/bundle-native-deps.mjs'
import { readSource } from './setup/read-source'

/**
 * 离线包的目标平台原生依赖补齐契约（事故：`CORE_NATIVE_DEPENDENCY_REPAIR_TIMEOUT`）。
 *
 * `deepseek-harness-pkg-linux.zip` 只有一份（与架构无关），带的是 x64 的原生可选
 * 依赖；aarch64 上运行时探测 sharp/koffi 失败后会现场 `npm install`，离线机器必然
 * 失败并放弃启动。这里锁住三件事：
 *
 *   1. 平台计划与 Rust `runtime::native_package_plan` 同源（含 Rust 单测里的
 *      darwin/arm64 夹具），版本取自随包内核自己的 `optionalDependencies`，不硬编码；
 *   2. 目标平台包名映射：`@img/sharp-<platform>-<arch>`、
 *      `@img/sharp-libvips-<platform>-<arch>`、`@koromix/koffi-<platform>-<arch>`；
 *   3. `prepare-bundle-resources` 真的会调用它 —— 否则计划再对也不会执行。
 */

const ACTION = readSource('.github/actions/prepare-bundle-resources/action.yml')
const scriptPath = fileURLToPath(new URL('../scripts/bundle-native-deps.mjs', import.meta.url))
const scratchRoot = fileURLToPath(new URL('../.temp/', import.meta.url))

/** 造一个只含原生可选依赖清单的假内核。 */
function fixture(manifests: { sharp?: unknown, koffi?: unknown }): string {
  mkdirSync(scratchRoot, { recursive: true })
  const root = mkdtempSync(join(scratchRoot, 'native-plan-'))
  const nodeModules = join(root, 'node_modules')
  mkdirSync(nodeModules, { recursive: true })
  for (const name of ['sharp', 'koffi'] as const) {
    const manifest = manifests[name]
    if (manifest === undefined)
      continue
    mkdirSync(join(nodeModules, name), { recursive: true })
    writeFileSync(join(nodeModules, name, 'package.json'), JSON.stringify(manifest))
  }
  return root
}

function withFixture<T>(manifests: { sharp?: unknown, koffi?: unknown }, run: (dshDir: string) => T): T {
  const root = fixture(manifests)
  try {
    return run(root)
  }
  finally {
    expect(resolve(root).startsWith(resolve(scratchRoot) + (process.platform === 'win32' ? '\\' : '/'))).toBe(true)
    rmSync(root, { recursive: true, force: true })
  }
}

describe('offline bundle native optional dependencies', () => {
  it('maps platforms to the Node identifiers used in package names', () => {
    expect(nativeDeps.platformToken('windows')).toBe('win32')
    expect(nativeDeps.platformToken('macos')).toBe('darwin')
    expect(nativeDeps.platformToken('linux')).toBe('linux')
    expect(() => nativeDeps.platformToken('freebsd' as 'linux')).toThrow(/BUNDLE_NATIVE:/)
  })

  // 与 Rust `native_plan_uses_manifest_versions_not_constants` 同一份夹具：两边一致
  // 才不会出现「构建期补齐的包不是运行时找的那个」。
  it('mirrors the Rust plan on the darwin/arm64 fixture', () => {
    const plan = withFixture(
      {
        sharp: { optionalDependencies: { '@img/sharp-darwin-arm64': '0.9.0', '@img/sharp-libvips-darwin-arm64': '1.2.0' } },
        koffi: { optionalDependencies: { '@koromix/koffi-darwin-arm64': '8.7.0' } },
      },
      dshDir => nativeDeps.nativePackagePlan({ dshDir, platform: 'macos', arch: 'arm64' }),
    )
    expect(plan).toEqual([
      '@img/sharp-darwin-arm64@0.9.0',
      '@img/sharp-libvips-darwin-arm64@1.2.0',
      '@koromix/koffi-darwin-arm64@8.7.0',
    ])
  })

  // 事故现场（desktop.log 的 CORE_NATIVE_DEPENDENCY_REPAIR 那一行）逐字复刻。
  it('plans the linux-arm64 packages the runtime would install at startup', () => {
    const plan = withFixture(
      {
        sharp: { optionalDependencies: { '@img/sharp-linux-arm64': '0.35.5', '@img/sharp-libvips-linux-arm64': '1.3.4', '@img/sharp-linux-x64': '0.35.5' } },
        koffi: { optionalDependencies: { '@koromix/koffi-linux-arm64': '3.3.2', '@koromix/koffi-linux-x64': '3.3.2' } },
      },
      dshDir => nativeDeps.nativePackagePlan({ dshDir, platform: 'linux', arch: 'arm64' }),
    )
    expect(plan).toEqual([
      '@img/sharp-linux-arm64@0.35.5',
      '@img/sharp-libvips-linux-arm64@1.3.4',
      '@koromix/koffi-linux-arm64@3.3.2',
    ])
  })

  it('falls back to the bare root packages when the core closure is gone', () => {
    const plan = withFixture({}, dshDir => nativeDeps.nativePackagePlan({ dshDir, platform: 'linux', arch: 'arm64' }))
    expect(plan).toEqual(['sharp', 'koffi'])
  })

  it('does not plan a platform package the manifest does not declare', () => {
    const plan = withFixture(
      { sharp: { optionalDependencies: {} }, koffi: { optionalDependencies: { '@koromix/koffi-win32-x64': '3.3.2' } } },
      dshDir => nativeDeps.nativePackagePlan({ dshDir, platform: 'linux', arch: 'arm64' }),
    )
    expect(plan).toEqual([])
  })

  it.each([
    ['@img/sharp-linux-arm64@0.35.5', '@img/sharp-linux-arm64'],
    ['@koromix/koffi-linux-arm64@3.3.2', '@koromix/koffi-linux-arm64'],
    ['sharp@0.35.5', 'sharp'],
    ['sharp', 'sharp'],
  ])('derives the package name from spec %s', (spec, expected) => {
    expect(nativeDeps.packageSpecName(spec)).toBe(expected)
  })

  it('runs during bundle preparation for the target architecture', () => {
    expect(ACTION).toContain('node scripts/bundle-native-deps.mjs --platform "$PLATFORM" --arch "$ARCH"')
  })

  // 没有随包内核时必须在动手之前就明确失败：静默跳过等于让一个起不来的离线包出厂。
  it.skipIf(process.platform === 'win32')('fails loudly when the bundled core is missing', () => {
    mkdirSync(scratchRoot, { recursive: true })
    const repo = mkdtempSync(join(scratchRoot, 'native-cli-'))
    try {
      const result = spawnSync(process.execPath, [scriptPath, '--repo', repo, '--platform', 'linux', '--arch', 'arm64'], {
        encoding: 'utf8',
        timeout: 30_000,
      })
      expect(result.status).toBe(1)
      expect(result.stderr).toContain('BUNDLE_NATIVE:')
      expect(result.stderr).toContain('bundled core node_modules is missing')
    }
    finally {
      rmSync(repo, { recursive: true, force: true })
    }
  })
})
