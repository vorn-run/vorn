import fs from 'node:fs'
import path from 'node:path'
import { describe, it, expect, beforeAll, afterAll } from 'vitest'
import { BACKGROUND_ONLY_ROWS, BLANKS_AS_SPACES, PALETTE_AS_256 } from './helpers/screen-parity'
import { replayView, type View } from './helpers/screen-view'
import {
  BytesClient,
  Vornd,
  home,
  killPid,
  until,
  upstream,
  vorndSessionsAvailable
} from './helpers/vornd-sessions'
import screenReference from './fixtures/js-reference/screen.json'

/**
 * vornd's screen against what the JavaScript path it replaced produced for the
 * same input, recorded from the last release that had it
 * (`fixtures/js-reference`). The JavaScript path is gone; these fixtures are
 * what a change to the Rust side is checked against. Output analysis is
 * checked against the same fixtures in Rust (`crates/analysis/tests`).
 *
 * Each case's input is printed, untouched, by a program in vornd's holder, and
 * the screen vornd serializes for an attaching client is replayed into the
 * client's emulator and read back per feature, as `screen-parity` defines it,
 * with every accepted difference named in `helpers/screen-parity.ts`.
 *
 * Runs in `yarn test:conformance`, which builds vornd and its holder; skipped
 * otherwise.
 */

interface ScreenCase {
  name: string
  cols: number
  rows: number
  input: string
  title: string
  view: View
}

const rowOf = (v: View, y: number): string[] =>
  v.styles.filter((s) => s.split(' ')[0].endsWith(`,${y}`))

describe.runIf(vorndSessionsAvailable)('vornd’s screen against the JS reference', () => {
  let server: Awaited<ReturnType<typeof upstream>>
  let dir: ReturnType<typeof home>
  let vornd: Vornd
  let holder: number | null = null
  const clients: BytesClient[] = []

  beforeAll(async () => {
    server = await upstream()
    dir = home()
    vornd = await Vornd.start(server.port, dir.dir)
    holder = await vornd.sessiondPid()
  }, 30_000)

  afterAll(async () => {
    for (const c of clients) c.close()
    await vornd.kill()
    killPid(holder)
    server.close()
    dir.remove()
  })

  /** The screen vornd holds once the case's input has all been printed. */
  async function vorndScreen(c: ScreenCase, n: number): Promise<View> {
    const input = path.join(dir.dir, `case-${n}.bin`)
    fs.writeFileSync(input, c.input)
    const client = new BytesClient()
    await client.connect(vornd.port)
    clients.push(client)
    // Raw, so the line discipline passes the bytes as they are, then a mark
    // the screen never shows, to say the input is all through.
    const id = await client.spawn(
      [
        'sh',
        '-c',
        'stty raw -echo; cat "$0"; printf "\\033]7;file:///done\\007"; exec sleep 60',
        input
      ],
      c.cols,
      c.rows
    )
    await until(`${c.name} to be printed`, async () => {
      // Refused until the session is up.
      const printed = await client
        .call<{ data: string }>('terminal:readScrollback', { id })
        .catch(() => ({ data: '' }))
      return printed.data.includes('file:///done')
    })
    const { data } = await client.call<{ data: string }>('terminal:attach', { id })
    return replayView(data, c.cols, c.rows)
  }

  ;(screenReference.cases as ScreenCase[]).forEach((c, n) => {
    it(c.name, async () => {
      const view = await vorndScreen(c, n)
      expect(view.rows).toEqual(c.view.rows)
      expect(view.cursor).toEqual(c.view.cursor)
      expect(view.alternate).toBe(c.view.alternate)
      if (c.name === 'a row of only background') {
        // BACKGROUND_ONLY_ROWS. When this fails, Ghostty has started writing
        // these rows: drop the special case and compare them like the rest.
        expect(rowOf(c.view, 1), `${BACKGROUND_ONLY_ROWS}`).toHaveLength(c.cols)
        expect(rowOf(view, 1), `${BACKGROUND_ONLY_ROWS}`).toEqual([])
        return
      }
      expect(view.styles, `${PALETTE_AS_256}, ${BLANKS_AS_SPACES}`).toEqual(c.view.styles)
    })
  })
})
