import { useState } from 'react'
import type { ToolCall } from '@/lib/reducer.ts'

/**
 * One tool invocation.
 *
 * Collapsed by default — a turn can contain ten of these and the answer is what
 * the user came for. The header carries enough to know what happened without
 * opening it: name, state, and the command's first line.
 */
export function ToolCard({ tool, awaiting = false }: { tool: ToolCall; awaiting?: boolean }) {
  const [open, setOpen] = useState(false)
  const summary = awaiting ? '等待你批准' : toolSummary(tool)
  const detail = toolDetail(tool)

  return (
    <div
      className={`animate-fade-up overflow-hidden rounded-lg border transition-colors ${
        awaiting ? 'border-amber-400/40 bg-amber-400/5' : 'border-border/60 bg-card/40'
      }`}
    >
      <button
        type="button"
        onClick={() => setOpen((previous) => !previous)}
        className="hover:bg-secondary/30 flex w-full items-center gap-2 px-2.5 py-1.5 text-left text-xs transition-colors"
        aria-expanded={open}
      >
        <StatusDot status={tool.status} awaiting={awaiting} />
        <span className="font-mono font-semibold">{tool.name}</span>
        <span className="text-muted-foreground truncate">{summary}</span>
      </button>

      <div
        className={`grid transition-[grid-template-rows] duration-250 ease-out ${
          open ? 'grid-rows-[1fr]' : 'grid-rows-[0fr]'
        }`}
      >
        <div className="overflow-hidden">
          <div className="border-border/60 border-t">
            {detail !== null && (
              <pre className="max-h-64 overflow-auto px-3 py-2 font-mono text-[11px] leading-relaxed whitespace-pre-wrap select-text">
                {detail}
              </pre>
            )}
            {tool.output !== null && tool.output !== '' && (
              <pre className="border-border/60 max-h-80 overflow-auto border-t px-3 py-2 font-mono text-[11px] leading-relaxed whitespace-pre-wrap select-text">
                {tool.output}
              </pre>
            )}
            {tool.output === null && (
              <p className="text-muted-foreground px-3 py-2 text-[11px]">
                {awaiting ? '内核在等你的决定 —— 见上方弹窗' : '等待结果…'}
              </p>
            )}
          </div>
        </div>
      </div>
    </div>
  )
}

function StatusDot({ status, awaiting }: { status: ToolCall['status']; awaiting: boolean }) {
  // A blocked call is not running: it is waiting on the user, and that is the one
  // state worth interrupting for.
  if (awaiting) {
    return (
      <span className="animate-breathe inline-block size-1.5 shrink-0 rounded-full bg-amber-400" />
    )
  }
  if (status === 'error') {
    return <span className="text-destructive shrink-0 leading-none">✗</span>
  }
  if (status === 'ok') {
    return <span className="shrink-0 leading-none text-emerald-400">✓</span>
  }
  return (
    <span className="text-muted-foreground animate-breathe inline-block size-1.5 shrink-0 rounded-full bg-current" />
  )
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
