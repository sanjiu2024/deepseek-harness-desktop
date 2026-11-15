import { useCallback, useEffect, useMemo, useState } from 'react'
import { BootPanel } from '@/features/boot/BootPanel.tsx'
import { ChatPanel } from '@/features/chat/ChatPanel.tsx'
import { SessionSidebar } from '@/features/sessions/SessionSidebar.tsx'
import { CommandPalette, type PaletteAction } from '@/features/palette/CommandPalette.tsx'
import { WorkspacePanel } from '@/features/workspace/WorkspacePanel.tsx'
import { useTheme } from '@/lib/theme.ts'
import { ipc, type SessionSummary } from '@/lib/ipc.ts'

/**
 * Shell.
 *
 * Two panes once the kernel is running: what has happened (the store's session list) and
 * what is happening (the conversation). Before that, the launch screen owns the window —
 * starting a kernel is the one step that cannot be implicit.
 */
export function App() {
  const [ready, setReady] = useState(false)
  const [sessionId, setSessionId] = useState<string>(() => crypto.randomUUID())
  const [restored, setRestored] = useState(false)
  const [historyKey, setHistoryKey] = useState(0)
  const [refreshKey, setRefreshKey] = useState(0)
  const [sessions, setSessions] = useState<SessionSummary[]>([])
  const [paletteOpen, setPaletteOpen] = useState(false)
  const [panelOpen, setPanelOpen] = useState(false)
  // A path the file tree asked to insert; the composer consumes it and clears it.
  const [pendingPath, setPendingPath] = useState<string | null>(null)
  const [theme, toggleTheme] = useTheme()

  const refresh = useCallback(() => setRefreshKey((key) => key + 1), [])

  /**
   * Reopen the session you were in last time.
   *
   * Restoring *which* conversation is the difference between an app and a scratchpad — the
   * list is right there, but needing to find your place again every launch is the thing that
   * makes small tools annoying. A session that no longer exists silently falls back to new.
   */
  useEffect(() => {
    if (!ready || restored) return
    setRestored(true)
    void (async () => {
      const settings = await ipc.settingsGet().catch(() => null)
      const remembered = settings?.lastSessionId ?? null
      if (remembered === null) return
      const exists = (await ipc.sessionList().catch(() => [])).some(
        (session) => session.id === remembered,
      )
      if (!exists) return
      setSessionId(remembered)
      setHistoryKey((key) => key + 1)
    })()
  }, [ready, restored])

  useEffect(() => {
    if (!ready) return
    void ipc.sessionList().then(setSessions).catch(() => {})
  }, [ready, refreshKey])

  // One shortcut, because one is enough to learn: everything else is in the palette.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'j') {
        event.preventDefault()
        setPanelOpen((open) => !open)
      }
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'k') {
        event.preventDefault()
        setPaletteOpen((open) => !open)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  const openNew = () => {
    const id = crypto.randomUUID()
    setSessionId(id)
    setHistoryKey((key) => key + 1)
    void ipc.settingsRememberSession(null)
  }

  const open = (session: SessionSummary) => {
    setSessionId(session.id)
    setHistoryKey((key) => key + 1)
    void ipc.settingsRememberSession(session.id)
  }

  const actions = useMemo<PaletteAction[]>(
    () => [
      { id: 'new', label: '新建会话', hint: '会话', run: openNew },
      { id: 'theme', label: theme === 'dark' ? '切到亮色主题' : '切到暗色主题', run: toggleTheme },
      {
        id: 'export',
        label: '导出当前会话为 Markdown',
        run: () => {
          void import('@/features/palette/CommandPalette.tsx').then((module) =>
            module.exportSession(sessionId),
          )
        },
      },
      { id: 'refresh', label: '重新读取会话列表', run: refresh },
    ],
    [refresh, sessionId, theme, toggleTheme],
  )

  if (!ready) {
    return <BootPanel onReady={() => setReady(true)} />
  }

  return (
    <div className="flex h-full">
      <SessionSidebar
        activeId={sessionId}
        refreshKey={refreshKey}
        onSelect={open}
        onNew={openNew}
        onChanged={refresh}
      />
      <main className="min-w-0 flex-1">
        <ChatPanel
          key={`${sessionId}-${historyKey}`}
          sessionId={sessionId}
          onActivity={refresh}
          onToggleTheme={toggleTheme}
          pendingPath={pendingPath}
          onPathConsumed={() => setPendingPath(null)}
        />
      </main>
      {panelOpen && (
        <WorkspacePanel onInsertPath={(path) => setPendingPath(path)} />
      )}
      {paletteOpen && (
        <CommandPalette
          sessions={sessions}
          actions={actions}
          onClose={() => setPaletteOpen(false)}
          onSelectSession={open}
          onOpenSession={(id) => {
            const known = sessions.find((session) => session.id === id)
            if (known) open(known)
            else {
              setSessionId(id)
              setHistoryKey((key) => key + 1)
            }
          }}
        />
      )}
    </div>
  )
}
