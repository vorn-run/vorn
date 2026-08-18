import { describe, it, expect, vi, beforeEach } from 'vitest'

/**
 * Clicking something you cannot see.
 *
 * `DOM.getBoxModel` reports layout-viewport coordinates, and for an element
 * below the fold it returns a perfectly plausible point rather than an error.
 * The click then dispatches at a coordinate outside the visible viewport,
 * Chromium hit-tests it against nothing, and the call still answers `ok` — the
 * failure mode this whole file exists for, because from the agent's side a
 * silent miss and a real click are the same two characters.
 *
 * These tests pin the ordering that fixes it: scroll first, measure second.
 * Measuring before the scroll would compute the pre-scroll position and click
 * the wrong place, which is indistinguishable from the bug being fixed.
 */

/** Every CDP method the registry sent, in order. */
let cdpCalls: { method: string; params?: Record<string, unknown> }[] = []
/** Whether `DOM.scrollIntoViewIfNeeded` should reject, as it does for a detached node. */
const scrollFails = { on: false }
/** The box the page reports, read at the moment `getBoxModel` is called. */
const boxAtMeasureTime = { value: [0, 0, 100, 0, 100, 40, 0, 40] }

vi.mock('electron', () => ({
  webContents: {
    fromId: () => ({
      isDestroyed: () => false,
      getURL: () => 'https://example.test/',
      getTitle: () => 'Example',
      debugger: {
        isAttached: () => false,
        attach: () => {},
        detach: () => {},
        on: () => {},
        off: () => {},
        removeListener: () => {},
        sendCommand: async (method: string, params?: Record<string, unknown>) => {
          cdpCalls.push({ method, params })
          if (method === 'DOM.scrollIntoViewIfNeeded') {
            if (scrollFails.on) throw new Error('Node does not have a layout object')
            // A real scroll moves the element into view, so the box model read
            // *after* it differs from the one before. Modelling that is the
            // only way a test can tell the two orderings apart.
            boxAtMeasureTime.value = [0, 300, 100, 300, 100, 340, 0, 340]
            return {}
          }
          if (method === 'DOM.getBoxModel') return { model: { content: boxAtMeasureTime.value } }
          if (method === 'Accessibility.getFullAXTree') {
            return {
              nodes: [
                {
                  nodeId: '1',
                  backendDOMNodeId: 42,
                  role: { value: 'button' },
                  name: { value: 'Emit warning' }
                }
              ]
            }
          }
          return {}
        }
      }
    })
  }
}))

import { attach, detach, readPage, interact, setRendererSend } from '../src/main/browser-registry'

/** Indices of the two calls whose order is the whole point. */
function order(): { scroll: number; measure: number } {
  return {
    scroll: cdpCalls.findIndex((c) => c.method === 'DOM.scrollIntoViewIfNeeded'),
    measure: cdpCalls.findIndex((c) => c.method === 'DOM.getBoxModel')
  }
}

const mouse = (): { method: string; params?: Record<string, unknown> }[] =>
  cdpCalls.filter((c) => c.method === 'Input.dispatchMouseEvent')

beforeEach(async () => {
  cdpCalls = []
  scrollFails.on = false
  boxAtMeasureTime.value = [0, 0, 100, 0, 100, 40, 0, 40]
  setRendererSend(() => {})
  detach('sess-scroll')
  attach('sess-scroll', 1)
  // Mint a real ref the way an agent does, rather than reaching into the entry.
  await readPage({ sessionId: 'sess-scroll' })
  cdpCalls = []
})

describe('a ref click brings its target into view', () => {
  it('scrolls before measuring, so the point describes where the element ended up', async () => {
    await interact({ sessionId: 'sess-scroll', action: 'click', target: { ref: 'g1_ref_1' } })

    const { scroll, measure } = order()
    expect(scroll).toBeGreaterThanOrEqual(0)
    expect(measure).toBeGreaterThan(scroll)
    expect(cdpCalls[scroll].params).toMatchObject({ backendNodeId: 42 })

    // The post-scroll box, not the stale one. Getting this backwards is the
    // original bug wearing the fix's clothes: the scroll happens, and the click
    // still lands where the element used to be.
    expect(mouse()[0].params).toMatchObject({ x: 50, y: 320 })
  })

  it('still clicks when the node cannot be scrolled to', async () => {
    // A failed scroll is not proof the element is unclickable — a fixed-position
    // node is already in view and has nothing to scroll. The box model below is
    // the real check, so this must not become a new way to refuse a good click.
    scrollFails.on = true

    await interact({ sessionId: 'sess-scroll', action: 'click', target: { ref: 'g1_ref_1' } })

    expect(order().measure).toBeGreaterThanOrEqual(0)
    expect(mouse()).toHaveLength(2) // press + release
  })

  it('leaves an explicit coordinate alone', async () => {
    // `{x, y}` means *that spot on the screen*. Scrolling first would move the
    // page out from under a caller who had already decided where to click.
    await interact({ sessionId: 'sess-scroll', action: 'click', target: { x: 12, y: 34 } })

    expect(order().scroll).toBe(-1)
    expect(mouse()[0].params).toMatchObject({ x: 12, y: 34 })
  })

  it('covers hover and typing too, which resolve through the same funnel', async () => {
    await interact({ sessionId: 'sess-scroll', action: 'hover', target: { ref: 'g1_ref_1' } })
    expect(order().scroll).toBeGreaterThanOrEqual(0)

    cdpCalls = []
    await interact({
      sessionId: 'sess-scroll',
      action: 'type',
      target: { ref: 'g1_ref_1' },
      text: 'hi'
    })
    expect(order().scroll).toBeGreaterThanOrEqual(0)
  })

  it('does not scroll for a stale ref, because there is nothing to scroll to', async () => {
    await expect(
      interact({ sessionId: 'sess-scroll', action: 'click', target: { ref: 'g1_ref_999' } })
    ).rejects.toThrow(/stale/)
    expect(cdpCalls).toHaveLength(0)
  })
})
