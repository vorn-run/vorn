import fs from 'node:fs'
import path from 'node:path'
import { execFile } from 'node:child_process'
import { promisify } from 'node:util'
import os from 'node:os'
import type { DeviceChrome, DeviceChromeButton, DeviceChromePiece } from '../shared/types'
import log from './logger'

const exec = promisify(execFile)

/**
 * The faceplate Apple already draws for each simulator.
 *
 * Xcode's Device Hub does not draw a rounded rectangle and hope: every
 * simulator's `.simdevicetype` names a *chrome bundle*, and each bundle
 * (`/Library/Developer/DeviceKit/Chrome/<name>.devicechrome`) carries the nine
 * pieces of the real device's body as vector art, the thickness of each side,
 * and the corner radius. It is the same artwork the person sees in Simulator
 * and in Device Hub, so borrowing it is how the pane stops looking like an
 * approximation of their phone and starts looking like their phone.
 *
 * Nothing is copied into Vorn. The art is Apple's, it is already on the
 * machine — a simulator cannot run without it — and it is read at runtime the
 * same way `simctl` is shelled out to. What is cached is a rendering of it
 * under this app's own data directory, because the originals are PDFs and no
 * browser draws a PDF as an image.
 *
 * Every step degrades: no Xcode, an unreadable bundle, a `sips` that will not
 * convert — each returns null, and the pane falls back to the frame it draws
 * itself. A missing faceplate must never cost anyone their device pane.
 */

const CHROME_BUNDLES = '/Library/Developer/DeviceKit/Chrome'

/** The bits of `chrome.json` this uses. The file carries a good deal more. */
interface ChromeManifest {
  identifier?: string
  images?: {
    topLeft?: string
    top?: string
    topRight?: string
    right?: string
    bottomRight?: string
    bottom?: string
    bottomLeft?: string
    left?: string
    sizing?: { leftWidth?: number; rightWidth?: number; topHeight?: number; bottomHeight?: number }
  }
  paths?: { simpleOutsideBorder?: { cornerRadiusX?: number; cornerRadiusY?: number } }
  inputs?: Array<{
    name?: string
    image?: string
    anchor?: string
    offsets?: { normal?: { x?: number; y?: number } }
  }>
}

/** Resolved chrome, keyed by device type, for as long as the app runs. */
const cache = new Map<string, DeviceChrome | null>()

/**
 * Where CoreSimulator keeps this device type's profile.
 *
 * Asked for rather than rebuilt from the identifier. The identifier spells a
 * name with hyphens and drops its punctuation — `iPad Air 11-inch (M4)`
 * becomes `iPad-Air-11-inch-M4` — so reconstructing a directory name from it
 * finds every iPhone and no iPad, which looks exactly like an iPad having no
 * artwork. `simctl` knows the real path.
 */
async function profileFor(deviceTypeIdentifier: string): Promise<string | null> {
  try {
    const { stdout } = await exec('xcrun', ['simctl', 'list', 'devicetypes', '-j'])
    const parsed = JSON.parse(stdout) as {
      devicetypes?: Array<{ identifier?: string; bundlePath?: string }>
    }
    const bundle = parsed.devicetypes?.find(
      (t) => t.identifier === deviceTypeIdentifier
    )?.bundlePath
    if (!bundle) return null
    const profile = path.join(bundle, 'Contents/Resources/profile.plist')
    return fs.existsSync(profile) ? profile : null
  } catch {
    return null
  }
}

/**
 * The chrome bundle a device type asks for.
 *
 * `profile.plist` is binary, so it is read through `plutil` rather than parsed
 * here — the same tool CoreSimulator ships with.
 */
async function chromeIdFor(deviceTypeIdentifier: string): Promise<string | null> {
  const profile = await profileFor(deviceTypeIdentifier)
  if (!profile) return null
  try {
    const { stdout } = await exec('plutil', ['-extract', 'chromeIdentifier', 'raw', profile])
    // `com.apple.dt.devicekit.chrome.phone11` → `phone11`.
    return stdout.trim().split('.').pop() || null
  } catch {
    return null
  }
}

/** Where converted art is kept, so a device type is rendered once per machine. */
function cacheDirFor(userDataDir: string, chromeId: string): string {
  return path.join(userDataDir, 'device-chrome', chromeId)
}

/**
 * One piece of the body: the art, and the size it is drawn at.
 *
 * The size matters as much as the picture. A corner is 110 points of body —
 * the whole curve, plus the rim — and squeezing it into the 18-point inset the
 * screen sits behind turns the device into a plain black border, which is
 * exactly what it looked like before this carried its own dimensions.
 *
 * Rasterized well above its natural size, since the pane draws it at whatever
 * the zoom asks for and a corner rendered at 1× goes soft at actual size on a
 * retina display.
 */
async function pieceOf(
  bundleResources: string,
  cacheDir: string,
  name: string
): Promise<DeviceChromePiece | null> {
  const source = path.join(bundleResources, `${name}.pdf`)
  if (!fs.existsSync(source)) return null
  const rendered = path.join(cacheDir, `${name.replace(/[^\w.-]+/g, '_')}.png`)
  try {
    const size = await naturalSize(source)
    if (!size) return null
    if (!fs.existsSync(rendered)) {
      fs.mkdirSync(cacheDir, { recursive: true })
      // `sips` is the one PDF rasterizer every Mac has. It writes through a
      // temporary file of its own choosing in the destination directory, so two
      // conversions landing there at once collide — the second fails with
      // "Cannot to rename temporary file" and the faceplate is dropped for a
      // reason that has nothing to do with the artwork. Each conversion gets a
      // directory to itself, and the result is moved into place in one step, so
      // a half-written PNG is never found and reused as if it were finished.
      const staging = fs.mkdtempSync(path.join(cacheDir, '.render-'))
      const temporary = path.join(staging, 'out.png')
      try {
        await exec('sips', ['-s', 'format', 'png', '-Z', '900', source, '--out', temporary])
        fs.renameSync(temporary, rendered)
      } finally {
        fs.rmSync(staging, { recursive: true, force: true })
      }
    }
    return {
      url: `data:image/png;base64,${fs.readFileSync(rendered).toString('base64')}`,
      width: size.width,
      height: size.height
    }
  } catch (err) {
    log.debug({ err, source }, '[device-chrome] could not render a piece')
    return null
  }
}

/** A PDF's own size in points, which is the size Apple drew it to be used at. */
async function naturalSize(source: string): Promise<{ width: number; height: number } | null> {
  const { stdout } = await exec('sips', ['-g', 'pixelWidth', '-g', 'pixelHeight', source])
  const width = Number(/pixelWidth:\s*([\d.]+)/.exec(stdout)?.[1])
  const height = Number(/pixelHeight:\s*([\d.]+)/.exec(stdout)?.[1])
  return width > 0 && height > 0 ? { width, height } : null
}

/**
 * The faceplate for one device type, or null to draw our own.
 *
 * Only macOS has any of this, and only a machine with Xcode's simulator
 * support installed — which is every machine that can reach this code, since
 * the device pane needs a simulator. It is still checked rather than assumed.
 */
export async function chromeFor(
  deviceTypeIdentifier: string,
  userDataDir: string = path.join(os.homedir(), '.vorn')
): Promise<DeviceChrome | null> {
  const cached = cache.get(deviceTypeIdentifier)
  if (cached !== undefined) return cached

  const resolved = await load(deviceTypeIdentifier, userDataDir)
  cache.set(deviceTypeIdentifier, resolved)
  return resolved
}

async function load(
  deviceTypeIdentifier: string,
  userDataDir: string
): Promise<DeviceChrome | null> {
  if (process.platform !== 'darwin') return null
  const chromeId = await chromeIdFor(deviceTypeIdentifier)
  if (!chromeId) return null
  const resources = path.join(CHROME_BUNDLES, `${chromeId}.devicechrome/Contents/Resources`)
  const manifestPath = path.join(resources, 'chrome.json')
  if (!fs.existsSync(manifestPath)) return null

  let manifest: ChromeManifest
  try {
    manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8')) as ChromeManifest
  } catch (err) {
    log.debug({ err, manifestPath }, '[device-chrome] unreadable chrome manifest')
    return null
  }

  const images = manifest.images
  const sizing = images?.sizing
  // Every side of the body has to be there. A frame missing one edge is worse
  // than a plain one: it reads as a rendering bug in Vorn, not a missing asset.
  const pieces = {
    topLeft: images?.topLeft,
    top: images?.top,
    topRight: images?.topRight,
    right: images?.right,
    bottomRight: images?.bottomRight,
    bottom: images?.bottom,
    bottomLeft: images?.bottomLeft,
    left: images?.left
  }
  if (Object.values(pieces).some((name) => !name)) return null

  const cacheDir = cacheDirFor(userDataDir, chromeId)
  // One at a time. This runs once per device type in the life of an install,
  // and conversions that overlap are how the whole body came back empty.
  const entries: Array<[string, DeviceChromePiece | null]> = []
  for (const [slot, name] of Object.entries(pieces)) {
    entries.push([slot, await pieceOf(resources, cacheDir, name as string)])
  }
  if (entries.some(([, piece]) => !piece)) return null

  const radius = manifest.paths?.simpleOutsideBorder?.cornerRadiusX ?? 0
  return {
    id: chromeId,
    // The art's own units, which are the device's points: the pane multiplies
    // them by whatever scale it is drawing the screen at.
    inset: {
      left: sizing?.leftWidth ?? 18,
      right: sizing?.rightWidth ?? 18,
      top: sizing?.topHeight ?? 18,
      bottom: sizing?.bottomHeight ?? 18
    },
    cornerRadius: radius,
    images: Object.fromEntries(entries) as DeviceChrome['images'],
    buttons: await buttonsOf(manifest, resources, cacheDir)
  }
}

/**
 * The switches and buttons, where the hardware has them.
 *
 * They are what makes a frame read as a particular phone rather than a black
 * rectangle — and Apple has already measured them, so there is nothing to
 * guess. A button whose art will not render is simply left out.
 */
async function buttonsOf(
  manifest: ChromeManifest,
  resources: string,
  cacheDir: string
): Promise<DeviceChromeButton[]> {
  const found: DeviceChromeButton[] = []
  for (const input of manifest.inputs ?? []) {
    const side = input.anchor === 'left' ? 'left' : input.anchor === 'right' ? 'right' : null
    const offset = input.offsets?.normal
    if (!side || !input.image || typeof offset?.y !== 'number') continue
    const piece = await pieceOf(resources, cacheDir, input.image)
    if (!piece) continue
    found.push({
      name: input.name ?? 'button',
      url: piece.url,
      width: piece.width,
      height: piece.height,
      side,
      // Apple writes the outward offset as a negative number on the right.
      out: Math.abs(offset.x ?? 0),
      top: offset.y
    })
  }
  return found
}

/** Forget what was resolved, for a test or a changed Xcode. */
export function resetChromeCache(): void {
  cache.clear()
}
