import { useEffect, useMemo, useRef, useState } from 'react'
import { ipc, onKernelFrame, promptSession, type FrameId, type KernelFrame } from '@/lib/ipc.ts'

const ipcAnswer = (id: FrameId, result: unknown) => ipc.kernelAnswer(id, result)
import {
  applyLiveChunk,
  emptyChat,
  hydrate,
  reduceEvent,
  type ChatState,
  type SessionEvent,
} from '@/lib/reducer.ts'
import {
  approvalResult,
  cancelledQuestion,
  parseInteraction,
  questionResult,
  type PendingInteraction,
} from '@/lib/interaction.ts'
import { InteractionHost } from './InteractionHost.tsx'
import { MessageRow } from './MessageRow.tsx'

/**
 * M1 chat surface: a real turn, streamed into real messages.
 *
 * It is deliberately plain — no virtual list, no markdown, no tool cards yet.
 * Those arrive once the event shapes below are confirmed against a live kernel
 * rather than guessed from the protocol docs.
 */
export function ChatPanel({
  sessionId,
  onActivity,
}: {
  sessionId: string
  /** Called when this session first produces something worth listing. */
  onActivity?: () => void
}) {
  const [chat, setChat] = useState<ChatState>(emptyChat)
  const [draft, setDraft] = useState('')
  const [sending, setSending] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [unknownKinds, setUnknownKinds] = useState<string[]>([])
  // Blocking requests are a queue: the kernel can ask again while one is open.
  const [interactions, setInteractions] = useState<PendingInteraction[]>([])
  // How many requests of each kind arrived in total. A pending approval is
  // invisible otherwise: the card says "running" and nothing else is different
  // from a slow call. If this stays at zero while a turn is stuck, the frame never
  // reached the client; if it counts up, the dialog has a rendering problem.
  const [requestCounts, setRequestCounts] = useState<Record<string, number>>({})
  const bottom = useRef<HTMLDivElement>(null)

  // A session that already exists on disk is opened, not replayed: its durable
  // rows go through the same reducer as a live turn, so history and streaming
  // cannot disagree about what a tool call or a reasoning block looks like.
  useEffect(() => {
    let cancelled = false
    void ipc
      .sessionLoad(sessionId)
      .then((history) => {
        if (cancelled || history.events.length === 0) return
        setChat(hydrate(history.events))
      })
      .catch(() => {
        // No stored log yet: this is a new session, which is not an error.
      })
    return () => {
      cancelled = true
    }
  }, [sessionId])

  useEffect(() => {
    let off: (() => void) | undefined
    let cancelled = false

    void onKernelFrame((frame) => {
      // A blocking request is not an event: ignoring it hangs the turn with no
      // error anywhere, so it is matched before anything else.
      if (frame.kind === 'request') {
        setRequestCounts((previous) => ({
          ...previous,
          [frame.method]: (previous[frame.method] ?? 0) + 1,
        }))
        const interaction = parseInteraction(frame.method, frame.id, frame.params)
        if (interaction !== null) {
          setInteractions((previous) => [...previous, interaction])
        } else {
          setUnknownKinds((previous) =>
            previous.includes(frame.method) ? previous : [...previous, frame.method],
          )
        }
        return
      }
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
      // A turn that ended cannot still be waiting on us: any queued decision
      // belongs to a tool call the kernel has already abandoned.
      if (event.type === 'turn/start') onActivity?.()
      if (event.type === 'turn/end') setInteractions([])
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

  /**
   * Call ids with a decision waiting on the user.
   *
   * Without this a blocked tool card is indistinguishable from a slow one: both
   * say "running". One of them needs you, which is the entire reason the card is
   * worth showing.
   */
  const awaiting = useMemo(() => {
    const ids = new Set<string>()
    for (const interaction of interactions) {
      if (interaction.kind === 'approval' && interaction.callId !== undefined) {
        ids.add(interaction.callId)
      }
    }
    return ids
  }, [interactions])

  /**
   * Answer the request in front and drop it from the queue.
   *
   * The kernel blocks on this reply: if it fails, the request stays queued so the
   * user can try again instead of the turn silently dying.
   */
  const answer = async (interaction: PendingInteraction, result: unknown) => {
    try {
      await ipcAnswer(interaction.id, result)
      setInteractions((previous) => previous.filter((entry) => entry.id !== interaction.id))
    } catch (cause) {
      setError(`应答失败：${String(cause)}`)
    }
  }

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
    <div className="mx-auto flex h-full w-full max-w-3xl flex-col">
      <header className="border-border/60 flex items-center justify-between gap-3 border-b px-6 py-3">
        <div className="flex min-w-0 flex-col">
          <h1 className="truncate text-sm font-semibold tracking-tight">
            {chat.title ?? '新会话'}
          </h1>
          <span className="text-muted-foreground font-mono text-[11px]">
            {chat.route !== null ? `${chat.route.provider} / ${chat.route.model}` : '未握手'}
          </span>
        </div>
        <span className="text-muted-foreground shrink-0 font-mono text-[11px]">
          seq {chat.lastSeq}
        </span>
      </header>

      <section className="min-h-0 flex-1 overflow-y-auto px-6 py-5">
        <ol className="flex flex-col gap-5">
          {chat.messages.length === 0 && (
            <li className="text-muted-foreground animate-fade-up text-sm">发一句话试试。</li>
          )}
          {chat.messages.map((message, index) => (
            <MessageRow
              key={`${message.id}-${message.seq}`}
              message={message}
              live={streaming && index === chat.messages.length - 1}
              awaitingApproval={awaiting}
            />
          ))}
        </ol>
        <div ref={bottom} />
      </section>

      {chat.failure !== null && (
        <div className="mx-6 mb-2">
          <FailureBanner message={chat.failure} />
        </div>
      )}

      {error !== null && (
        <pre className="border-destructive/40 bg-destructive/10 text-destructive mx-6 mb-2 max-h-32 overflow-auto rounded-lg border p-2.5 text-xs whitespace-pre-wrap">
          {error}
        </pre>
      )}

      {interactions[0] !== undefined && (
        <InteractionHost
          interaction={interactions[0]}
          onAnswer={(outcome) => void answer(interactions[0]!, approvalResult(outcome))}
          onQuestion={(answers) => void answer(interactions[0]!, questionResult(answers))}
          onCancelQuestion={() => void answer(interactions[0]!, cancelledQuestion())}
        />
      )}

      <footer className="border-border/60 bg-background/80 flex flex-col gap-1.5 border-t px-6 py-3 backdrop-blur">
        <div className="border-input bg-card focus-within:border-ring flex items-end gap-2 rounded-xl border p-2 transition-colors">
          <textarea
            value={draft}
            onChange={(event) => setDraft(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === 'Enter' && !event.shiftKey) {
                event.preventDefault()
                void send()
              }
            }}
            rows={2}
            placeholder="Enter 发送，Shift+Enter 换行"
            className="max-h-40 min-h-9 flex-1 resize-none bg-transparent px-1 text-sm outline-none select-text"
          />
          <button
            type="button"
            onClick={() => void send()}
            disabled={sending || draft.trim() === ''}
            className="bg-primary text-primary-foreground hover:bg-primary/90 mb-0.5 flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-sm font-medium transition-all active:scale-[0.97] disabled:opacity-30"
          >
            {sending ? '发送中' : '发送'}
            <svg viewBox="0 0 12 12" aria-hidden="true" className="size-3">
              <path d="M2 6h7M6 3l3 3-3 3" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
            </svg>
          </button>
        </div>

        <div className="flex items-center justify-between">
          <span className="text-muted-foreground flex items-center gap-1.5 text-[11px]">
            {streaming && (
              <>
                <span className="animate-breathe inline-block size-1.5 rounded-full bg-current" />
                回复中…
              </>
            )}
          </span>
          <span className="text-muted-foreground font-mono text-[10px]">{sessionId.slice(0, 8)}</span>
        </div>

        {Object.keys(requestCounts).length > 0 && (
          <p className="text-muted-foreground text-[10px]">
            收到的请求帧：
            {Object.entries(requestCounts)
              .map(([method, count]) => `${method} ×${count}`)
              .join(' · ')}
          </p>
        )}

        {unknownKinds.length > 0 && (
          <details className="text-muted-foreground text-[10px]">
            <summary className="cursor-pointer select-none">
              未渲染的内核事件 · {unknownKinds.length} 种
            </summary>
            <p className="mt-0.5 font-mono break-all">{unknownKinds.join(' · ')}</p>
          </details>
        )}
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
