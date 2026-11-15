import { useEffect, useState } from 'react'
import { FileTree } from './FileTree.tsx'
import { TerminalPanel } from './TerminalPanel.tsx'
import { ipc } from '@/lib/ipc.ts'

/**
 * The right-hand panel: what is on disk, and a shell pointed at it.
 *
 * One panel with two tabs rather than two panels, because a 280px column that has to hold
 * both would hold neither. `Ctrl/Cmd+J` toggles it.
 */
export function WorkspacePanel({ onInsertPath }: { onInsertPath: (path: string) => void }) {
  const [tab, setTab] = useState<'files' | 'terminal'>('files')
  const [preview, setPreview] = useState<{ path: string; text: string } | null>(null)
  const [error, setError] = useState<string | null>(null)
  // One shell per panel mount, created on first use: a terminal nobody opened must not
  // leave a process behind.
  const [terminalId, setTerminalId] = useState<string | null>(null)

  useEffect(() => {
    if (tab !== 'terminal' || terminalId !== null) return
    const id = crypto.randomUUID()
    void ipc
      .ptyOpen(id, 24, 80)
      .then(() => setTerminalId(id))
      .catch((cause: unknown) => setError(String(cause)))
  }, [tab, terminalId])

  useEffect(() => {
    return () => {
      if (terminalId !== null) void ipc.ptyClose(terminalId)
    }
  }, [terminalId])

  const open = async (path: string) => {
    setError(null)
    try {
      // `fsRead` returns the encoding too; the panel only needs the text today, and the
      // encoding is there for whichever UI wants to show it.
      const preview = await ipc.fsRead(path)
      setPreview({ path, text: preview.text })
    } catch (cause) {
      setError(String(cause))
    }
  }

  return (
    <aside className="border-border/60 bg-card/20 flex h-full w-72 shrink-0 flex-col border-l">
      <div className="border-border/60 flex border-b">
        {(['files', 'terminal'] as const).map((name) => (
          <button
            key={name}
            type="button"
            onClick={() => setTab(name)}
            className={`flex-1 px-2 py-1.5 text-[11px] transition-colors ${
              tab === name
                ? 'text-foreground border-foreground/60 border-b-2'
                : 'text-muted-foreground hover:text-foreground'
            }`}
          >
            {name === 'files' ? '文件' : '终端'}
          </button>
        ))}
      </div>

      <div className="min-h-0 flex-1 overflow-auto">
        {tab === 'files' ? (
          <FileTree onOpen={(path) => void open(path)} />
        ) : terminalId === null ? (
          <p className="text-muted-foreground px-2 py-1 text-[11px]">启动 shell…</p>
        ) : (
          <TerminalPanel id={terminalId} />
        )}
      </div>

      {error !== null && (
        <p className="text-destructive border-border/60 border-t px-2 py-1 text-[10px] whitespace-pre-wrap">
          {error}
        </p>
      )}

      {tab === 'files' && preview !== null && (
        <div className="border-border/60 flex max-h-[45%] flex-col border-t">
          <div className="flex items-center gap-1 px-2 py-1">
            <span className="text-muted-foreground truncate font-mono text-[10px]" title={preview.path}>
              {preview.path.split(/[\\/]/).pop()}
            </span>
            <button
              type="button"
              onClick={() => onInsertPath(preview.path)}
              title="把路径插到输入框"
              className="text-muted-foreground hover:text-foreground ml-auto shrink-0 rounded px-1 text-[10px] transition-colors"
            >
              插入路径
            </button>
            <button
              type="button"
              onClick={() => setPreview(null)}
              className="text-muted-foreground hover:text-foreground shrink-0 rounded px-1 text-[10px] transition-colors"
            >
              关闭
            </button>
          </div>
          <pre className="min-h-0 flex-1 overflow-auto px-2 pb-2 font-mono text-[10px] leading-relaxed whitespace-pre-wrap select-text">
            {preview.text.slice(0, 20_000)}
          </pre>
        </div>
      )}
    </aside>
  )
}
