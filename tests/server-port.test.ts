import { describe, it, expect } from 'vitest'
import { resolveServerPort, shouldRememberPort } from '../packages/server/src/server-args'
import { DEFAULT_SERVER_PORT } from '../packages/shared/src/protocol'

/**
 * Which port the server asks for, and whether it writes the answer down.
 *
 * This decision lived inline in `startServer`, between a database init and a
 * websocket registration, where nothing could reach it — and it was wrong for
 * as long as it was there. The direct-run entry point passed `port ?? 0`, so
 * "no flag given" arrived as an explicit zero, and `0 ?? remembered` is zero
 * because `??` only falls through on null and undefined. Every launch drew a
 * fresh ephemeral port, wrote it to the configuration, and never read it back.
 *
 * The cost was not cosmetic. A browser keys `localStorage` by origin, so a
 * moving port leaves the web client's token behind at the old one; a phone
 * paired to `ws://host:port/ws` loses the pairing the same way.
 */
describe('resolveServerPort', () => {
  it('takes the remembered port when no flag was given', () => {
    // The regression the extraction exists for. Against the old code this is
    // where `undefined` became `0` and the remembered value stopped mattering.
    expect(
      resolveServerPort({ explicit: undefined, remembered: 50091, fallback: DEFAULT_SERVER_PORT })
    ).toBe(50091)
  })

  it('never resolves to an ephemeral port while it has anything else to try', () => {
    // Stated separately from the case above because zero is the specific wrong
    // answer: it is what the OS reads as "any port", and it is what the bug
    // produced on every launch.
    for (const remembered of [undefined, 50091]) {
      expect(
        resolveServerPort({ explicit: undefined, remembered, fallback: DEFAULT_SERVER_PORT })
      ).not.toBe(0)
    }
  })

  it('lets an explicit port win over a remembered one', () => {
    expect(resolveServerPort({ explicit: 51234, remembered: 50091, fallback: 50091 })).toBe(51234)
  })

  it('falls to the default on an install that remembers nothing', () => {
    expect(
      resolveServerPort({
        explicit: undefined,
        remembered: undefined,
        fallback: DEFAULT_SERVER_PORT
      })
    ).toBe(DEFAULT_SERVER_PORT)
  })

  it('treats a remembered zero as a port, since that is what it would have written', () => {
    // Not hypothetical: the bug wrote `actualPort`, and an install that ran the
    // broken build carries whatever it last bound. Zero itself was never stored
    // — `listen(0)` reports the real port — so this only pins that the function
    // has no special case pretending otherwise.
    expect(resolveServerPort({ remembered: 0, fallback: DEFAULT_SERVER_PORT })).toBe(0)
  })
})

describe('a null where a number was expected', () => {
  /**
   * Defaults come back through `JSON.parse(row.value)`, so a stored JSON null
   * arrives as `null` and would satisfy a strict `!== undefined`. The `??` chain
   * this logic replaced tolerated it; a strict check would have narrowed that
   * silently and handed `listen()` a null port, which it reads as "any port" —
   * an ephemeral one produced by a value that meant "nothing set".
   */
  const nothing = null as unknown as undefined

  it('reads a null remembered port as nothing remembered', () => {
    expect(resolveServerPort({ remembered: nothing, fallback: DEFAULT_SERVER_PORT })).toBe(
      DEFAULT_SERVER_PORT
    )
  })

  it('reads a null explicit port as no flag given', () => {
    expect(
      resolveServerPort({ explicit: nothing, remembered: 50091, fallback: DEFAULT_SERVER_PORT })
    ).toBe(50091)
  })

  it('still lets an explicit zero through, which is not nothing', () => {
    // `--port 0` asks for an ephemeral port on purpose. Loosening the check must
    // not sweep it up with the empty values.
    expect(resolveServerPort({ explicit: 0, remembered: 50091, fallback: 50091 })).toBe(0)
    expect(shouldRememberPort({ explicit: 0, fellBack: false })).toBe(false)
  })

  it('settles a first run whose remembered port is null and whose default is taken', () => {
    expect(shouldRememberPort({ remembered: nothing, fellBack: true })).toBe(true)
  })
})

describe('shouldRememberPort', () => {
  it('writes the port it asked for and got', () => {
    expect(shouldRememberPort({ remembered: 50091, fellBack: false })).toBe(true)
  })

  it('never writes an explicit --port', () => {
    // An instruction for one launch, not a new preference. Writing it back would
    // let a single test run quietly repoint every later launch — and the dev
    // override exists exactly so it can differ from the stored value rather than
    // redefine it.
    expect(shouldRememberPort({ explicit: 51234, remembered: 50091, fellBack: false })).toBe(false)
    expect(shouldRememberPort({ explicit: 51234, fellBack: true })).toBe(false)
  })

  it('does not let a fallback overwrite a port the install already had', () => {
    // The case is a dev server started beside the packaged app on one data
    // directory. The dev server loses the race, and writing what it got instead
    // would move the packaged app on its next launch — one instance corrupting
    // the other through the configuration they share.
    expect(shouldRememberPort({ remembered: 50091, fellBack: true })).toBe(false)
  })

  it('does write a fallback when there was nothing to protect', () => {
    // A first run whose default is squatted by something unrelated. Refusing
    // here would hand out a fresh random port on every launch forever, which is
    // the failure this whole mechanism exists to prevent.
    expect(shouldRememberPort({ remembered: undefined, fellBack: true })).toBe(true)
  })
})

describe('the default itself', () => {
  it('is a real port a client could be told in advance', () => {
    expect(Number.isInteger(DEFAULT_SERVER_PORT)).toBe(true)
    expect(DEFAULT_SERVER_PORT).toBeGreaterThan(1023)
    expect(DEFAULT_SERVER_PORT).toBeLessThan(65536)
  })
})
