// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from 'vitest'
import { render, screen, cleanup, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

import { NodeShell, NodeFooter } from '../src/renderer/components/workflow-editor/nodes/NodeShell'
import {
  NODE_SELECTED,
  NODE_UNSELECTED,
  truncate
} from '../src/renderer/components/workflow-editor/node-visuals'
import { WORKFLOW_STATUS_DOT_PULSE } from '../src/renderer/lib/workflow-status'

const noop = (): void => {}
const base = { icon: <span data-testid="icon" />, label: 'A step', subtitle: 'does a thing' }

afterEach(cleanup)

describe('NodeShell', () => {
  it('is the outermost element, carrying the shape every card shares', () => {
    // node-selection.test.tsx reads container.firstChild as the card root and
    // checks the shape there, so a wrapper element here would silently defeat
    // it. The classes are also written literally because Tailwind scans source
    // text — a name composed at runtime is never generated.
    const { container } = render(<NodeShell {...base} onClick={noop} />)
    const root = container.firstElementChild as HTMLElement
    for (const cls of ['rounded-md', 'px-3', 'py-2.5', 'w-[280px]', 'relative']) {
      expect(root.className).toContain(cls)
    }
  })

  it('marks selection with the shared border and never the accent', () => {
    // Bronzo says work is blocked on the person. Selecting a node is neither.
    const on = render(<NodeShell {...base} selected onClick={noop} />)
    const selected = on.container.firstElementChild as HTMLElement
    expect(selected.className).toContain(NODE_SELECTED)
    expect(selected.className).not.toContain('bronzo')
    cleanup()

    const off = render(<NodeShell {...base} onClick={noop} />)
    expect((off.container.firstElementChild as HTMLElement).className).toContain(NODE_UNSELECTED)
  })

  it('shows the state a live run put this node in', () => {
    for (const status of ['running', 'waiting', 'error', 'success'] as const) {
      const { container } = render(<NodeShell {...base} executionStatus={status} onClick={noop} />)
      expect(
        container.querySelector(`.${WORKFLOW_STATUS_DOT_PULSE[status].split(' ')[0]}`)
      ).toBeInTheDocument()
      cleanup()
    }
  })

  it('draws no dot on a canvas that is only showing a definition', () => {
    const { container } = render(<NodeShell {...base} onClick={noop} />)
    expect(container.querySelector('span.absolute.rounded-full')).toBeNull()
  })

  it('outlines a card that stands for nothing yet', () => {
    // A loop with no body is the shape of a step rather than a step.
    const { container } = render(<NodeShell {...base} dashed onClick={noop} />)
    expect((container.firstElementChild as HTMLElement).className).toContain('border-dashed')
  })

  it('carries the three things a card may add to the header', () => {
    render(
      <NodeShell
        {...base}
        onClick={noop}
        meta={<span data-testid="meta" />}
        trailing={<span data-testid="trailing" />}
      >
        <NodeFooter>a preview</NodeFooter>
      </NodeShell>
    )
    expect(screen.getByTestId('icon')).toBeInTheDocument()
    expect(screen.getByTestId('meta')).toBeInTheDocument()
    expect(screen.getByTestId('trailing')).toBeInTheDocument()
    expect(screen.getByText('a preview')).toBeInTheDocument()
  })

  it('opens the node without letting the canvas take the click too', () => {
    // The canvas clears selection on its own background click, so a card that
    // let the event through would select and immediately deselect.
    const onClick = vi.fn()
    const onCanvas = vi.fn()
    render(
      <div onClick={onCanvas}>
        <NodeShell {...base} onClick={onClick} />
      </div>
    )
    fireEvent.click(screen.getByText('A step'))
    expect(onClick).toHaveBeenCalled()
    expect(onCanvas).not.toHaveBeenCalled()
  })
})

describe('NodeFooter', () => {
  it('divides itself from the header the same way every card did by hand', () => {
    const { container } = render(<NodeFooter>text</NodeFooter>)
    const el = container.firstElementChild as HTMLElement
    expect(el.className).toContain('border-t')
    expect(el.className).toContain('truncate')
    expect(el.className).not.toContain('font-mono')
  })

  it('sets a preview of the work itself in mono', () => {
    const { container } = render(<NodeFooter mono>ls -la</NodeFooter>)
    expect((container.firstElementChild as HTMLElement).className).toContain('font-mono')
  })

  it('leaves a list to style its own rows', () => {
    // A loop body brings its own numbering and truncation per line, so the
    // single-line preview treatment would fight it.
    const { container } = render(
      <NodeFooter rows>
        <div>one</div>
      </NodeFooter>
    )
    const el = container.firstElementChild as HTMLElement
    expect(el.className).toContain('space-y-0.5')
    expect(el.className).not.toContain('truncate')
  })
})

describe('truncate', () => {
  it('leaves a line that fits alone', () => {
    expect(truncate('short', 10)).toBe('short')
    expect(truncate('exactly10!', 10)).toBe('exactly10!')
  })

  it('cuts a longer line and says it was cut', () => {
    expect(truncate('abcdefghijk', 10)).toBe('abcdefghij...')
  })
})
