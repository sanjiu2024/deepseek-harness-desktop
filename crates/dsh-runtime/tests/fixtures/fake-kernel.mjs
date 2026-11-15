/**
 * A fake kernel for the approval round-trip test.
 *
 * It speaks just enough of the SDK wire to prove one thing: when the kernel sends
 * a request with a **string** id, this client classifies it as a request and
 * answers with the same id, quoted. No model, no sandbox, no timing.
 *
 * Prints `OK <id> <outcome>` and exits 0 on success, `FAIL <reason>` and exits 1
 * otherwise, so the Rust side can assert on one line.
 */
import { createInterface } from 'node:readline'

const REQUEST_ID = 'req_0123456789abcdef'

process.stdout.write(
  `${JSON.stringify({
    jsonrpc: '2.0',
    id: 1,
    result: { serverInfo: { name: 'fake-kernel', version: '0.0.0' } },
  })}\n`,
)

const lines = createInterface({ input: process.stdin })
lines.on('line', (line) => {
  let message
  try {
    message = JSON.parse(line)
  } catch {
    process.stdout.write('FAIL unparseable-line\n')
    process.exit(1)
  }

  // The client's initialize.
  if (message.id === 1 && message.method === 'initialize') {
    process.stdout.write(
      `${JSON.stringify({ jsonrpc: '2.0', id: 1, result: { ok: true } })}\n`,
    )
    // Ask for approval with a string id, exactly like the real kernel.
    process.stdout.write(
      `${JSON.stringify({
        jsonrpc: '2.0',
        id: REQUEST_ID,
        method: 'approval/request',
        params: { sessionId: 'fake', toolName: 'pwsh', callId: 'call_1', reason: 'test' },
      })}\n`,
    )
    return
  }

  // The client's answer must come back with the same id, as a JSON string.
  if (message.id === REQUEST_ID && message.result !== undefined) {
    const outcome = message.result.outcome
    process.stdout.write(`OK ${message.id} ${outcome}\n`)
    setTimeout(() => process.exit(0), 50)
    return
  }

  if (message.id !== undefined) {
    process.stdout.write(`FAIL answer-shape ${JSON.stringify(message)}\n`)
    process.exit(1)
  }
})

setTimeout(() => {
  process.stdout.write('FAIL timeout\n')
  process.exit(1)
}, 20_000)
