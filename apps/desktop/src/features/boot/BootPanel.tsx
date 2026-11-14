import { useEffect, useRef, useState } from 'react'
import {
  ipc,
  onKernelExit,
  onKernelFrame,
  onKernelLagged,
  type KernelExit,
  type KernelFrame,
  type KernelPaths,
} from '@/lib/ipc.ts'

type Phase = 'idle' | 'starting' | 'handshaking' | 'ready' | 'failed'

/** One line in the frame log. Kept small on purpose: this is a probe, not a UI. */
interface LogLine {
  id: number
  label: string
  detail: string
}

const MAX_LOG = 200

/**
 * M0 probe panel.
 *
 * Proves the whole path in one screen: React → invoke → Rust → kernel stdio →
 * broadcast → Tauri event → React. Everything below it in M1 replaces this with
 * the real workspace; until then this is the acceptance instrument and it stays
 * honest about what has and has not happened.
 */
export function BootPanel({ onReady }: { onReady: () => void }) {
  const [paths, setPaths] = useState<KernelPaths | null>(null)
  const [phase, setPhase] = useState<Phase>('idle')
  const [error, setError] = useState<string | null>(null)
  const [log, setLog] = useState<LogLine[]>([])
  const [dropped, setDropped] = useState(0)
  const nextId = useRef(1)

  const append = (label: string, detail: string) => {
    const id = nextId.current++
    setLog((previous) => [...previous, { id, label, detail }].slice(-MAX_LOG))
  }

  useEffect(() => {
    void ipc
      .kernelPaths()
      .then(setPaths)
      .catch((cause: unknown) => setError(String(cause)))
  }, [])

  useEffect(() => {
    const unlisteners: Array<() => void> = []
    let cancelled = false

    void (async () => {
      const frame = await onKernelFrame((incoming) => append(...describe(incoming)))
      const lagged = await onKernelLagged((skipped) => setDropped((total) => total + skipped))
      const exit = await onKernelExit((status) => {
        append('exit', describeExit(status))
        setPhase('idle')
      })
      if (cancelled) {
        frame()
        lagged()
        exit()
        return
      }
      unlisteners.push(frame, lagged, exit)
    })()

    return () => {
      cancelled = true
      for (const off of unlisteners) off()
    }
  }, [])

  const start = async () => {
    setError(null)
    setPhase('starting')
    try {
      await ipc.kernelStart()
      append('start', 'kernel process spawned')
      setPhase('handshaking')
      const result = await ipc.kernelInitialize({
        cwd: paths?.kernelDir ?? '.',
        provider: 'opencode-go',
        model: 'deepseek-v4.1-flash',
      })
      append('initialize', JSON.stringify(result))
      setPhase('ready')
      onReady()
    } catch (cause) {
      setError(String(cause))
      setPhase('failed')
    }
  }

  const stop = async () => {
    try {
      await ipc.kernelStop()
      setPhase('idle')
    } catch (cause) {
      setError(String(cause))
    }
  }

  const busy = phase === 'starting' || phase === 'handshaking'

  return (
    <div className="mx-auto flex h-full w-full max-w-4xl flex-col gap-4 p-6">
      <header className="space-y-1">
        <h1 className="text-xl font-semibold tracking-tight">DeepSeek Harness Desktop</h1>
        <p className="text-muted-foreground text-sm">
          M0 探针：这一步只证明 React → Rust → 内核 → 事件的链路通，不是最终界面。
        </p>
      </header>

      <dl className="grid grid-cols-[7rem_1fr] gap-x-4 gap-y-1 text-sm">
        <dt className="text-muted-foreground">内核目录</dt>
        <dd className="truncate font-mono text-xs" title={paths?.kernelDir}>
          {paths?.kernelDir ?? '…'}
        </dd>
        <dt className="text-muted-foreground">已拉取</dt>
        <dd>{paths === null ? '…' : paths.fetched ? '是' : '否 —— 先跑 pnpm kernel:fetch'}</dd>
        <dt className="text-muted-foreground">内核 HOME</dt>
        <dd className="truncate font-mono text-xs" title={paths?.home}>
          {paths?.home ?? '…'}
        </dd>
        <dt className="text-muted-foreground">状态</dt>
        <dd className="font-mono text-xs">{phase}</dd>
      </dl>

      <div className="flex gap-2">
        <button
          type="button"
          onClick={() => void start()}
          disabled={busy || paths?.fetched !== true}
          className="bg-primary text-primary-foreground rounded-md px-4 py-1.5 text-sm font-medium disabled:opacity-40"
        >
          {busy ? '握手中…' : '启动内核并握手'}
        </button>
        <button
          type="button"
          onClick={() => void stop()}
          disabled={phase === 'idle' || busy}
          className="bg-secondary text-secondary-foreground rounded-md px-4 py-1.5 text-sm font-medium disabled:opacity-40"
        >
          停止
        </button>
      </div>

      {error !== null && (
        <pre className="border-destructive/40 bg-destructive/10 text-destructive max-h-40 overflow-auto rounded-md border p-3 text-xs whitespace-pre-wrap">
          {error}
        </pre>
      )}

      <section className="border-border flex min-h-0 flex-1 flex-col rounded-md border">
        <div className="border-border flex items-center justify-between border-b px-3 py-1.5 text-xs">
          <span className="text-muted-foreground">内核帧（{log.length} 条）</span>
          {dropped > 0 && <span className="text-destructive">丢弃 {dropped} 条</span>}
        </div>
        <ol className="min-h-0 flex-1 select-text overflow-auto p-3 font-mono text-xs leading-relaxed">
          {log.length === 0 && <li className="text-muted-foreground">还没有帧。</li>}
          {log.map((line) => (
            <li key={line.id} className="flex gap-2">
              <span className="text-muted-foreground w-24 shrink-0 truncate">{line.label}</span>
              <span className="break-all">{line.detail}</span>
            </li>
          ))}
        </ol>
      </section>
    </div>
  )
}

function describe(frame: KernelFrame): [string, string] {
  switch (frame.kind) {
    case 'notification':
      return [frame.method, compact(frame.params)]
    case 'request':
      return [`${frame.method} #${frame.id}`, compact(frame.params)]
    case 'response':
      return [
        `response #${frame.id}`,
        frame.payload.status === 'ok' ? compact(frame.payload) : `${frame.payload.message}`,
      ]
  }
}

function describeExit(status: KernelExit): string {
  const how = status.requested ? 'requested' : 'unexpected'
  return `code=${status.code ?? 'n/a'} (${how})`
}

function compact(value: unknown): string {
  const text = JSON.stringify(value)
  if (text === undefined) return ''
  return text.length > 240 ? `${text.slice(0, 240)}…` : text
}
