import { useEffect, useMemo, useRef, useState } from 'react'
import { ipc, type SearchHit, type SessionSummary } from '@/lib/ipc.ts'

/**
 * Commands reachable without the mouse.
 *
 * The palette is not decoration: the session list is long, the settings live behind a gear,
 * and every one of these actions already had a UI somewhere else. This is the same set of
 * actions, addressed by name.
 */
export interface PaletteAction {
  id: string
  label: string
  hint?: string
  run: () => void
}

export function CommandPalette({
  sessions,
  actions,
  onClose,
  onSelectSession,
  onOpenSession,
}: {
  sessions: SessionSummary[]
  actions: PaletteAction[]
  onClose: () => void
  onSelectSession: (session: SessionSummary) => void
  /** Open a session by id, used by content hits. */
  onOpenSession: (sessionId: string) => void
}) {
  const [query, setQuery] = useState('')
  const [cursor, setCursor] = useState(0)
  const [searchHits, setSearchHits] = useState<SearchHit[]>([])

  // Content search is a scan over every session log, so it waits for two characters and for
  // typing to settle: one keystroke is never a question about the archive.
  useEffect(() => {
    const needle = query.trim()
    if (needle.length < 2) {
      setSearchHits([])
      return
    }
    let cancelled = false
    const timer = setTimeout(() => {
      void ipc
        .sessionSearch(needle)
        .then((hits) => {
          if (!cancelled) setSearchHits(hits)
        })
        .catch(() => {})
    }, 200)
    return () => {
      cancelled = true
      clearTimeout(timer)
    }
  }, [query])
  const input = useRef<HTMLInputElement>(null)

  useEffect(() => {
    input.current?.focus()
  }, [])

  const items = useMemo(() => {
    const needle = query.trim().toLowerCase()
    const matchedActions = actions.filter((action) =>
      needle === '' ? true : action.label.toLowerCase().includes(needle),
    )
    // Sessions are the other thing worth addressing by name: "那个讲 X 的会话".
    const matchedSessions = sessions
      .filter((session) => (needle === '' ? false : session.title.toLowerCase().includes(needle)))
      .slice(0, 8)
      .map<PaletteAction>((session) => ({
        id: `session-${session.id}`,
        label: session.title,
        hint: '会话',
        run: () => onSelectSession(session),
      }))
    const matchedContent = searchHits
      .slice(0, 8)
      .map<PaletteAction>((hit) => ({
        id: `content-${hit.sessionId}-${hit.seq}`,
        label: hit.snippet,
        hint: hit.title,
        run: () => onOpenSession(hit.sessionId),
      }))
    return [...matchedActions, ...matchedSessions, ...matchedContent]
  }, [actions, onOpenSession, onSelectSession, query, searchHits, sessions])

  useEffect(() => {
    setCursor(0)
  }, [query])

  const choose = (at: number) => {
    const item = items[at]
    if (item === undefined) return
    item.run()
    onClose()
  }

  return (
    <div
      className="animate-backdrop fixed inset-0 z-50 flex items-start justify-center bg-black/55 p-6 pt-[15vh] backdrop-blur-[2px]"
      onClick={onClose}
    >
      <div
        className="animate-pop-in bg-card border-border w-full max-w-lg overflow-hidden rounded-xl border shadow-2xl"
        onClick={(event) => event.stopPropagation()}
      >
        <input
          ref={input}
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === 'Escape') onClose()
            if (event.key === 'ArrowDown') {
              event.preventDefault()
              setCursor((at) => Math.min(at + 1, items.length - 1))
            }
            if (event.key === 'ArrowUp') {
              event.preventDefault()
              setCursor((at) => Math.max(at - 1, 0))
            }
            if (event.key === 'Enter') {
              event.preventDefault()
              choose(cursor)
            }
          }}
          placeholder="搜命令，或搜会话标题…"
          className="border-border w-full border-b bg-transparent px-3.5 py-3 text-sm outline-none select-text"
        />
        <ul className="max-h-80 overflow-y-auto py-1">
          {items.length === 0 && (
            <li className="text-muted-foreground px-3.5 py-2 text-xs">没有匹配项</li>
          )}
          {items.map((item, at) => (
            <li key={item.id}>
              <button
                type="button"
                onMouseEnter={() => setCursor(at)}
                onClick={() => choose(at)}
                className={`flex w-full items-center gap-2 px-3.5 py-1.5 text-left text-sm transition-colors ${
                  at === cursor ? 'bg-secondary/70' : ''
                }`}
              >
                <span className="truncate">{item.label}</span>
                {item.hint !== undefined && (
                  <span className="text-muted-foreground ml-auto shrink-0 text-[10px]">
                    {item.hint}
                  </span>
                )}
              </button>
            </li>
          ))}
        </ul>
        <div className="border-border/60 text-muted-foreground flex gap-3 border-t px-3.5 py-1.5 text-[10px]">
          <span>↑↓ 选择</span>
          <span>Enter 执行</span>
          <span>Esc 关闭</span>
        </div>
      </div>
    </div>
  )
}

/**
 * Export through the store and a save dialog.
 *
 * Writing the file is a Rust command rather than the fs plugin: the transcript is already
 * produced there, and handing the path back for a second round-trip would only add a way for
 * the two halves to disagree.
 */
export async function exportSession(id: string): Promise<void> {
  const { save } = await import('@tauri-apps/plugin-dialog')
  const path = await save({
    defaultPath: `session-${id.slice(0, 8)}.md`,
    filters: [{ name: 'Markdown', extensions: ['md'] }],
  })
  if (path === null) return
  await ipc.sessionExportTo(id, path)
}
