import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { Markdown } from './markdown.tsx'

describe('markdown', () => {
  it('renders a fenced block with its language and a copy button', async () => {
    const writeText = vi.fn(async () => {})
    Object.assign(navigator, { clipboard: { writeText } })

    render(<Markdown text={'```ts\nconst a = 1\n```'} />)

    expect(screen.getByText('ts')).toBeTruthy()
    await userEvent.click(screen.getByText('复制'))
    // The code, not the whole message: a copy button that copies too much is worse than none.
    expect(writeText).toHaveBeenCalledWith(expect.stringContaining('const a = 1'))
    expect(await screen.findByText('已复制')).toBeTruthy()
  })

  it('opens links outside the app', () => {
    // In-window navigation would replace the client with a web page and no way back.
    render(<Markdown text={'[文档](https://example.com)'} />)
    const link = screen.getByText('文档') as HTMLAnchorElement
    expect(link.target).toBe('_blank')
    expect(link.rel).toContain('noreferrer')
  })

  it('keeps inline code inline', () => {
    render(<Markdown text={'用 `pnpm test` 跑'} />)
    expect(screen.getByText('pnpm test').tagName).toBe('CODE')
    expect(screen.queryByText('复制')).toBeNull()
  })
})
