import { describe, it, expect, vi, beforeEach, afterAll } from 'vitest'

/**
 * Borrowing Apple's faceplate, and never depending on it.
 *
 * The artwork belongs to Xcode and is read from the machine at runtime, so
 * every step of the lookup is somewhere it can be absent: no Xcode, a device
 * type Apple has not shipped a body for, a bundle whose manifest has moved on,
 * a `sips` that will not rasterize. Each one has to end in "draw the plain
 * frame instead" rather than in a pane that fails to render a device someone
 * is working against.
 */

const files = new Map<string, string>()
const execCalls: string[][] = []
const execFails = { on: false }

vi.mock('node:fs', () => {
  const api = {
    existsSync: (p: string) => files.has(String(p)),
    readFileSync: (p: string) => {
      const found = files.get(String(p))
      if (found === undefined) throw new Error(`ENOENT ${p}`)
      return Buffer.from(found)
    },
    mkdirSync: () => undefined,
    mkdtempSync: (prefix: string) => `${prefix}test`,
    renameSync: (from: string, to: string) => {
      const data = files.get(String(from))
      if (data === undefined) throw new Error(`ENOENT ${from}`)
      files.delete(String(from))
      files.set(String(to), data)
    },
    rmSync: () => undefined
  }
  return { default: api, ...api }
})

vi.mock('node:child_process', () => ({
  execFile: (cmd: string, args: string[], cb: (err: unknown, out: unknown) => void): void => {
    execCalls.push([cmd, ...args])
    if (execFails.on) {
      cb(new Error(`${cmd} failed`), null)
      return
    }
    if (cmd === 'xcrun') {
      // `simctl` knows where each device type really lives. The identifier
      // cannot be turned back into that path: it drops the punctuation, so
      // `iPad Air 11-inch (M4)` comes back as `iPad-Air-11-inch-M4`.
      cb(null, {
        stdout: JSON.stringify({
          devicetypes: [
            { identifier: TYPE, bundlePath: BUNDLE_PATH },
            { identifier: IPAD_TYPE, bundlePath: IPAD_BUNDLE_PATH }
          ]
        })
      })
      return
    }
    if (cmd === 'plutil') {
      cb(null, { stdout: 'com.apple.dt.devicekit.chrome.phone11\n' })
      return
    }
    if (cmd === 'sips' && !args.includes('--out')) {
      // The natural size query: a corner as Apple drew it.
      cb(null, { stdout: '  pixelWidth: 110.000\n  pixelHeight: 110.000\n' })
      return
    }
    // `sips` writes the PNG; the next read finds it because the test put it there.
    const out = args[args.indexOf('--out') + 1]
    files.set(out, 'rendered-png-bytes')
    cb(null, { stdout: '' })
  }
}))

vi.mock('../src/main/logger', () => ({
  default: { debug: () => {}, warn: () => {}, info: () => {} }
}))

const { chromeFor, resetChromeCache } = await import('../src/main/device-chrome')

// All of this is macOS-only, and CI is not a Mac: without saying so every case
// would pass for the wrong reason, by taking the platform guard's early exit.
const realPlatform = process.platform
function pretendPlatform(value: string): void {
  Object.defineProperty(process, 'platform', { value, configurable: true })
}
afterAll(() => pretendPlatform(realPlatform))

const BUNDLE_PATH =
  '/Library/Developer/CoreSimulator/Profiles/DeviceTypes/iPhone 18 Pro.simdevicetype'
const PROFILE = `${BUNDLE_PATH}/Contents/Resources/profile.plist`
const BUNDLE = '/Library/Developer/DeviceKit/Chrome/phone11.devicechrome/Contents/Resources'
const TYPE = 'com.apple.CoreSimulator.SimDeviceType.iPhone-18-Pro'
/** A name the identifier cannot be turned back into, which is the point. */
const IPAD_BUNDLE_PATH =
  '/Library/Developer/CoreSimulator/Profiles/DeviceTypes/iPad Air 11-inch (M4).simdevicetype'
const IPAD_PROFILE = `${IPAD_BUNDLE_PATH}/Contents/Resources/profile.plist`
const IPAD_TYPE = 'com.apple.CoreSimulator.SimDeviceType.iPad-Air-11-inch-M4'

/** The real shape of a chrome manifest, trimmed to what is read. */
const MANIFEST = JSON.stringify({
  identifier: 'com.apple.dt.devicekit.chrome.phone11',
  images: {
    topLeft: 'Phone TL',
    top: 'Phone Top',
    topRight: 'Phone TR',
    right: 'Phone Right',
    bottomRight: 'Phone BR',
    bottom: 'Phone Base',
    bottomLeft: 'Phone BL',
    left: 'Phone Left',
    sizing: { leftWidth: 18, rightWidth: 18, topHeight: 18, bottomHeight: 22 }
  },
  paths: { simpleOutsideBorder: { cornerRadiusX: 80, cornerRadiusY: 80 } },
  inputs: [
    { name: 'action', image: 'Mute BTN', anchor: 'left', offsets: { normal: { x: 8, y: 160 } } },
    {
      name: 'power',
      image: 'X_Power BTN',
      anchor: 'right',
      offsets: { normal: { x: -8, y: 262 } }
    },
    // A rail across the top, which some devices have: the two offsets swap.
    { name: 'volume-up', image: 'Vol BTN', anchor: 'top', offsets: { normal: { x: 316, y: 7 } } }
  ]
})

/** The same bundle, as the ones with placeholder slices actually ship. */
const MANIFEST_WITH_COMPOSITE = JSON.stringify({
  ...JSON.parse(MANIFEST),
  images: { ...JSON.parse(MANIFEST).images, composite: 'PhoneComposite' }
})

function installBundle(): void {
  files.set(PROFILE, 'binary plist')
  files.set(`${BUNDLE}/chrome.json`, MANIFEST)
  for (const art of [
    'Phone TL',
    'Phone Top',
    'Phone TR',
    'Phone Right',
    'Phone BR',
    'Phone Base',
    'Phone BL',
    'Phone Left',
    'Mute BTN',
    'X_Power BTN',
    'Vol BTN'
  ]) {
    files.set(`${BUNDLE}/${art}.pdf`, '%PDF')
  }
}

beforeEach(() => {
  pretendPlatform('darwin')
  files.clear()
  execCalls.length = 0
  execFails.on = false
  resetChromeCache()
})

describe('finding the faceplate', () => {
  it('follows the device type to the bundle Apple ships for it', async () => {
    installBundle()
    const chrome = await chromeFor(TYPE, '/data')
    expect(chrome?.id).toBe('phone11')
    // The thicknesses and the radius are the device's own, not a guess.
    expect(chrome?.inset).toEqual({ left: 18, right: 18, top: 18, bottom: 22 })
    expect(chrome?.cornerRadius).toBe(80)
    const pieces = Object.values(chrome!.images ?? {})
    expect(pieces.every((p) => p.url.startsWith('data:image/png;base64,'))).toBe(true)
    // The artwork's own size travels with it: a corner is 110 points of body,
    // and drawing it at the 18-point inset instead loses the whole curve.
    expect(pieces.every((p) => p.width > 0 && p.height > 0)).toBe(true)
  })

  it('places the buttons where the hardware has them', async () => {
    installBundle()
    const chrome = await chromeFor(TYPE, '/data')
    // Apple writes the right-hand offset as a negative number; the pane only
    // needs to know how far the button stands out from that edge.
    expect(chrome?.buttons).toEqual([
      expect.objectContaining({ name: 'action', side: 'left', out: 8, along: 160 }),
      expect.objectContaining({ name: 'power', side: 'right', out: 8, along: 262 }),
      expect.objectContaining({ name: 'volume-up', side: 'top', out: 7, along: 316 })
    ])
  })

  it('finds a device whose name the identifier cannot spell', async () => {
    // `iPad-Air-11-inch-M4` rebuilt as a directory name gives
    // "iPad Air 11 inch M4", which does not exist — so every iPad used to come
    // back with no artwork at all, looking exactly like a device Apple ships
    // no body for.
    installBundle()
    files.set(IPAD_PROFILE, 'binary plist')
    expect((await chromeFor(IPAD_TYPE, '/data'))?.id).toBe('phone11')
  })

  it('prefers one picture of the body over nine pieces of it', async () => {
    // Some bundles fill all nine slices with a red placeholder reading
    // "unused" and ship the real artwork as the composite — drawing the pieces
    // there wraps the device in a red slab.
    installBundle()
    files.set(`${BUNDLE}/chrome.json`, MANIFEST_WITH_COMPOSITE)
    files.set(`${BUNDLE}/PhoneComposite.pdf`, '%PDF')
    const chrome = await chromeFor(TYPE, '/data')
    expect(chrome?.composite?.url).toMatch(/^data:image\/png;base64,/)
    expect(chrome?.images).toBeNull()
  })

  it('renders each piece once, then remembers it', async () => {
    installBundle()
    await chromeFor(TYPE, '/data')
    const rasterized = execCalls.filter((c) => c[0] === 'sips' && c.includes('--out')).length
    // Eight pieces of body, plus the three buttons this device has.
    expect(rasterized).toBe(11)
    await chromeFor(TYPE, '/data')
    // Same device type, so nothing is rasterized or read a second time.
    expect(execCalls.filter((c) => c[0] === 'sips' && c.includes('--out'))).toHaveLength(rasterized)
  })

  it('says nothing rather than half a frame when a piece is missing', async () => {
    installBundle()
    files.delete(`${BUNDLE}/Phone Left.pdf`)
    // A body missing one edge reads as a bug in Vorn; a plain frame does not.
    expect(await chromeFor(TYPE, '/data')).toBeNull()
  })

  it('leaves every other platform alone', async () => {
    installBundle()
    pretendPlatform('linux')
    expect(await chromeFor(TYPE, '/data')).toBeNull()
  })

  it('falls back when the machine has no Xcode device profiles', async () => {
    expect(await chromeFor(TYPE, '/data')).toBeNull()
  })

  it('falls back when the artwork cannot be rasterized', async () => {
    installBundle()
    execFails.on = true
    expect(await chromeFor(TYPE, '/data')).toBeNull()
  })

  it('falls back on a manifest it cannot read', async () => {
    installBundle()
    files.set(`${BUNDLE}/chrome.json`, 'not json')
    expect(await chromeFor(TYPE, '/data')).toBeNull()
  })
})
