import {
  appendScrollback,
  readScrollback,
  resetScrollback
} from '../../packages/server/src/terminal-scrollback'
import {
  createScreen,
  feedScreen,
  serializeScreen,
  resetScreens
} from '../../packages/server/src/terminal-screen'
import { chunks, ms, CHUNKS, COLS, ROWS } from './measure-output'

/**
 * How long the server spends on each chunk of terminal output.
 *
 * This is the hottest path in the process: every byte an agent prints arrives
 * here, and anything added to it is paid for on every keystroke of every session
 * at once. Two things changed on it — the byte buffer stopped rebuilding itself
 * per chunk, and a screen model was added — and "is it faster or slower than
 * before" is not answerable by reasoning about which sounds heavier.
 *
 * So all three shapes are measured against the same bytes: the buffer as it used
 * to be, the buffer as it is, and the buffer as it is with the model beside it.
 * Prints one line of JSON; the test reads it and decides.
 */

const MAX_UNITS = 256 * 1024

/** The buffer exactly as it was: concatenate the whole thing, then re-slice it. */
function appendTheOldWay(state: { buf: string }, data: string): void {
  const joined = state.buf + data
  if (joined.length <= MAX_UNITS) {
    state.buf = joined
    return
  }
  const cut = joined.length - MAX_UNITS
  const boundary = joined.indexOf('\n', cut)
  state.buf = boundary === -1 ? joined.slice(cut) : joined.slice(boundary + 1)
}

async function main(): Promise<void> {
  const data = chunks()

  // Warm the JIT on each shape, so the first one measured is not paying for
  // compilation the others avoid.
  const warm = data.slice(0, 500)
  const scratch = { buf: '' }
  for (const c of warm) appendTheOldWay(scratch, c)
  for (const c of warm) appendScrollback('warm', c)
  createScreen('warm', COLS, ROWS)
  for (const c of warm) feedScreen('warm', c)
  await serializeScreen('warm')
  resetScrollback()
  resetScreens()

  const before = { buf: '' }
  const oldBuffer = ms(() => {
    for (const c of data) appendTheOldWay(before, c)
  })

  // The read is inside the clock, not after it. The old shape paid its trim on
  // every append, so timing the new shape's writes alone against that would be
  // the old total against the new half -- flattering, and not the same
  // question. This is every cost either shape pays to end up with the same
  // bytes available.
  resetScrollback()
  const newBuffer = ms(() => {
    for (const c of data) appendScrollback('s', c)
    readScrollback('s')
  })

  resetScrollback()
  resetScreens()
  createScreen('s', COLS, ROWS)
  const newBufferAndScreen = ms(() => {
    for (const c of data) {
      appendScrollback('s', c)
      feedScreen('s', c)
    }
    readScrollback('s')
  })
  // The parse is queued, so the write is not finished when the loop is. Waiting
  // for it is the difference between measuring the work and measuring the
  // scheduling of it.
  const drained = process.hrtime.bigint()
  await serializeScreen('s')
  const drainMs = Number(process.hrtime.bigint() - drained) / 1e6

  process.stdout.write(
    JSON.stringify({
      chunks: CHUNKS,
      bytes: data.reduce((n, c) => n + c.length, 0),
      oldBufferMs: +oldBuffer.toFixed(1),
      newBufferMs: +newBuffer.toFixed(1),
      newBufferAndScreenMs: +(newBufferAndScreen + drainMs).toFixed(1),
      screenOnlyMs: +(newBufferAndScreen + drainMs - newBuffer).toFixed(1)
    }) + '\n'
  )
}

void main()
