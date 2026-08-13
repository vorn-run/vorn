// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { WORKFLOW_STATUS_DOT } from '../src/renderer/lib/workflow-status'
import { render, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import { ApprovalNode } from '../src/renderer/components/workflow-editor/nodes/ApprovalNode'
import { NODE_SELECTED } from '../src/renderer/components/workflow-editor/node-visuals'

describe('ApprovalNode', () => {
  it('renders label and default subtitle when no timeout', () => {
    const { container } = render(<ApprovalNode label="Human gate" config={{}} onClick={vi.fn()} />)
    expect(container.textContent).toContain('Human gate')
    expect(container.textContent).toContain('Waits for approval')
    expect(container.textContent).not.toContain('timeout')
  })

  it('shows timeout in subtitle when configured', () => {
    const { container } = render(
      <ApprovalNode label="Gate" config={{ timeoutMs: 120000 }} onClick={vi.fn()} />
    )
    expect(container.textContent).toContain('120s timeout')
  })

  it('renders the message preview truncated to 60 chars', () => {
    const long = 'a'.repeat(80)
    const { container } = render(
      <ApprovalNode label="Gate" config={{ message: long }} onClick={vi.fn()} />
    )
    expect(container.textContent).toContain('aaaa...')
  })

  it('does not truncate short messages', () => {
    const { container } = render(
      <ApprovalNode label="Gate" config={{ message: 'short' }} onClick={vi.fn()} />
    )
    expect(container.textContent).toContain('short')
    expect(container.textContent).not.toContain('...')
  })

  it('fires onClick when the card is clicked and stops propagation', () => {
    const parent = vi.fn()
    const inner = vi.fn()
    const { container } = render(
      <div onClick={parent}>
        <ApprovalNode label="Gate" config={{}} onClick={inner} />
      </div>
    )
    const card = container.querySelector('[class*="cursor-pointer"]') as HTMLElement
    fireEvent.click(card)
    expect(inner).toHaveBeenCalled()
    expect(parent).not.toHaveBeenCalled()
  })

  it('renders status dot when executionStatus maps to a class', () => {
    const { container } = render(
      <ApprovalNode label="Gate" config={{}} executionStatus="waiting" onClick={vi.fn()} />
    )
    expect(container.querySelector(`.${WORKFLOW_STATUS_DOT.waiting}`)).toBeTruthy()
  })

  it('applies selected border when selected', () => {
    const { container } = render(
      <ApprovalNode label="Gate" config={{}} selected onClick={vi.fn()} />
    )
    expect(container.querySelector(`[class*="${NODE_SELECTED}"]`)).toBeTruthy()
  })
})
