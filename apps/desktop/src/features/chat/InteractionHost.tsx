import { useState } from 'react'
import {
  type ApprovalOutcome,
  type PendingInteraction,
  type Question,
} from '@/lib/interaction.ts'

/**
 * The blocking-request host.
 *
 * Only one decision is shown at a time: two stacked modals is how a user ends up
 * clicking the wrong "allow". The rest stay queued and surface in order.
 */
export function InteractionHost({
  interaction,
  onAnswer,
  onQuestion,
  onCancelQuestion,
}: {
  interaction: PendingInteraction
  onAnswer: (outcome: ApprovalOutcome) => void
  onQuestion: (answers: Array<{ id: string; selected: string[]; custom?: string }>) => void
  onCancelQuestion: () => void
}) {
  return interaction.kind === 'approval' ? (
    <ApprovalDialog interaction={interaction} onAnswer={onAnswer} />
  ) : (
    <QuestionDialog
      interaction={interaction}
      onAnswer={onQuestion}
      onCancel={onCancelQuestion}
    />
  )
}

function ApprovalDialog({
  interaction,
  onAnswer,
}: {
  interaction: Extract<PendingInteraction, { kind: 'approval' }>
  onAnswer: (outcome: ApprovalOutcome) => void
}) {
  const short = interaction.sessionId.slice(0, 8)
  return (
    <Shell title="这个工具需要你批准">
      <dl className="grid grid-cols-[5rem_1fr] gap-x-3 gap-y-1 text-sm">
        <dt className="text-muted-foreground">工具</dt>
        <dd className="font-mono text-xs">{interaction.toolName}</dd>
        <dt className="text-muted-foreground">会话</dt>
        <dd className="font-mono text-xs">{short}</dd>
        {interaction.callId !== undefined && (
          <>
            <dt className="text-muted-foreground">调用</dt>
            <dd className="font-mono text-xs break-all">{interaction.callId}</dd>
          </>
        )}
      </dl>

      {interaction.reason !== undefined && (
        <p className="bg-secondary/40 rounded-md p-2 text-xs whitespace-pre-wrap select-text">
          {interaction.reason}
        </p>
      )}

      <p className="text-muted-foreground text-xs">
        允许只对这一次生效，不是长期授权。
      </p>

      <div className="flex justify-end gap-2">
        <button
          type="button"
          onClick={() => onAnswer('rejected')}
          className="bg-secondary text-secondary-foreground rounded-md px-3 py-1.5 text-sm"
        >
          拒绝
        </button>
        <button
          type="button"
          onClick={() => onAnswer('allowed-once')}
          className="bg-primary text-primary-foreground rounded-md px-3 py-1.5 text-sm font-medium"
        >
          允许这一次
        </button>
      </div>
    </Shell>
  )
}

function QuestionDialog({
  interaction,
  onAnswer,
  onCancel,
}: {
  interaction: Extract<PendingInteraction, { kind: 'question' }>
  onAnswer: (answers: Array<{ id: string; selected: string[]; custom?: string }>) => void
  onCancel: () => void
}) {
  const [draft, setDraft] = useState<Record<string, string[]>>({})
  const [custom, setCustom] = useState<Record<string, string>>({})

  const toggle = (question: Question, label: string) => {
    setDraft((previous) => {
      const current = previous[question.id] ?? []
      const next = question.multiSelect === true
        ? current.includes(label)
          ? current.filter((entry) => entry !== label)
          : [...current, label]
        : [label]
      return { ...previous, [question.id]: next }
    })
  }

  const submit = () => {
    onAnswer(
      interaction.questions.map((question) => {
        const selected = draft[question.id] ?? []
        const text = custom[question.id]?.trim() ?? ''
        return text === ''
          ? { id: question.id, selected }
          : { id: question.id, selected, custom: text }
      }),
    )
  }

  return (
    <Shell title="模型在问你">
      <ul className="flex flex-col gap-4">
        {interaction.questions.map((question) => (
          <li key={question.id} className="flex flex-col gap-2">
            {question.header !== undefined && (
              <span className="text-muted-foreground text-xs">{question.header}</span>
            )}
            <p className="text-sm font-medium whitespace-pre-wrap select-text">{question.question}</p>
            {question.detail !== undefined && (
              <p className="text-muted-foreground text-xs whitespace-pre-wrap select-text">
                {question.detail}
              </p>
            )}

            {question.options !== undefined && question.options.length > 0 && (
              <div className="flex flex-col gap-1">
                {question.options.map((option) => {
                  const active = (draft[question.id] ?? []).includes(option.label)
                  return (
                    <button
                      key={option.label}
                      type="button"
                      onClick={() => toggle(question, option.label)}
                      className={`rounded-md border px-3 py-1.5 text-left text-sm ${
                        active
                          ? 'border-primary bg-primary/10'
                          : 'border-input hover:bg-secondary/40'
                      }`}
                    >
                      <span>{option.label}</span>
                      {option.description !== undefined && (
                        <span className="text-muted-foreground ml-2 text-xs">{option.description}</span>
                      )}
                    </button>
                  )
                })}
              </div>
            )}

            <input
              value={custom[question.id] ?? ''}
              onChange={(event) =>
                setCustom((previous) => ({ ...previous, [question.id]: event.target.value }))
              }
              placeholder="或者自己填"
              className="border-input bg-card rounded-md border px-2 py-1 text-sm outline-none select-text"
            />
          </li>
        ))}
      </ul>

      <div className="flex justify-end gap-2">
        <button
          type="button"
          onClick={onCancel}
          className="bg-secondary text-secondary-foreground rounded-md px-3 py-1.5 text-sm"
        >
          放弃
        </button>
        <button
          type="button"
          onClick={submit}
          className="bg-primary text-primary-foreground rounded-md px-3 py-1.5 text-sm font-medium"
        >
          回答
        </button>
      </div>
    </Shell>
  )
}

/** Modal frame shared by both dialogs. */
function Shell({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-6">
      <div className="bg-card border-border flex w-full max-w-lg flex-col gap-3 rounded-lg border p-4 shadow-xl">
        <h2 className="text-sm font-semibold">{title}</h2>
        {children}
      </div>
    </div>
  )
}
