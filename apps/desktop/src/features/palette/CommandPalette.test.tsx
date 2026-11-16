import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { CommandPalette, type PaletteAction } from './CommandPalette.tsx'
import type { SessionSummary } from '@/lib/ipc.ts'

// The palette reaches the kernel for content search; a real  does not exist in jsdom.
const invoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invoke(...args),
}))

beforeEach(() => {
  invoke.mockReset()
})

const sessions: SessionSummary[] = [
  {
    id: 'a',
    title: '关于内核的对话',
    cwd: null,
    createdAt: 1,
    updatedAt: 2,
    eventCount: 3,
    isSubagent: false,
    parentId: null,
  },
]

const actions: PaletteAction[] = [
  { id: 'new', label: '新建会话', run: () => {} },
  { id: 'theme', label: '切到亮色主题', run: () => {} },
]

describe('command palette', () => {
  it('lists every action when nothing is typed', () => {
    render(
      <CommandPalette sessions={sessions} actions={actions} onClose={() => {}} onSelectSession={() => {}}
        onOpenSession={() => {}} />,
    )
    expect(screen.getByText('新建会话')).toBeTruthy()
    expect(screen.getByText('切到亮色主题')).toBeTruthy()
  })

  it('finds an action by name and runs it on Enter', async () => {
    const run = vi.fn()
    const onClose = vi.fn()
    render(
      <CommandPalette
        sessions={sessions}
        actions={[{ id: 'theme', label: '切到亮色主题', run }]}
        onClose={onClose}
        onSelectSession={() => {}}
        onOpenSession={() => {}}
      />,
    )

    await userEvent.type(screen.getByRole('textbox'), '亮色')
    await userEvent.keyboard('{Enter}')

    expect(run).toHaveBeenCalled()
    expect(onClose).toHaveBeenCalled()
  })

  it('finds a session by title', async () => {
    const picked: string[] = []
    render(
      <CommandPalette
        sessions={sessions}
        actions={[]}
        onClose={() => {}}
        onSelectSession={(session) => picked.push(session.id)}
        onOpenSession={() => {}}
      />,
    )

    await userEvent.type(screen.getByRole('textbox'), '内核')
    await userEvent.click(screen.getByText('关于内核的对话'))

    expect(picked).toEqual(['a'])
  })

  it('says so when nothing matches instead of showing an empty box', async () => {
    render(
      <CommandPalette sessions={sessions} actions={actions} onClose={() => {}} onSelectSession={() => {}}
        onOpenSession={() => {}} />,
    )
    await userEvent.type(screen.getByRole('textbox'), 'zzzz')
    expect(screen.getByText('没有匹配项')).toBeTruthy()
  })

  it('closes on Escape', async () => {
    const onClose = vi.fn()
    render(
      <CommandPalette sessions={[]} actions={actions} onClose={onClose} onSelectSession={() => {}}
        onOpenSession={() => {}} />,
    )
    await userEvent.keyboard('{Escape}')
    expect(onClose).toHaveBeenCalled()
  })
})

describe('command palette — content search', () => {
  it('searches session text once the query is long enough, and opens the hit', async () => {
    const opened: string[] = []
    invoke.mockResolvedValue({
      hits: [{ sessionId: 'a', title: '关于内核', seq: 3, time: 1, snippet: '…怎么配置 Tauri 的窗口…' }],
      truncated: false,
      scannedBytes: 1024,
    })
    render(
      <CommandPalette
        sessions={[]}
        actions={[]}
        onClose={() => {}}
        onSelectSession={() => {}}
        onOpenSession={(id) => opened.push(id)}
      />,
    )

    await userEvent.type(screen.getByRole('textbox'), 'Tauri')
    const hit = await screen.findByText('…怎么配置 Tauri 的窗口…')
    await userEvent.click(hit)

    expect(invoke).toHaveBeenCalledWith('session_search', { query: 'Tauri', limit: undefined })
    expect(opened).toEqual(['a'])
  })

  it('does not hit the archive for a single character', async () => {
    render(
      <CommandPalette
        sessions={[]}
        actions={[]}
        onClose={() => {}}
        onSelectSession={() => {}}
        onOpenSession={() => {}}
      />,
    )
    await userEvent.type(screen.getByRole('textbox'), 'T')
    // One keystroke is never a question about the archive.
    const searched = invoke.mock.calls.some((call) => call[0] === 'session_search')
    expect(searched).toBe(false)
  })
})
