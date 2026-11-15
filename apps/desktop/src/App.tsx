import { useState } from 'react'
import { BootPanel } from '@/features/boot/BootPanel.tsx'
import { ChatPanel } from '@/features/chat/ChatPanel.tsx'
import { SessionSidebar } from '@/features/sessions/SessionSidebar.tsx'
import type { SessionSummary } from '@/lib/ipc.ts'

/**
 * Shell.
 *
 * Two panes once the kernel is running: what has happened (the store's session
 * list) and what is happening (the conversation). Before that, the launch screen
 * owns the window — starting a kernel is the one step that cannot be implicit.
 */
export function App() {
  const [ready, setReady] = useState(false)
  const [sessionId, setSessionId] = useState<string>(() => crypto.randomUUID())
  const [historyKey, setHistoryKey] = useState(0)
  const [refreshKey, setRefreshKey] = useState(0)

  if (!ready) {
    return <BootPanel onReady={() => setReady(true)} />
  }

  const openNew = () => {
    setSessionId(crypto.randomUUID())
    // Remounting is the honest way to drop the old conversation: the panel owns
    // its state, and a fresh session has no history to hydrate.
    setHistoryKey((key) => key + 1)
  }

  const open = (session: SessionSummary) => {
    setSessionId(session.id)
    setHistoryKey((key) => key + 1)
  }

  return (
    <div className="flex h-full">
      <SessionSidebar
        activeId={sessionId}
        refreshKey={refreshKey}
        onSelect={open}
        onNew={openNew}
      />
      <main className="min-w-0 flex-1">
        <ChatPanel
          key={`${sessionId}-${historyKey}`}
          sessionId={sessionId}
          onActivity={() => setRefreshKey((key) => key + 1)}
        />
      </main>
    </div>
  )
}
