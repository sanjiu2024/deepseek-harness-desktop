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
  // Arrival order, not type order.
  //
  // Grouping all reasoning into one block and all tool calls after it read as a
  // single wall of thought followed by unrelated cards; the kernel actually
  // interleaves think → call → think. Sorting by the sequence each block arrived
  // at restores what happened.
  const ordered = [
    ...message.parts.map((part) => ({ kind: 'part' as const, seq: part.seq, part })),
    ...message.tools.map((tool) => ({ kind: 'tool' as const, seq: tool.seq, tool })),
  ]
    .sort((left, right) => (left.seq === right.seq ? (left.kind === 'part' ? -1 : 1) : left.seq - right.seq))
    .map((entry) => entry)

  const reasoning = message.parts.filter((part) => part.kind === 'reasoning')
  const hasAnswer = message.parts.some((part) => part.kind === 'text' && part.text !== '')

  return (
    <li className="flex flex-col gap-1.5">
      <span className="text-muted-foreground text-xs">{label}</span>

      {reasoning.length > 0 && <ReasoningBlock parts={reasoning} live={live && !hasAnswer} />}

      {ordered.map((entry, index) =>
        entry.kind === 'tool' ? (
          <ToolCard
            key={`tool-${entry.tool.callId}`}
            tool={entry.tool}
            awaiting={awaitingApproval.has(entry.tool.callId)}
          />
        ) : entry.part.kind === 'reasoning' ? null : message.role === 'user' ? (
          <div key={index} className="text-sm leading-relaxed whitespace-pre-wrap select-text">
            {entry.part.text}
          </div>
        ) : (
          <Markdown key={index} text={entry.part.text} />
        ),
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
