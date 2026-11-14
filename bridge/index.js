/**
 * dsh-desktop live bridge.
 *
 * The SDK wire (`session.event`) carries durable events only, so an answer
 * arrives whole when the turn ends. The kernel does produce a live stream — the
 * global `agent/assistant-stream` event — but nothing forwards it to an SDK
 * client.
 *
 * This plugin is that forwarder. It is a kernel-side plugin loaded by profile
 * patch, not a source change: the pinned kernel stays untouched and upgradeable.
 *
 * Why stderr instead of stdout: stdout belongs to the JSON-RPC transport, and a
 * second writer there can interleave half-frames. stderr is already a separate
 * pipe the client reads, so tagged lines cost nothing and cannot corrupt
 * protocol data.
 *
 * Wire format, one line each:
 *   \u0001DSH-LIVE <json>
 *
 * where <json> is
 *   { "kind": "start", "sessionId", "attemptId", "turn", "step" }
 *   { "kind": "chunk", "sessionId", "attemptId", "index", "chunk" }
 *   { "kind": "end",   "sessionId", "attemptId", "outcome" }
 */

const PREFIX = '\u0001DSH-LIVE '

export const name = 'dsh-desktop-live-bridge'

/**
 * The event is declared `{ global: true }` by its producer, so a subscriber must
 * ask for the same scope or it only sees agents it created — which is none.
 */
export function apply(ctx) {
  ctx.on(
    'agent/assistant-stream',
    ({ agent, frame }) => {
      const sessionId = agent?.session?.id
      if (sessionId === undefined || frame === undefined) return

      switch (frame.type) {
        case 'start':
          emit({
            kind: 'start',
            sessionId,
            attemptId: frame.attemptId,
            turn: frame.turn,
            step: frame.step,
          })
          break
        case 'chunk':
          emit({
            kind: 'chunk',
            sessionId,
            attemptId: frame.attemptId,
            index: frame.index,
            chunk: frame.chunk,
          })
          break
        case 'end':
          emit({
            kind: 'end',
            sessionId,
            attemptId: frame.attemptId,
            outcome: frame.outcome,
          })
          break
        default:
          break
      }
    },
    { global: true },
  )
}

function emit(payload) {
  try {
    process.stderr.write(`${PREFIX}${JSON.stringify(payload)}\n`)
  } catch {
    // A broken diagnostics pipe must never take the agent down with it.
  }
}
