import { beforeEach, describe, expect, it } from 'vitest'
import { emptyChat, reduceEvent, reduceEvents, type ChatState, type SessionEvent } from './reducer.ts'
import capture from './__fixtures__/events-sample.json' with { type: 'json' }

/**
 * The captured fixture is the point of this file: it is a real kernel stream
 * (`scripts/capture-events.mjs`), so the field names asserted below are the ones
 * the kernel actually sends, not the ones the protocol docs implied.
 */
const captured: SessionEvent[] = capture.events.map((entry) => entry.event)

let seq: number
beforeEach(() => {
  seq = 1000
})

function event(type: string, data: unknown, time = 1_700_000_000_000): SessionEvent {
  seq += 1
  return { type, seq, time, data }
}

/** Fold a fresh batch with deterministic sequences, so two batches compare cleanly. */
function fold(events: SessionEvent[]): ChatState {
  seq = 1000
  return reduceEvents(emptyChat, events)
}

describe('against a captured kernel stream', () => {
  const state = reduceEvents(emptyChat, captured)

  it('draws one user message for a prompt that commits five', () => {
    // Observed: the prompt plus four `<system-reminder>` context blocks all
    // arrive as `user/message`, distinguished only by `source.kind`.
    const userEvents = captured.filter((entry) => entry.type === 'user/message')
    expect(userEvents).toHaveLength(5)
    expect(state.messages.filter((message) => message.role === 'user')).toHaveLength(1)
    expect(state.messages[0]?.parts[0]?.text).toBe(capture.prompt)
  })

  it('uses the kernel message id rather than synthesising one', () => {
    expect(state.messages[0]?.id).not.toMatch(/^user-\d+$/)
  })

  it('takes the fallback session title', () => {
    expect(state.title).toBe(capture.prompt)
  })

  it('records the route from request/header', () => {
    expect(state.route).toEqual({ provider: 'opencode-go', model: 'deepseek-v4.1-flash' })
  })

  it('records the route the kernel actually used', () => {
    expect(state.route).toEqual({ provider: 'opencode-go', model: 'deepseek-v4.1-flash' })
  })

  it('renders the answer and its reasoning as separate blocks', () => {
    const assistant = state.messages.find((message) => message.role === 'assistant')
    expect(assistant?.parts.map((part) => part.kind)).toEqual(['reasoning', 'text'])
    expect(assistant?.parts.at(-1)?.text).toContain('deepseek-v4.1-flash')
  })

  it('completes a turn without reporting a failure', () => {
    expect(state.failure).toBeNull()
  })

  it('leaves no turn active once turn/end has folded', () => {
    expect(state.active).toBeNull()
  })

  it('advances lastSeq to the highest captured sequence', () => {
    expect(state.lastSeq).toBe(Math.max(...captured.map((entry) => entry.seq)))
  })

  it('does not draw a system message as a chat bubble', () => {
    expect(state.messages.some((message) => message.role === 'system')).toBe(false)
  })
})

describe('streaming deltas', () => {
  it('appends every text-delta to the same assistant message', () => {
    const state = fold([
      event('user/message', { id: 'u1', source: { kind: 'user' }, content: [{ type: 'text', text: 'q' }] }),
      event('assistant/live-chunk', { chunk: { type: 'text-delta', index: 0, text: 'He' } }),
      event('assistant/live-chunk', { chunk: { type: 'text-delta', index: 0, text: 'llo' } }),
      event('assistant/live-chunk', { chunk: { type: 'text-delta', index: 0, text: '!' } }),
    ])
    expect(state.messages).toHaveLength(2)
    expect(state.messages[1]?.parts).toEqual([{ kind: 'text', text: 'Hello!' }])
  })

  it('keeps reasoning apart from answer text', () => {
    const state = fold([
      event('assistant/live-chunk', { chunk: { type: 'reasoning-delta', index: 0, text: 'hmm' } }),
      event('assistant/live-chunk', { chunk: { type: 'text-delta', index: 1, text: 'answer' } }),
    ])
    expect(state.messages[0]?.parts).toEqual([
      { kind: 'reasoning', text: 'hmm' },
      { kind: 'text', text: 'answer' },
    ])
  })

  it('opens a new assistant message only after a user turn', () => {
    const state = fold([
      event('user/message', { id: 'u1', content: 'q1' }),
      event('assistant/live-chunk', { chunk: { type: 'text-delta', index: 0, text: 'a1' } }),
      event('user/message', { id: 'u2', content: 'q2' }),
      event('assistant/live-chunk', { chunk: { type: 'text-delta', index: 0, text: 'a2' } }),
    ])
    expect(state.messages.map((message) => message.role)).toEqual([
      'user',
      'assistant',
      'user',
      'assistant',
    ])
  })

  it('ignores chunk kinds that carry no text', () => {
    const base = fold([event('user/message', { id: 'u1', content: 'q' })])
    const after = fold([
      event('user/message', { id: 'u1', content: 'q' }),
      event('assistant/live-chunk', { chunk: { type: 'block-start', index: 0, blockType: 'text' } }),
      event('assistant/live-chunk', { chunk: { type: 'usage', usage: {} } }),
    ])
    expect(after.messages).toEqual(base.messages)
  })

  it('accumulates streamed tool arguments under one call', () => {
    const state = fold([
      event('user/message', { id: 'u1', content: 'q' }),
      event('assistant/live-chunk', { chunk: { type: 'tool-call-delta', index: 0, id: 'c1', name: 'bash', argumentsDelta: '{"comm' } }),
      event('assistant/live-chunk', { chunk: { type: 'tool-call-delta', index: 0, id: 'c1', argumentsDelta: 'and":"ls"}' } }),
    ])
    const tools = state.messages[1]?.tools ?? []
    expect(tools).toHaveLength(1)
    expect(tools[0]).toMatchObject({ callId: 'c1', name: 'bash', arguments: '{"command":"ls"}' })
  })
})

describe('durable attempt records', () => {
  it('replaces streamed text with the attempt record', () => {
    const state = fold([
      event('user/message', { id: 'u1', content: 'q' }),
      event('assistant/live-chunk', { chunk: { type: 'text-delta', index: 0, text: 'part' } }),
      event('assistant/attempt', {
        turn: 1,
        step: 1,
        stream: [{ chunk: { type: 'text-delta', index: 0, text: 'the full answer' } }],
      }),
    ])
    expect(state.messages[1]?.parts).toEqual([{ kind: 'text', text: 'the full answer' }])
  })

  it('surfaces a failure recorded inside the attempt stream', () => {
    const state = fold([
      event('assistant/attempt', {
        turn: 1,
        step: 1,
        stream: [
          {
            chunk: {
              type: 'finish',
              reason: { kind: 'error', failure: { message: 'no API key' } },
            },
          },
        ],
      }),
    ])
    expect(state.failure).toBe('no API key')
  })
})

describe('tool events (unverified shape)', () => {
  it('pairs a result with its call by callId', () => {
    const state = fold([
      event('user/message', { id: 'u1', content: 'q' }),
      event('tool/call', { callId: 'c1', name: 'bash', arguments: { command: 'ls' } }),
      event('tool/call', { callId: 'c2', name: 'read', arguments: { path: 'a' } }),
      event('tool/result', { callId: 'c1', output: 'file.txt' }),
    ])
    const tools = state.messages[1]?.tools ?? []
    expect(tools[0]).toMatchObject({ callId: 'c1', status: 'ok', output: 'file.txt' })
    expect(tools[1]).toMatchObject({ callId: 'c2', status: 'running', output: null })
  })

  it('marks a failed result as an error', () => {
    const state = fold([
      event('tool/call', { callId: 'c1', name: 'bash' }),
      event('tool/result', { callId: 'c1', isError: true, output: 'exit 1' }),
    ])
    expect(state.messages[0]?.tools[0]).toMatchObject({ status: 'error', output: 'exit 1' })
  })
})

describe('idempotence', () => {
  it('drops an event at or below the last folded sequence', () => {
    const first = event('user/message', { id: 'u1', content: 'hi' })
    const state = reduceEvent(emptyChat, first)
    expect(reduceEvent(state, first)).toBe(state)
  })

  it('is unchanged by replaying the captured batch', () => {
    const once = fold(captured)
    expect(reduceEvents(once, captured)).toEqual(once)
  })

  it('advances the sequence even for kinds it does not draw', () => {
    const state = fold([event('permission/preset', { preset: 'workspace-write' })])
    expect(state.messages).toHaveLength(0)
    expect(state.lastSeq).toBe(1001)
  })
})
