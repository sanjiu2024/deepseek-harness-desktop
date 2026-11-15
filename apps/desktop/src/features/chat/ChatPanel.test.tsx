import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { ChatPanel } from './ChatPanel.tsx'

/**
 * The last hop: does clicking the approval button actually send the decision?
 *
 * Everything below this was covered by Rust tests; everything above by the
 * reducer and interaction tests. This is the seam that had no test at all, and
 * it is the one a user's finger touches.
 */

const invoke = vi.fn()
let deliver: ((frame: unknown) => void) | null = null

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invoke(...args),
}))

vi.mock('@tauri-apps/api/event', () => ({
  listen: async (_name: string, handler: (event: { payload: unknown }) => void) => {
    deliver = (frame: unknown) => handler({ payload: frame })
    return () => {}
  },
}))

const SESSION = 'session-1'

function approvalFrame(id: string) {
  return {
    kind: 'request',
    id,
    method: 'approval/request',
    params: { sessionId: SESSION, toolName: 'pwsh', callId: 'call_1', reason: 'needs to write outside the workspace' },
  }
}

beforeEach(() => {
  invoke.mockReset()
  invoke.mockResolvedValue(undefined)
  deliver = null
})

describe('approval dialog', () => {
  it('opens for a request frame and sends the decision with the kernel string id', async () => {
    render(<ChatPanel sessionId={SESSION} />)

    // The listener registers in an effect.
    await waitFor(() => expect(deliver).not.toBeNull())
    deliver?.(approvalFrame('req_abc123'))

    const allow = await screen.findByRole('button', { name: '允许这一次' })
    await userEvent.click(allow)

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith('kernel_answer', {
        id: 'req_abc123',
        result: { outcome: 'allowed-once' },
      })
    })
  })

  it('sends a rejection as rejected, not as a cancel', async () => {
    render(<ChatPanel sessionId={SESSION} />)
    await waitFor(() => expect(deliver).not.toBeNull())
    deliver?.(approvalFrame('req_def456'))

    await userEvent.click(await screen.findByRole('button', { name: '拒绝' }))

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith('kernel_answer', {
        id: 'req_def456',
        result: { outcome: 'rejected' },
      })
    })
  })

  it('closes the dialog once the answer is sent', async () => {
    render(<ChatPanel sessionId={SESSION} />)
    await waitFor(() => expect(deliver).not.toBeNull())
    deliver?.(approvalFrame('req_ghi789'))

    await userEvent.click(await screen.findByRole('button', { name: '允许这一次' }))

    await waitFor(() => {
      expect(screen.queryByRole('button', { name: '允许这一次' })).toBeNull()
    })
  })

  it('shows the tool card as waiting for approval, not as running', async () => {
    render(<ChatPanel sessionId={SESSION} />)
    await waitFor(() => expect(deliver).not.toBeNull())

    deliver?.({
      kind: 'notification',
      method: 'session.event',
      params: {
        sessionId: SESSION,
        event: { type: 'tool/call', seq: 1, time: 0, data: { callId: 'call_1', name: 'pwsh', arguments: '{"command":"ls"}' } },
      },
    })
    deliver?.(approvalFrame('req_jkl012'))

    expect(await screen.findByText('等待你批准')).toBeTruthy()
  })
})
