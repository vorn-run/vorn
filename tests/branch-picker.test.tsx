// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { useRef } from 'react'
import { render, screen, act } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

const mockListBranches = vi.fn()
const mockListRemoteBranches = vi.fn()

Object.defineProperty(window, 'api', {
  value: {
    listBranches: (...args: unknown[]) => mockListBranches(...args),
    listRemoteBranches: (...args: unknown[]) => mockListRemoteBranches(...args)
  },
  writable: true
})

import { BranchPicker } from '../packages/web/src/renderer/components/BranchPicker'

function Harness({
  onClose = () => {},
  onSelect = () => {}
}: {
  onClose?: () => void
  onSelect?: (b: string) => void
}) {
  const anchorRef = useRef<HTMLButtonElement>(null)
  return (
    <div>
      <button ref={anchorRef} data-testid="anchor">
        main
      </button>
      <BranchPicker
        projectPath="/tmp/proj"
        currentBranch="main"
        onSelect={onSelect}
        onClose={onClose}
        anchorRef={anchorRef}
      />
    </div>
  )
}

describe('BranchPicker', () => {
  beforeEach(() => {
    mockListBranches.mockReset().mockResolvedValue({ local: ['main', 'dev'] })
    mockListRemoteBranches.mockReset().mockResolvedValue([])
    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue({
      top: 100,
      left: 200,
      right: 260,
      bottom: 120,
      width: 60,
      height: 20,
      x: 200,
      y: 100,
      toJSON: () => ({})
    })
  })

  afterEach(() => {
    vi.restoreAllMocks()
  })

  it('positions the portal based on the anchor rect', () => {
    render(<Harness />)
    const portal = screen.getByPlaceholderText('Filter branches...').closest('div.fixed')
    expect(portal).toBeInTheDocument()
    expect((portal as HTMLElement).style.visibility).toBe('visible')
    expect((portal as HTMLElement).style.top).not.toBe('')
    expect((portal as HTMLElement).style.left).not.toBe('')
  })

  it('repositions on scroll via rAF', () => {
    const rafSpy = vi.spyOn(window, 'requestAnimationFrame')
    render(<Harness />)
    act(() => {
      window.dispatchEvent(new Event('scroll'))
      window.dispatchEvent(new Event('scroll'))
    })
    expect(rafSpy).toHaveBeenCalled()
    rafSpy.mockRestore()
  })

  it('does not call onClose when mousedown originates on the anchor', () => {
    const onClose = vi.fn()
    render(<Harness onClose={onClose} />)
    const anchor = screen.getByTestId('anchor')
    act(() => {
      anchor.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }))
    })
    expect(onClose).not.toHaveBeenCalled()
  })

  it('calls onClose when mousedown is outside picker and anchor', () => {
    const onClose = vi.fn()
    render(<Harness onClose={onClose} />)
    act(() => {
      document.body.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }))
    })
    expect(onClose).toHaveBeenCalled()
  })
})
