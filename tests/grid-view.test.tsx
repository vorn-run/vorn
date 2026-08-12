// @vitest-environment jsdom
import { forwardRef } from 'react'
import { describe, it, expect, vi, beforeEach, beforeAll, afterAll } from 'vitest'
import { render, screen, act } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

Object.defineProperty(window, 'api', {
  value: {
    isWorktreeDirty: vi.fn().mockResolvedValue(false),
    getGitDiffStat: vi.fn().mockResolvedValue(null),
    getGitBranch: vi.fn().mockResolvedValue(null),
    notifyWidgetStatus: vi.fn()
  },
  writable: true
})

type ResizeCb = (entries: Array<{ contentRect: DOMRectReadOnly }>, ob: ResizeObserver) => void
class MockResizeObserver {
  constructor(public cb: ResizeCb) {}
  observe(el: Element) {
    // Fire once synchronously with the element's "measured" rect so
    // useContainerSize() receives a real size (jsdom defaults to 0×0).
    const r = el.getBoundingClientRect()
    this.cb([{ contentRect: r as unknown as DOMRectReadOnly }], this as unknown as ResizeObserver)
  }
  unobserve() {}
  disconnect() {}
}

const originalGetBoundingClientRect = Element.prototype.getBoundingClientRect
beforeAll(() => {
  vi.stubGlobal('ResizeObserver', MockResizeObserver)
  // Stub getBoundingClientRect so measured sizes are non-zero in jsdom.
  Element.prototype.getBoundingClientRect = function () {
    return {
      width: 1920,
      height: 1080,
      top: 0,
      left: 0,
      right: 1920,
      bottom: 1080,
      x: 0,
      y: 0,
      toJSON: () => ({})
    }
  }
})
afterAll(() => {
  vi.unstubAllGlobals()
  Element.prototype.getBoundingClientRect = originalGetBoundingClientRect
})

// Stub AgentCard with forwardRef so GridView's ref callback (which calls
// cardRefs.current.set(id, el)) actually fires. Sessions are the grid's only
// layout unit now — a session's panes render inside its own card.
vi.mock('../src/renderer/components/AgentCard', () => ({
  AgentCard: forwardRef<
    HTMLDivElement,
    { terminalId: string; index: number; isDragTarget: boolean }
  >(function MockAgentCard({ terminalId, index, isDragTarget }, ref) {
    return (
      <div
        ref={ref}
        data-testid={`card-${terminalId}`}
        data-index={index}
        data-drag-target={isDragTarget ? 'yes' : 'no'}
      />
    )
  })
}))

vi.mock('../src/renderer/components/PromptLauncher', () => ({
  PromptLauncher: () => null
}))

vi.mock('../src/renderer/components/GridContextMenu', () => ({
  GridContextMenu: () => null
}))

vi.mock('../src/renderer/hooks/useIsMobile', () => ({
  useIsMobile: () => false
}))

vi.mock('../src/renderer/hooks/useFilteredHeadless', () => ({
  useFilteredHeadless: () => []
}))

import { useAppStore } from '../src/renderer/stores'
import { GridView } from '../src/renderer/components/GridView'
import { pickAutoLayout, fitMaxRows } from '../src/renderer/lib/auto-grid-layout'

const mockConfig = {
  projects: [
    {
      name: 'Vorn',
      path: '/tmp/vorn',
      icon: 'Rocket',
      iconColor: '#ff0000',
      preferredAgents: ['claude' as const]
    }
  ],
  workflows: [],
  defaults: { defaultAgent: 'claude' as const, rowHeight: 208 },
  remoteHosts: [],
  workspaces: []
}

function makeTerminal(id: string, status: 'idle' | 'running' = 'idle') {
  return {
    id,
    session: {
      id,
      agentType: 'claude' as const,
      projectName: 'Vorn',
      projectPath: '/tmp/vorn',
      isWorktree: false,
      branch: 'main',
      createdAt: Date.now()
    },
    status,
    lastOutputTimestamp: Date.now()
  }
}

beforeEach(() => {
  const terminals = new Map()
  terminals.set('term-a', makeTerminal('term-a'))
  terminals.set('term-b', makeTerminal('term-b', 'running'))

  useAppStore.setState({
    terminals,
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    config: mockConfig as any,
    activeProject: 'Vorn',
    activeWorktreePath: null,
    activeWorkspace: 'personal',
    gridColumns: 2,
    sortMode: 'manual',
    statusFilter: 'all',
    focusedTerminalId: null,
    previewTerminalId: null,
    minimizedTerminals: new Set<string>(),
    filesPanes: new Set<string>(),
    editorPanes: new Map(),
    maximizedPaneId: null,
    flexibleLayouts: {},
    rowHeight: 208,
    terminalOrder: ['term-a', 'term-b']
  })
})

describe('GridView', () => {
  it('renders an AgentCard for each terminal in the active project', () => {
    render(<GridView />)
    expect(screen.getByTestId('card-term-a')).toBeInTheDocument()
    expect(screen.getByTestId('card-term-b')).toBeInTheDocument()
  })

  it('passes the card index in sort order', () => {
    render(<GridView />)
    expect(screen.getByTestId('card-term-a').dataset.index).toBe('0')
    expect(screen.getByTestId('card-term-b').dataset.index).toBe('1')
  })

  it('omits the drag-start handler when sort mode is not manual', () => {
    useAppStore.setState({ sortMode: 'created' })
    render(<GridView />)
    expect(screen.getByTestId('card-term-a').dataset.dragTarget).toBe('no')
  })
})

describe('GridView smart Auto mode', () => {
  it('fit mode on 1920x1080 with 2 cards produces a 2×1 grid-template', () => {
    useAppStore.setState({ gridColumns: 0 })
    const { container } = render(<GridView />)
    const grid = container.querySelector('.grid') as HTMLElement | null
    expect(grid).not.toBeNull()
    expect(grid!.style.gridTemplateColumns).toBe('repeat(2, 1fr)')
    expect(grid!.style.gridTemplateRows).toBe('repeat(1, 1fr)')
  })

  it('scroll mode kicks in once the card count exceeds the fit cap', () => {
    // On 1920×1080 maxCols=4, maxRows=3 → fit cap is 12. Render 14 cards to spill.
    const terminals = new Map()
    const order: string[] = []
    for (let i = 0; i < 14; i++) {
      const id = `t-${i}`
      terminals.set(id, {
        id,
        session: {
          id,
          agentType: 'claude' as const,
          projectName: 'Vorn',
          projectPath: '/tmp/vorn',
          isWorktree: false,
          branch: 'main',
          createdAt: Date.now()
        },
        status: 'idle',
        lastOutputTimestamp: Date.now()
      })
      order.push(id)
    }
    useAppStore.setState({ gridColumns: 0, terminals, terminalOrder: order })
    const { container } = render(<GridView />)
    const grid = container.querySelector('.grid') as HTMLElement | null
    expect(grid).not.toBeNull()
    expect(grid!.style.overflowY).toBe('auto')
    expect(grid!.style.gridAutoRows).toMatch(/\d+px/)
  })
})

describe('pickAutoLayout', () => {
  const W = 1920
  const H = 1080

  it('n=0 returns 1x1 fit', () => {
    expect(pickAutoLayout(0, W, H)).toEqual({ cols: 1, rows: 1, mode: 'fit' })
  })

  it('n=1 is a single fullscreen card', () => {
    expect(pickAutoLayout(1, W, H)).toEqual({ cols: 1, rows: 1, mode: 'fit' })
    expect(pickAutoLayout(1, 800, 600)).toEqual({ cols: 1, rows: 1, mode: 'fit' })
  })

  it('n=2 splits the screen into two columns', () => {
    expect(pickAutoLayout(2, W, H)).toEqual({ cols: 2, rows: 1, mode: 'fit' })
  })

  it('n=3 splits into three columns', () => {
    expect(pickAutoLayout(3, W, H)).toEqual({ cols: 3, rows: 1, mode: 'fit' })
  })

  it('n=4 on 1920x1080 prefers 2x2 over 4x1', () => {
    const layout = pickAutoLayout(4, W, H)
    expect(layout.mode).toBe('fit')
    expect(layout.cols).toBe(2)
    expect(layout.rows).toBe(2)
  })

  it('n=6 on 1920x1080 picks 3x2', () => {
    expect(pickAutoLayout(6, W, H)).toEqual({ cols: 3, rows: 2, mode: 'fit' })
  })

  it('n=6 on MacBook Pro retina (~2200x1400) picks 3x2, not 2x3', () => {
    // Regression: earlier aspect target 16:10 picked 2 cols × 3 rows (wide-short
    // cards) on this viewport. Squarer target should give 3 cols × 2 rows.
    expect(pickAutoLayout(6, 2200, 1400)).toEqual({ cols: 3, rows: 2, mode: 'fit' })
  })

  it('n=9 on 1920x1080 packs into 3x3 fit', () => {
    expect(pickAutoLayout(9, W, H)).toEqual({ cols: 3, rows: 3, mode: 'fit' })
  })

  it('n=10 on 1920x1080 fits in 4x3 (4-col cap on landscape)', () => {
    const layout = pickAutoLayout(10, W, H)
    expect(layout.mode).toBe('fit')
    expect(layout.cols).toBe(4)
    expect(layout.rows).toBe(3)
  })

  it('n=6 on a small-ish laptop window (1230x860) picks 3x2', () => {
    // Regression: earlier comfy threshold 480px blocked 3 cols here, forcing 2x3.
    // With the hard min (320), 1230/320=3 cols is allowed, and 3x2 wins.
    expect(pickAutoLayout(6, 1230, 860)).toEqual({ cols: 3, rows: 2, mode: 'fit' })
  })

  it('n=10 on a laptop window (1230x860) spills to scroll — 4 rows too short', () => {
    // Row cap: 860/280=3 rows, cols cap: 1230/320=3 cols → 3x3=9 fit cap.
    // Prevents the 3x4 layout where each row would be ~215px tall.
    const layout = pickAutoLayout(10, 1230, 860)
    expect(layout.mode).toBe('scroll')
  })

  it('n=10 on a small window (1000x700) scrolls (cap is lower)', () => {
    // 1000/320=3 cols, 700/280=2 rows → 3x2 = 6 cap; n=10 spills.
    const layout = pickAutoLayout(10, 1000, 700)
    expect(layout.mode).toBe('scroll')
  })

  it('n=16 on a 4K monitor packs into 4x4 fit', () => {
    // 2560x1440 is comfortable for 4 cols (640 wide) and 4 rows (360 tall)
    const layout = pickAutoLayout(16, 2560, 1440)
    expect(layout).toEqual({ cols: 4, rows: 4, mode: 'fit' })
  })

  it('n=12 on a 4K monitor packs into 4x3 fit', () => {
    const layout = pickAutoLayout(12, 2560, 1440)
    expect(layout.mode).toBe('fit')
    expect(layout.cols).toBe(4)
    expect(layout.rows).toBe(3)
  })

  it('n=17 on a 4K monitor spills to scroll (fit cap is 16)', () => {
    const layout = pickAutoLayout(17, 2560, 1440)
    expect(layout.mode).toBe('scroll')
  })

  it('fit mode never exceeds 4x4 even on huge viewports', () => {
    for (let n = 1; n <= 16; n++) {
      const layout = pickAutoLayout(n, 5120, 2880)
      if (layout.mode === 'fit') {
        expect(layout.cols).toBeLessThanOrEqual(4)
        expect(layout.rows).toBeLessThanOrEqual(4)
      }
    }
  })

  it('falls back to scroll when cards would be unusably small', () => {
    const layout = pickAutoLayout(20, 1280, 720)
    expect(layout.mode).toBe('scroll')
    expect(layout.cols).toBeGreaterThanOrEqual(1)
    expect(layout.cols).toBeLessThanOrEqual(4)
    expect(layout.rows).toBe(Math.ceil(20 / layout.cols))
  })

  it('never exceeds the card count in columns', () => {
    for (let n = 1; n <= 12; n++) {
      const layout = pickAutoLayout(n, W, H)
      expect(layout.cols).toBeLessThanOrEqual(n)
      expect(layout.cols * layout.rows).toBeGreaterThanOrEqual(n)
    }
  })

  it('narrow viewport falls through n=2 single-row shortcut', () => {
    // W=500: 500/2=250 < AUTO_MIN_CARD_W(320), so we bypass the shortcut
    // and the scoring loop picks a 1-col, 2-row layout.
    const layout = pickAutoLayout(2, 500, 900)
    expect(layout.cols).toBe(1)
    expect(layout.rows).toBe(2)
  })
})

describe('fitMaxRows', () => {
  it('clamps to 1 on very short viewports', () => {
    expect(fitMaxRows(100)).toBe(1)
  })

  it('scales with viewport height', () => {
    expect(fitMaxRows(600)).toBe(2)
    expect(fitMaxRows(900)).toBe(3)
  })

  it('caps at the hard max (4) on very tall viewports', () => {
    expect(fitMaxRows(5000)).toBe(4)
  })
})

describe('GridView — session-owned panes', () => {
  it('gives a session one cell no matter how many panes it owns', () => {
    act(() => {
      useAppStore.getState().openFilesPane('term-a')
      useAppStore.getState().openEditorPane('term-a', '/repo/a.ts')
    })
    render(<GridView />)

    // Panes render inside their owner's card, so they never claim grid cells.
    expect(screen.getByTestId('card-term-a')).toBeInTheDocument()
    expect(screen.queryByTestId('card-files:term-a')).not.toBeInTheDocument()
    expect(screen.getByTestId('card-term-b')).toBeInTheDocument()
    expect(screen.getByTestId('card-term-a').dataset.index).toBe('0')
    expect(screen.getByTestId('card-term-b').dataset.index).toBe('1')
  })

  it('leaves the grid untouched while a pane is maximized', () => {
    act(() => {
      useAppStore.getState().openFilesPane('term-a')
      useAppStore.getState().setMaximizedPane('files:term-a')
    })
    render(<GridView />)

    // A maximized pane fills its own card; the grid neither hides cells nor
    // spans them, so the compare case (other sessions visible) always holds.
    expect(screen.getByTestId('card-term-a')).toBeInTheDocument()
    expect(screen.getByTestId('card-term-b')).toBeInTheDocument()
    expect(screen.getByTestId('card-term-a').parentElement?.style.gridColumn).toBe('')
  })

  it('renders sessions in the flexible layout too', () => {
    act(() => {
      useAppStore.setState({ gridColumns: -1 })
      useAppStore.getState().openFilesPane('term-a')
    })
    render(<GridView />)

    expect(screen.getByTestId('card-term-a')).toBeInTheDocument()
    expect(screen.queryByTestId('card-files:term-a')).not.toBeInTheDocument()
  })
})
