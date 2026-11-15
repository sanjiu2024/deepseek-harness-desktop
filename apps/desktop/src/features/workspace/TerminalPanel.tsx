import { useEffect, useRef } from 'react'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import '@xterm/xterm/css/xterm.css'
import { ipc, onPtyData, onPtyExit } from '@/lib/ipc.ts'

/**
 * A real shell.
 *
 * The PTY lives in Rust and outlives this component, so switching tabs does not kill the
 * shell — a `ls` you started is still there when you come back. What lives here is the
 * emulator: it answers the terminal's own queries (cursor position, device attributes),
 * which is why a shell that looked hung in a bare pipe answers immediately here.
 */
export function TerminalPanel({ id }: { id: string }) {
  const host = useRef<HTMLDivElement>(null)
  const terminal = useRef<Terminal | null>(null)

  useEffect(() => {
    const element = host.current
    if (element === null) return

    const term = new Terminal({
      fontFamily: 'ui-monospace, SFMono-Regular, Consolas, monospace',
      fontSize: 12,
      cursorBlink: true,
      // The app paints its own background; an opaque canvas would cut a hole in the panel.
      theme: { background: '#00000000' },
      allowProposedApi: true,
    })
    const fit = new FitAddon()
    term.loadAddon(fit)
    term.open(element)
    fit.fit()
    terminal.current = term

    let disposed = false
    term.onData((data) => void ipc.ptyWrite(id, data))

    const offData = onPtyData((payload) => {
      if (payload.id !== id || disposed) return
      // Base64 in, bytes out: xterm must see raw bytes, or a split UTF-8 sequence becomes
      // replacement characters in the middle of Chinese output.
      const binary = atob(payload.data)
      const bytes = new Uint8Array(binary.length)
      for (let index = 0; index < binary.length; index += 1) {
        bytes[index] = binary.charCodeAt(index)
      }
      term.write(bytes)
    })
    const offExit = onPtyExit((payload) => {
      if (payload.id !== id) return
      term.write(`\r\n\x1b[2m[进程结束${payload.code === null ? '' : ` ${payload.code}`}]\x1b[0m\r\n`)
    })

    const observer = new ResizeObserver(() => {
      fit.fit()
      void ipc.ptyResize(id, term.rows, term.cols)
    })
    observer.observe(element)

    void offData.then((off) => {
      if (disposed) off()
    })
    void offExit.then((off) => {
      if (disposed) off()
    })

    return () => {
      disposed = true
      observer.disconnect()
      term.dispose()
    }
  }, [id])

  return <div ref={host} className="h-full w-full px-2 py-1" />
}
