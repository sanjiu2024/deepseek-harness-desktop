import { useCallback, useEffect, useState } from 'react'
import { ipc, type TreeEntry } from '@/lib/ipc.ts'

/**
 * The workspace tree.
 *
 * One directory per request: a tree that reads everything up front freezes on a large
 * repository, and most of what it reads is never opened.
 */
export function FileTree({ onOpen }: { onOpen: (path: string) => void }) {
  const [roots, setRoots] = useState<TreeEntry[] | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    void ipc
      .fsTree(null)
      .then(setRoots)
      .catch((cause: unknown) => setError(String(cause)))
  }, [])

  if (error !== null) {
    return <p className="text-destructive px-2 py-1 text-[11px] whitespace-pre-wrap">{error}</p>
  }
  if (roots === null) {
    return <p className="text-muted-foreground px-2 py-1 text-[11px]">读取中…</p>
  }

  return (
    <ul className="flex flex-col">
      {roots.map((entry) => (
        <Node key={entry.path} entry={entry} depth={0} onOpen={onOpen} />
      ))}
    </ul>
  )
}

function Node({
  entry,
  depth,
  onOpen,
}: {
  entry: TreeEntry
  depth: number
  onOpen: (path: string) => void
}) {
  const [open, setOpen] = useState(false)
  const [children, setChildren] = useState<TreeEntry[] | null>(null)
  const [error, setError] = useState<string | null>(null)

  const load = useCallback(async () => {
    try {
      setChildren(await ipc.fsTree(entry.path))
    } catch (cause) {
      setError(String(cause))
    }
  }, [entry.path])

  const toggle = () => {
    const next = !open
    setOpen(next)
    if (next && children === null) void load()
  }

  const directory = entry.kind === 'directory'

  return (
    <li>
      <button
        type="button"
        onClick={() => (directory ? toggle() : onOpen(entry.path))}
        className="hover:bg-secondary/40 flex w-full items-center gap-1 rounded px-1 py-0.5 text-left text-[11px] transition-colors"
        style={{ paddingLeft: `${depth * 10 + 4}px` }}
        title={entry.path}
      >
        <span className="text-muted-foreground w-3 shrink-0 text-center">
          {directory ? (open ? '▾' : '▸') : '·'}
        </span>
        <span className={`truncate ${directory ? 'font-medium' : ''}`}>{entry.name}</span>
      </button>

      {open && children !== null && (
        <ul className="flex flex-col">
          {children.map((child) => (
            <Node key={child.path} entry={child} depth={depth + 1} onOpen={onOpen} />
          ))}
        </ul>
      )}
      {error !== null && (
        <p className="text-destructive pl-4 text-[10px] whitespace-pre-wrap">{error}</p>
      )}
    </li>
  )
}
