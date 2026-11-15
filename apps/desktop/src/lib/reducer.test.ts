import { readFileSync } from 'node:fs'
import { beforeEach, describe, expect, it } from 'vitest'
import {
  applyLiveChunk,
  emptyChat,
  hydrate,
  reduceEvent,
  reduceEvents,
  type ChatState,
  type SessionEvent,
} from './reducer.ts'
import capture from './__fixtures__/events-sample.json' with { type: 'json' }

/**
 * The captured fixture is the point of this file: it is a real kernel stream
 * (`scripts/capture-events.mjs`), so the field names asserted below are the ones
 * the kernel actually sends, not the ones the protocol docs implied.
 */
const captured: SessionEvent[] = capture.events.map((entry) => entry.event)
const live = capture.live as Array<{ kind: string; sessionId: string; chunk?: unknown }>

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

  it('renders the answer text from the durable message', () => {
    const assistant = state.messages.find((message) => message.role === 'assistant')
    // Block order is the kernel's: this capture answered without reasoning, so
    // the assertion is about the text arriving, not about a fixed block list.
    expect(assistant?.parts.at(-1)?.kind).toBe('text')
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
    expect(state.messages[1]?.parts).toMatchObject([{ kind: 'text', text: 'Hello!' }])
  })

  it('keeps reasoning apart from answer text', () => {
    const state = fold([
      event('assistant/live-chunk', { chunk: { type: 'reasoning-delta', index: 0, text: 'hmm' } }),
      event('assistant/live-chunk', { chunk: { type: 'text-delta', index: 1, text: 'answer' } }),
    ])
    expect(state.messages[0]?.parts).toMatchObject([
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
    expect(state.messages[1]?.parts).toMatchObject([{ kind: 'text', text: 'the full answer' }])
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

describe('live stream from the kernel-side bridge', () => {
  it('captured live frames carry the answer as text deltas', () => {
    const deltas = live
      .filter((frame) => frame.kind === 'chunk')
      .map((frame) => frame.chunk as { type: string; text?: string })
      .filter((chunk) => chunk.type === 'text-delta')
      .map((chunk) => chunk.text ?? '')

    expect(deltas.length).toBeGreaterThan(1)
    expect(deltas.join('')).toContain('deepseek-v4.1-flash')
  })

  it('streams text incrementally before the durable message lands', () => {
    // The whole point of the bridge: something readable exists mid-turn.
    let state = reduceEvents(emptyChat, captured.filter((entry) => entry.type === 'user/message'))
    for (const frame of live) {
      if (frame.kind !== 'chunk') continue
      state = applyLiveChunk(state, frame.chunk)
    }
    const assistant = state.messages.find((message) => message.role === 'assistant')
    expect(assistant).toBeDefined()
    expect(assistant?.parts.at(-1)?.text).toContain('deepseek-v4.1-flash')
  })

  it('does not advance the durable sequence', () => {
    const base = reduceEvents(emptyChat, captured)
    const after = applyLiveChunk(base, { type: 'text-delta', index: 0, text: 'x' })
    expect(after.lastSeq).toBe(base.lastSeq)
  })

  it('appends successive deltas to one message', () => {
    let state = emptyChat
    for (const text of ['a', 'b', 'c']) {
      state = applyLiveChunk(state, { type: 'text-delta', index: 0, text })
    }
    expect(state.messages).toHaveLength(1)
    expect(state.messages[0]?.parts).toMatchObject([{ kind: 'text', text: 'abc' }])
  })

  it('keeps reasoning deltas in their own part', () => {
    let state = emptyChat
    state = applyLiveChunk(state, { type: 'reasoning-delta', index: 0, text: 'think' })
    state = applyLiveChunk(state, { type: 'text-delta', index: 1, text: 'answer' })
    expect(state.messages[0]?.parts.map((part) => part.kind)).toEqual(['reasoning', 'text'])
  })

  it('ignores chunk kinds that carry no text', () => {
    const state = applyLiveChunk(emptyChat, { type: 'usage', usage: { inputTokens: 1 } })
    expect(state.messages).toHaveLength(0)
  })
})

describe('turn lifecycle', () => {
  it('clears the previous failure when a new turn starts', () => {
    let state = reduceEvent(emptyChat, event('turn/end', { reason: { kind: 'error', error: { message: 'boom' } } }))
    expect(state.failure).toBe('boom')
    state = reduceEvent(state, event('turn/start', { turn: 2 }))
    expect(state.failure).toBeNull()
  })

  it('reports no failure for a completed turn', () => {
    const state = reduceEvent(emptyChat, event('turn/end', { reason: { kind: 'completed' } }))
    expect(state.failure).toBeNull()
    expect(state.active).toBeNull()
  })
})

describe('reasoning survives the durable message', () => {
  it('keeps streamed reasoning when the durable message omits it', () => {
    let state = emptyChat
    state = applyLiveChunk(state, { type: 'reasoning-delta', index: 0, text: '先算脚数' })
    state = applyLiveChunk(state, { type: 'text-delta', index: 1, text: '鸡' })
    state = reduceEvent(state, event('assistant/message', {
      message: { role: 'assistant', content: [{ type: 'text', text: '鸡 23 只，兔 12 只' }] },
    }))
    expect(state.messages[0]?.parts).toMatchObject([
      { kind: 'reasoning', text: '先算脚数' },
      { kind: 'text', text: '鸡 23 只，兔 12 只' },
    ])
  })

  it('prefers the durable reasoning when it carries one', () => {
    let state = emptyChat
    state = applyLiveChunk(state, { type: 'reasoning-delta', index: 0, text: 'partial' })
    state = reduceEvent(state, event('assistant/message', {
      message: {
        role: 'assistant',
        content: [
          { type: 'reasoning', text: 'the full thought' },
          { type: 'text', text: 'answer' },
        ],
      },
    }))
    expect(state.messages[0]?.parts).toMatchObject([
      { kind: 'reasoning', text: 'the full thought' },
      { kind: 'text', text: 'answer' },
    ])
  })
})

describe('tool events from a captured run', () => {
  const tools = JSON.parse(
    readFileSync(new URL('./__fixtures__/tools-sample.json', import.meta.url), 'utf8'),
  ) as SessionEvent[]

  it('keeps the kernel arguments string intact instead of re-encoding it', () => {
    const state = fold([event('user/message', { id: 'u', content: 'q' })])
    const after = tools.reduce(reduceEvent, state)
    const first = after.messages[1]?.tools[0]
    const raw = tools[0]?.data as { arguments?: unknown }
    expect(first?.arguments).toBe(raw.arguments)
    expect(JSON.parse(first!.arguments)).toHaveProperty('command')
  })

  it('reads the outcome out of the tool message rather than flat fields', () => {
    const state = tools.reduce(reduceEvent, fold([event('user/message', { id: 'u', content: 'q' })]))
    const first = state.messages[1]?.tools[0]
    expect(first?.status).toBe('ok')
    expect(first?.output).toContain('dsh-outside-test')
  })

  it('settles each result against its own call', () => {
    const state = tools.reduce(reduceEvent, fold([event('user/message', { id: 'u', content: 'q' })]))
    const calls = state.messages[1]?.tools ?? []
    expect(calls).toHaveLength(2)
    expect(new Set(calls.map((call) => call.callId)).size).toBe(2)
    expect(calls.every((call) => call.status === 'ok')).toBe(true)
  })
})

describe('a failed model call ends the turn', () => {
  it('clears the running state so the view stops claiming it is generating', () => {
    // The screenshot case: turn started, the model request timed out, and the
    // kernel posted the attempt error without a turn/end — the window showed the
    // failure banner and "generating" at the same time.
    let state = reduceEvent(emptyChat, event('turn/start', { turn: 1 }))
    expect(state.active).not.toBeNull()

    state = reduceEvent(state, event('assistant/attempt', {
      turn: 1,
      step: 1,
      stream: [
        {
          chunk: {
            type: 'finish',
            reason: { kind: 'error', failure: { message: 'Connect Timeout Error (UND_ERR_CONNECT_TIMEOUT)' } },
          },
        },
      ],
    }))

    expect(state.failure).toContain('CONNECT_TIMEOUT')
    expect(state.active).toBeNull()
  })

  it('lets a retry reopen the turn', () => {
    let state = reduceEvent(emptyChat, event('turn/start', { turn: 1 }))
    state = reduceEvent(state, event('assistant/attempt', {
      turn: 1,
      step: 1,
      stream: [{ chunk: { type: 'finish', reason: { kind: 'error', failure: { message: 'boom' } } } }],
    }))
    state = reduceEvent(state, event('turn/start', { turn: 2 }))
    expect(state.active).not.toBeNull()
    expect(state.failure).toBeNull()
  })
})

describe('arrival order', () => {
  it('splits reasoning around a tool call instead of merging it into one block', () => {
    // Explicit sequences: this is about ordering, and the shared helpers reset
    // their counter per fold, which hides the very thing under test.
    let state = reduceEvent(emptyChat, { type: 'user/message', seq: 1, time: 0, data: { id: 'u', content: 'q' } })
    state = applyLiveChunk(state, { type: 'reasoning-delta', index: 0, text: 'first thought' })
    state = reduceEvent(state, {
      type: 'tool/call',
      seq: 2,
      time: 0,
      data: { callId: 'c1', name: 'pwsh', arguments: '{}' },
    })
    state = applyLiveChunk(state, { type: 'reasoning-delta', index: 0, text: 'second thought' })

    const parts = state.messages[1]?.parts ?? []
    expect(parts.map((part) => part.text)).toEqual(['first thought', 'second thought'])

    const tool = state.messages[1]?.tools[0]
    expect(tool?.seq).toBe(2)
    expect(parts[0]?.seq).toBeLessThan(tool!.seq)
    expect(parts[1]?.seq).toBeGreaterThanOrEqual(tool!.seq - 1)
  })

  it('keeps appending to the trailing thought while no tool intervenes', () => {
    let state = emptyChat
    state = applyLiveChunk(state, { type: 'reasoning-delta', index: 0, text: 'a' })
    state = applyLiveChunk(state, { type: 'reasoning-delta', index: 0, text: 'b' })
    expect(state.messages[0]?.parts).toHaveLength(1)
  })
})

describe('hydrate', () => {
  it('rebuilds a conversation from a stored log', () => {
    // Opening a session replays durable rows through the same reducer as a live
    // turn, so history and streaming cannot disagree.
    const stored = [
      { type: 'user/message', seq: 0, time: 10, data: { id: 'u1', source: { kind: 'user' }, content: [{ type: 'text', text: '旧的问题' }] } },
      { type: 'assistant/message', seq: 1, time: 20, data: { message: { role: 'assistant', content: [{ type: 'reasoning', text: '想过' }, { type: 'text', text: '旧的回答' }] } } },
      { type: 'tool/call', seq: 2, time: 30, data: { callId: 'c1', name: 'pwsh', arguments: '{"command":"ls"}' } },
      { type: 'tool/result', seq: 3, time: 40, data: { message: { toolCallId: 'c1', content: [{ type: 'text', text: 'file.txt' }] } } },
    ]

    const state = hydrate(stored)

    expect(state.messages).toHaveLength(2)
    expect(state.messages[0]?.parts[0]?.text).toBe('旧的问题')
    expect(state.messages[1]?.parts.map((part) => part.kind)).toEqual(['reasoning', 'text'])
    expect(state.messages[1]?.tools[0]).toMatchObject({ name: 'pwsh', status: 'ok', output: 'file.txt' })
    expect(state.lastSeq).toBe(3)
  })

  it('takes the title from the stored log', () => {
    const state = hydrate([
      { type: 'session/title', seq: 0, time: 1, data: { title: '旧会话' } },
      { type: 'user/message', seq: 1, time: 2, data: { id: 'u', content: 'q' } },
    ])
    expect(state.title).toBe('旧会话')
  })
})
