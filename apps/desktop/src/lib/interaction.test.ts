import { describe, expect, it } from 'vitest'
import {
  approvalResult,
  cancelledQuestion,
  parseInteraction,
  questionResult,
} from './interaction.ts'

describe('parseInteraction', () => {
  it('reads an approval request', () => {
    const parsed = parseInteraction('approval/request', 7, {
      sessionId: 's1',
      toolName: 'bash',
      callId: 'call-1',
      reason: 'needs to write a file',
    })
    expect(parsed).toEqual({
      kind: 'approval',
      id: 7,
      sessionId: 's1',
      toolName: 'bash',
      callId: 'call-1',
      reason: 'needs to write a file',
    })
  })

  it('leaves absent optional approval fields absent', () => {
    const parsed = parseInteraction('approval/request', 1, { sessionId: 's', toolName: 'read' })
    expect(parsed).not.toBeNull()
    expect(parsed !== null && 'callId' in parsed).toBe(false)
    expect(parsed !== null && 'reason' in parsed).toBe(false)
  })

  it('rejects an approval without a tool name', () => {
    expect(parseInteraction('approval/request', 1, { sessionId: 's' })).toBeNull()
  })

  it('reads a user question with options', () => {
    const parsed = parseInteraction('user-question/ask', 3, {
      requestId: 'r1',
      sessionId: 's1',
      questions: [
        {
          id: 'q1',
          question: '选哪个？',
          header: '确认',
          multiSelect: true,
          options: [{ label: 'A', description: '第一个' }, { label: 'B' }],
        },
      ],
    })
    expect(parsed).toEqual({
      kind: 'question',
      id: 3,
      requestId: 'r1',
      sessionId: 's1',
      questions: [
        {
          id: 'q1',
          question: '选哪个？',
          header: '确认',
          multiSelect: true,
          options: [{ label: 'A', description: '第一个' }, { label: 'B' }],
        },
      ],
    })
  })

  it('drops questions with no id or text instead of rendering blanks', () => {
    const parsed = parseInteraction('user-question/ask', 1, {
      requestId: 'r',
      sessionId: 's',
      questions: [{ id: 'ok', question: 'fine' }, { id: 'broken' }, 'junk'],
    })
    expect(parsed !== null && parsed.kind === 'question' ? parsed.questions : []).toHaveLength(1)
  })

  it('ignores methods it does not implement', () => {
    expect(parseInteraction('something/else', 1, {})).toBeNull()
  })

  it('ignores a non-object params body', () => {
    expect(parseInteraction('approval/request', 1, null)).toBeNull()
  })
})

describe('result bodies', () => {
  it('wraps the approval outcome with the upstream key', () => {
    expect(approvalResult('allowed-once')).toEqual({ outcome: 'allowed-once' })
    expect(approvalResult('rejected')).toEqual({ outcome: 'rejected' })
  })

  it('wraps answers under `answers`', () => {
    expect(questionResult([{ id: 'q1', selected: ['A'] }])).toEqual({
      answers: [{ id: 'q1', selected: ['A'] }],
    })
  })

  it('uses the dismissal shape, not an empty answer', () => {
    expect(cancelledQuestion()).toEqual({ cancelled: true })
  })
})
