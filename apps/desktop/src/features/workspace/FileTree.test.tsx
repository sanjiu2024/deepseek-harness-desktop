import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { FileTree } from './FileTree.tsx'

const invoke = vi.fn()

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invoke(...args),
}))

function entry(name: string, kind: 'file' | 'directory', path: string) {
  return { name, path, kind, size: kind === 'file' ? 10 : null, modified: null }
}

/** The command now answers with a capped listing, not a bare array. */
function listing(entries: ReturnType<typeof entry>[]) {
  return { entries, total: entries.length, truncated: false }
}

beforeEach(() => {
  invoke.mockReset()
})

describe('file tree', () => {
  it('loads one directory at a time, not the whole repository', async () => {
    invoke.mockResolvedValue(listing([entry('src', 'directory', 'D:/repo/src'), entry('README.md', 'file', 'D:/repo/README.md')]))
    render(<FileTree onOpen={() => {}} />)

    await screen.findByText('src')
    // One call for the root: children are fetched only when expanded, or a large repository
    // would be walked before the user asks for any of it.
    expect(invoke).toHaveBeenCalledTimes(1)
    expect(invoke).toHaveBeenCalledWith('fs_tree', { path: null, showNoisy: false, order: 'name' })
  })

  it('expands a directory on click and fetches its children', async () => {
    invoke
      .mockResolvedValueOnce(listing([entry('src', 'directory', 'D:/repo/src')]))
      .mockResolvedValueOnce(listing([entry('main.ts', 'file', 'D:/repo/src/main.ts')]))
    render(<FileTree onOpen={() => {}} />)

    await userEvent.click(await screen.findByText('src'))
    expect(await screen.findByText('main.ts')).toBeTruthy()
    expect(invoke).toHaveBeenLastCalledWith('fs_tree', { path: 'D:/repo/src', showNoisy: false, order: 'name' })
  })

  it('opens a file rather than expanding it', async () => {
    const opened: string[] = []
    invoke.mockResolvedValue(listing([entry('README.md', 'file', 'D:/repo/README.md')]))
    render(<FileTree onOpen={(path) => opened.push(path)} />)

    await userEvent.click(await screen.findByText('README.md'))
    expect(opened).toEqual(['D:/repo/README.md'])
  })

  it('shows the error instead of an empty tree', async () => {
    invoke.mockRejectedValue('path escapes the workspace: D:/etc')
    render(<FileTree onOpen={() => {}} />)
    expect(await screen.findByText(/path escapes the workspace/)).toBeTruthy()
  })
})
