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
/** Whether `simctl shutdown` should take its real, slow time to answer. */
const slowShutdown = { on: false }
/** Args of each exec call, in the order it *completed*. */
const execCompletions: string[][] = []
/** What `simctl list` returns. A test can swap this to change the fixture. */
const simctlList = {
  value: JSON.stringify({
    devices: {
      'com.apple.CoreSimulator.SimRuntime.iOS-26-2': [
        { udid: 'udid-1', name: 'iPhone 17', state: 'Shutdown', isAvailable: true },
        { udid: 'udid-2', name: 'iPad Pro', state: 'Booted', isAvailable: true }
      ]
    }
  })
}
vi.mock('node:child_process', () => ({
  execFile: (cmd: string, args: string[], cb: (e: unknown, out: unknown) => void) => {
    execCalls.push([cmd, ...args])
    // `promisify` hands the callback value straight through as the resolved
    // value, and callers destructure `{ stdout }` off it.
    const done = (): void => {
      // Completion order, not invocation order: what matters is when the
      // simulator actually goes down relative to the new boot, and a slow
      // shutdown is dispatched long before it lands.
      execCompletions.push(args)
      cb(null, { stdout: args.includes('list') ? simctlList.value : '' })
    }
    // `shutdown` is deliberately slow to answer. A synchronous mock makes every
    // ordering test vacuous — nothing can interleave — and the real hazard is
    // precisely that shutdown takes seconds while a new boot starts underneath
    // it. `slowShutdown` lets one test model that and stay honest.
    if (slowShutdown.on && args.includes('shutdown')) setTimeout(done, 20)
    else done()
  }
}))

const stopped: string[] = []
const treeJson = { value: JSON.stringify([{ role: 'AXWindow' }]) }
/**
 * A distinct handle per spawn, and the exit callback kept.
 *
 * Both matter for the reopen race: a shared handle object cannot express the
 * difference between a companion and its replacement, which is exactly the
 * distinction the registry has to draw when the old one's exit arrives late.
 */
const spawned: Array<{ handle: { client: object }; onExit: (u: string, h: unknown) => void }> = []
vi.mock('../src/main/device-companion', () => ({
  startCompanion: async (udid: string, onExit: (u: string, h: unknown) => void) => {
    const handle = { client: {}, udid }
    spawned.push({ handle, onExit })
    return handle
  },
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
  claim,
  openPane,
  setRendererSend,
  setEntryForTests,
  entryForTests,
  resetForTests,
  EDGE_BAND_POINTS,
  formatRuntime,
  keycodesFor,
  clampMaxEdge,
  type AXElement
} from '../src/main/device-registry'

beforeEach(() => {
  resetForTests()
  execCalls.length = 0
  stopped.length = 0
  spawned.length = 0
  slowShutdown.on = false
  execCompletions.length = 0
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

describe('runtime formatting', () => {
  it('keeps the version readable', () => {
    // "iOS 26 2" reads as a typo and cannot be matched against what Xcode
    // reports, which is the whole point of showing the runtime at all.
    expect(formatRuntime('com.apple.CoreSimulator.SimRuntime.iOS-26-2')).toBe('iOS 26.2')
  })

  it('handles a runtime with no version suffix', () => {
    expect(formatRuntime('com.apple.CoreSimulator.SimRuntime.iOS')).toBe('iOS')
  })
})

describe('typing keycodes', () => {
  it('maps the characters it claims to support', () => {
    expect(keycodesFor('a')).toEqual([4])
    expect(keycodesFor('0')).toEqual([39])
    expect(keycodesFor('\n')).toEqual([40])
  })

  it('refuses a character it cannot type rather than typing another one', () => {
    // The failure this guards is silent: '@' previously mapped to the keycode
    // for '2', so typing an email address reported success and entered
    // something else. An error naming the character costs a turn; a wrongly
    // typed password costs far more.
    expect(() => keycodesFor('user@example.com')).toThrow(/@/)
  })

  it('refuses upper case rather than silently lower-casing it', () => {
    expect(() => keycodesFor('Hello')).toThrow(/H/)
  })
})

describe('screenshot edge clamping', () => {
  it('honours a sane request from the pane', () => {
    expect(clampMaxEdge(600)).toBe(600)
  })

  it('caps a window dragged large, so the poll cannot balloon', () => {
    // maxEdge comes from a rect the person can drag as big as they like. Above
    // the raw capture the resize is skipped entirely and a ~2.9MB PNG crosses
    // IPC twice a second — the exact cost the downscale exists to avoid.
    expect(clampMaxEdge(9000)).toBe(2000)
  })

  it('falls back to the agent default when the pane has no box yet', () => {
    expect(clampMaxEdge(undefined)).toBe(1000)
  })

  it('ignores a nonsense edge rather than resizing to nothing', () => {
    // A zero or NaN reaches resize() as a 0-width image: a blank pane that
    // looks like the device died, with nothing naming the cause.
    expect(clampMaxEdge(0)).toBe(1000)
    expect(clampMaxEdge(Number.NaN)).toBe(1000)
    expect(clampMaxEdge(-50)).toBe(1000)
  })
})

describe('closing a device pane and reopening it straight away', () => {
  /**
   * `stopCompanion` drops its handle and sends SIGTERM, but the child can take
   * seconds to die. Reopen the same device inside that window and a second
   * companion is already running when the first one's exit finally lands — so
   * the registry has to tell them apart. Keyed on udid alone, the corpse's exit
   * marks the *live* entry unattached, and the freshly opened pane sits on
   * "the connection dropped" forever while a perfectly healthy companion runs
   * on, orphaned. Closing and reopening again is the only escape, and only if
   * you happen to do it slowly.
   */
  it('ignores a dead companion’s exit once its replacement is running', async () => {
    await claim({ sessionId: 's1', udid: 'udid-1' })
    const first = spawned[0]
    void release({ sessionId: 's1' })
    await claim({ sessionId: 's1', udid: 'udid-1' })
    expect(spawned).toHaveLength(2)

    // The first companion finally dies, long after it was replaced.
    first.onExit('udid-1', first.handle)

    const entry = entryForTests('s1')
    expect(entry?.companion).toBe(spawned[1].handle)
  })

  it('still marks the entry unattached when its own companion dies', async () => {
    // The identity check must not become a way of ignoring real drops: a
    // companion killed from outside has to leave the entry detached, or the
    // next call hangs on a dead socket instead of saying so.
    await claim({ sessionId: 's1', udid: 'udid-1' })
    const live = spawned[0]
    live.onExit('udid-1', live.handle)
    expect(entryForTests('s1')?.companion).toBeNull()
  })

  it('forgets every ref when the connection really drops', async () => {
    // Refs are coordinates that were correct at read time. Surviving a
    // reconnect they would tap whatever has since animated into that frame.
    await claim({ sessionId: 's1', udid: 'udid-1' })
    const entry = entryForTests('s1')!
    entry.refs.set('g0_el_1', { x: 10, y: 10, label: 'Settings' })
    const before = entry.generation
    spawned[0].onExit('udid-1', spawned[0].handle)
    expect(entry.refs.size).toBe(0)
    expect(entry.generation).toBeGreaterThan(before)
  })

  it('does not shut down the simulator the reopened pane just booted', async () => {
    // Release runs `simctl shutdown` for a Vorn-booted device and claim runs
    // `boot` — both slow. Unordered, the outgoing shutdown lands after the new
    // boot and takes down the simulator the pane is already showing, which
    // reads as the device dying for no reason at all.
    await claim({ sessionId: 's1', udid: 'udid-1' })
    // The shutdown takes its real, slow time. Without this nothing can
    // interleave and the assertion holds with or without the lock.
    slowShutdown.on = true
    const pending = release({ sessionId: 's1' })
    await claim({ sessionId: 's1', udid: 'udid-1' })
    await pending

    const order = execCompletions
      .filter((c) => c.includes('shutdown') || c.includes('boot'))
      .map((c) => (c.includes('shutdown') ? 'shutdown' : 'boot'))
    expect(order[order.length - 1]).toBe('boot')
  })
})
