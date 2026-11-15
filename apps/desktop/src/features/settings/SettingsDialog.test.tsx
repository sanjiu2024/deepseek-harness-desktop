import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { SettingsDialog } from './SettingsDialog.tsx'

const base = { provider: 'opencode-go', model: 'deepseek-v4.1-flash', hasCredential: true }

describe('settings dialog', () => {
  it('shows the stored route', () => {
    render(<SettingsDialog settings={base} onChange={async () => {}} onClose={() => {}} />)
    expect(screen.getByDisplayValue('opencode-go')).toBeTruthy()
    expect(screen.getByDisplayValue('deepseek-v4.1-flash')).toBeTruthy()
  })

  it('saves only when something changed', async () => {
    const onChange = vi.fn(async () => {})
    render(<SettingsDialog settings={base} onChange={onChange} onClose={() => {}} />)

    const save = screen.getByRole('button', { name: '保存' }) as HTMLButtonElement
    expect(save.disabled).toBe(true)

    const model = screen.getByDisplayValue('deepseek-v4.1-flash')
    await userEvent.clear(model)
    await userEvent.type(model, 'deepseek-v4.1-pro')

    expect(save.disabled).toBe(false)
    await userEvent.click(save)
    expect(onChange).toHaveBeenCalledWith('opencode-go', 'deepseek-v4.1-pro')
  })

  it('warns when no credential is present, without showing one', () => {
    render(
      <SettingsDialog settings={{ ...base, hasCredential: false }} onChange={async () => {}} onClose={() => {}} />,
    )
    expect(screen.getByText(/没有密钥/)).toBeTruthy()
    expect(screen.getByText(/OPENCODE_API_KEY/)).toBeTruthy()
  })

  it('keeps the dialog open and reports a failed save', async () => {
    const onClose = vi.fn()
    render(
      <SettingsDialog
        settings={base}
        onChange={async () => {
          throw new Error('disk full')
        }}
        onClose={onClose}
      />,
    )

    const model = screen.getByDisplayValue('deepseek-v4.1-flash')
    await userEvent.clear(model)
    await userEvent.type(model, 'other-model')
    await userEvent.click(screen.getByRole('button', { name: '保存' }))

    expect(await screen.findByText(/disk full/)).toBeTruthy()
    expect(onClose).not.toHaveBeenCalled()
  })
})
