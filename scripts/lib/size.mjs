/**
 * Directory size, in bytes.
 *
 * `du -sh` is not available everywhere and its output is for humans. This walks the tree and
 * returns a number, so callers can compare and sum without parsing text.
 */
import { readdirSync, statSync } from 'node:fs'
import { join } from 'node:path'

/** Total bytes under `path`. Symlinks are counted, not followed. */
export function duBytes(path) {
  let total = 0
  let entries
  try {
    entries = readdirSync(path, { withFileTypes: true })
  } catch {
    return 0
  }
  for (const entry of entries) {
    const child = join(path, entry.name)
    try {
      if (entry.isDirectory()) {
        total += duBytes(child)
      } else {
        total += statSync(child).size
      }
    } catch {
      // A file removed mid-walk, or one this process may not stat: skipping it is better than
      // reporting a size that is wrong for the whole directory.
    }
  }
  return total
}

/** "822 MB" — one decimal below ten units, none above, so columns line up. */
export function human(bytes) {
  const units = ['B', 'KB', 'MB', 'GB']
  let value = bytes
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024
    unit += 1
  }
  return `${value >= 10 || unit === 0 ? Math.round(value) : value.toFixed(1)} ${units[unit]}`
}
