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

/** One line in the frame log. */
interface LogLine {
  id: number
  label: string
  detail: string
}

const MAX_LOG = 200
const PROVIDER = 'opencode-go'
const MODEL = 'deepseek-v4.1-flash'

/**
 * Launch screen.
 *
 * Starting a kernel is the one step the app cannot do silently — it spawns a
 * process, needs a key, and can be confused by twelve different things. So it
 * says what it found, what it is doing, and what failed, before handing over to
 * the conversation.
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

  // Auto-start once, after the paths are known: the button needs the kernel
  // directory to exist, and starting before that reads as a failure.
  const started = useRef(false)
  useEffect(() => {
    if (started.current || paths?.autoStart !== true || paths.fetched !== true) return
    started.current = true
    void start()
  }, [paths])

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
        provider: PROVIDER,
        model: MODEL,
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
    <div className="relative flex h-full w-full items-center justify-center overflow-hidden p-6">
      {/* A soft wash behind the card. An empty black rectangle reads as broken;
          a lit one reads as waiting. */}
      <div
        aria-hidden="true"
        className="pointer-events-none absolute inset-0"
        style={{
          background:
            'radial-gradient(52rem 26rem at 50% -8%, oklch(0.34 0.10 265 / 40%), transparent 70%)',
        }}
      />

      <div className="animate-pop-in border-border/70 bg-card/70 relative w-full max-w-md rounded-2xl border p-6 shadow-2xl backdrop-blur">
        <div className="flex flex-col items-center gap-1 text-center">
          <div className="bg-primary/10 ring-primary/20 mb-2 flex size-12 items-center justify-center rounded-2xl ring-1">
            <svg viewBox="0 0 24 24" aria-hidden="true" className="text-primary size-6">
              <circle cx="12" cy="12" r="3.6" fill="currentColor" />
              <circle cx="12" cy="12" r="8.4" fill="none" stroke="currentColor" strokeWidth="1.5" opacity="0.5" />
            </svg>
          </div>
          <h1 className="text-base font-semibold tracking-tight">DeepSeek Harness Desktop</h1>
          <p className="text-muted-foreground text-xs">只取内核，界面自己写</p>
        </div>

        <dl className="mt-5 grid grid-cols-[4.5rem_1fr] gap-x-3 gap-y-2 text-xs">
          <dt className="text-muted-foreground">内核</dt>
          <dd className={paths?.fetched === false ? 'text-amber-300' : ''}>
            {paths === null ? '…' : paths.fetched ? '已就绪' : '未拉取 —— 先跑 pnpm kernel:fetch'}
          </dd>
          <dt className="text-muted-foreground">路由</dt>
          <dd className="truncate font-mono opacity-80">
            {PROVIDER} / {MODEL}
          </dd>
          <dt className="text-muted-foreground">状态</dt>
          <dd className="flex items-center gap-1.5 font-mono">
            {busy && <span className="animate-breathe inline-block size-1.5 rounded-full bg-current" />}
            {phase}
          </dd>
        </dl>

        {error !== null && (
          <pre className="border-destructive/40 bg-destructive/10 text-destructive mt-4 max-h-40 overflow-auto rounded-lg border p-2.5 text-[11px] whitespace-pre-wrap">
            {error}
          </pre>
        )}

        <div className="mt-5 flex gap-2">
          <button
            type="button"
            onClick={() => void start()}
            disabled={busy || paths?.fetched !== true}
            className="bg-primary text-primary-foreground hover:bg-primary/90 flex-1 rounded-lg px-4 py-2 text-sm font-medium transition-all active:scale-[0.98] disabled:opacity-30"
          >
            {busy ? '握手中…' : '启动内核并握手'}
          </button>
          <button
            type="button"
            onClick={() => void stop()}
            disabled={phase === 'idle' || busy}
            className="bg-secondary text-secondary-foreground hover:bg-secondary/70 rounded-lg px-3 py-2 text-sm transition-colors active:scale-[0.98] disabled:opacity-30"
          >
            停止
          </button>
        </div>

        {dropped > 0 && <p className="text-destructive mt-2 text-[11px]">丢弃 {dropped} 条帧</p>}

        {log.length > 0 && (
          <details className="text-muted-foreground mt-4 text-[11px]">
            <summary className="cursor-pointer select-none">启动日志 · {log.length} 条</summary>
            <ol className="mt-1 max-h-32 overflow-auto font-mono">
              {log.map((line) => (
                <li key={line.id} className="flex gap-2">
                  <span className="w-20 shrink-0 truncate opacity-60">{line.label}</span>
                  <span className="break-all">{line.detail}</span>
                </li>
              ))}
            </ol>
          </details>
        )}
      </div>
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
