import { describe, it, expect, vi, beforeEach, afterAll } from 'vitest'
import { mkdtempSync, mkdirSync, writeFileSync, rmSync, realpathSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { pathToFileURL } from 'node:url'

/**
 * Opening a local page from a standing start.
 *
 * Whether a session may read a file depends on its root, and the root arrives
 * with the pane. So the first `browser_navigate` to a file url is the awkward
 * one: judged before the pane exists, every in-root file looks like a scheme
 * the pane may not load — and the refusal talks about the address rather than
 * the timing, which sends anyone reading it looking in the wrong place.
 */

let cdpCalls: { method: string; params?: Record<string, unknown> }[] = []

vi.mock('electron', () => ({
  webContents: {
    fromId: () => ({
      isDestroyed: () => false,
      getURL: () => 'about:blank',
      getTitle: () => '',
      debugger: {
        isAttached: () => false,
        attach: () => {},
        detach: () => {},
        on: () => {},
        off: () => {},
        removeListener: () => {},
        sendCommand: async (method: string, params?: Record<string, unknown>) => {
          cdpCalls.push({ method, params })
          return {}
        }
      }
    })
  }
}))

import { attach, detach, navigate, setRendererSend } from '../src/main/browser-registry'
import { setFileRoot, resetFileRoots } from '../src/main/browser-file-scope'

// A real directory, because containment is decided by the filesystem.
const tmp = realpathSync(mkdtempSync(join(tmpdir(), 'vorn-cold-')))
const project = join(tmp, 'project')
mkdirSync(project)
writeFileSync(join(project, 'index.html'), '<h1>local</h1>')
const outside = join(tmp, 'outside')
mkdirSync(outside)
writeFileSync(join(outside, 'secret.txt'), 'no')

afterAll(() => rmSync(tmp, { recursive: true, force: true }))

const fileUrl = (p: string): string => pathToFileURL(p).href
const SESSION = 'sess-cold'

/**
 * The renderer's half: a pane appears when asked for, and reports the root the
 * way `BROWSER_ATTACH` does. Wired as a real callback rather than stubbed out,
 * because the ordering between "pane opens" and "root known" is the thing
 * under test.
 */
function paneOpensOnRequest(root?: string): void {
  setRendererSend((channel) => {
    if (!channel.endsWith('openPane')) return
    attach(SESSION, 1)
    setFileRoot(SESSION, root)
  })
}

const navigated = (): string | undefined =>
  cdpCalls.find((c) => c.method === 'Page.navigate')?.params?.url as string | undefined

beforeEach(() => {
  cdpCalls = []
  resetFileRoots()
  detach(SESSION)
  setRendererSend(() => {})
})

describe('the first navigation to a local file', () => {
  it('opens the pane before judging the url, so an in-root file loads', async () => {
    paneOpensOnRequest(project)
    const url = fileUrl(join(project, 'index.html'))

    const result = await navigate({ sessionId: SESSION, url })

    expect(result.url).toBe(url)
  })

  it('still refuses a file outside the root once the pane exists', async () => {
    // Opening the pane first must not become a way in: the root is known by
    // the time the url is judged, and it says no.
    paneOpensOnRequest(project)

    await expect(
      navigate({ sessionId: SESSION, url: fileUrl(join(outside, 'secret.txt')) })
    ).rejects.toThrow(/not an allowed web address/)
    expect(navigated()).toBeUndefined()
  })

  it('refuses a file url for a session that has no root at all', async () => {
    // A headless session, or a pane belonging to no project. Absent must mean
    // nothing, never everything — even though a pane was opened to ask.
    paneOpensOnRequest(undefined)

    await expect(
      navigate({ sessionId: SESSION, url: fileUrl(join(project, 'index.html')) })
    ).rejects.toThrow(/not an allowed web address/)
  })

  it('does not open a pane to evaluate a url that could never load', async () => {
    // `javascript:` is refused on shape alone. Opening a pane to find that out
    // would leave a blank pane on screen as the side effect of a refusal.
    let opened = 0
    setRendererSend(() => {
      opened++
    })

    await expect(navigate({ sessionId: SESSION, url: 'javascript:alert(1)' })).rejects.toThrow(
      /not an allowed web address/
    )
    expect(opened).toBe(0)
  })

  it('leaves http navigation on its existing path', async () => {
    // http needs no root, so it must not pay for the file-only detour.
    paneOpensOnRequest(project)

    const result = await navigate({ sessionId: SESSION, url: 'example.com' })

    expect(result.url).toBe('https://example.com/')
  })
})
