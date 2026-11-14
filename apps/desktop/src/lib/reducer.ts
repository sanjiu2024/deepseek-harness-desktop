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

    case 'assistant/message':
      return settleAssistantMessage(next, event)

    case 'assistant/live-chunk':
      return foldLiveChunk(next, event)

    case 'assistant/attempt':
      // Durable record of one attempt, including the chunks that were streamed.
      return finalizeAttempt(next, event)

    case 'turn/start':
      return {
        ...next,
        active: { turn: asNumber(read(event.data, 'turn')) ?? 1, step: 0 },
        failure: null,
      }

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

/**
 * `assistant/live-chunk` carries one `StreamChunk`.
 *
 * **This event does not exist on the SDK wire.** It is a Web-client presentation
 * (`dsh-api-session-controller`), and the SDK server forwards only durable
 * `session/event` rows. It is handled here anyway because a future kernel-side
 * forwarder can re-emit it, and because a fake in tests is how the append rules
 * get exercised without a network. Until that forwarder exists, an answer
 * arrives whole, in {@link settleAssistantMessage}.
 */
/**
 * Fold one live stream chunk into the open assistant message.
 *
 * Separate from {@link reduceEvent} on purpose: live frames have no durable
 * sequence, so they must not touch `lastSeq`. Giving them a synthetic seq would
 * either collide with the next durable event or force the gate to learn an
 * exception — both worse than a second entry point.
 */
export function applyLiveChunk(state: ChatState, chunk: unknown, time = Date.now()): ChatState {
  const kind = asString(read(chunk, 'type'))
  const synthetic = { seq: state.lastSeq, time, data: chunk } as SessionEvent

  switch (kind) {
    case 'text-delta':
      return appendText(state, synthetic, 'text', asString(read(chunk, 'text')) ?? '')
    case 'reasoning-delta':
      return appendText(state, synthetic, 'reasoning', asString(read(chunk, 'text')) ?? '')
    case 'tool-call-delta': {
      const callId = asString(read(chunk, 'id')) ?? `call-${state.lastSeq + 1}`
      const name = asString(read(chunk, 'name'))
      const delta = asString(read(chunk, 'argumentsDelta')) ?? ''
      return patchTools(state, synthetic, (tools) => {
        const at = tools.findIndex((tool) => tool.callId === callId)
        if (at === -1) {
          return [
            ...tools,
            { callId, name: name ?? 'tool', arguments: delta, status: 'running', output: null },
          ]
        }
        return tools.map((tool, index) =>
          index === at ? { ...tool, name: name ?? tool.name, arguments: tool.arguments + delta } : tool,
        )
      })
    }
    default:
      // `block-start`, `block-end`, `usage`, `finish` carry no renderable text.
      return state
  }
}

/**
 * `assistant/live-chunk` as a durable-shaped event.
 *
 * Kept for the case where the kernel itself publishes the Web-shaped event; the
 * desktop bridge sends raw `StreamChunk`s through {@link applyLiveChunk}.
 */
function foldLiveChunk(state: ChatState, event: SessionEvent): ChatState {
  return applyLiveChunk(state, read(event.data, 'chunk'), event.time)
}

/**
 * The durable assistant message. This is the only place an answer arrives over
 * the SDK wire — see the note on {@link foldLiveChunk} about why.
 *
 * Content is a block array, and a reasoning block is a separate block, not a
 * prefix of the answer.
 */
function settleAssistantMessage(state: ChatState, event: SessionEvent): ChatState {
  const message = read(event.data, 'message')
  const content = read(message, 'content')
  if (!Array.isArray(content)) return state

  const parts: MessagePart[] = []
  for (const block of content) {
    const kind = asString(read(block, 'type'))
    const text = asString(read(block, 'text'))
    if (text === null) continue
    if (kind === 'text' || kind === 'reasoning') parts.push({ kind, text })
  }
  if (parts.length === 0) return state

  const messages = [...state.messages]
  const last = messages.at(-1)
  if (last === undefined || last.role !== 'assistant') {
    messages.push({
      id: `assistant-${event.seq}`,
      role: 'assistant',
      parts,
      tools: [],
      time: event.time,
      seq: event.seq,
    })
    return { ...state, messages }
  }
  // The durable message wins for what it carries, but some routes omit
  // reasoning from it; keeping the streamed reasoning avoids erasing a
  // thought the user already watched arrive.
  messages[messages.length - 1] = { ...last, parts: mergeParts(parts, last.parts), seq: event.seq }
  return { ...state, messages }
}

/**
 * Reconcile the durable message with what the stream produced.
 *
 * The durable message is authoritative for the content it carries. It is not
 * always complete, though: some provider routes omit reasoning from it while
 * the stream delivered those deltas. In that case the streamed reasoning is kept,
 * because replacing parts wholesale would erase a thought the user already
 * watched arrive.
 */
function mergeParts(durable: MessagePart[], streamed: MessagePart[]): MessagePart[] {
  if (durable.some((part) => part.kind === 'reasoning')) return durable
  const reasoning = streamed.filter((part) => part.kind === 'reasoning')
  return [...reasoning, ...durable]
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
  const raw = read(event.data, 'arguments')
  const call: ToolCall = {
    callId: asString(read(event.data, 'callId')) ?? `call-${event.seq}`,
    name: asString(read(event.data, 'name')) ?? 'tool',
    // `arguments` arrives already serialised as a JSON string; re-encoding a
    // string would double-quote the whole payload.
    arguments: typeof raw === 'string' ? raw : JSON.stringify(raw ?? null),
    status: 'running',
    output: null,
  }
  return patchTools(state, event, (tools) => [...tools, call])
}

/**
 * `tool/result` carries the outcome inside a tool message, not in flat fields:
 * `{ message: { toolCallId, content: [{ type: 'text', text }], isError? } }`.
 */
function settleTool(state: ChatState, event: SessionEvent): ChatState {
  const message = read(event.data, 'message')
  const callId =
    asString(read(event.data, 'toolCallId')) ??
    asString(read(message, 'toolCallId')) ??
    asString(read(read(message, 'source'), 'callId'))

  const content = read(message, 'content')
  const output = Array.isArray(content)
    ? content
        .map((block) => asString(read(block, 'text')) ?? '')
        .join('')
    : asString(content)

  const failed = read(message, 'isError') === true

  return {
    ...state,
    messages: state.messages.map((message) => ({
      ...message,
      tools: message.tools.map((tool) =>
        tool.callId === callId || (callId === null && tool.status === 'running')
          ? { ...tool, status: failed ? 'error' : 'ok', output: output ?? '' }
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
