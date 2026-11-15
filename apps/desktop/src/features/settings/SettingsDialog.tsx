import { useState } from 'react'
import type { SettingsSnapshot } from '@/lib/ipc.ts'

/**
 * Route settings.
 *
 * Two fields and a credential indicator, because that is the entire free surface:
 * everything else about a kernel run is decided by the kernel's own profile.
 */
export function SettingsDialog({
  settings,
  onChange,
  onClose,
}: {
  settings: SettingsSnapshot
  /** Persist and return the value read back from disk; throws on failure. */
  onChange: (provider: string, model: string) => Promise<void>
  onClose: () => void
}) {
  const [provider, setProvider] = useState(settings.provider)
  const [model, setModel] = useState(settings.model)
  const [error, setError] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)

  const save = async () => {
    setError(null)
    setSaving(true)
    try {
      await onChange(provider, model)
      onClose()
    } catch (cause) {
      setError(String(cause))
    } finally {
      setSaving(false)
    }
  }

  const dirty = provider !== settings.provider || model !== settings.model

  return (
    <div className="animate-backdrop fixed inset-0 z-50 flex items-center justify-center bg-black/55 p-6 backdrop-blur-[2px]">
      <div className="animate-pop-in bg-card border-border flex w-full max-w-md flex-col gap-4 rounded-xl border p-4 shadow-2xl">
        <header className="flex items-start justify-between">
          <div>
            <h2 className="text-sm font-semibold">模型路由</h2>
            <p className="text-muted-foreground text-[11px]">下一次启动内核时生效</p>
          </div>
          <button
            type="button"
            onClick={onClose}
            className="text-muted-foreground hover:text-foreground -mt-1 text-lg leading-none transition-colors"
            aria-label="关闭"
          >
            ×
          </button>
        </header>

        <label className="flex flex-col gap-1">
          <span className="text-muted-foreground text-[11px]">Provider</span>
          <input
            value={provider}
            onChange={(event) => setProvider(event.target.value)}
            spellCheck={false}
            className="border-input bg-background focus:border-ring rounded-md border px-2.5 py-1.5 font-mono text-sm outline-none transition-colors select-text"
          />
        </label>

        <label className="flex flex-col gap-1">
          <span className="text-muted-foreground text-[11px]">Model</span>
          <input
            value={model}
            onChange={(event) => setModel(event.target.value)}
            spellCheck={false}
            className="border-input bg-background focus:border-ring rounded-md border px-2.5 py-1.5 font-mono text-sm outline-none transition-colors select-text"
          />
        </label>

        <div className="flex items-center gap-2 text-[11px]">
          <span
            className={`inline-block size-1.5 rounded-full ${
              settings.hasCredential ? 'bg-emerald-400' : 'bg-amber-400'
            }`}
          />
          <span className="text-muted-foreground">
            {settings.hasCredential
              ? '密钥已在环境里'
              : '没有密钥 —— 启动后每一轮都会失败，先设 OPENCODE_API_KEY'}
          </span>
        </div>

        {error !== null && (
          <pre className="border-destructive/40 bg-destructive/10 text-destructive max-h-24 overflow-auto rounded-md border p-2 text-[11px] whitespace-pre-wrap">
            {error}
          </pre>
        )}

        <div className="flex justify-end gap-2">
          <button
            type="button"
            onClick={onClose}
            className="bg-secondary text-secondary-foreground hover:bg-secondary/70 rounded-md px-3 py-1.5 text-sm transition-colors active:scale-[0.98]"
          >
            取消
          </button>
          <button
            type="button"
            onClick={() => void save()}
            disabled={saving || !dirty || provider.trim() === '' || model.trim() === ''}
            className="bg-primary text-primary-foreground hover:bg-primary/90 rounded-md px-3.5 py-1.5 text-sm font-medium transition-colors active:scale-[0.98] disabled:opacity-30"
          >
            {saving ? '保存中…' : '保存'}
          </button>
        </div>
      </div>
    </div>
  )
}
