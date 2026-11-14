import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'

/**
 * Markdown for assistant text.
 *
 * The kernel sends plain markdown; rendering it as preformatted text is what
 * makes an answer read like source code. Raw HTML stays disabled on purpose —
 * the text comes from a model, and the client has no reason to execute it.
 */
export function Markdown({ text }: { text: string }) {
  return (
    <div className="markdown">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          // Links open outside the app; an in-window navigation would replace
          // the client with a web page and no way back.
          a: ({ href, children }) => (
            <a href={href} target="_blank" rel="noreferrer noopener">
              {children}
            </a>
          ),
        }}
      >
        {text}
      </ReactMarkdown>
    </div>
  )
}
