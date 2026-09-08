import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

/**
 * Which window answers what is selected.
 *
 * Every window hears the request, including a browser one showing a different
 * project, and the first answer wins. A window that is not drawing the terminal
 * has nothing true to say about it, so it must say nothing rather than say ''.
 */

const mounted = new Set<string>()
const selections = new Map<string, string>()

vi.mock('../src/renderer/lib/terminal-registry', () => ({
  hasTerminal: (id: string) => mounted.has(id),
  getTerminalSelection: (id: string) => selections.get(id) ?? ''
}))

const { listenForSelectionRequests } = await import('../src/renderer/lib/extension-selection')

const sent: Array<{ requestId: number; text: string }> = []
let deliver: ((payload: { requestId: number; sessionId: string }) => void) | undefined

beforeEach(() => {
  mounted.clear()
  selections.clear()
  sent.length = 0
  deliver = undefined
  ;(globalThis as { window?: unknown }).window = {
    api: {
      onExtensionSelectionRequest: (
        handler: (p: { requestId: number; sessionId: string }) => void
      ) => {
        deliver = handler
        return () => {
          deliver = undefined
        }
      },
      sendExtensionSelection: (requestId: number, text: string) => sent.push({ requestId, text })
    }
  }
})

afterEach(() => {
  delete (globalThis as { window?: unknown }).window
})

describe('answering what is selected', () => {
  it('answers for a terminal this window draws', () => {
    mounted.add('s1')
    selections.set('s1', 'the highlighted text')
    const stop = listenForSelectionRequests()
    deliver?.({ requestId: 1, sessionId: 's1' })
    expect(sent).toEqual([{ requestId: 1, text: 'the highlighted text' }])
    stop()
  })

  it('says nothing for a terminal it never drew', () => {
    selections.set('s1', 'not this window')
    const stop = listenForSelectionRequests()
    deliver?.({ requestId: 2, sessionId: 's1' })
    expect(sent).toEqual([])
    stop()
  })

  it('answers an empty selection for a terminal it draws with nothing highlighted', () => {
    mounted.add('s1')
    const stop = listenForSelectionRequests()
    deliver?.({ requestId: 3, sessionId: 's1' })
    expect(sent).toEqual([{ requestId: 3, text: '' }])
    stop()
  })
})
