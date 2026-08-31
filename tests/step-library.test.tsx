// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from 'vitest'
import { render, cleanup, fireEvent, screen } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

vi.mock('../src/renderer/lib/use-connections', () => ({
  useConnections: () => [
    { id: 'c1', name: 'GitHub' },
    { id: 'c2', name: 'Azure DevOps' }
  ],
  useConnectorIdFor: () => null,
  useConnectionIconFor: () => undefined
}))

const listConnectionActions = vi.fn(async (id: string) =>
  id === 'c1'
    ? [
        { type: 'createIssue', label: 'Create issue', configFields: [] },
        { type: 'closeIssue', label: 'Close issue', configFields: [] }
      ]
    : [{ type: 'createWorkItem', label: 'Create work item', configFields: [] }]
)
;(window as unknown as { api: Record<string, unknown> }).api = {
  ...(window as unknown as { api?: Record<string, unknown> }).api,
  listConnectionActions
}

import {
  StepLibrary,
  LibraryPick
} from '../src/renderer/components/workflow-editor/panels/StepLibrary'

afterEach(cleanup)

function renderLibrary(scope = { bodyOnly: false, insideBranch: false }) {
  const onPick = vi.fn()
  const onClose = vi.fn()
  const utils = render(<StepLibrary scope={scope} onPick={onPick} onClose={onClose} />)
  return { ...utils, onPick, onClose }
}

describe('the step library', () => {
  it('lists steps first, then each connection with its actions grouped', async () => {
    renderLibrary()
    for (const label of [
      'Agent',
      'Script',
      'Condition',
      'Approval gate',
      'Loop',
      'Parallel branch'
    ]) {
      expect(screen.getByText(label)).toBeInTheDocument()
    }
    expect(await screen.findByText('GitHub')).toBeInTheDocument()
    expect(screen.getByText('Create issue')).toBeInTheDocument()
    expect(screen.getByText('Azure DevOps')).toBeInTheDocument()
    expect(screen.getByText('Create work item')).toBeInTheDocument()
  })

  it('searches steps and actions together', async () => {
    renderLibrary()
    await screen.findByText('GitHub')
    fireEvent.change(screen.getByPlaceholderText('Search steps and actions'), {
      target: { value: 'issue' }
    })
    expect(screen.getByText('Create issue')).toBeInTheDocument()
    expect(screen.queryByText('Agent')).toBeNull()
    expect(screen.queryByText('Create work item')).toBeNull()
  })

  it('withholds loop and parallel inside a branch', () => {
    renderLibrary({ bodyOnly: false, insideBranch: true })
    expect(screen.queryByText('Loop')).toBeNull()
    expect(screen.queryByText('Parallel branch')).toBeNull()
    expect(screen.getByText('Condition')).toBeInTheDocument()
  })

  it('offers only repeatable steps to a loop body', async () => {
    renderLibrary({ bodyOnly: true, insideBranch: false })
    expect(screen.getByText('Agent')).toBeInTheDocument()
    expect(screen.getByText('Script')).toBeInTheDocument()
    expect(screen.queryByText('Condition')).toBeNull()
    expect(screen.queryByText('Parallel branch')).toBeNull()
    await Promise.resolve()
    expect(screen.queryByText('GitHub')).toBeNull()
  })

  it('picks with click and with Enter on the highlighted row', async () => {
    const { onPick, container } = renderLibrary()
    fireEvent.click(await screen.findByText('Create issue'))
    expect(onPick).toHaveBeenCalledWith({
      kind: 'connectorAction',
      connectionId: 'c1',
      action: 'createIssue'
    } satisfies LibraryPick)

    const root = container.querySelector('[data-step-library]') as HTMLElement
    fireEvent.keyDown(root, { key: 'ArrowDown' })
    fireEvent.keyDown(root, { key: 'Enter' })
    expect(onPick).toHaveBeenLastCalledWith({ kind: 'type', type: 'script' } satisfies LibraryPick)
  })

  it('closes on Escape and on its close button', () => {
    const { onClose, container } = renderLibrary()
    fireEvent.keyDown(container.querySelector('[data-step-library]') as HTMLElement, {
      key: 'Escape'
    })
    fireEvent.click(screen.getByRole('button', { name: 'Close' }))
    expect(onClose).toHaveBeenCalledTimes(2)
  })
})
