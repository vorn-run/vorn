// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'

const writeTerminal = vi.fn()
const attachTerminal = vi.fn()
Object.defineProperty(window, 'api', {
  value: { writeTerminal, attachTerminal, notifyWidgetStatus: vi.fn(), openExternal: vi.fn() },
  writable: true
})

import { registerSlot, hydrateTerminal } from '../src/renderer/lib/terminal-registry'

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
