// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from 'vitest'
import { render, screen, cleanup, fireEvent, act } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

vi.mock('../src/renderer/components/file-icons', () => ({
  FileTypeIcon: () => <span data-testid="file-icon" />
}))

import {
  DIFF_BLOCK_ROWS,
  DiffContent,
  estimateBlockHeight,
  parseDiffRows,
  type DiffComment
} from '../src/renderer/components/DiffSidebar'
import type { GitFileDiff } from '../src/shared/types'

const DIFF = [
  'diff --git a/a.ts b/a.ts',
  'index 1..2 100644',
  '--- a/a.ts',
  '+++ b/a.ts',
  '@@ -10,3 +10,3 @@ fn',
  ' same',
  '-old',
  '+new',
  '\\ No newline at end of file'
].join('\n')

const file = (diff: string, filePath = 'a.ts'): GitFileDiff =>
  ({ filePath, status: 'modified', insertions: 1, deletions: 1, diff }) as GitFileDiff

function props(over: Partial<Parameters<typeof DiffContent>[0]> = {}) {
  return {
    files: [file(DIFF)],
    selectedFile: null,
    comments: [] as DiffComment[],
    commentingLine: null,
    onClickLine: vi.fn(),
    onAddComment: vi.fn(),
    onCancelComment: vi.fn(),
    onRemoveComment: vi.fn(),
    ...over
  }
}

afterEach(() => {
  cleanup()
  vi.unstubAllGlobals()
})

describe('parseDiffRows', () => {
  it('numbers each side from the hunk, and keys each row by its line in the raw diff', () => {
    expect(parseDiffRows(DIFF)).toEqual([
      { kind: 'hunk', index: 4, text: '@@ -10,3 +10,3 @@ fn' },
      { kind: 'ctx', index: 5, text: ' same', oldLine: 10, newLine: 10 },
      { kind: 'del', index: 6, text: '-old', oldLine: 11 },
      { kind: 'add', index: 7, text: '+new', newLine: 11 }
    ])
  })

  it('keeps metadata before any hunk, such as a binary file, and drops blank lines', () => {
    expect(parseDiffRows('Binary files a and b differ\n\n')).toEqual([
      { kind: 'meta', index: 0, text: 'Binary files a and b differ' }
    ])
  })
})

describe('DiffContent', () => {
  it('draws the lines with both line numbers and hands a clicked change to onClickLine', () => {
    const p = props()
    render(<DiffContent {...p} />)
    expect(screen.getByText('same')).toBeInTheDocument()
    fireEvent.click(screen.getByText('new'))
    expect(p.onClickLine).toHaveBeenCalledWith('a.ts', 7, '+new')
    // A context line is not a change to comment on.
    fireEvent.click(screen.getByText('same'))
    expect(p.onClickLine).toHaveBeenCalledTimes(1)
  })

  it('shows a line’s comments under it, removes the right one, and counts them on the file', () => {
    const comments: DiffComment[] = [
      { filePath: 'other.ts', lineIndex: 7, lineContent: '+x', comment: 'elsewhere' },
      { filePath: 'a.ts', lineIndex: 7, lineContent: '+new', comment: 'rename this' }
    ]
    const p = props({ comments })
    render(<DiffContent {...p} />)
    expect(screen.getByText('rename this')).toBeInTheDocument()
    expect(screen.queryByText('elsewhere')).not.toBeInTheDocument()
    expect(screen.getByText('1 comment')).toBeInTheDocument()
    fireEvent.click(screen.getByText('rename this').parentElement!.querySelector('button')!)
    expect(p.onRemoveComment).toHaveBeenCalledWith(1)
  })

  it('opens the comment box on the line being commented', () => {
    render(<DiffContent {...props({ commentingLine: { filePath: 'a.ts', lineIndex: 6 } })} />)
    expect(screen.getByPlaceholderText('Add review comment...')).toBeInTheDocument()
  })
})

describe('a long diff', () => {
  /** An IntersectionObserver the test drives: every block starts out of view. */
  class FakeObserver {
    static all: FakeObserver[] = []
    targets: Element[] = []
    constructor(public callback: IntersectionObserverCallback) {
      FakeObserver.all.push(this)
    }
    observe(el: Element): void {
      this.targets.push(el)
    }
    disconnect(): void {}
    show(visible: boolean): void {
      this.callback(
        [{ isIntersecting: visible } as IntersectionObserverEntry],
        this as unknown as IntersectionObserver
      )
    }
  }

  const longDiff = (lines: number): string =>
    ['@@ -1,1 +1,' + lines + ' @@', ...Array.from({ length: lines }, (_, i) => `+line ${i}`)].join(
      '\n'
    )

  it('draws only the blocks near the view, and stands empty boxes of the same height in for the rest', () => {
    FakeObserver.all = []
    vi.stubGlobal('IntersectionObserver', FakeObserver)
    // The panel is 600px tall and every block starts far below it.
    vi.spyOn(Element.prototype, 'getBoundingClientRect').mockImplementation(function (
      this: Element
    ) {
      return (
        this.hasAttribute('data-diff-scroll')
          ? { top: 0, bottom: 600, height: 600 }
          : { top: 50_000, bottom: 52_000, height: 2000 }
      ) as DOMRect
    })
    // With the hunk header, three full blocks.
    const rowsTotal = DIFF_BLOCK_ROWS * 3 - 1
    const { container } = render(<DiffContent {...props({ files: [file(longDiff(rowsTotal))] })} />)
    // Nothing starts near the view.
    expect(screen.queryByText('line 0')).not.toBeInTheDocument()
    const blocks = container.querySelectorAll('pre > div')
    expect(blocks).toHaveLength(3)
    const firstRows = parseDiffRows(longDiff(rowsTotal)).slice(0, DIFF_BLOCK_ROWS)
    expect((blocks[0] as HTMLElement).style.height).toBe(`${estimateBlockHeight(firstRows)}px`)

    act(() => FakeObserver.all[1].show(true))
    expect(screen.getByText('line 150')).toBeInTheDocument()
    expect(screen.queryByText('line 0')).not.toBeInTheDocument()
    act(() => FakeObserver.all[1].show(false))
    expect(screen.queryByText('line 150')).not.toBeInTheDocument()
    // The box left behind is as tall as the block was when drawn.
    expect((container.querySelectorAll('pre > div')[1] as HTMLElement).style.height).toBe('2000px')
    vi.restoreAllMocks()
  })

  it("forgets a drawn block's height when the diff changes under it", () => {
    FakeObserver.all = []
    vi.stubGlobal('IntersectionObserver', FakeObserver)
    vi.spyOn(Element.prototype, 'getBoundingClientRect').mockImplementation(function (
      this: Element
    ) {
      return (
        this.hasAttribute('data-diff-scroll')
          ? { top: 0, bottom: 600, height: 600 }
          : { top: 50_000, bottom: 52_000, height: 2000 }
      ) as DOMRect
    })
    const p = props({ files: [file(longDiff(DIFF_BLOCK_ROWS * 2))] })
    const { container, rerender } = render(<DiffContent {...p} />)
    act(() => FakeObserver.all[1].show(true))
    act(() => FakeObserver.all[1].show(false))
    const second = (): HTMLElement => container.querySelectorAll('pre > div')[1] as HTMLElement
    expect(second().style.height).toBe('2000px')
    // The second block now holds one row.
    const shorter = longDiff(DIFF_BLOCK_ROWS)
    rerender(<DiffContent {...p} files={[file(shorter)]} />)
    const lastRows = parseDiffRows(shorter).slice(DIFF_BLOCK_ROWS)
    expect(second().style.height).toBe(`${estimateBlockHeight(lastRows)}px`)
    vi.restoreAllMocks()
  })

  it('always draws a block with a comment in it, so nothing being read or typed is taken away', () => {
    FakeObserver.all = []
    vi.stubGlobal('IntersectionObserver', FakeObserver)
    const diff = longDiff(DIFF_BLOCK_ROWS * 2)
    render(
      <DiffContent
        {...props({
          files: [file(diff)],
          comments: [{ filePath: 'a.ts', lineIndex: 151, lineContent: '+line 150', comment: 'hm' }],
          commentingLine: { filePath: 'a.ts', lineIndex: 2 }
        })}
      />
    )
    expect(screen.getByText('hm')).toBeInTheDocument()
    expect(screen.getByPlaceholderText('Add review comment...')).toBeInTheDocument()
  })

  it('counts a hunk header a little taller than a line', () => {
    expect(
      estimateBlockHeight([
        { kind: 'hunk', index: 0, text: '@@' },
        { kind: 'add', index: 1, text: '+x', newLine: 1 }
      ])
    ).toBeCloseTo(19.2 * 2 + 4)
  })
})

describe('a block already in view when the panel opens', () => {
  it('is drawn in the first frame, before any observer reports', () => {
    vi.stubGlobal(
      'IntersectionObserver',
      class {
        observe(): void {}
        disconnect(): void {}
      }
    )
    // Everything at the top of a 600px panel.
    vi.spyOn(Element.prototype, 'getBoundingClientRect').mockReturnValue({
      top: 0,
      bottom: 600,
      height: 600
    } as DOMRect)
    render(<DiffContent {...props()} />)
    expect(screen.getByText('new')).toBeInTheDocument()
    vi.restoreAllMocks()
  })
})
