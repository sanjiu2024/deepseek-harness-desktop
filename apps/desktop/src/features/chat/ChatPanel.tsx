import { useEffect, useMemo, useRef, useState } from 'react'
import { onKernelFrame, promptSession, type KernelFrame } from '@/lib/ipc.ts'
import {
  applyLiveChunk,
  emptyChat,
  reduceEvent,
  type ChatState,
  type Message,
  type SessionEvent,
} from '@/lib/reducer.ts'

/**
 * M1 chat surface: a real turn, streamed into real messages.
 *
 * It is deliberately plain — no virtual list, no markdown, no tool cards yet.
 * Those arrive once the event shapes below are confirmed against a live kernel
 * rather than guessed from the protocol docs.
 */
export function ChatPanel({ sessionId }: { sessionId: string }) {
  const [chat, setChat] = useState<ChatState>(emptyChat)
  const [draft, setDraft] = useState('')
  const [sending, setSending] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [unknownKinds, setUnknownKinds] = useState<string[]>([])
  const bottom = useRef<HTMLDivElement>(null)

  useEffect(() => {
    let off: (() => void) | undefined
    let cancelled = false

    void onKernelFrame((frame) => {
      // The kernel-side bridge republishes live stream chunks under its own
      // method; they carry no durable sequence, so they fold separately.
      if (frame.kind === 'notification' && frame.method === LIVE_CHUNK_METHOD) {
        const chunk = liveChunkOf(frame.params, sessionId)
        if (chunk !== null) setChat((state) => applyLiveChunk(state, chunk))
        return
      }
      const event = toSessionEvent(frame, sessionId)
      if (event === null) return
      if (!KNOWN_KINDS.has(event.type)) {
        // Recording what we do not draw yet is how the next milestone knows
        // which shapes the kernel actually sends.
        setUnknownKinds((previous) =>
          previous.includes(event.type) ? previous : [...previous, event.type],
        )
      }
      setChat((state) => reduceEvent(state, event))
    }).then((unlisten) => {
      if (cancelled) unlisten()
      else off = unlisten
    })

    return () => {
      cancelled = true
      off?.()
    }
  }, [sessionId])

  useEffect(() => {
    bottom.current?.scrollIntoView({ block: 'end' })
  }, [chat.messages])

  // With live chunks the last message is the assistant's from the first delta,
  // so the state has to come from the turn, not from the message list.
  const streaming = useMemo(() => chat.active !== null, [chat.active])

  const send = async () => {
    const text = draft.trim()
    if (text === '' || sending) return
    setError(null)
    setSending(true)
    try {
      await promptSession(sessionId, text)
      setDraft('')
    } catch (cause) {
      setError(String(cause))
    } finally {
      setSending(false)
    }
  }

  return (
    <div className="mx-auto flex h-full w-full max-w-3xl flex-col gap-3 p-6">
      <header className="flex items-baseline justify-between">
        <h1 className="text-lg font-semibold tracking-tight">{chat.title ?? '新会话'}</h1>
        <span className="text-muted-foreground font-mono text-xs">
          seq {chat.lastSeq} · {sessionId.slice(0, 8)}
        </span>
      </header>

      <section className="min-h-0 flex-1 overflow-y-auto pr-1">
        <ol className="flex flex-col gap-4">
          {chat.messages.length === 0 && (
            <li className="text-muted-foreground text-sm">发一句话试试。</li>
          )}
          {chat.messages.map((message) => (
            <MessageRow key={`${message.id}-${message.seq}`} message={message} />
          ))}
        </ol>
        <div ref={bottom} />
      </section>

      {chat.failure !== null && (
        <FailureBanner message={chat.failure} />
      )}

      {error !== null && (
        <pre className="border-destructive/40 bg-destructive/10 text-destructive max-h-32 overflow-auto rounded-md border p-2 text-xs whitespace-pre-wrap">
          {error}
        </pre>
      )}

      <footer className="flex flex-col gap-2">
        <textarea
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === 'Enter' && !event.shiftKey) {
              event.preventDefault()
              void send()
            }
          }}
          rows={3}
          placeholder="Enter 发送，Shift+Enter 换行"
          className="border-input bg-card resize-none rounded-md border p-2 text-sm outline-none select-text"
        />
        <div className="flex items-center justify-between">
          <span className="text-muted-foreground text-xs">
            {streaming ? '回复中…' : unknownKinds.length > 0 ? `未渲染：${unknownKinds.join(' ')}` : ''}
          </span>
          <button
            type="button"
            onClick={() => void send()}
            disabled={sending || draft.trim() === ''}
            className="bg-primary text-primary-foreground rounded-md px-4 py-1.5 text-sm font-medium disabled:opacity-40"
          >
            发送
          </button>
        </div>
      </footer>
    </div>
  )
}

/**
 * A failed turn used to render as nothing at all: the events arrived, the reducer
 * recorded the reason, and the view never looked. A chat that silently swallows a
 * failed turn is worse than one that errors loudly.
 */
function FailureBanner({ message }: { message: string }) {
  return (
    <div className="border-destructive/40 bg-destructive/10 rounded-md border p-3">
      <p className="text-destructive text-sm font-medium">这一轮失败了</p>
      <p className="text-destructive/90 mt-1 font-mono text-xs whitespace-pre-wrap select-text">
        {message}
      </p>
    </div>
  )
}

function MessageRow({ message }: { message: Message }) {
  const label = message.role === 'user' ? '你' : message.role === 'assistant' ? '助手' : '系统'
  return (
    <li className="flex flex-col gap-1">
      <span className="text-muted-foreground text-xs">{label}</span>
      <div className="select-text text-sm leading-relaxed whitespace-pre-wrap">
        {message.parts.map((part, index) => (
          <p key={index} className={part.kind === 'reasoning' ? 'text-muted-foreground italic' : ''}>
            {part.text}
          </p>
        ))}
      </div>
      {message.tools.length > 0 && (
        <ul className="border-border flex flex-col gap-1 rounded-md border p-2 font-mono text-xs">
          {message.tools.map((tool) => (
            <li key={tool.callId} className="flex gap-2">
              <span className={tool.status === 'error' ? 'text-destructive' : 'text-muted-foreground'}>
                {tool.status === 'error' ? '✗' : tool.status === 'ok' ? '✓' : '…'}
              </span>
              <span className="font-semibold">{tool.name}</span>
              <span className="text-muted-foreground truncate">{tool.output ?? tool.arguments}</span>
            </li>
          ))}
        </ul>
      )}
    </li>
  )
}

/** Method the Rust bridge republishes live stream frames under. */
const LIVE_CHUNK_METHOD = 'dsh-desktop/live-chunk'

/** Pull the StreamChunk out of a bridge frame, keeping only our session. */
function liveChunkOf(params: unknown, sessionId: string): unknown {
  if (params === null || typeof params !== 'object') return null
  const frame = params as { sessionId?: unknown; kind?: unknown; chunk?: unknown }
  if (frame.sessionId !== sessionId) return null
  return frame.kind === 'chunk' ? (frame.chunk ?? null) : null
}

/** Event kinds the panel draws today; anything else is reported, not guessed at. */
const KNOWN_KINDS = new Set([
  'user/message',
  'assistant/chunk',
  'assistant/message',
  'tool/call',
  'tool/result',
  'session/title',
  'system/message',
])

/**
 * `session.event` carries every session in the runtime, so a subagent's events
 * arrive on the same stream. Only the root session is drawn here.
 */
function toSessionEvent(frame: KernelFrame, sessionId: string): SessionEvent | null {
  if (frame.kind !== 'notification' || frame.method !== 'session.event') return null
  const params = frame.params as { sessionId?: unknown; event?: unknown } | null
  if (params === null || typeof params !== 'object') return null
  if (params.sessionId !== sessionId) return null
  const event = params.event as Partial<SessionEvent> | undefined
  if (event === undefined || typeof event.type !== 'string' || typeof event.seq !== 'number') {
    return null
  }
  return {
    type: event.type,
    seq: event.seq,
    time: typeof event.time === 'number' ? event.time : Date.now(),
    data: event.data,
  }
}
