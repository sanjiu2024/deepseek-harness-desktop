/**
 * The two kernel requests that block an agent until the client answers.
 *
 * Both arrive as JSON-RPC requests on the same stream as notifications, so they
 * are easy to miss: a client that only listens for events never sees them, and
 * the turn hangs with no error anywhere. That is why they live in their own
 * module with their own shapes rather than being folded into the event reducer.
 */

import type { FrameId } from './ipc.ts'

/** AskUserQuestionItem, narrowed to what a client has to render. */
export interface QuestionOption {
  label: string
  description?: string
}

export interface Question {
  id: string
  question: string
  detail?: string
  header?: string
  options?: QuestionOption[]
  multiSelect?: boolean
}

/** One decision waiting on the user. */
export type PendingInteraction =
  | {
      kind: 'approval'
      /** JSON-RPC id to answer, in whatever form the kernel used. */
      id: FrameId
      sessionId: string
      toolName: string
      callId?: string
      reason?: string
    }
  | {
      kind: 'question'
      id: FrameId
      requestId: string
      sessionId: string
      questions: Question[]
    }

/** The kernel's approval outcome vocabulary. Only the first one permits. */
export type ApprovalOutcome = 'allowed-once' | 'rejected' | 'cancelled'

/**
 * Classify one inbound frame as a blocking request.
 *
 * Returns `null` for everything else, including requests this client does not
 * implement — answering an unknown request wrongly is worse than not answering,
 * because the kernel would treat a malformed result as a user decision.
 */
export function parseInteraction(
  method: string,
  id: FrameId,
  params: unknown,
): PendingInteraction | null {
  const body = asRecord(params)
  if (body === null) return null

  if (method === 'approval/request') {
    const sessionId = asString(body.sessionId)
    const toolName = asString(body.toolName)
    if (sessionId === null || toolName === null) return null
    return {
      kind: 'approval',
      id,
      sessionId,
      toolName,
      ...optional('callId', asString(body.callId)),
      ...optional('reason', asString(body.reason)),
    }
  }

  if (method === 'user-question/ask') {
    const sessionId = asString(body.sessionId)
    const requestId = asString(body.requestId)
    if (sessionId === null || requestId === null) return null
    const questions = Array.isArray(body.questions)
      ? body.questions.map(readQuestion).filter((question): question is Question => question !== null)
      : []
    if (questions.length === 0) return null
    return { kind: 'question', id, requestId, sessionId, questions }
  }

  return null
}

/** The `approval/request` result body. */
export function approvalResult(outcome: ApprovalOutcome): { outcome: ApprovalOutcome } {
  return { outcome }
}

/** One answer per question, as `user-question/ask` expects. */
export function questionResult(
  answers: ReadonlyArray<{ id: string; selected: string[]; custom?: string }>,
): { answers: ReadonlyArray<{ id: string; selected: string[]; custom?: string }> } {
  return { answers }
}

/** The dismissal variant: the user closed the dialog without deciding. */
export function cancelledQuestion(): { cancelled: true } {
  return { cancelled: true }
}

function readQuestion(value: unknown): Question | null {
  const body = asRecord(value)
  if (body === null) return null
  const id = asString(body.id)
  const question = asString(body.question)
  if (id === null || question === null) return null

  const options = Array.isArray(body.options)
    ? body.options
        .map((option) => {
          const entry = asRecord(option)
          const label = entry === null ? null : asString(entry.label)
          if (label === null) return null
          const description = asString(entry?.description)
          return description === null ? { label } : { label, description }
        })
        .filter((option): option is QuestionOption => option !== null)
    : null

  return {
    id,
    question,
    ...optional('detail', asString(body.detail)),
    ...optional('header', asString(body.header)),
    ...(options === null || options.length === 0 ? {} : { options }),
    ...(body.multiSelect === true ? { multiSelect: true } : {}),
  }
}

/** Build a property only when the value exists, so optional fields stay absent. */
function optional<K extends string, V>(key: K, value: V | null): Record<K, V> | Record<string, never> {
  return value === null ? {} : ({ [key]: value } as Record<K, V>)
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return typeof value === 'object' && value !== null ? (value as Record<string, unknown>) : null
}

function asString(value: unknown): string | null {
  return typeof value === 'string' ? value : null
}
