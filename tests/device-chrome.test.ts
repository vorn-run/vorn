import { describe, it, expect, vi, beforeEach } from 'vitest'

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

const PROFILE =
  '/Library/Developer/CoreSimulator/Profiles/DeviceTypes/iPhone 18 Pro.simdevicetype/Contents/Resources/profile.plist'
const BUNDLE = '/Library/Developer/DeviceKit/Chrome/phone11.devicechrome/Contents/Resources'
const TYPE = 'com.apple.CoreSimulator.SimDeviceType.iPhone-18-Pro'

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
    { name: 'power', image: 'X_Power BTN', anchor: 'right', offsets: { normal: { x: -8, y: 262 } } }
  ]
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
    'X_Power BTN'
  ]) {
    files.set(`${BUNDLE}/${art}.pdf`, '%PDF')
  }
}

beforeEach(() => {
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
    const pieces = Object.values(chrome!.images)
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
      expect.objectContaining({ name: 'action', side: 'left', out: 8, top: 160 }),
      expect.objectContaining({ name: 'power', side: 'right', out: 8, top: 262 })
    ])
  })

  it('renders each piece once, then remembers it', async () => {
    installBundle()
    await chromeFor(TYPE, '/data')
    const rasterized = execCalls.filter((c) => c[0] === 'sips' && c.includes('--out')).length
    // Eight pieces of body, plus the two buttons this device has.
    expect(rasterized).toBe(10)
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
