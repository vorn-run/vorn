// @vitest-environment jsdom
import { describe, it, expect, beforeEach, vi } from 'vitest'
import '@testing-library/jest-dom/vitest'
import { render, screen, fireEvent, act } from '@testing-library/react'
import { useRef } from 'react'
import { SplitDivider } from '../src/renderer/components/SplitDivider'
import { clampSplitRatio } from '../src/renderer/lib/split-ratio'
import { useAppStore } from '../src/renderer/stores'

/** A container with a known rect, so a drag maps to a predictable ratio. */
function Harness({
  axis,
  onCommit,
  onChange
}: {
  axis: 'x' | 'y'
  onCommit: (r: number) => void
  onChange?: (r: number) => void
}) {
  const ref = useRef<HTMLDivElement | null>(null)
  return (
    <div ref={ref} data-testid="container">
      <SplitDivider
        axis={axis}
        containerRef={ref}
        onRatioChange={onChange ?? (() => {})}
        onRatioCommit={onCommit}
        label="Resize"
        testId="divider"
      />
    </div>
  )
}

function stubRect(el: HTMLElement): void {
  vi.spyOn(el, 'getBoundingClientRect').mockReturnValue({
    top: 0,
    left: 0,
    right: 1000,
    bottom: 1000,
    width: 1000,
    height: 1000,
    x: 0,
    y: 0,
    toJSON: () => ({})
  })
}

describe('clampSplitRatio', () => {
  it('keeps both sides of a split visible', () => {
    // A 0-ratio pane is unrecoverable: its divider would sit off-screen.
    expect(clampSplitRatio(0)).toBe(0.15)
    expect(clampSplitRatio(1)).toBe(0.85)
    expect(clampSplitRatio(0.4)).toBe(0.4)
  })

  it('falls back to an even split for a corrupted value', () => {
    expect(clampSplitRatio(NaN)).toBe(0.5)
    expect(clampSplitRatio(Infinity)).toBe(0.5)
  })
})

describe('SplitDivider', () => {
  it('commits on pointerup only, never mid-drag', () => {
    const onCommit = vi.fn()
    const onChange = vi.fn()
    render(<Harness axis="y" onCommit={onCommit} onChange={onChange} />)
    stubRect(screen.getByTestId('container'))

    fireEvent.pointerDown(screen.getByTestId('divider'), { clientY: 500 })
    fireEvent(document, new PointerEvent('pointermove', { clientY: 700 }))

    // The live drag drives local state; persisting per move would write to
    // storage dozens of times for one resize.
    expect(onChange).toHaveBeenCalledWith(0.7)
    expect(onCommit).not.toHaveBeenCalled()

    fireEvent(document, new PointerEvent('pointerup'))
    expect(onCommit).toHaveBeenCalledExactlyOnceWith(0.7)
  })

  it('reads clientX on the horizontal axis', () => {
    const onCommit = vi.fn()
    render(<Harness axis="x" onCommit={onCommit} />)
    stubRect(screen.getByTestId('container'))

    fireEvent.pointerDown(screen.getByTestId('divider'), { clientX: 500 })
    // clientY is deliberately the opposite value: an axis mix-up would read it.
    fireEvent(document, new PointerEvent('pointermove', { clientX: 300, clientY: 800 }))
    fireEvent(document, new PointerEvent('pointerup'))

    expect(onCommit).toHaveBeenCalledExactlyOnceWith(0.3)
  })

  it('does not let a resize select the card it lives in', () => {
    const onCardPointerDown = vi.fn()
    const onCommit = vi.fn()
    render(
      <div onPointerDown={onCardPointerDown}>
        <Harness axis="x" onCommit={onCommit} />
      </div>
    )

    fireEvent.pointerDown(screen.getByTestId('divider'), { clientX: 500 })
    // Cards select themselves on pointerdown; without stopPropagation, dragging
    // a divider would yank selection around as a side effect.
    expect(onCardPointerDown).not.toHaveBeenCalled()
  })

  it('stops after pointerup rather than tracking the cursor forever', () => {
    const onChange = vi.fn()
    render(<Harness axis="y" onCommit={() => {}} onChange={onChange} />)
    stubRect(screen.getByTestId('container'))

    fireEvent.pointerDown(screen.getByTestId('divider'), { clientY: 500 })
    fireEvent(document, new PointerEvent('pointerup'))
    onChange.mockClear()

    fireEvent(document, new PointerEvent('pointermove', { clientY: 200 }))
    expect(onChange).not.toHaveBeenCalled()
  })
})

const session = (id: string) =>
  ({
    id,
    projectName: 'p',
    projectPath: '/p',
    agentType: 'claude',
    createdAt: 0,
    displayName: id
  }) as never

describe('cardSplits store', () => {
  beforeEach(() => {
    localStorage.clear()
    ;(window as unknown as { api: Record<string, unknown> }).api = {
      ...(window as unknown as { api?: Record<string, unknown> }).api,
      notifyWidgetStatus: vi.fn(),
      reorderSessions: vi.fn()
    }
    act(() => {
      useAppStore.setState({
        terminals: new Map([
          ['t1', { id: 't1', session: session('t1'), status: 'idle', lastOutputTimestamp: 1 }]
        ]) as never,
        cardSplits: {},
        filesPanes: new Set(),
        editorPanes: new Map(),
        browserPanes: new Map()
      })
    })
  })

  it('persists a committed split under vorn:cardSplits', () => {
    act(() => useAppStore.getState().setCardSplit('t1', { terminal: 0.6, panes: [0.4] }))

    expect(useAppStore.getState().cardSplits.t1).toEqual({ terminal: 0.6, panes: [0.4] })
    expect(JSON.parse(localStorage.getItem('vorn:cardSplits') as string)).toEqual({
      t1: { terminal: 0.6, panes: [0.4] }
    })
  })

  it('clamps the terminal split and drops a corrupted pane weight', () => {
    act(() => useAppStore.getState().setCardSplit('t1', { terminal: 0.01, panes: [NaN] }))

    // The terminal ratio is two-sided and clamps; pane weights are column shares
    // that must not be floored, so a corrupted one is dropped and `splitPaneWeights`
    // pads the list back to the pane count on read.
    expect(useAppStore.getState().cardSplits.t1).toEqual({ terminal: 0.15, panes: [] })
  })

  it("drops a session's split when the session closes", () => {
    act(() => useAppStore.getState().setCardSplit('t1', { terminal: 0.6, panes: [] }))

    act(() => useAppStore.getState().removeTerminal('t1'))
    // A recycled id must not inherit a divider position it never set.
    expect(useAppStore.getState().cardSplits.t1).toBeUndefined()
  })

  it('reconciles a split whose session never came back after a restart', () => {
    act(() =>
      useAppStore.setState({
        cardSplits: { t1: { terminal: 0.6, panes: [] }, ghost: { terminal: 0.3, panes: [] } },
        // What the server has: t1 came back, ghost did not.
        knownSessionIds: new Set(['t1'])
      })
    )

    act(() => useAppStore.getState().setVisibleTerminalIds(['t1']))

    expect(useAppStore.getState().cardSplits.ghost).toBeUndefined()
    expect(useAppStore.getState().cardSplits.t1).toBeDefined()
    expect(JSON.parse(localStorage.getItem('vorn:cardSplits') as string).ghost).toBeUndefined()
  })
})
