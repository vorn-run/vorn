// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import '@testing-library/jest-dom/vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import type { ArtifactComment } from '../src/shared/types'
import { ArtifactRail } from '../src/renderer/components/browser/ArtifactRail'

const comment = (id: string, over: Partial<ArtifactComment> = {}): ArtifactComment => ({
  id,
  artifactId: 'a1',
  version: 1,
  anchor: { kind: 'quote', quote: `quote ${id}`, prefix: '', suffix: '' },
  body: `body ${id}`,
  state: 'draft',
  createdAt: '2026-09-23T10:00:00.000Z',
  updatedAt: '2026-09-23T10:00:00.000Z',
  ...over
})

function rail(props: Partial<Parameters<typeof ArtifactRail>[0]> = {}) {
  const handlers = {
    onSend: vi.fn(),
    onEdit: vi.fn(),
    onDelete: vi.fn(),
    onReveal: vi.fn(),
    onAddNote: vi.fn()
  }
  render(
    <ArtifactRail
      drafts={[]}
      sent={[]}
      version={1}
      found={{}}
      agent="claude"
      queued={false}
      sending={false}
      {...handlers}
      {...props}
    />
  )
  return handlers
}

describe('ArtifactRail', () => {
  it('shows the hint and a disabled send when there are no drafts', () => {
    rail()
    expect(screen.getByText('Select words on the page to comment on them.')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Nothing to send yet' })).toBeDisabled()
  })

  it('names each kind of target', () => {
    rail({
      drafts: [
        comment('q'),
        comment('e', { anchor: { kind: 'edit', before: 'old', after: 'new' } }),
        comment('p', {
          anchor: { kind: 'point', artboard: 'Phone', x: 1, y: 2, element: 'button "Download"' }
        }),
        comment('n', { anchor: null })
      ]
    })
    expect(screen.getByText('quote q')).toBeInTheDocument()
    expect(screen.getByText('old → new')).toBeInTheDocument()
    expect(screen.getByText('Phone · button "Download"')).toBeInTheDocument()
    expect(screen.getByText('The whole version')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Send 4 comments to claude' })).toBeEnabled()
  })

  it('edits, cancels, deletes and reveals a draft', () => {
    const h = rail({ drafts: [comment('a')], found: { a: false } })
    expect(screen.getByText('words changed')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Show on the page' }))
    expect(h.onReveal).toHaveBeenCalled()
    fireEvent.click(screen.getByText('edit'))
    const box = screen.getByLabelText('Edit comment')
    fireEvent.change(box, { target: { value: 'sharper' } })
    fireEvent.click(screen.getByText('save'))
    expect(h.onEdit).toHaveBeenCalledWith('a', 'sharper')
    fireEvent.click(screen.getByText('edit'))
    fireEvent.keyDown(screen.getByLabelText('Edit comment'), { key: 'Escape' })
    fireEvent.click(screen.getByText('edit'))
    fireEvent.change(screen.getByLabelText('Edit comment'), { target: { value: 'by keys' } })
    fireEvent.keyDown(screen.getByLabelText('Edit comment'), { key: 'Enter', metaKey: true })
    expect(h.onEdit).toHaveBeenLastCalledWith('a', 'by keys')
    fireEvent.click(screen.getByText('edit'))
    fireEvent.click(screen.getByText('cancel'))
    fireEvent.click(screen.getByText('delete'))
    expect(h.onDelete).toHaveBeenCalledWith('a')
  })

  it('says what became of the last batch', () => {
    const h = rail({
      version: 2,
      sent: [comment('kept', { state: 'sent' }), comment('lost', { state: 'sent' })],
      found: { kept: true, lost: false }
    })
    expect(screen.getByText('Sent with v1')).toBeInTheDocument()
    expect(screen.getByText('still anchored')).toBeInTheDocument()
    expect(screen.getByText('words changed in v2')).toBeInTheDocument()
    fireEvent.click(screen.getAllByRole('button', { name: 'Show on the page' })[0])
    expect(h.onReveal).toHaveBeenCalled()
  })

  it('marks a batch sent on the version in front as sent', () => {
    rail({ sent: [comment('s', { state: 'sent' })] })
    expect(screen.getByText('sent')).toBeInTheDocument()
  })

  it('adds a note by button or by keys, and says when a send is queued', () => {
    const h = rail({ drafts: [comment('a')], queued: true })
    expect(screen.getByText(/Queued\. It goes once claude/)).toBeInTheDocument()
    const note = screen.getByLabelText('Note about the whole version')
    fireEvent.change(note, { target: { value: 'Tighten it' } })
    fireEvent.click(screen.getByRole('button', { name: 'Add note' }))
    expect(h.onAddNote).toHaveBeenCalledWith('Tighten it')
    fireEvent.change(note, { target: { value: 'Again' } })
    fireEvent.keyDown(note, { key: 'Enter', ctrlKey: true })
    expect(h.onAddNote).toHaveBeenLastCalledWith('Again')
    fireEvent.click(screen.getByRole('button', { name: 'Send 1 comment to claude' }))
    expect(h.onSend).toHaveBeenCalled()
  })

  it('leaves out the send footer at a gate', () => {
    rail({ onSend: undefined, heading: 'Comments' })
    expect(screen.getByText('Comments')).toBeInTheDocument()
    expect(screen.queryByLabelText('Note about the whole version')).not.toBeInTheDocument()
  })
})
