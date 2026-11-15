import { useState } from 'react'
import type { ApprovalOutcome, PendingInteraction, Question } from '@/lib/interaction.ts'

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
    <QuestionDialog interaction={interaction} onAnswer={onQuestion} onCancel={onCancelQuestion} />
  )
}

function ApprovalDialog({
  interaction,
  onAnswer,
}: {
  interaction: Extract<PendingInteraction, { kind: 'approval' }>
  onAnswer: (outcome: ApprovalOutcome) => void
}) {
  return (
    <Shell
      title="这个工具需要你批准"
      subtitle={`${interaction.toolName} · ${interaction.sessionId.slice(0, 8)}`}
      tone="amber"
    >
      {interaction.reason !== undefined && (
        <p className="bg-secondary/50 rounded-lg p-2.5 text-xs leading-relaxed whitespace-pre-wrap select-text">
          {interaction.reason}
        </p>
      )}

      {interaction.callId !== undefined && (
        <p className="text-muted-foreground truncate font-mono text-[11px]">
          调用 {interaction.callId}
        </p>
      )}

      <p className="text-muted-foreground text-[11px]">允许只对这一次生效，不是长期授权。</p>

      <div className="mt-1 flex justify-end gap-2">
        <button
          type="button"
          onClick={() => onAnswer('rejected')}
          className="bg-secondary text-secondary-foreground hover:bg-secondary/70 rounded-md px-3 py-1.5 text-sm transition-colors active:scale-[0.98]"
        >
          拒绝
        </button>
        <button
          type="button"
          onClick={() => onAnswer('allowed-once')}
          className="bg-primary text-primary-foreground hover:bg-primary/90 rounded-md px-3.5 py-1.5 text-sm font-medium transition-colors active:scale-[0.98]"
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
      const next =
        question.multiSelect === true
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
        return text === '' ? { id: question.id, selected } : { id: question.id, selected, custom: text }
      }),
    )
  }

  return (
    <Shell title="模型在问你" subtitle={`${interaction.questions.length} 个问题`} tone="primary">
      <ul className="flex max-h-[60vh] flex-col gap-4 overflow-y-auto">
        {interaction.questions.map((question) => (
          <li key={question.id} className="flex flex-col gap-2">
            {question.header !== undefined && (
              <span className="text-muted-foreground text-[11px] font-medium tracking-wide uppercase">
                {question.header}
              </span>
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
                      className={`rounded-md border px-3 py-1.5 text-left text-sm transition-colors ${
                        active
                          ? 'border-primary bg-primary/10'
                          : 'border-input hover:bg-secondary/40'
                      }`}
                    >
                      <span>{option.label}</span>
                      {option.description !== undefined && (
                        <span className="text-muted-foreground ml-2 text-xs">
                          {option.description}
                        </span>
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
              className="border-input bg-card focus:border-ring rounded-md border px-2.5 py-1.5 text-sm outline-none transition-colors select-text"
            />
          </li>
        ))}
      </ul>

      <div className="mt-1 flex justify-end gap-2">
        <button
          type="button"
          onClick={onCancel}
          className="bg-secondary text-secondary-foreground hover:bg-secondary/70 rounded-md px-3 py-1.5 text-sm transition-colors active:scale-[0.98]"
        >
          放弃
        </button>
        <button
          type="button"
          onClick={submit}
          className="bg-primary text-primary-foreground hover:bg-primary/90 rounded-md px-3.5 py-1.5 text-sm font-medium transition-colors active:scale-[0.98]"
        >
          回答
        </button>
      </div>
    </Shell>
  )
}

/** Modal frame shared by both dialogs. */
function Shell({
  title,
  subtitle,
  tone,
  children,
}: {
  title: string
  subtitle: string
  tone: 'amber' | 'primary'
  children: React.ReactNode
}) {
  return (
    <div className="animate-backdrop fixed inset-0 z-50 flex items-center justify-center bg-black/55 p-6 backdrop-blur-[2px]">
      <div className="animate-pop-in bg-card border-border flex w-full max-w-lg flex-col gap-3 rounded-xl border p-4 shadow-2xl">
        <div className="flex items-start justify-between gap-3">
          <h2 className="text-sm font-semibold">{title}</h2>
          <span
            className={`shrink-0 rounded-full px-2 py-0.5 text-[10px] ${
              tone === 'amber' ? 'bg-amber-400/15 text-amber-300' : 'bg-primary/15 text-primary-foreground'
            }`}
          >
            {subtitle}
          </span>
        </div>
        {children}
      </div>
    </div>
  )
}
