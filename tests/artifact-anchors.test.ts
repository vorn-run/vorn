// @vitest-environment jsdom
import { describe, it, expect, beforeEach } from 'vitest'
import { ANCHOR_LIB, anchorCall } from '../src/main/artifact-anchors'
import { latestSentBatch, marksFor, placePopover } from '../src/renderer/lib/artifact-comments'
import type { ArtifactComment } from '../src/shared/types'

interface Lib {
  index: () => { text: string }
  locate: (
    idx: { text: string },
    a: { quote: string; prefix: string; suffix: string }
  ) => { start: number; end: number } | null
  selection: () => {
    anchor: { kind: 'quote'; quote: string; prefix: string; suffix: string }
  } | null
  paint: (marks: unknown[]) => Record<string, boolean>
  clear: () => void
}
const lib = (): Lib => window.eval(ANCHOR_LIB) as Lib

beforeEach(() => {
  document.body.innerHTML = `
    <h3>Step 7:   Evaluate</h3>
    <p>I scored four contenders
       on the same alerts, and <em>two API models</em>, a frontier model and a cheap one.</p>
    <script>var two = 'two API models'</script>
    <p>Later, two API models again, both through OpenRouter.</p>`
})

describe('anchors inside the page', () => {
  it('reads text with whitespace collapsed and scripts left out', () => {
    const { text } = lib().index()
    expect(text).toContain('Step 7: Evaluate I scored four contenders on the same alerts')
    expect(text).not.toContain('var two')
  })

  it('finds the occurrence whose surroundings match, not merely the first', () => {
    const l = lib()
    const idx = l.index()
    const second = l.locate(idx, {
      quote: 'two API models',
      prefix: 'Later, ',
      suffix: ' again, both'
    })
    const first = l.locate(idx, {
      quote: 'two API models',
      prefix: 'same alerts, and ',
      suffix: ', a frontier'
    })
    expect(second && idx.text.slice(second.start - 7, second.start)).toBe('Later, ')
    expect(first!.start).toBeLessThan(second!.start)
  })

  it('reports a quote whose words are gone instead of pinning it elsewhere', () => {
    const l = lib()
    expect(l.locate(l.index(), { quote: 'three API models', prefix: '', suffix: '' })).toBeNull()
  })

  it('matches a quote across markup and line breaks', () => {
    const l = lib()
    const hit = l.locate(l.index(), {
      quote: 'four contenders on the same alerts, and two API models',
      prefix: '',
      suffix: ''
    })
    expect(hit).not.toBeNull()
  })

  it('reads the selection as a quote with the words either side', () => {
    const em = document.querySelector('em')!
    const range = document.createRange()
    range.selectNodeContents(em)
    const sel = window.getSelection()!
    sel.removeAllRanges()
    sel.addRange(range)

    const read = lib().selection()!
    expect(read.anchor.quote).toBe('two API models')
    expect(read.anchor.prefix.endsWith('same alerts, and ')).toBe(true)
    expect(read.anchor.suffix.startsWith(', a frontier')).toBe(true)
    expect(lib().locate(lib().index(), read.anchor)).not.toBeNull()

    lib().clear()
    expect(lib().selection()).toBeNull()
  })

  it('answers which marks found their words, even where highlights are unsupported', () => {
    const found = lib().paint([
      { id: 'a', quote: 'two API models', prefix: '', suffix: '', state: 'draft' },
      { id: 'b', quote: 'no such words', prefix: '', suffix: '', state: 'sent' }
    ])
    expect(found).toEqual({ a: true, b: false })
  })

  it('passes a quote in as data, so a quote with quotes in it stays a quote', () => {
    const expr = anchorCall('paint', [
      { id: 'x', quote: `'); alert(1); ('`, prefix: '', suffix: '', state: 'draft' }
    ])
    expect(window.eval(expr)).toEqual({ x: false })
  })
})

const c = (id: string, over: Partial<ArtifactComment>): ArtifactComment => ({
  id,
  artifactId: 'a1',
  version: 3,
  anchor: { kind: 'quote', quote: id, prefix: '', suffix: '' },
  body: 'b',
  state: 'draft',
  createdAt: '',
  updatedAt: '',
  ...over
})

describe('rail helpers', () => {
  it('keeps the last batch sent', () => {
    const comments = [
      c('old', { state: 'sent', batchId: 'b1', sentAt: '2026-09-23T09:00:00Z' }),
      c('new1', { state: 'sent', batchId: 'b2', sentAt: '2026-09-23T10:00:00Z' }),
      c('new2', { state: 'sent', batchId: 'b2', sentAt: '2026-09-23T10:00:00Z' }),
      c('draft', {})
    ]
    expect(latestSentBatch(comments).map((x) => x.id)).toEqual(['new1', 'new2'])
  })

  it('marks drafts, the sent batch and the focused one, and skips notes with no words', () => {
    const marks = marksFor(
      [c('d1', {}), c('note', { anchor: null })],
      [c('s1', { state: 'sent' })],
      'd1'
    )
    expect(marks.map((m) => [m.id, m.state])).toEqual([
      ['s1', 'sent'],
      ['d1', 'focus']
    ])
  })

  it('keeps the popover inside the page, flipping above near the bottom', () => {
    expect(
      placePopover({ x: 900, y: 40, width: 50, height: 16 }, { width: 800, height: 600 })
    ).toEqual({
      x: 532,
      y: 64
    })
    expect(
      placePopover({ x: 10, y: 560, width: 50, height: 16 }, { width: 800, height: 600 }).y
    ).toBe(402)
  })
})
