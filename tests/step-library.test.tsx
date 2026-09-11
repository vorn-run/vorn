// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach, beforeEach } from 'vitest'
import { render, cleanup, fireEvent, screen } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

const connections = [
  { id: 'c1', name: 'GitHub', connectorId: 'github' },
  { id: 'c2', name: 'Azure DevOps', connectorId: 'azure-devops' }
]

vi.mock('../src/renderer/lib/use-connections', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../src/renderer/lib/use-connections')>()),
  useConnections: () => connections,
  useInstalledPacks: () => []
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

beforeEach(() => localStorage.clear())
afterEach(cleanup)

function renderLibrary(scope = { bodyOnly: false, insideBranch: false }) {
  const onPick = vi.fn()
  const onClose = vi.fn()
  const utils = render(<StepLibrary scope={scope} onPick={onPick} onClose={onClose} />)
  const root = utils.container.querySelector('[data-step-library]') as HTMLElement
  return { ...utils, root, onPick, onClose }
}

const group = (name: RegExp) => screen.findByRole('button', { name })

describe('the step library', () => {
  it('lists steps first, then each connection folded under its name', async () => {
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
    const github = await group(/GitHub/)
    expect(screen.queryByText('Create issue')).toBeNull()
    fireEvent.click(github)
    expect(github).toHaveAttribute('aria-expanded', 'true')
    expect(screen.getByText('Create issue')).toBeInTheDocument()
    expect(screen.queryByText('Create work item')).toBeNull()
  })

  it('searches steps and actions together, in one run that names each connection', async () => {
    renderLibrary()
    await group(/GitHub/)
    fireEvent.change(screen.getByPlaceholderText('Search steps and actions'), {
      target: { value: 'issue' }
    })
    expect(screen.getByRole('button', { name: /Create issue.*GitHub/ })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /Close issue.*GitHub/ })).toBeInTheDocument()
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
    const { onPick, root } = renderLibrary()
    fireEvent.click(await group(/GitHub/))
    fireEvent.click(screen.getByText('Create issue'))
    expect(onPick).toHaveBeenCalledWith({
      kind: 'connectorAction',
      connectionId: 'c1',
      action: 'createIssue',
      actionLabel: 'Create issue'
    } satisfies LibraryPick)

    fireEvent.keyDown(root, { key: 'ArrowDown' })
    fireEvent.keyDown(root, { key: 'Enter' })
    expect(onPick).toHaveBeenLastCalledWith({ kind: 'type', type: 'script' } satisfies LibraryPick)
  })

  it('opens and folds a group from the keys', async () => {
    const { root } = renderLibrary()
    const github = await group(/GitHub/)
    const reachable = screen
      .getAllByRole('button')
      .filter((b) => b.getAttribute('aria-label') !== 'Close')
    for (let i = 0; i < reachable.indexOf(github); i++) {
      fireEvent.keyDown(root, { key: 'ArrowDown' })
    }
    fireEvent.keyDown(root, { key: 'ArrowRight' })
    expect(screen.getByText('Create issue')).toBeInTheDocument()
    fireEvent.keyDown(root, { key: 'Enter' })
    expect(screen.queryByText('Create issue')).toBeNull()
  })

  it('opens the way it was left, with the actions picked lately first', async () => {
    renderLibrary()
    fireEvent.click(await group(/Azure DevOps/))
    fireEvent.click(await group(/GitHub/))
    fireEvent.click(screen.getByText('Close issue'))
    fireEvent.click(await group(/GitHub/))
    cleanup()

    renderLibrary()
    expect(await screen.findByText('Create work item')).toBeInTheDocument()
    expect(screen.getByText('Recent')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /Close issue.*GitHub/ })).toBeInTheDocument()
    expect(screen.queryByText('Create issue')).toBeNull()
  })

  it('closes on Escape and on its close button', () => {
    const { onClose, root } = renderLibrary()
    fireEvent.keyDown(root, { key: 'Escape' })
    fireEvent.click(screen.getByRole('button', { name: 'Close' }))
    expect(onClose).toHaveBeenCalledTimes(2)
  })
})
