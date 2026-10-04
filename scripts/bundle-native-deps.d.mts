/**
 * `scripts/bundle-native-deps.mjs` 的类型声明。
 *
 * 与 `bundle-metadata.d.mts` 同一约定：脚本是给 CI 直接调用的纯 ESM，这里声明它导出
 * 的函数，让 `test/bundle-native-deps.test.ts` 的导入在 `pnpm typecheck` 下可解析。
 */

export type BundlePlatform = 'windows' | 'macos' | 'linux'
export type BundleArch = 'x64' | 'arm64'

export interface NativeProbeFailure {
  name: string
  message: string
}

export function platformToken(platform: BundlePlatform): string

export function nativePackagePlan(input: {
  dshDir: string
  platform: BundlePlatform
  arch: BundleArch
}): string[]

export function packageSpecName(spec: string): string

export function probeNativeModules(input: {
  node: string
  dshDir: string
}): NativeProbeFailure[]

export function seedNativePackages(input: {
  dshDir: string
  node: string
  platform: BundlePlatform
  arch: BundleArch
}): { ok: boolean, seeded: string[], probe: NativeProbeFailure[] }
