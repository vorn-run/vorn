// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from 'vitest'
import { render, screen, cleanup, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

vi.mock('../src/renderer/components/file-icons', () => ({
  FileTypeIcon: () => <span data-testid="file-icon" />
}))

import { DiffFileList } from '../src/renderer/components/DiffSidebar'
import type { GitFileDiff } from '../src/shared/types'

const file = (over: Partial<GitFileDiff> = {}): GitFileDiff =>
  ({
    filePath: 'src/renderer/App.tsx',
    status: 'modified',
    insertions: 3,
    deletions: 1,
    ...over
  }) as GitFileDiff

const props = { selectedFile: null, onSelectFile: vi.fn() }

afterEach(cleanup)

describe('DiffFileList', () => {
  it('marks what happened to a file with a letter, not a colour', () => {
    // Four hues for four categories is the pattern this pass is removing: the
    // letter already says which, and the diff itself is the thing allowed to
    // carry colour here.
    const { container } = render(
      <DiffFileList
        {...props}
        files={[
          file({ filePath: 'a.ts', status: 'modified' }),
          file({ filePath: 'b.ts', status: 'added' }),
          file({ filePath: 'c.ts', status: 'deleted' }),
          file({ filePath: 'd.ts', status: 'renamed' })
        ]}
      />
    )

    expect(['M', 'A', 'D', 'R'].map((l) => !!screen.getByText(l))).toEqual([true, true, true, true])

    // One tone for all four, and that tone is off the ink ramp rather than a
    // category hue — asserting the family rather than the step so a retune of
    // the ramp does not have to come back through here.
    const tones = [...container.querySelectorAll('span.font-bold')].map((el) => el.className)
    expect(new Set(tones).size).toBe(1)
    expect(tones.filter((c) => !/\btext-ink(-|\b)/.test(c))).toEqual([])
  })

  it('falls back to modified for a status it does not know', () => {
    render(
      <DiffFileList {...props} files={[file({ status: 'teleported' as GitFileDiff['status'] })]} />
    )
    expect(screen.getByText('M')).toBeInTheDocument()
  })

  it('shows an insertion and deletion count only when there is one', () => {
    render(
      <DiffFileList
        {...props}
        files={[file({ filePath: 'only-added.ts', insertions: 9, deletions: 0 })]}
      />
    )
    expect(screen.getByText('+9')).toBeInTheDocument()
    expect(screen.queryByText('-0')).not.toBeInTheDocument()
  })

  it('selects the file that was clicked', () => {
    const onSelectFile = vi.fn()
    render(<DiffFileList {...props} onSelectFile={onSelectFile} files={[file()]} />)
    fireEvent.click(screen.getByText('src/renderer/App.tsx'))
    expect(onSelectFile).toHaveBeenCalledWith('src/renderer/App.tsx')
  })
})
