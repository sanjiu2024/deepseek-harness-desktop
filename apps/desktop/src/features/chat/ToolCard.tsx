import { useState } from 'react'
import type { ToolCall } from '@/lib/reducer.ts'

/**
 * One tool invocation.
 *
 * Collapsed by default: a turn can contain ten of these and the answer is what
 * the user came for. The header carries enough to know what happened without
 * opening it — name, status, and the command's first line.
 */
export function ToolCard({ tool }: { tool: ToolCall }) {
  const [open, setOpen] = useState(false)
  const summary = toolSummary(tool)
  const detail = toolDetail(tool)

  return (
    <details
      open={open}
      onToggle={(event) => setOpen((event.currentTarget as HTMLDetailsElement).open)}
      className="border-border/70 rounded-md border"
    >
      <summary className="flex cursor-pointer items-center gap-2 px-2 py-1 text-xs select-none">
        <StatusDot status={tool.status} />
        <span className="font-mono font-semibold">{tool.name}</span>
        <span className="text-muted-foreground truncate">{summary}</span>
      </summary>
      <div className="border-border/70 border-t">
        {detail !== null && (
          <pre className="max-h-64 overflow-auto px-3 py-2 font-mono text-[11px] leading-relaxed whitespace-pre-wrap select-text">
            {detail}
          </pre>
        )}
        {tool.output !== null && tool.output !== '' && (
          <pre className="border-border/70 max-h-80 overflow-auto border-t px-3 py-2 font-mono text-[11px] leading-relaxed whitespace-pre-wrap select-text">
            {tool.output}
          </pre>
        )}
        {tool.output === null && (
          <p className="text-muted-foreground px-3 py-2 text-[11px]">等待结果…</p>
        )}
      </div>
    </details>
  )
}

function StatusDot({ status }: { status: ToolCall['status'] }) {
  if (status === 'error') return <span className="text-destructive">✗</span>
  if (status === 'ok') return <span className="text-green-500">✓</span>
  return <span className="text-muted-foreground animate-pulse">●</span>
}

/**
 * `arguments` is a JSON string from the kernel. It is shown as the command when
 * one is present, because `{"command":"ls"}` is noise next to `ls`.
 */
function toolDetail(tool: ToolCall): string | null {
  const parsed = parseArguments(tool.arguments)
  if (parsed === null) return tool.arguments === '' ? null : tool.arguments

  const command = parsed.command
  if (typeof command === 'string') return command

  const path = parsed.path ?? parsed.file_path ?? parsed.pattern
  if (typeof path === 'string') return path

  return JSON.stringify(parsed, null, 2)
}

function toolSummary(tool: ToolCall): string {
  if (tool.status === 'running') return '执行中…'
  const detail = toolDetail(tool)
  const first = detail?.split('\n')[0] ?? ''
  return first.length > 60 ? `${first.slice(0, 60)}…` : first
}

function parseArguments(raw: string): Record<string, unknown> | null {
  if (!raw.startsWith('{')) return null
  try {
    const parsed: unknown = JSON.parse(raw)
    return typeof parsed === 'object' && parsed !== null
      ? (parsed as Record<string, unknown>)
      : null
  } catch {
    return null
  }
}
