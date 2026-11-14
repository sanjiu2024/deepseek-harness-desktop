import { useState } from 'react'
import { BootPanel } from '@/features/boot/BootPanel.tsx'
import { ChatPanel } from '@/features/chat/ChatPanel.tsx'

/**
 * Shell. The boot screen owns starting and handshaking the kernel; once that
 * succeeds the session panel takes over. Routing arrives in M3 with history.
 */
export function App() {
  const [ready, setReady] = useState(false)
  // One id per app run: the kernel lazily creates the agent+session pair on the
  // first prompt, so there is nothing to allocate up front.
  const [sessionId] = useState(() => crypto.randomUUID())

  return ready ? <ChatPanel sessionId={sessionId} /> : <BootPanel onReady={() => setReady(true)} />
}
