import { useEffect, useMemo, useRef, useState } from 'react'
import { ipc, type SessionSummary } from '@/lib/ipc.ts'
import { exportSession } from '@/features/palette/CommandPalette.tsx'

/**
 * The session list.
 *
 * Grouped by day, because "yesterday" is how people remember a conversation — an absolute
 * timestamp in every row is precision nobody asked for. A filter box on top: with forty
 * sessions, scrolling is not the way to find one.
 *
 * Subagent runs nest under the session that spawned them; a child is part of a conversation,
 * not a sibling of it.
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
  const [filter, setFilter] = useState('')

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

  const filtered = useMemo(() => {
    const needle = filter.trim().toLowerCase()
    if (needle === '' || sessions === null) return sessions ?? []
    return sessions.filter((session) => session.title.toLowerCase().includes(needle))
  }, [filter, sessions])

  const roots = filtered.filter(
    (session) =>
      session.parentId === null || !filtered.some((candidate) => candidate.id === session.parentId),
  )
  const childrenOf = (id: string) => filtered.filter((session) => session.parentId === id)

  // Day buckets in the order they appear: the store already sorts newest first, so grouping
  // only has to notice when the bucket changes.
  const groups: Array<{ label: string; items: SessionSummary[] }> = []
  for (const session of roots) {
    const label = dayLabel(session.updatedAt)
    const last = groups[groups.length - 1]
    if (last !== undefined && last.label === label) last.items.push(session)
    else groups.push({ label, items: [session] })
  }

  return (
    <aside className="border-border/60 bg-card/30 flex h-full w-64 shrink-0 flex-col border-r">
      <div className="flex items-center gap-1 px-2 pt-2.5">
        <input
          value={filter}
          onChange={(event) => setFilter(event.target.value)}
          placeholder="筛选会话"
          className="border-input bg-background/60 focus:border-ring min-w-0 flex-1 rounded-md border px-2 py-1 text-[11px] outline-none transition-colors select-text"
        />
        <button
          type="button"
          onClick={onNew}
          title="新会话（Ctrl/Cmd+K → 新建）"
          className="text-muted-foreground hover:bg-secondary/60 hover:text-foreground flex size-6 shrink-0 items-center justify-center rounded-md text-base leading-none transition-colors active:scale-95"
        >
          +
        </button>
      </div>

      {error !== null && (
        <p className="text-destructive px-3 pt-2 text-[11px] whitespace-pre-wrap">{error}</p>
      )}

      <nav className="min-h-0 flex-1 overflow-y-auto px-2 py-2">
        {sessions === null && <p className="text-muted-foreground px-1 text-xs">读取中…</p>}
        {sessions !== null && sessions.length === 0 && (
          <p className="text-muted-foreground px-1 text-xs">还没有历史会话。发一句话试试。</p>
        )}
        {sessions !== null && sessions.length > 0 && roots.length === 0 && (
          <p className="text-muted-foreground px-1 text-xs">没有匹配的会话。</p>
        )}

        {groups.map((group) => (
          <section key={group.label} className="mb-2">
            <h3 className="text-muted-foreground px-1 pb-1 text-[10px] font-medium tracking-wide uppercase">
              {group.label}
            </h3>
            <ul className="flex flex-col gap-0.5">
              {group.items.map((session) => (
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
          </section>
        ))}
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
  const input = useRef<HTMLInputElement>(null)

  useEffect(() => {
    if (renaming) input.current?.select()
  }, [renaming])

  if (renaming) {
    return (
      <input
        ref={input}
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
        <span className="flex items-center gap-1 truncate text-xs font-medium">
          {child && <span className="text-muted-foreground shrink-0 text-[10px]">↳</span>}
          <span className="truncate">{session.title}</span>
        </span>
        <span className="text-muted-foreground font-mono text-[10px]">
          {clockTime(session.updatedAt)} · {session.eventCount} 条
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

/** 今天 / 昨天 / 本周 / 更早 — the buckets people actually remember in. */
function dayLabel(epochMs: number): string {
  const start = new Date()
  start.setHours(0, 0, 0, 0)
  const midnight = start.getTime()
  if (epochMs >= midnight) return '今天'
  if (epochMs >= midnight - 86_400_000) return '昨天'
  if (epochMs >= midnight - 7 * 86_400_000) return '本周'
  return '更早'
}

/** HH:MM inside a day bucket; the bucket already carries the date. */
function clockTime(epochMs: number): string {
  const date = new Date(epochMs)
  return `${String(date.getHours()).padStart(2, '0')}:${String(date.getMinutes()).padStart(2, '0')}`
}
