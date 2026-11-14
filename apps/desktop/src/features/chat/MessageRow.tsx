import { useState } from 'react'
import { Markdown } from '@/components/markdown.tsx'
import type { Message, MessagePart, ToolCall } from '@/lib/reducer.ts'

/**
 * One conversation turn.
 *
 * Reasoning and answer are different things and are shown as such: a thought is
 * a collapsible trace you can open, not a paragraph of the reply. Mixing them is
 * how a model's scratchpad ends up looking like its answer.
 */
export function MessageRow({ message }: { message: Message }) {
  const label = message.role === 'user' ? '你' : message.role === 'assistant' ? '助手' : '系统'
  const reasoning = message.parts.filter((part) => part.kind === 'reasoning')
  const answer = message.parts.filter((part) => part.kind === 'text')

  return (
    <li className="flex flex-col gap-1.5">
      <span className="text-muted-foreground text-xs">{label}</span>

      {reasoning.length > 0 && <ReasoningBlock parts={reasoning} />}

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

/** Collapsed by default: the trace is available, not imposed. */
function ReasoningBlock({ parts }: { parts: MessagePart[] }) {
  const [open, setOpen] = useState(false)
  const text = parts.map((part) => part.text).join('')

  return (
    <details
      open={open}
      onToggle={(event) => setOpen((event.currentTarget as HTMLDetailsElement).open)}
      className="border-border/70 rounded-md border"
    >
      <summary className="text-muted-foreground cursor-pointer px-2 py-1 text-xs select-none">
        思考过程 · {text.length} 字{open ? '' : '（点击展开）'}
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
