/**
 * Apply `kernel-patches/*.patch` to the fetched kernel.
 *
 * The kernel is not vendored — 819 MB of `node_modules` do not belong in git — but it
 * must still be changeable. Three layers, in increasing cost:
 *
 *   1. **A plugin** (`bridge/`) mounted through the profile patch. This is how the
 *      client already modifies kernel behaviour, and it survives every upgrade.
 *   2. **A patch here.** For a change that has to be *inside* a kernel module — a
 *      bug fix before upstream ships it, a log line, a hard-coded path. Patches live
 *      in the repository, apply to the pinned version, and are re-applied on every
 *      fetch.
 *   3. **An upstream PR.** Anything that stays patched for more than a release
 *      should go upstream instead, so this list stays short and honest.
 *
 * A patch that does not apply is a hard failure: silently building against a
 * half-patched kernel is how "it works on my machine" happens.
 */
import { execFileSync } from 'node:child_process'
import { existsSync, readdirSync, statSync } from 'node:fs'
import { join } from 'node:path'

export function applyPatches({ kernelDir, patchDir }) {
  if (!existsSync(patchDir)) {
    console.log('[kernel] no patches directory — using the kernel as published')
    return []
  }

  const patches = readdirSync(patchDir)
    .filter((name) => name.endsWith('.patch'))
    .sort()
    .map((name) => join(patchDir, name))

  if (patches.length === 0) {
    console.log('[kernel] no patches to apply')
    return []
  }

  for (const patch of patches) {
    const size = statSync(patch).size
    try {
      execFileSync('git', ['apply', '--directory=.', '--verbose', patch], {
        cwd: kernelDir,
        stdio: 'inherit',
        shell: process.platform === 'win32',
      })
      console.log(`[kernel] applied ${patch.split(/[\\/]/).pop()} (${size} bytes)`)
    } catch {
      throw new Error(
        `patch failed: ${patch}\n` +
          '  The kernel version probably changed shape. Rebase the patch on the pinned\n' +
          '  version, or drop it if upstream fixed the same thing.',
      )
    }
  }

  return patches
}
