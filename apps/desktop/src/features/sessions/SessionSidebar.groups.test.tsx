import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { SessionSidebar } from './SessionSidebar.tsx'
import type { SessionSummary } from '@/lib/ipc.ts'

const invoke = vi.fn()

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invoke(...args),
}))

function session(id: string, title: string, updatedAt: number): SessionSummary {
  return {
    id,
    title,
    cwd: null,
    createdAt: 1,
    updatedAt,
    eventCount: 3,
    isSubagent: false,
    parentId: null,
  }
}

// Fixtures are anchored to **local midnight**, not to "one hour ago".
//
// The first version used `Date.now() - HOUR` and passed locally, then failed on CI: the
// runner was at 00:09 UTC, so "an hour ago" was the previous day and the fixture landed in
// 昨天 instead of 今天. A test that depends on the wall clock is a test that fails at night.
const MIDNIGHT = (() => {
  const start = new Date()
  start.setHours(0, 0, 0, 0)
  return start.getTime()
})()
const HOUR = 3_600_000
const DAY = 24 * HOUR
/** 30 minutes after local midnight: unambiguously "today" whatever the hour. */
const TODAY = MIDNIGHT + 30 * 60_000
const YESTERDAY = MIDNIGHT - HOUR
const LONG_AGO = MIDNIGHT - 30 * DAY

beforeEach(() => {
  invoke.mockReset()
})

describe('session sidebar — grouping and filtering', () => {
  it('buckets by the day people remember, not by timestamp', async () => {
    invoke.mockResolvedValue([
      session('a', '今天的对话', TODAY),
      session('b', '昨天的对话', YESTERDAY),
      session('c', '很久以前', LONG_AGO),
    ])
    render(<SessionSidebar activeId="a" refreshKey={0} onSelect={() => {}} onNew={() => {}} />)

    await screen.findByText('今天的对话')
    expect(screen.getByText('今天')).toBeTruthy()
    expect(screen.getByText('昨天')).toBeTruthy()
    expect(screen.getByText('更早')).toBeTruthy()
  })

  it('filters as you type and says so when nothing matches', async () => {
    invoke.mockResolvedValue([session('a', '关于内核', TODAY), session('b', '关于界面', TODAY)])
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
