// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from 'vitest'
import { render, cleanup, fireEvent, screen } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import {
  NodePalette,
  PalettePick
} from '../src/renderer/components/workflow-editor/panels/NodePalette'

afterEach(cleanup)

const connectorItems = [
  { connectionId: 'c1', action: 'createIssue', label: 'Create issue', source: 'GitHub' },
  { connectionId: 'c1', action: 'closeIssue', label: 'Close issue', source: 'GitHub' }
]

function renderPalette(over: Partial<Parameters<typeof NodePalette>[0]> = {}) {
  const onPick = vi.fn()
  const onClose = vi.fn()
  const utils = render(
    <NodePalette
      position={{ x: 10, y: 20 }}
      allowLoop
      connectorItems={connectorItems}
      onPick={onPick}
      onClose={onClose}
      {...over}
    />
  )
  return { ...utils, onPick, onClose }
}

describe('the node search panel', () => {
  it('lists every step type and every installed connector action', () => {
    renderPalette()
    for (const label of [
      'Add an agent',
      'Add a script',
      'Add a condition',
      'Add an approval gate',
      'Repeat steps until…',
      'Call a connector action',
      'Create issue',
      'Close issue'
    ]) {
      expect(screen.getByText(label)).toBeInTheDocument()
    }
    expect(screen.getAllByText('GitHub')).toHaveLength(2)
  })

  it('filters as you type, matching actions by label and source', () => {
    renderPalette()
    fireEvent.change(screen.getByPlaceholderText('Search steps and actions'), {
      target: { value: 'issue' }
    })
    expect(screen.getByText('Create issue')).toBeInTheDocument()
    expect(screen.queryByText('Add an agent')).toBeNull()
    fireEvent.change(screen.getByPlaceholderText('Search steps and actions'), {
      target: { value: 'github' }
    })
    expect(screen.getByText('Close issue')).toBeInTheDocument()
  })

  it('says so when nothing matches', () => {
    renderPalette()
    fireEvent.change(screen.getByPlaceholderText('Search steps and actions'), {
      target: { value: 'zzz' }
    })
    expect(screen.getByText('Nothing matches')).toBeInTheDocument()
  })

  it('withholds the loop inside a branch', () => {
    renderPalette({ allowLoop: false })
    expect(screen.queryByText('Repeat steps until…')).toBeNull()
  })

  it('picks the highlighted row with Enter and moves the highlight with arrows', () => {
    const { onPick, container } = renderPalette()
    const root = container.querySelector('[data-node-palette]') as HTMLElement
    fireEvent.keyDown(root, { key: 'ArrowDown' })
    fireEvent.keyDown(root, { key: 'ArrowUp' })
    fireEvent.keyDown(root, { key: 'ArrowDown' })
    fireEvent.keyDown(root, { key: 'Enter' })
    expect(onPick).toHaveBeenCalledWith({ kind: 'type', type: 'script' } satisfies PalettePick)
  })

  it('reports a connector pick with its connection and action', () => {
    const { onPick } = renderPalette()
    fireEvent.click(screen.getByText('Create issue'))
    expect(onPick).toHaveBeenCalledWith({
      kind: 'connectorAction',
      connectionId: 'c1',
      action: 'createIssue'
    } satisfies PalettePick)
  })

  it('closes on Escape and on a click outside', () => {
    const { onClose, container } = renderPalette()
    fireEvent.keyDown(container.querySelector('[data-node-palette]') as HTMLElement, {
      key: 'Escape'
    })
    expect(onClose).toHaveBeenCalledTimes(1)
    fireEvent.mouseDown(document.body)
    expect(onClose).toHaveBeenCalledTimes(2)
  })
})
