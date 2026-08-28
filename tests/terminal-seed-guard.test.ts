// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'

const writeTerminal = vi.fn()
const attachTerminal = vi.fn()
Object.defineProperty(window, 'api', {
  value: { writeTerminal, attachTerminal, notifyWidgetStatus: vi.fn(), openExternal: vi.fn() },
  writable: true
})

const seeded = vi.hoisted(() => [] as string[])
vi.mock('../src/renderer/lib/command-blocks', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../src/renderer/lib/command-blocks')>()),
  markSeededFromServer: (id: string) => seeded.push(id)
}))

import {
  registerSlot,
  hydrateTerminal,
  destroyTerminal
} from '../src/renderer/lib/terminal-registry'

/**
 * A replayed screen must not be answered.
 *
 * Recordings carry the questions the old program asked its terminal. Written
 * into a real emulator they are asked again and this one answers, and the answer
 * goes down the pty as though it had been typed.
 */
const ESC = '\x1b'

beforeEach(() => {
  vi.clearAllMocks()
  seeded.length = 0
})

describe('seeding a pane with a screen from the past', () => {
  it('does not answer the questions in it', async () => {
    // Device attributes, cursor position, background colour -- all things a
    // terminal replies to, all things a serialized screen can contain.
    attachTerminal.mockResolvedValue({
      data: `${ESC}[c${ESC}[6n${ESC}]11;?${ESC}\\`,
      seq: 0,
      live: true
    })
    registerSlot('seeded', document.createElement('div'))

    await hydrateTerminal('seeded')
    await new Promise((r) => setTimeout(r, 60))

    expect(writeTerminal.mock.calls).toEqual([])
  })
})

describe('a seed whose pane goes while the attach is in flight', () => {
  it('does not land on top of the pane that took the id after it', async () => {
    // A view swap, or a pane closed and reopened, destroys the terminal and
    // mounts a new one under the same id. The closure still holds the old entry,
    // so the stale seed would write into a disposed terminal -- which xterm
    // accepts silently -- and hand the new pane's status handler the old pane's
    // bytes. Mounting seeds on its own, so the live pane's own seed is expected;
    // what must not happen is a second one arriving from the pane that is gone.
    let answerTheDeadPane: (v: { data: string; seq: number; live: boolean }) => void = () => {}
    attachTerminal
      .mockReturnValueOnce(
        new Promise((resolve) => {
          answerTheDeadPane = resolve
        })
      )
      .mockResolvedValue({ data: 'what the live pane was given', seq: 0, live: true })

    registerSlot('swapped', document.createElement('div'))

    // The pane goes, and the id is taken again before the server answers.
    destroyTerminal('swapped')
    registerSlot('swapped', document.createElement('div'))
    await new Promise((r) => setTimeout(r, 30))

    answerTheDeadPane({ data: 'from the pane that is gone', seq: 0, live: true })
    await new Promise((r) => setTimeout(r, 30))

    expect(seeded).toEqual(['swapped'])
  })

  it('lands normally when the pane is still the one that asked', async () => {
    // The contrast, so the test above cannot pass by seeds never arriving.
    attachTerminal.mockResolvedValue({ data: 'a screen', seq: 0, live: true })
    registerSlot('kept', document.createElement('div'))

    await hydrateTerminal('kept')
    await new Promise((r) => setTimeout(r, 30))

    expect(seeded).toEqual(['kept'])
  })
})
