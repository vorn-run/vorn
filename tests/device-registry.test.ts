import { describe, it, expect, vi, beforeEach } from 'vitest'

/**
 * The device registry's ways of being quietly wrong.
 *
 * Every case below is one where the honest failure and the silent one look
 * identical from the agent's side: a stale ref taps whatever animated into that
 * frame, a shared device produces interleaved actions that read as app bugs, a
 * bezel swipe is swallowed by iOS and reported as a successful swipe, and a
 * coordinate read off a screenshot lands at a third of the intended position on
 * a 3× screen. Each one gets a test that fails loudly when the guard goes.
 */

const execCalls: string[][] = []
vi.mock('node:child_process', () => ({
  execFile: (cmd: string, args: string[], cb: (e: unknown, out: string) => void) => {
    execCalls.push([cmd, ...args])
    cb(null, '')
  }
}))

const stopped: string[] = []
const treeJson = { value: JSON.stringify([{ role: 'AXWindow' }]) }
vi.mock('../src/main/device-companion', () => ({
  startCompanion: async () => ({ client: {} }),
  stopCompanion: (udid: string) => stopped.push(udid),
  call: async () => ({ json: treeJson.value }),
  callStreaming: async () => ({})
}))

vi.mock('electron', () => ({ nativeImage: { createFromBuffer: () => ({}) } }))

import {
  newEntry,
  claimFor,
  tapPointFor,
  parseCursor,
  toElement,
  matchElements,
  elementAtPoint,
  inkBounds,
  elementsUnderInk,
  frameIntersects,
  inEdgeBand,
  pointsToPixels,
  pixelsToPoints,
  interact,
  release,
  openPane,
  setRendererSend,
  setEntryForTests,
  entryForTests,
  resetForTests,
  EDGE_BAND_POINTS,
  type AXElement
} from '../src/main/device-registry'

beforeEach(() => {
  resetForTests()
  execCalls.length = 0
  stopped.length = 0
})

const button = (label: string, frame: AXElement['frame']): AXElement => ({
  role: 'AXButton',
  AXLabel: label,
  enabled: true,
  frame
})

describe('element handles', () => {
  it('refuses a ref minted against an earlier screen, naming both generations', () => {
    const entry = newEntry()
    toElement(button('Save', { x: 0, y: 0, width: 100, height: 40 }), entry)
    // The input that invalidates it: mobile has no navigation event, so the
    // generation bump is the only thing that knows the screen moved.
    entry.generation = 5
    entry.refs.clear()
    expect(() => tapPointFor('g1_el_1', entry)).toThrow(/earlier screen \(g1, now g5\)/)
  })

  it('resolves a live ref to the centre of its frame', () => {
    const entry = newEntry()
    const el = toElement(button('Save', { x: 10, y: 20, width: 100, height: 40 }), entry)
    expect(el?.ref).toBe('g1_el_1')
    expect(tapPointFor(el!.ref!, entry)).toEqual({ x: 60, y: 40 })
  })

  it('does not mint a handle for a disabled control', () => {
    const entry = newEntry()
    const el = toElement(
      { ...button('Save', { x: 0, y: 0, width: 10, height: 10 }), enabled: false },
      entry
    )
    expect(el?.ref).toBeUndefined()
    expect(el?.disabled).toBe(true)
  })

  it('drops a cursor from a screen that has since changed', () => {
    expect(parseCursor('1:120', 1)).toBe(120)
    expect(parseCursor('1:120', 2)).toBe(0)
  })
})

describe('ownership', () => {
  const held = new Map([['sess-a', { udid: 'udid-1', sessionId: 'sess-a' }]])

  it('refuses a contested device by name and offers the free ones', () => {
    const d = claimFor('udid-1', 'sess-b', held, ['iPhone 17 (udid-2)'])
    expect(d.ok).toBe(false)
    if (d.ok) throw new Error('unreachable')
    expect(d.error).toContain('sess-a')
    expect(d.error).toContain('udid-2')
  })

  it('lets the holder re-claim its own device', () => {
    expect(claimFor('udid-1', 'sess-a', held)).toEqual({ ok: true, alreadyMine: true })
  })

  it('allows an unheld device', () => {
    expect(claimFor('udid-9', 'sess-b', held)).toEqual({ ok: true, alreadyMine: false })
  })

  it('shuts down only a simulator Vorn booted', async () => {
    const mine = newEntry('s1', 'udid-mine')
    mine.bootedByVorn = true
    setEntryForTests(mine)
    await release({ sessionId: 's1' })
    expect(execCalls.some((c) => c.includes('shutdown') && c.includes('udid-mine'))).toBe(true)

    execCalls.length = 0
    const theirs = newEntry('s2', 'udid-theirs')
    theirs.bootedByVorn = false
    setEntryForTests(theirs)
    await release({ sessionId: 's2' })
    // Shutting down a simulator the person booted is destructive and surprising.
    expect(execCalls.some((c) => c.includes('shutdown'))).toBe(false)
    expect(stopped).toEqual(['udid-mine', 'udid-theirs'])
    expect(entryForTests('s2')).toBeUndefined()
  })
})

describe('searching the whole tree', () => {
  it('finds a match past the read budget', () => {
    // PR #435's regression: `find` built on the paginated read came back "not
    // found" for anything past node 200 — on exactly the long screens that are
    // the reason to search instead of read.
    const children: AXElement[] = []
    for (let i = 0; i < 400; i++) {
      children.push(
        button(i === 350 ? 'Delete account' : `Row ${i}`, {
          x: 0,
          y: i * 40,
          width: 320,
          height: 40
        })
      )
    }
    const tree: AXElement = {
      role: 'AXWindow',
      frame: { x: 0, y: 0, width: 320, height: 16000 },
      children
    }
    const hits = matchElements(tree, newEntry(), 'delete account', 5)
    expect(hits).toHaveLength(1)
    expect(hits[0].label).toBe('Delete account')
  })

  it('bounds what comes back, not what is searched', () => {
    const children = Array.from({ length: 50 }, (_, i) =>
      button(`Row ${i}`, { x: 0, y: i * 40, width: 320, height: 40 })
    )
    expect(matchElements({ role: 'AXWindow', children }, newEntry(), 'row', 3)).toHaveLength(3)
  })
})

describe('pointing and drawing', () => {
  // An iOS tree nests a button inside a cell inside a table inside a window —
  // all four contain the point, and only the smallest is what was pointed at.
  const nested: AXElement = {
    role: 'AXWindow',
    AXLabel: 'Settings',
    frame: { x: 0, y: 0, width: 402, height: 874 },
    children: [
      {
        role: 'AXCell',
        AXLabel: 'General row',
        enabled: true,
        frame: { x: 0, y: 100, width: 402, height: 60 },
        children: [button('General', { x: 16, y: 110, width: 80, height: 40 })]
      },
      button('Wi-Fi', { x: 16, y: 300, width: 80, height: 40 })
    ]
  }

  it('picks the smallest element containing the point, not the outermost', () => {
    const el = elementAtPoint(nested, newEntry(), { x: 40, y: 130 })
    expect(el?.label).toBe('General')
  })

  it('describes nothing when the point is off every frame', () => {
    expect(elementAtPoint(nested, newEntry(), { x: 4000, y: 4000 })).toBeNull()
  })

  it('boxes the ink and returns what it overlaps, smallest first', () => {
    // A circle drawn round a button clips it rather than containing it, so
    // overlap is the test; demanding containment would return nothing for the
    // most natural gesture there is.
    const bounds = inkBounds([
      {
        points: [
          { x: 30, y: 120 },
          { x: 200, y: 140 }
        ]
      }
    ])
    expect(bounds).toEqual({ x: 30, y: 120, width: 170, height: 20 })
    const under = elementsUnderInk(nested, newEntry(), bounds!, 10)
    // The button is clipped by the ink, not contained in it — the usual result
    // of circling something — and it is still the first thing marked.
    expect(under[0].label).toBe('General')
    expect(under.map((e) => e.label)).not.toContain('Wi-Fi')
  })

  it('has no box for ink with no points', () => {
    expect(inkBounds([])).toBeNull()
    expect(frameIntersects({ role: 'AXButton' }, { x: 0, y: 0, width: 1, height: 1 })).toBe(false)
  })
})

describe('geometry', () => {
  const screen = { width: 402, height: 874 }

  it('rejects a stroke starting in the system-gesture band', () => {
    expect(inEdgeBand({ x: 5, y: 400 }, screen)).toBe(true)
    expect(inEdgeBand({ x: 200, y: 870 }, screen)).toBe(true)
    expect(inEdgeBand({ x: 200, y: 400 }, screen)).toBe(false)
    expect(EDGE_BAND_POINTS).toBe(20)
  })

  it('converts a screenshot coordinate to the right tap point at 3×', () => {
    // The silent mis-tap: handing main a pixel coordinate would put the touch
    // at a third of the intended position.
    expect(pixelsToPoints({ x: 300, y: 900 }, 3)).toEqual({ x: 100, y: 300 })
    expect(pointsToPixels({ x: 100, y: 300 }, 3)).toEqual({ x: 300, y: 900 })
  })
})

describe('asking the renderer for a pane', () => {
  /** A claimed device, which `openPane` requires before it will show one. */
  function claimedFor(sessionId: string): void {
    const e = newEntry(sessionId, 'udid-1')
    e.companion = {} as never
    setEntryForTests(e)
  }

  it('refuses instead of reporting success when main never wired the renderer', async () => {
    // The live bug: main wired the browser registry and not this one, so the
    // default send was a no-op — `openPane` returned `{ udid }` and no pane
    // ever appeared, with nothing in the reply to say why.
    claimedFor('s1')
    await expect(openPane({ sessionId: 's1' })).rejects.toThrow(/never wired to the renderer/)
  })

  it('tells the renderer which device to show once wired', async () => {
    const sent: Array<[string, unknown]> = []
    setRendererSend((channel, params) => sent.push([channel, params]))
    claimedFor('s1')
    await expect(openPane({ sessionId: 's1' })).resolves.toEqual({ udid: 'udid-1' })
    expect(sent).toHaveLength(1)
    expect(sent[0][1]).toMatchObject({ sessionId: 's1', udid: 'udid-1' })
  })
})

describe('the swipe guard fails closed', () => {
  /** A claimed device whose companion answers, so `interact` runs for real. */
  function claimed(): void {
    const e = newEntry('s1', 'udid-1')
    e.companion = {} as never
    setEntryForTests(e)
  }

  beforeEach(() => {
    treeJson.value = JSON.stringify([
      { role: 'AXWindow', frame: { x: 0, y: 0, width: 402, height: 874 } }
    ])
  })

  it('refuses a bezel swipe once the screen size is known', async () => {
    claimed()
    await expect(
      interact({
        sessionId: 's1',
        action: 'swipe',
        target: { x: 5, y: 400 },
        to: { x: 300, y: 400 }
      })
    ).rejects.toThrow(/within 20pt of the bezel/)
  })

  it('refuses rather than passes when the screen size cannot be read', async () => {
    // Verified live: a read taken while the device is mid-transition comes back
    // with no root frame. A guard that opts out when it cannot measure lets the
    // first swipe after a navigating tap through — and iOS reports a swallowed
    // system gesture as a perfectly successful swipe.
    treeJson.value = JSON.stringify([{ role: 'AXWindow' }])
    claimed()
    await expect(
      interact({
        sessionId: 's1',
        action: 'swipe',
        target: { x: 5, y: 400 },
        to: { x: 300, y: 400 }
      })
    ).rejects.toThrow(/screen size could not be read/)
  })

  it('lets a swipe well inside the screen through', async () => {
    claimed()
    await expect(
      interact({
        sessionId: 's1',
        action: 'swipe',
        target: { x: 200, y: 400 },
        to: { x: 200, y: 100 }
      })
    ).resolves.toMatchObject({ ok: true })
  })

  it('honours an explicit system gesture, even unmeasured', async () => {
    treeJson.value = JSON.stringify([{ role: 'AXWindow' }])
    claimed()
    await expect(
      interact({
        sessionId: 's1',
        action: 'swipe',
        target: { x: 5, y: 400 },
        to: { x: 300, y: 400 },
        systemGesture: true
      })
    ).resolves.toMatchObject({ ok: true })
  })
})
