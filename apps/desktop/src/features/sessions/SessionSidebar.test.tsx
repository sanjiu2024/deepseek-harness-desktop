import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { SessionSidebar } from './SessionSidebar.tsx'
import type { SessionSummary } from '@/lib/ipc.ts'

const invoke = vi.fn()

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invoke(...args),
}))

function session(id: string, title: string, overrides: Partial<SessionSummary> = {}): SessionSummary {
  return {
    id,
    title,
    cwd: 'D:\\repo',
    createdAt: 1,
    updatedAt: Date.now() - 5 * 60_000,
    eventCount: 12,
    isSubagent: false,
    parentId: null,
    ...overrides,
  }
}

beforeEach(() => {
  invoke.mockReset()
})

describe('session sidebar', () => {
  it('lists what the store returned, newest first as the store ordered it', async () => {
    invoke.mockResolvedValue([session('a', '第一个'), session('b', '第二个')])
    render(<SessionSidebar activeId="a" refreshKey={0} onSelect={() => {}} onNew={() => {}} />)

    expect(await screen.findByText('第一个')).toBeTruthy()
    expect(screen.getByText('第二个')).toBeTruthy()
    expect(invoke).toHaveBeenCalledWith('session_list')
  })

  it('opens a session when clicked', async () => {
    const opened: string[] = []
    invoke.mockResolvedValue([session('a', '第一个'), session('b', '第二个')])
    render(
      <SessionSidebar activeId="a" refreshKey={0} onSelect={(s) => opened.push(s.id)} onNew={() => {}} />,
    )

    await userEvent.click(await screen.findByText('第二个'))
    expect(opened).toEqual(['b'])
  })

  it('nests a subagent run under the session that spawned it', async () => {
    invoke.mockResolvedValue([
      session('parent', '主线'),
      session('child', '子任务', { isSubagent: true, parentId: 'parent' }),
    ])
    render(<SessionSidebar activeId="x" refreshKey={0} onSelect={() => {}} onNew={() => {}} />)

    await screen.findByText('主线')
    // The nesting marker is what tells a reader these two are one conversation.
    expect(screen.getByText('↳')).toBeTruthy()
    expect(screen.getByText('子任务')).toBeTruthy()
  })

  it('re-reads the store when the shell asks it to', async () => {
    invoke.mockResolvedValue([session('a', '第一个')])
    const { rerender } = render(
      <SessionSidebar activeId="a" refreshKey={0} onSelect={() => {}} onNew={() => {}} />,
    )
    await screen.findByText('第一个')
    expect(invoke).toHaveBeenCalledTimes(1)

    rerender(<SessionSidebar activeId="a" refreshKey={1} onSelect={() => {}} onNew={() => {}} />)
    await waitFor(() => expect(invoke).toHaveBeenCalledTimes(2))
  })

  it('says so when there is nothing yet, instead of looking broken', async () => {
    invoke.mockResolvedValue([])
    render(<SessionSidebar activeId="a" refreshKey={0} onSelect={() => {}} onNew={() => {}} />)
    expect(await screen.findByText('还没有历史会话。发一句话试试。')).toBeTruthy()
  })
})
