import fs from 'node:fs'
import path from 'node:path'
import { describe, it, expect } from 'vitest'
import { loadNativeCore, NATIVE_STATUS } from '../packages/server/src/native-core'
import { BACKGROUND_ONLY_ROWS, BLANKS_AS_SPACES, PALETTE_AS_256 } from './helpers/screen-parity'
import { replayView, type View } from './helpers/screen-view'
import { REDRAWN_LINES, redrawnLines, withoutRedrawn } from './helpers/analysis-parity'
import screenReference from './fixtures/js-reference/screen.json'
import analysisReference from './fixtures/js-reference/analysis.json'

/**
 * The native terminal path against what the JavaScript path it replaced
 * produced for the same input, recorded from the last release that had it
 * (`fixtures/js-reference`). The JavaScript path is gone; these fixtures are
 * what a change to the Rust side is checked against.
 *
 * The screen is compared as `screen-parity` defines it: each serialized screen
 * replayed into the client's emulator and read back per feature, with every
 * accepted difference named in `helpers/screen-parity.ts`. Analysis is
 * compared read for read, the status after each read, then by the output
 * lines agents read back, with its accepted differences named in
 * `helpers/analysis-parity.ts`.
 */

const builtCore = path.resolve(__dirname, '../packages/core/vorn_core.node')
const core = fs.existsSync(builtCore) ? loadNativeCore([builtCore]) : null

interface ScreenCase {
  name: string
  cols: number
  rows: number
  input: string
  title: string
  view: View
}

interface AnalysisCase {
  name: string
  reads: string[]
  statuses: string[]
  lines: string[]
  partial: string
}

async function nativeScreen(c: ScreenCase): Promise<{ view: View; title: string }> {
  const pipeline = new core!.TerminalPipeline!(c.cols, c.rows, () => {})
  try {
    pipeline.feed(c.input)
    const snap = pipeline.serialize()
    return { view: await replayView(snap.screen, c.cols, c.rows), title: snap.title }
  } finally {
    pipeline.free()
  }
}

const rowOf = (v: View, y: number): string[] =>
  v.styles.filter((s) => s.split(' ')[0].endsWith(`,${y}`))

describe.runIf(core?.TerminalPipeline)('the native screen against the JS reference', () => {
  for (const c of screenReference.cases as ScreenCase[]) {
    it(c.name, async () => {
      const native = await nativeScreen(c)
      expect(native.title).toBe(c.title)
      expect(native.view.rows).toEqual(c.view.rows)
      expect(native.view.cursor).toEqual(c.view.cursor)
      expect(native.view.alternate).toBe(c.view.alternate)
      if (c.name === 'a row of only background') {
        // BACKGROUND_ONLY_ROWS. When this fails, Ghostty has started writing
        // these rows: drop the special case and compare them like the rest.
        expect(rowOf(c.view, 1), `${BACKGROUND_ONLY_ROWS}`).toHaveLength(c.cols)
        expect(rowOf(native.view, 1), `${BACKGROUND_ONLY_ROWS}`).toEqual([])
        return
      }
      expect(native.view.styles, `${PALETTE_AS_256}, ${BLANKS_AS_SPACES}`).toEqual(c.view.styles)
    })
  }
})

describe.runIf(core?.Analyzer)('native output analysis against the JS reference', () => {
  for (const c of analysisReference.cases as AnalysisCase[]) {
    it(c.name, () => {
      const analyzer = new core!.Analyzer!()
      try {
        let status = 'running'
        const statuses: string[] = []
        for (const read of c.reads) {
          status = NATIVE_STATUS[analyzer.append(read, true)] ?? status
          statuses.push(status)
        }
        expect(statuses).toEqual(c.statuses)
        const redrawn = redrawnLines(c.reads)
        expect(withoutRedrawn(analyzer.output(), redrawn), `${REDRAWN_LINES}`).toEqual(
          withoutRedrawn(c.lines, redrawn)
        )
        // The line in progress comes after the last full one.
        if (!redrawn.has(c.lines.length)) expect(analyzer.partial()).toBe(c.partial)
      } finally {
        analyzer.free()
      }
    })
  }
})
