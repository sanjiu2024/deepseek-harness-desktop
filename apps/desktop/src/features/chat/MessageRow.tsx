import { useEffect, useRef, useState } from 'react'
import { Markdown } from '@/components/markdown.tsx'
import type { Message, MessagePart } from '@/lib/reducer.ts'
import { ToolCard } from './ToolCard.tsx'

/**
 * One conversation turn.
 *
 * Reasoning and answer are different things and are shown as such: a thought is a
 * collapsible trace you can open, not a paragraph of the reply. Blocks render in
 * the order they arrived, so a turn that thinks, calls a tool, then thinks again
 * reads the way it happened.
 */
export function MessageRow({
  message,
  live = false,
  awaitingApproval = new Set<string>(),
}: {
  message: Message
  live?: boolean
  /** Call ids whose tool call is blocked on a decision the user has not made. */
  awaitingApproval?: ReadonlySet<string>
}) {
  const label = message.role === 'user' ? '你' : message.role === 'assistant' ? '助手' : '系统'
  const reasoning = message.parts.filter((part) => part.kind === 'reasoning')
  const hasAnswer = message.parts.some((part) => part.kind === 'text' && part.text !== '')

  const ordered = [
    ...message.parts.map((part) => ({ kind: 'part' as const, seq: part.seq, part })),
    ...message.tools.map((tool) => ({ kind: 'tool' as const, seq: tool.seq, tool })),
  ].sort((left, right) => {
    if (left.seq === right.seq) return left.kind === 'part' ? -1 : 1
    return left.seq - right.seq
  })

  // The caret belongs to whatever text is still arriving, which is only ever the
  // last block of a live turn.
  const streamingIndex = live ? ordered.length - 1 : -1

  return (
    <li className="animate-fade-up flex flex-col gap-2">
      <span className="text-muted-foreground text-[11px] font-medium tracking-wide uppercase">
        {label}
      </span>

      {reasoning.length > 0 && <ReasoningBlock parts={reasoning} live={live && !hasAnswer} />}

      {ordered.map((entry, index) =>
        entry.kind === 'tool' ? (
          <ToolCard
            key={`tool-${entry.tool.callId}`}
            tool={entry.tool}
            awaiting={awaitingApproval.has(entry.tool.callId)}
          />
        ) : entry.part.kind === 'reasoning' ? null : message.role === 'user' ? (
          <div
            key={index}
            className="bg-secondary/50 rounded-lg px-3 py-2 text-sm leading-relaxed whitespace-pre-wrap select-text"
          >
            {entry.part.text}
          </div>
        ) : (
          <div key={index} className={index === streamingIndex ? 'streaming-caret' : undefined}>
            <Markdown text={entry.part.text} />
          </div>
        ),
      )}
    </li>
  )
}

/**
 * The trace opens itself while the model is thinking and closes once the answer
 * starts — a collapsed disclosure on a live turn looks exactly like no thinking
 * at all. A manual toggle then outranks the streaming state.
 */
function ReasoningBlock({ parts, live }: { parts: MessagePart[]; live: boolean }) {
  const [open, setOpen] = useState(live)
  const touched = useRef(false)
  const text = parts.map((part) => part.text).join('')

  useEffect(() => {
    if (live) setOpen(true)
    else if (!touched.current) setOpen(false)
  }, [live])

  return (
    <div className="border-border/60 bg-card/40 overflow-hidden rounded-lg border">
      <button
        type="button"
        onClick={() => {
          touched.current = true
          setOpen((previous) => !previous)
        }}
        className="text-muted-foreground hover:text-foreground flex w-full items-center gap-1.5 px-2.5 py-1.5 text-xs transition-colors select-none"
        aria-expanded={open}
      >
        <svg
          viewBox="0 0 12 12"
          aria-hidden="true"
          className={`size-2.5 transition-transform duration-200 ${open ? 'rotate-90' : ''}`}
        >
          <path d="M4 2l4 4-4 4" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
        </svg>
        {live ? (
          <span className="flex items-center gap-1.5">
            <span className="animate-breathe inline-block size-1.5 rounded-full bg-current" />
            思考中…
          </span>
        ) : (
          <span>思考过程</span>
        )}
        <span className="opacity-60">· {text.length} 字</span>
      </button>

      {/* grid-rows 0fr→1fr animates height without measuring it. */}
      <div
        className={`grid transition-[grid-template-rows] duration-250 ease-out ${
          open ? 'grid-rows-[1fr]' : 'grid-rows-[0fr]'
        }`}
      >
        <div className="overflow-hidden">
          <div className="text-muted-foreground max-h-72 overflow-y-auto px-3 pt-0.5 pb-2.5 text-xs leading-relaxed whitespace-pre-wrap select-text">
            {text}
          </div>
        </div>
      </div>
    </div>
  )
}
