/**
 * A kernel that stays alive and answers `initialize`.
 *
 * Exists for pool tests: eviction is about *which process gets killed*, so the fixtures must be
 * cheap to start, identical, and long-lived. A real kernel would make each assertion take
 * seconds and tell us nothing extra about the bookkeeping.
 */
import { createInterface } from 'node:readline'

createInterface({ input: process.stdin }).on('line', (line) => {
  let message
  try {
    message = JSON.parse(line)
  } catch {
    return
  }
  if (message.method === 'shutdown') {
    process.exit(0)
  }
  if (message.id !== undefined) {
    process.stdout.write(
      `${JSON.stringify({ jsonrpc: '2.0', id: message.id, result: { ok: true } })}\n`,
    )
  }
})

// Stay alive until asked to stop.
setInterval(() => {}, 1_000)
