import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { SessionSidebar } from './SessionSidebar.tsx'
import type { SessionSummary } from '@/lib/ipc.ts'

const invoke = vi.fn()

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invoke(...args),
}))

function session(id: string, title: string, ago: number): SessionSummary {
  return {
    id,
    title,
    cwd: null,
    createdAt: 1,
    updatedAt: Date.now() - ago,
    eventCount: 3,
    isSubagent: false,
    parentId: null,
  }
}

const HOUR = 3_600_000
const DAY = 24 * HOUR

beforeEach(() => {
  invoke.mockReset()
})

describe('session sidebar — grouping and filtering', () => {
  it('buckets by the day people remember, not by timestamp', async () => {
    invoke.mockResolvedValue([
      session('a', '今天的对话', HOUR),
      session('b', '昨天的对话', DAY + HOUR),
      session('c', '很久以前', 30 * DAY),
    ])
    render(<SessionSidebar activeId="a" refreshKey={0} onSelect={() => {}} onNew={() => {}} />)

    await screen.findByText('今天的对话')
    expect(screen.getByText('今天')).toBeTruthy()
    expect(screen.getByText('昨天')).toBeTruthy()
    expect(screen.getByText('更早')).toBeTruthy()
  })

  it('filters as you type and says so when nothing matches', async () => {
    invoke.mockResolvedValue([session('a', '关于内核', HOUR), session('b', '关于界面', HOUR)])
    render(<SessionSidebar activeId="a" refreshKey={0} onSelect={() => {}} onNew={() => {}} />)

    await screen.findByText('关于内核')
    await userEvent.type(screen.getByPlaceholderText('筛选会话'), '界面')

    expect(screen.queryByText('关于内核')).toBeNull()
    expect(screen.getByText('关于界面')).toBeTruthy()

    await userEvent.clear(screen.getByPlaceholderText('筛选会话'))
    await userEvent.type(screen.getByPlaceholderText('筛选会话'), 'zzz')
    expect(screen.getByText('没有匹配的会话。')).toBeTruthy()
  })
})
