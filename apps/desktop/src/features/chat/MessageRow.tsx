import { useEffect, useState } from 'react'
import { Markdown } from '@/components/markdown.tsx'
import type { Message, MessagePart, ToolCall } from '@/lib/reducer.ts'

/**
 * One conversation turn.
 *
 * Reasoning and answer are different things and are shown as such: a thought is
 * a collapsible trace you can open, not a paragraph of the reply. Mixing them is
 * how a model's scratchpad ends up looking like its answer.
 */
export function MessageRow({ message, live = false }: { message: Message; live?: boolean }) {
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

      {message.tools.length > 0 && <ToolList tools={message.tools} />}
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
  const text = parts.map((part) => part.text).join('')

  useEffect(() => {
    if (live) setOpen(true)
  }, [live])

  return (
    <details
      open={open}
      onToggle={(event) => setOpen((event.currentTarget as HTMLDetailsElement).open)}
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

function ToolList({ tools }: { tools: ToolCall[] }) {
  return (
    <ul className="border-border flex flex-col gap-1 rounded-md border p-2 font-mono text-xs">
      {tools.map((tool) => (
        <li key={tool.callId} className="flex gap-2">
          <span className={tool.status === 'error' ? 'text-destructive' : 'text-muted-foreground'}>
            {tool.status === 'error' ? '✗' : tool.status === 'ok' ? '✓' : '…'}
          </span>
          <span className="font-semibold">{tool.name}</span>
          <span className="text-muted-foreground truncate">{tool.output ?? tool.arguments}</span>
        </li>
      ))}
    </ul>
  )
}
