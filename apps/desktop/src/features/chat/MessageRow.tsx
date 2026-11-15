import { useEffect, useRef, useState } from 'react'
import { Markdown } from '@/components/markdown.tsx'
import type { Message, MessagePart } from '@/lib/reducer.ts'
import { ToolCard } from './ToolCard.tsx'

/**
 * One conversation turn.
 *
 * Reasoning and answer are different things and are shown as such: a thought is
 * a collapsible trace you can open, not a paragraph of the reply. Mixing them is
 * how a model's scratchpad ends up looking like its answer.
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
  const answer = message.parts.filter((part) => part.kind === 'text')

  return (
    <li className="flex flex-col gap-1.5">
      <span className="text-muted-foreground text-xs">{label}</span>

      {reasoning.length > 0 && <ReasoningBlock parts={reasoning} live={live && answer.length === 0} />}

      {answer.length > 0 &&
        (message.role === 'user' ? (
          <div className="text-sm leading-relaxed whitespace-pre-wrap select-text">
            {answer.map((part, index) => (
              <p key={index}>{part.text}</p>
            ))}
          </div>
        ) : (
          <div className="flex flex-col gap-3">
            {answer.map((part, index) => (
              <Markdown key={index} text={part.text} />
            ))}
          </div>
        ))}

      {message.tools.length > 0 && (
        <div className="flex flex-col gap-1.5">
          {message.tools.map((tool) => (
            <ToolCard key={tool.callId} tool={tool} awaiting={awaitingApproval.has(tool.callId)} />
          ))}
        </div>
      )}
    </li>
  )
}

/**
 * The trace opens itself while the model is thinking and closes once the answer
 * starts — a collapsed disclosure on a live turn looks exactly like no thinking
 * at all, which is how this was reported as a bug.
 */
function ReasoningBlock({ parts, live }: { parts: MessagePart[]; live: boolean }) {
  const [open, setOpen] = useState(live)
  // A manual toggle is an instruction, not a default: once the user has opened
  // or closed this trace, the streaming state stops moving it.
  const touched = useRef(false)
  const text = parts.map((part) => part.text).join('')

  useEffect(() => {
    if (live) setOpen(true)
    else if (!touched.current) setOpen(false)
  }, [live])

  return (
    <details
      open={open}
      onToggle={(event) => {
        const element = event.currentTarget as HTMLDetailsElement
        if (element.open !== open) touched.current = true
        setOpen(element.open)
      }}
      className="border-border/70 rounded-md border"
    >
      <summary className="text-muted-foreground cursor-pointer px-2 py-1 text-xs select-none">
        {live ? '思考中…' : '思考过程'} · {text.length} 字{open ? '' : '（点击展开）'}
      </summary>
      <div className="text-muted-foreground max-h-72 overflow-y-auto px-3 pt-1 pb-2 text-xs leading-relaxed whitespace-pre-wrap select-text">
        {text}
      </div>
    </details>
  )
}
