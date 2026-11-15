import { useEffect, useState } from 'react'
import { ipc, type SessionSummary } from '@/lib/ipc.ts'
import { exportSession } from '@/features/palette/CommandPalette.tsx'

/**
 * The session list.
 *
 * The kernel keeps every session on disk but offers no "list" call, so this is a
 * projection of the store rather than something the runtime tells us about. Subagent runs
 * nest under the session that spawned them — a child is part of a conversation, not a
 * sibling of it.
 */
export function SessionSidebar({
  activeId,
  refreshKey,
  onSelect,
  onNew,
  onChanged,
}: {
  activeId: string
  /** Bump to re-read the store; a session only exists after its first turn. */
  refreshKey: number
  onSelect: (session: SessionSummary) => void
  onNew: () => void
  /** Called after a rename or delete so the shell can refresh too. */
  onChanged?: () => void
}) {
  const [sessions, setSessions] = useState<SessionSummary[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [renaming, setRenaming] = useState<string | null>(null)
  const [confirming, setConfirming] = useState<string | null>(null)

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

  const rename = async (id: string, title: string) => {
    setRenaming(null)
    try {
      await ipc.sessionRename(id, title)
      onChanged?.()
    } catch (cause) {
      setError(String(cause))
    }
  }

  const remove = async (id: string) => {
    setConfirming(null)
    try {
      await ipc.sessionDelete(id)
      onChanged?.()
    } catch (cause) {
      setError(String(cause))
    }
  }

  // Subagent runs hang off their parent; orphans stay at the top level so nothing is lost.
  const roots = (sessions ?? []).filter(
    (session) =>
      session.parentId === null ||
      !(sessions ?? []).some((candidate) => candidate.id === session.parentId),
  )
  const childrenOf = (id: string) =>
    (sessions ?? []).filter((session) => session.parentId === id)

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
          {roots.map((session) => (
            <li key={session.id}>
              <Row
                session={session}
                active={session.id === activeId}
                renaming={renaming === session.id}
                confirming={confirming === session.id}
                onSelect={() => onSelect(session)}
                onRename={() => setRenaming(session.id)}
                onRenameDone={(title) => void rename(session.id, title)}
                onRenameCancel={() => setRenaming(null)}
                onAskDelete={() => setConfirming(session.id)}
                onDelete={() => void remove(session.id)}
                onCancelDelete={() => setConfirming(null)}
              />
              {childrenOf(session.id).length > 0 && (
                <ul className="border-border/50 ml-3 flex flex-col gap-0.5 border-l pl-2">
                  {childrenOf(session.id).map((child) => (
                    <li key={child.id}>
                      <Row
                        session={child}
                        active={child.id === activeId}
                        renaming={renaming === child.id}
                        confirming={confirming === child.id}
                        child
                        onSelect={() => onSelect(child)}
                        onRename={() => setRenaming(child.id)}
                        onRenameDone={(title) => void rename(child.id, title)}
                        onRenameCancel={() => setRenaming(null)}
                        onAskDelete={() => setConfirming(child.id)}
                        onDelete={() => void remove(child.id)}
                        onCancelDelete={() => setConfirming(null)}
                      />
                    </li>
                  ))}
                </ul>
              )}
            </li>
          ))}
        </ul>
      </nav>
    </aside>
  )
}

function Row({
  session,
  active,
  renaming,
  confirming,
  child = false,
  onSelect,
  onRename,
  onRenameDone,
  onRenameCancel,
  onAskDelete,
  onDelete,
  onCancelDelete,
}: {
  session: SessionSummary
  active: boolean
  renaming: boolean
  confirming: boolean
  child?: boolean
  onSelect: () => void
  onRename: () => void
  onRenameDone: (title: string) => void
  onRenameCancel: () => void
  onAskDelete: () => void
  onDelete: () => void
  onCancelDelete: () => void
}) {
  const [draft, setDraft] = useState(session.title)

  if (renaming) {
    return (
      <input
        // eslint-disable-next-line jsx-a11y/no-autofocus -- it just opened; focus is the point
        autoFocus
        value={draft}
        onChange={(event) => setDraft(event.target.value)}
        onBlur={() => onRenameDone(draft)}
        onKeyDown={(event) => {
          if (event.key === 'Enter') onRenameDone(draft)
          if (event.key === 'Escape') onRenameCancel()
        }}
        className="border-ring bg-background w-full rounded-md border px-2 py-1 text-xs outline-none select-text"
      />
    )
  }

  if (confirming) {
    return (
      <div className="border-destructive/40 bg-destructive/10 flex items-center gap-1.5 rounded-md border px-2 py-1 text-[11px]">
        <span className="text-destructive truncate">删除这条会话？</span>
        <button
          type="button"
          onClick={onDelete}
          className="bg-destructive/20 text-destructive hover:bg-destructive/30 ml-auto shrink-0 rounded px-1.5 py-0.5 transition-colors"
        >
          删
        </button>
        <button
          type="button"
          onClick={onCancelDelete}
          className="text-muted-foreground hover:text-foreground shrink-0 rounded px-1 py-0.5 transition-colors"
        >
          算了
        </button>
      </div>
    )
  }

  return (
    <div
      className={`group flex items-center gap-1 rounded-lg pr-1 transition-colors ${
        active ? 'bg-secondary/70' : 'hover:bg-secondary/40'
      }`}
    >
      <button
        type="button"
        onClick={onSelect}
        onDoubleClick={onRename}
        className="flex min-w-0 flex-1 flex-col gap-0.5 px-2 py-1.5 text-left"
      >
        <span className="truncate text-xs font-medium">
          {child && <span className="text-muted-foreground mr-1 text-[10px]">↳</span>}
          {session.title}
        </span>
        <span className="text-muted-foreground font-mono text-[10px]">
          {relativeTime(session.updatedAt)} · {session.eventCount} 条
        </span>
      </button>

      {/* Actions appear on hover or focus so the list stays readable. */}
      <div className="flex shrink-0 items-center opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100">
        <button
          type="button"
          onClick={onRename}
          title="重命名"
          className="text-muted-foreground hover:text-foreground flex size-5 items-center justify-center rounded transition-colors"
        >
          ✎
        </button>
        <button
          type="button"
          onClick={() => void exportSession(session.id)}
          title="导出 Markdown"
          className="text-muted-foreground hover:text-foreground flex size-5 items-center justify-center rounded transition-colors"
        >
          ↓
        </button>
        <button
          type="button"
          onClick={onAskDelete}
          title="删除"
          className="text-muted-foreground hover:text-destructive flex size-5 items-center justify-center rounded transition-colors"
        >
          ×
        </button>
      </div>
    </div>
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
