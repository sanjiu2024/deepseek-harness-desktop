import { useEffect, useRef, useState } from 'react'
import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'

/**
 * Markdown for assistant text.
 *
 * The kernel sends plain markdown; rendering it as preformatted text is what makes an answer
 * read like source code. Raw HTML stays disabled on purpose — the text comes from a model, and
 * the client has no reason to execute it.
 *
 * Code blocks get a language label and a copy button, because the single most common thing a
 * person does with an answer containing code is take the code somewhere else.
 */
export function Markdown({ text }: { text: string }) {
  return (
    <div className="markdown">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          // Links open outside the app; an in-window navigation would replace the client with
          // a web page and no way back.
          a: ({ href, children }) => (
            <a href={href} target="_blank" rel="noreferrer noopener">
              {children}
            </a>
          ),
          pre: ({ children }) => <CodeBlock>{children}</CodeBlock>,
          // A bare `code` inside a paragraph is inline; keep it quiet.
          code: ({ className, children }) =>
            className === undefined ? (
              <code>{children}</code>
            ) : (
              <code className={className}>{children}</code>
            ),
        }}
      >
        {text}
      </ReactMarkdown>
    </div>
  )
}

/**
 * A fenced block with its language and a copy button.
 *
 * The language comes from the `class="language-*"` react-markdown puts on the inner `<code>`.
 * It is read after mount, not during render: on the first render the ref is still null, and a
 * label computed then would say "text" for every block in the transcript.
 */
function CodeBlock({ children }: { children?: React.ReactNode }) {
  const host = useRef<HTMLDivElement>(null)
  const [copied, setCopied] = useState(false)
  const [language, setLanguage] = useState<string | null>(null)

  useEffect(() => {
    setLanguage(readLanguage(host.current))
  }, [])

  useEffect(() => {
    if (!copied) return
    const timer = setTimeout(() => setCopied(false), 1500)
    return () => clearTimeout(timer)
  }, [copied])

  const copy = () => {
    // Read at click time, not render time: the text is whatever is on screen now.
    const code = host.current?.textContent ?? ''
    void navigator.clipboard.writeText(code).then(() => setCopied(true))
  }

  return (
    <div className="group/code border-border/70 bg-background/60 relative my-2 overflow-hidden rounded-lg border">
      <div className="border-border/50 flex items-center justify-between border-b px-2 py-1">
        <span className="text-muted-foreground font-mono text-[10px] uppercase">
          {language ?? 'text'}
        </span>
        <button
          type="button"
          onClick={copy}
          className="text-muted-foreground hover:text-foreground rounded px-1.5 py-0.5 font-mono text-[10px] opacity-0 transition-all group-hover/code:opacity-100 focus-visible:opacity-100"
        >
          {copied ? '已复制' : '复制'}
        </button>
      </div>
      <div ref={host} className="overflow-x-auto">
        <pre className="m-0 px-3 py-2 font-mono text-[11.5px] leading-relaxed select-text">
          {children}
        </pre>
      </div>
    </div>
  )
}

/** `language-ts` → `ts`; nothing means the fence had no language. */
function readLanguage(host: HTMLElement | null): string | null {
  const code = host?.querySelector('code')
  const match = /language-([\w+-]+)/.exec(code?.className ?? '')
  return match?.[1] ?? null
}
