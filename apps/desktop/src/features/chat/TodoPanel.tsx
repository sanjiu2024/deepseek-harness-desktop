import type { Todo } from '@/lib/reducer.ts'

/**
 * The model's own task list.
 *
 * Shown above the composer rather than inside the transcript: it is state, not history — the
 * latest `todo_write` replaces the whole list, so rendering it inline would leave every
 * superseded snapshot in the scrollback and make the current one impossible to find.
 *
 * Collapsed to one line when everything is done, because a finished checklist is noise the
 * moment the work is finished.
 */
export function TodoPanel({ todos }: { todos: Todo[] }) {
  if (todos.length === 0) return null

  const done = todos.filter((todo) => todo.status === 'completed').length
  const active = todos.find((todo) => todo.status === 'in_progress')
  const allDone = done === todos.length

  return (
    <details
      open={!allDone}
      className="border-border/60 bg-card/30 mx-6 mb-2 rounded-lg border text-xs"
    >
      <summary className="flex cursor-pointer items-center gap-2 px-2.5 py-1.5 select-none">
        <span className="text-muted-foreground text-[11px]">任务</span>
        <span className="font-mono text-[11px] tabular-nums">
          {done}/{todos.length}
        </span>
        {active !== undefined && !allDone && (
          <span className="text-muted-foreground truncate text-[11px]">{active.content}</span>
        )}
        {allDone && <span className="text-[11px] text-emerald-400">全部完成</span>}
      </summary>

      <ul className="flex flex-col gap-0.5 px-2.5 pb-2">
        {todos.map((todo) => (
          <li key={todo.content} className="flex items-start gap-1.5">
            <Glyph status={todo.status} />
            <span
              className={
                todo.status === 'completed'
                  ? 'text-muted-foreground line-through'
                  : todo.status === 'in_progress'
                    ? 'font-medium'
                    : ''
              }
            >
              {todo.content}
            </span>
          </li>
        ))}
      </ul>
    </details>
  )
}

function Glyph({ status }: { status: Todo['status'] }) {
  if (status === 'completed') return <span className="shrink-0 text-emerald-400">✓</span>
  if (status === 'in_progress') {
    return <span className="animate-breathe mt-1 shrink-0 text-amber-400">◐</span>
  }
  return <span className="text-muted-foreground mt-0.5 shrink-0 text-[10px]">○</span>
}
