/**
 * The session event reducer.
 *
 * Kernel events arrive as a flat, ordered stream of `{type, seq, time, data}`.
 * This module turns that into what a chat view renders, and it is deliberately
 * free of React: these rules are what break when they break, so they are tested
 * against a captured stream (`__fixtures__/events-sample.json`) rather than
 * against what the protocol docs suggest the shapes are.
 *
 * Shapes below are the ones observed in that capture. The two marked
 * *unverified* have not been seen from a live kernel yet — they come from the
 * upstream type definitions and need a real run with an API key to confirm.
 */

/** One durable kernel event, as it appears on the wire. */
export interface SessionEvent {
  type: string
  seq: number
  time: number
  data: unknown
}

export type Role = 'user' | 'assistant' | 'system'

export interface MessagePart {
  kind: 'text' | 'reasoning'
  text: string
}

export interface ToolCall {
  /** Provider call id, once the stream names one. */
  callId: string
  name: string
  /** Raw JSON string; the kernel streams arguments as text. */
  arguments: string
  status: 'running' | 'ok' | 'error'
  output: string | null
}

export interface Message {
  /** The kernel's message id for user turns, a synthetic key for assistant turns. */
  id: string
  role: Role
  parts: MessagePart[]
  tools: ToolCall[]
  time: number
  seq: number
}

export interface ChatState {
  messages: Message[]
  /** Highest sequence folded in; makes the reducer idempotent across replays. */
  lastSeq: number
  /** Title from `session/title` (the fallback title is the prompt's first line). */
  title: string | null
  /** `{turn, step}` of the running turn; `null` when idle. */
  active: { turn: number; step: number } | null
  /** Route the kernel reported in `request/header`, for the status bar. */
  route: { provider: string; model: string } | null
  /** Last failure, from `turn/end` or a failed attempt. */
  failure: string | null
}

export const emptyChat: ChatState = {
  messages: [],
  lastSeq: -1,
  title: null,
  active: null,
  route: null,
  failure: null,
}

/**
 * Fold one event into the state.
 *
 * Three rules carry the weight:
 *
 * 1. **Idempotent by `seq`.** Replays and reconnects must not duplicate text.
 * 2. **`assistant/live-chunk` appends to the open assistant message.** A reply is
 *    hundreds of these; treating each as a message renders one answer as many.
 * 3. **Only `source.kind === 'user'` messages are the user's.** One prompt also
 *    commits the workspace instructions, the session block, the skill catalog
 *    and the clock — four more `user/message` events that are context, not
 *    conversation.
 */
export function reduceEvent(state: ChatState, event: SessionEvent): ChatState {
  if (event.seq <= state.lastSeq) return state
  const next: ChatState = { ...state, lastSeq: event.seq }

  switch (event.type) {
    case 'session/title': {
      const title = asString(read(event.data, 'title'))
      return title === null ? next : { ...next, title }
    }

    case 'user/message':
      return isUserAuthored(event.data)
        ? { ...next, messages: [...state.messages, userMessage(event)] }
        : next

    case 'assistant/live-chunk':
      return foldLiveChunk(next, event)

    case 'assistant/attempt':
      // Durable record of one attempt, including the chunks that were streamed.
      return finalizeAttempt(next, event)

    case 'turn/start':
      return { ...next, active: { turn: asNumber(read(event.data, 'turn')) ?? 1, step: 0 }, failure: null }

    case 'step/start':
      return {
        ...next,
        active: {
          turn: asNumber(read(event.data, 'turn')) ?? 1,
          step: asNumber(read(event.data, 'step')) ?? 1,
        },
      }

    case 'turn/end': {
      const reason = read(event.data, 'reason')
      const error = read(reason, 'error')
      // A failed turn reports through `reason.error.message`; a clean one has no reason.
      return { ...next, active: null, failure: asString(read(error, 'message')) }
    }

    case 'request/header': {
      // Observed shape: `data.header.config.{provider,model}`, not `data.header`.
      const config = read(read(event.data, 'header'), 'config')
      const provider = asString(read(config, 'provider'))
      const model = asString(read(config, 'model'))
      if (provider === null || model === null) return next
      return { ...next, route: { provider, model } }
    }

    // Unverified: seen only in upstream type definitions so far.
    case 'tool/call':
      return appendTool(next, event)

    case 'tool/result':
      return settleTool(next, event)

    default:
      // Unhandled kinds still advance `lastSeq`: they are durable history, just
      // not something this view draws yet.
      return next
  }
}

/** Fold a batch. Idempotent, and later events win. */
export function reduceEvents(state: ChatState, events: readonly SessionEvent[]): ChatState {
  return events.reduce(reduceEvent, state)
}

function userMessage(event: SessionEvent): Message {
  return {
    id: asString(read(event.data, 'id')) ?? `user-${event.seq}`,
    role: 'user',
    parts: [{ kind: 'text', text: contentText(event.data) ?? '' }],
    tools: [],
    time: event.time,
    seq: event.seq,
  }
}

/** `assistant/live-chunk` carries one `StreamChunk`; only deltas and tool calls matter here. */
function foldLiveChunk(state: ChatState, event: SessionEvent): ChatState {
  const chunk = read(event.data, 'chunk')
  const kind = asString(read(chunk, 'type'))

  switch (kind) {
    case 'text-delta':
      return appendText(state, event, 'text', asString(read(chunk, 'text')) ?? '')
    case 'reasoning-delta':
      return appendText(state, event, 'reasoning', asString(read(chunk, 'text')) ?? '')
    case 'tool-call-delta': {
      const callId = asString(read(chunk, 'id')) ?? `call-${event.seq}`
      const name = asString(read(chunk, 'name'))
      const delta = asString(read(chunk, 'argumentsDelta')) ?? ''
      return patchTools(state, event, (tools) => {
        const index = tools.findIndex((tool) => tool.callId === callId)
        if (index === -1) {
          return [
            ...tools,
            { callId, name: name ?? 'tool', arguments: delta, status: 'running', output: null },
          ]
        }
        return tools.map((tool, at) =>
          at === index
            ? {
                ...tool,
                name: name ?? tool.name,
                arguments: tool.arguments + delta,
              }
            : tool,
        )
      })
    }
    default:
      // `block-start`, `block-end`, `usage`, `finish` carry no renderable text.
      return state
  }
}

/** Replace the accumulated deltas with the attempt's own durable text. */
function finalizeAttempt(state: ChatState, event: SessionEvent): ChatState {
  const stream = read(event.data, 'stream')
  if (!Array.isArray(stream)) return state

  const text = stream
    .map((entry) => {
      const chunk = read(entry, 'chunk')
      return asString(read(chunk, 'type')) === 'text-delta' ? (asString(read(chunk, 'text')) ?? '') : ''
    })
    .join('')

  const failure = stream
    .map((entry) => {
      const chunk = read(entry, 'chunk')
      if (asString(read(chunk, 'type')) !== 'finish') return null
      return asString(read(read(read(chunk, 'reason'), 'failure'), 'message'))
    })
    .find((message) => message !== null)

  const settled = text === '' ? state : replaceLastAssistant(state, event, text)
  return failure === undefined || failure === null ? settled : { ...settled, failure }
}

function appendText(
  state: ChatState,
  event: SessionEvent,
  kind: MessagePart['kind'],
  delta: string,
): ChatState {
  if (delta === '') return state

  const messages = [...state.messages]
  const last = messages.at(-1)

  if (last === undefined || last.role !== 'assistant') {
    messages.push({
      id: `assistant-${event.seq}`,
      role: 'assistant',
      parts: [{ kind, text: delta }],
      tools: [],
      time: event.time,
      seq: event.seq,
    })
    return { ...state, messages }
  }

  const at = last.parts.findIndex((part) => part.kind === kind)
  const parts =
    at === -1
      ? [...last.parts, { kind, text: delta }]
      : last.parts.map((part, index) => (index === at ? { ...part, text: part.text + delta } : part))
  messages[messages.length - 1] = { ...last, parts, seq: event.seq }
  return { ...state, messages }
}

function replaceLastAssistant(state: ChatState, event: SessionEvent, text: string): ChatState {
  const messages = [...state.messages]
  const last = messages.at(-1)
  if (last === undefined || last.role !== 'assistant') {
    messages.push({
      id: `assistant-${event.seq}`,
      role: 'assistant',
      parts: [{ kind: 'text', text }],
      tools: [],
      time: event.time,
      seq: event.seq,
    })
    return { ...state, messages }
  }
  const others = last.parts.filter((part) => part.kind !== 'text')
  messages[messages.length - 1] = {
    ...last,
    parts: [{ kind: 'text', text }, ...others],
    seq: event.seq,
  }
  return { ...state, messages }
}

function appendTool(state: ChatState, event: SessionEvent): ChatState {
  const call: ToolCall = {
    callId: asString(read(event.data, 'callId')) ?? `call-${event.seq}`,
    name: asString(read(event.data, 'name')) ?? 'tool',
    arguments: JSON.stringify(read(event.data, 'arguments') ?? null),
    status: 'running',
    output: null,
  }
  return patchTools(state, event, (tools) => [...tools, call])
}

function settleTool(state: ChatState, event: SessionEvent): ChatState {
  const callId = asString(read(event.data, 'callId'))
  const output =
    asString(read(event.data, 'output')) ?? JSON.stringify(read(event.data, 'result') ?? null)
  const failed = read(event.data, 'isError') === true

  return {
    ...state,
    messages: state.messages.map((message) => ({
      ...message,
      tools: message.tools.map((tool) =>
        tool.callId === callId || (callId === null && tool.status === 'running')
          ? { ...tool, status: failed ? 'error' : 'ok', output }
          : tool,
      ),
    })),
  }
}

/** Apply a tool mutation to the assistant turn that owns it, opening one if needed. */
function patchTools(
  state: ChatState,
  event: SessionEvent,
  mutate: (tools: ToolCall[]) => ToolCall[],
): ChatState {
  const messages = [...state.messages]
  const last = messages.at(-1)
  if (last === undefined || last.role !== 'assistant') {
    const tools = mutate([])
    if (tools.length === 0) return state
    messages.push({
      id: `assistant-${event.seq}`,
      role: 'assistant',
      parts: [],
      tools,
      time: event.time,
      seq: event.seq,
    })
    return { ...state, messages }
  }
  messages[messages.length - 1] = { ...last, tools: mutate(last.tools), seq: event.seq }
  return { ...state, messages }
}

function isUserAuthored(data: unknown): boolean {
  const source = read(data, 'source')
  // Missing source is treated as the user's own message rather than dropped.
  if (source === undefined || source === null) return true
  return read(source, 'kind') === 'user'
}

function contentText(data: unknown): string | null {
  const content = read(data, 'content')
  if (typeof content === 'string') return content
  if (Array.isArray(content)) {
    const parts = content
      .map((block) => (typeof block === 'string' ? block : asString(read(block, 'text'))))
      .filter((text): text is string => text !== null)
    return parts.length === 0 ? null : parts.join('')
  }
  return asString(read(data, 'text'))
}

function read(value: unknown, key: string): unknown {
  if (typeof value !== 'object' || value === null) return undefined
  return (value as Record<string, unknown>)[key]
}

function asString(value: unknown): string | null {
  return typeof value === 'string' ? value : null
}

function asNumber(value: unknown): number | null {
  return typeof value === 'number' ? value : null
}
