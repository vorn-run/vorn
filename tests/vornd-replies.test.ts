import { describe, it, expect } from 'vitest'
import { Terminal as Headless } from '@xterm/headless'
import type { Terminal } from '@xterm/xterm'
import { swallowQueries } from '../src/renderer/lib/vornd-replies'

/**
 * TP-T12, the client's half: with vornd answering a session's queries, every
 * client attached to it must answer none. Real xterm.js, headless, so what is
 * tested is what xterm.js itself would have sent down the pty.
 */

const QUERIES: Record<string, string> = {
  DA1: '\x1b[c',
  DA2: '\x1b[>c',
  'DSR 6': '\x1b[6n',
  'DSR 5': '\x1b[5n',
  DECRQM: '\x1b[?2004$p',
  XTVERSION: '\x1b[>q',
  DECRQSS: '\x1bP$qm\x1b\\',
  'OSC 11': '\x1b]11;?\x1b\\',
  'OSC 10': '\x1b]10;?\x07',
  'OSC 4': '\x1b]4;1;?\x07'
}

/** What a terminal sends back after being written `bytes`. */
async function replies(bytes: string, answeredElsewhere: boolean): Promise<string[]> {
  const term = new Headless({ allowProposedApi: true, cols: 80, rows: 24 })
  swallowQueries(term as unknown as Terminal, () => answeredElsewhere)
  const sent: string[] = []
  term.onData((d) => sent.push(d))
  await new Promise<void>((resolve) => term.write(bytes, resolve))
  term.dispose()
  return sent
}

describe('query replies for a session vornd holds', () => {
  for (const [name, query] of Object.entries(QUERIES)) {
    it(`leaves ${name} to vornd`, async () => {
      expect(await replies(query, true)).toEqual([])
    })
  }

  it('still answers them for a session the server holds', async () => {
    // Only the queries xterm.js answers at all are worth checking here.
    const answered: Record<string, number> = {}
    for (const name of ['DA1', 'DSR 6', 'DECRQM']) {
      answered[name] = (await replies(QUERIES[name], false)).length
    }
    expect(answered).toEqual({ DA1: 1, 'DSR 6': 1, DECRQM: 1 })
  })

  it('never claims a colour being set, only one being asked for', async () => {
    const term = new Headless({ allowProposedApi: true })
    // Registered first, so xterm.js reaches it only when the newer handler declines.
    const reached: string[] = []
    for (const ident of [4, 11]) {
      term.parser.registerOscHandler(ident, (data) => {
        reached.push(`${ident};${data}`)
        return true
      })
    }
    swallowQueries(term as unknown as Terminal, () => true)
    await new Promise<void>((resolve) =>
      term.write('\x1b]4;1;rgb:12/34/56\x07\x1b]4;1;?\x07\x1b]11;#101010\x07\x1b]11;?\x07', resolve)
    )
    expect(reached).toEqual(['4;1;rgb:12/34/56', '11;#101010'])
    term.dispose()
  })
})
