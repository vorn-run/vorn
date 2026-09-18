// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { NodeExecutionState } from '../src/shared/types'

// Stub shiki so the source view's highlighter resolves without WASM in jsdom.
vi.mock('shiki', () => ({
  createHighlighter: async () => ({
    loadLanguage: async () => undefined,
    codeToTokens: () => ({ tokens: [] })
  }),
  createJavaScriptRegexEngine: () => ({})
}))

// A gate is answered by asking the server; the editor only shapes what is sent.
const resolveWorkflowGate = vi.fn()

beforeEach(() => {
  resolveWorkflowGate.mockReset()
  resolveWorkflowGate.mockResolvedValue({ accepted: true })
  ;(window as unknown as { api: unknown }).api = { resolveWorkflowGate }
})

const { GateActions } = await import('../src/renderer/components/workflow-runs/GateActions')

const FINDINGS = [
  { id: 1, title: 'Leaky pool', done: false },
  { id: 2, title: 'Stale cache', severity: 'high', done: true, note: null }
]

function openEditor(value: unknown): void {
  const state: NodeExecutionState = {
    nodeId: 'gate',
    status: 'waiting',
    editableText: JSON.stringify(value, null, 2)
  }
  render(<GateActions runId="run-1" state={state} nodes={[]} />)
  fireEvent.click(screen.getByRole('button', { name: /^Edit$/ }))
}

/** What the last answer to the gate carried as its rewrite, parsed. */
function sentEdit(): unknown {
  const call = resolveWorkflowGate.mock.calls.at(-1)?.[0] as { edited?: string }
  return call.edited === undefined ? undefined : JSON.parse(call.edited)
}

const approve = (): HTMLElement => screen.getByRole('button', { name: 'Approve' })
const save = (): HTMLElement => screen.getByRole('button', { name: 'Save' })

describe('GateJsonEditor', () => {
  it('draws a column for every key any row has, in the order they appear', () => {
    openEditor(FINDINGS)
    const headers = screen.getAllByRole('columnheader').map((h) => h.textContent)
    expect(headers).toEqual(['id', 'title', 'done', 'severity', 'note'])
    expect(screen.getByLabelText('severity, row 1')).toHaveValue('')
    expect(screen.getByLabelText('severity, row 2')).toHaveValue('high')
  })

  it('removes a row and puts it back, counting what is kept', () => {
    openEditor(FINDINGS)
    expect(screen.getByText('2 of 2 kept')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Remove row 1' }))
    expect(screen.getByText('1 of 2 kept')).toBeInTheDocument()
    const undo = screen.getByRole('button', { name: 'Undo removing row 1' })
    expect(undo).toHaveTextContent('Undo')
    fireEvent.click(undo)
    expect(screen.getByText('2 of 2 kept')).toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: 'Remove row 1' }))
    fireEvent.click(save())
    fireEvent.click(approve())
    expect(sentEdit()).toEqual([FINDINGS[1]])
  })

  it('keeps a number a number, and will not send one that is not', () => {
    openEditor(FINDINGS)
    const cell = screen.getByLabelText('id, row 1')
    fireEvent.change(cell, { target: { value: '7' } })
    expect(cell).not.toHaveAttribute('aria-invalid')
    fireEvent.change(cell, { target: { value: 'seven' } })
    expect(cell).toHaveAttribute('aria-invalid', 'true')
    expect(save()).toBeDisabled()
    expect(approve()).toBeDisabled()
    expect(approve().title).toContain('Row 1, id: not a number.')

    fireEvent.change(cell, { target: { value: '7.5' } })
    expect(save()).toBeEnabled()
    fireEvent.click(approve())
    expect((sentEdit() as { id: unknown }[])[0].id).toBe(7.5)
  })

  it('does not hold up the gate over a bad number in a removed row', () => {
    openEditor(FINDINGS)
    fireEvent.change(screen.getByLabelText('id, row 1'), { target: { value: 'x' } })
    fireEvent.click(screen.getByRole('button', { name: 'Remove row 1' }))
    expect(approve()).toBeEnabled()
  })

  it('toggles a boolean', () => {
    openEditor(FINDINGS)
    fireEvent.click(screen.getByLabelText('done, row 1'))
    fireEvent.click(approve())
    expect((sentEdit() as { done: boolean }[])[0].done).toBe(true)
  })

  it('reads an emptied null cell as null, and a filled blank as a new string', () => {
    openEditor(FINDINGS)
    const note = screen.getByLabelText('note, row 2')
    fireEvent.change(note, { target: { value: 'seen' } })
    fireEvent.change(note, { target: { value: '' } })
    fireEvent.change(screen.getByLabelText('severity, row 1'), { target: { value: 'low' } })
    fireEvent.click(approve())
    const sent = sentEdit() as Record<string, unknown>[]
    expect(sent[1].note).toBeNull()
    expect(sent[0].severity).toBe('low')
  })

  it('shows a nested value read-only, pointing at the source', () => {
    openEditor([{ id: 1, tags: ['a', 'b'] }])
    const tags = screen.getByTitle('Edit in Source')
    expect(tags).toHaveTextContent('["a","b"]')
    expect(tags.tagName).not.toBe('INPUT')
  })

  it('redraws the table from JSON edited in the source', () => {
    openEditor(FINDINGS)
    fireEvent.click(screen.getByRole('button', { name: 'Source' }))
    const box = screen.getByLabelText('The text to approve') as HTMLTextAreaElement
    expect(JSON.parse(box.value)).toEqual(FINDINGS)
    fireEvent.change(box, {
      target: { value: JSON.stringify([...FINDINGS, { id: 3, title: 'New one' }]) }
    })
    expect(screen.getByRole('status')).toHaveTextContent('Valid JSON · 3 items')
    fireEvent.click(screen.getByRole('button', { name: 'Table' }))
    expect(screen.getByText('3 of 3 kept')).toBeInTheDocument()
    expect(screen.getByLabelText('title, row 3')).toHaveValue('New one')
  })

  it('says where broken JSON broke, and shuts the table and Approve until it is fixed', () => {
    openEditor(FINDINGS)
    fireEvent.click(screen.getByRole('button', { name: 'Source' }))
    const box = screen.getByLabelText('The text to approve')
    fireEvent.change(box, { target: { value: '[\n  {"id": 1,}\n]' } })
    expect(screen.getByRole('status').textContent).toMatch(/^Line 2, column \d+: /)
    expect(screen.getByTestId('editor-error-gutter')).toHaveTextContent('2')
    expect(screen.getByRole('button', { name: 'Table' })).toBeDisabled()
    expect(save()).toBeDisabled()
    expect(approve()).toBeDisabled()
    expect(approve().title).toMatch(/Line 2, column \d+/)

    fireEvent.change(box, { target: { value: '[{"id": 1}]' } })
    expect(screen.getByRole('button', { name: 'Table' })).toBeEnabled()
    expect(approve()).toBeEnabled()
  })

  it('sends no rewrite when nothing changed, however the source is spaced', () => {
    openEditor(FINDINGS)
    fireEvent.click(save())
    fireEvent.click(approve())
    expect(resolveWorkflowGate).toHaveBeenLastCalledWith({
      runId: 'run-1',
      nodeId: 'gate',
      decision: 'approve'
    })

    fireEvent.click(screen.getByRole('button', { name: /^Edit/ }))
    fireEvent.click(screen.getByRole('button', { name: 'Source' }))
    fireEvent.change(screen.getByLabelText('The text to approve'), {
      target: { value: JSON.stringify(FINDINGS) }
    })
    fireEvent.click(approve())
    expect(resolveWorkflowGate).toHaveBeenLastCalledWith({
      runId: 'run-1',
      nodeId: 'gate',
      decision: 'approve'
    })
  })

  it('puts the kept rows back in the object that held them', () => {
    openEditor({ summary: 'Two things', findings: FINDINGS, count: 2 })
    fireEvent.click(screen.getByRole('button', { name: 'Remove row 2' }))
    fireEvent.click(save())
    expect(screen.getByRole('button', { name: /^Edited$/ })).toBeInTheDocument()
    fireEvent.click(approve())
    expect(sentEdit()).toEqual({ summary: 'Two things', findings: [FINDINGS[0]], count: 2 })
  })

  it('reopens on the rewrite, and Revert goes back to what the gate had', () => {
    openEditor(FINDINGS)
    fireEvent.click(screen.getByRole('button', { name: 'Remove row 1' }))
    fireEvent.click(save())
    fireEvent.click(screen.getByRole('button', { name: /^Edited$/ }))
    expect(screen.getByText('1 of 1 kept')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Revert' }))
    expect(screen.getByText('2 of 2 kept')).toBeInTheDocument()
  })

  it('leaves prose to the plain textarea', () => {
    const state: NodeExecutionState = {
      nodeId: 'gate',
      status: 'waiting',
      editableText: 'A tidy draft.'
    }
    render(<GateActions runId="run-1" state={state} nodes={[]} />)
    fireEvent.click(screen.getByRole('button', { name: /^Edit$/ }))
    expect(screen.queryByRole('button', { name: 'Table' })).not.toBeInTheDocument()
    expect(screen.getByLabelText('The text to approve')).toHaveValue('A tidy draft.')
    expect(approve()).toBeEnabled()
  })
})
