import { useEffect, useState } from 'react'
import { ipc, type SessionSummary } from '@/lib/ipc.ts'

/**
 * The session list.
 *
 * The kernel keeps every session on disk but offers no "list" call, so this is a
 * projection of the store rather than something the runtime tells us about. It
 * refreshes on demand and after a turn starts, which is when a session first
 * becomes discoverable.
 */
export function SessionSidebar({
  activeId,
  refreshKey,
  onSelect,
  onNew,
}: {
  activeId: string
  /** Bump this to re-read the store; a new session only exists after its first turn. */
  refreshKey: number
  onSelect: (session: SessionSummary) => void
  onNew: () => void
}) {
  const [sessions, setSessions] = useState<SessionSummary[] | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    void ipc
      .sessionList()
      .then((listed) => {
        if (!cancelled) setSessions(listed)
      })
      .catch((cause: unknown) => {
        if (!cancelled) setError(String(cause))
      })
    return () => {
      cancelled = true
    }
  }, [refreshKey])

  return (
    <aside className="border-border/60 bg-card/30 flex h-full w-64 shrink-0 flex-col border-r">
      <div className="flex items-center justify-between px-3 py-3">
        <span className="text-muted-foreground text-[11px] font-medium tracking-wide uppercase">
          会话
        </span>
        <button
          type="button"
          onClick={onNew}
          title="新会话"
          className="text-muted-foreground hover:bg-secondary/60 hover:text-foreground flex size-6 items-center justify-center rounded-md text-base leading-none transition-colors active:scale-95"
        >
          +
        </button>
      </div>

      {error !== null && (
        <p className="text-destructive px-3 pb-2 text-[11px] whitespace-pre-wrap">{error}</p>
      )}

      <nav className="min-h-0 flex-1 overflow-y-auto px-2 pb-3">
        {sessions === null && <p className="text-muted-foreground px-1 text-xs">读取中…</p>}
        {sessions !== null && sessions.length === 0 && (
          <p className="text-muted-foreground px-1 text-xs">还没有历史会话。发一句话试试。</p>
        )}

        <ul className="flex flex-col gap-0.5">
          {(sessions ?? []).map((session) => {
            const active = session.id === activeId
            return (
              <li key={session.id}>
                <button
                  type="button"
                  onClick={() => onSelect(session)}
                  className={`group flex w-full flex-col gap-0.5 rounded-lg px-2 py-1.5 text-left transition-colors ${
                    active ? 'bg-secondary/70' : 'hover:bg-secondary/40'
                  }`}
                >
                  <span className="flex items-center gap-1.5">
                    {session.isSubagent && (
                      <span className="text-muted-foreground shrink-0 rounded bg-white/10 px-1 text-[9px]">
                        子
                      </span>
                    )}
                    <span className="truncate text-xs font-medium">{session.title}</span>
                  </span>
                  <span className="text-muted-foreground font-mono text-[10px]">
                    {relativeTime(session.updatedAt)} · {session.eventCount} 条
                  </span>
                </button>
              </li>
            )
          })}
        </ul>
      </nav>
    </aside>
  )
}

/** "刚刚 / 5 分钟 / 3 小时 / 2 天 / 日期" — enough precision to recognise a session. */
function relativeTime(epochMs: number): string {
  const delta = Date.now() - epochMs
  if (delta < 60_000) return '刚刚'
  if (delta < 3_600_000) return `${Math.floor(delta / 60_000)} 分钟前`
  if (delta < 86_400_000) return `${Math.floor(delta / 3_600_000)} 小时前`
  if (delta < 7 * 86_400_000) return `${Math.floor(delta / 86_400_000)} 天前`
  const date = new Date(epochMs)
  return `${date.getMonth() + 1}/${date.getDate()}`
}
