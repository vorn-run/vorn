import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

interface FakeWindow {
  destroyed: boolean
  load(): void
  webContents: { executeJavaScript: ReturnType<typeof vi.fn> }
}

const windows = vi.hoisted(() => [] as FakeWindow[])

vi.mock('electron', () => {
  class BrowserWindow {
    destroyed = false
    url = ''
    load = (): void => {}
    private closed: (() => void) | undefined
    webContents = {
      getURL: (): string => this.url,
      executeJavaScript: vi.fn(async () => ({ status: 200, headers: {}, body: '{}' }))
    }
    constructor() {
      windows.push(this as unknown as FakeWindow)
    }
    loadURL(url: string): Promise<void> {
      return new Promise((resolve) => {
        this.load = () => {
          this.url = url
          resolve()
        }
      })
    }
    on(event: string, fn: () => void): void {
      if (event === 'closed') this.closed = fn
    }
    isDestroyed(): boolean {
      return this.destroyed
    }
    destroy(): void {
      this.destroyed = true
      this.closed?.()
    }
  }
  const profile = {
    setUserAgent: vi.fn(),
    getUserAgent: () => 'Mozilla/5.0 Electron/41.0.0',
    setPermissionRequestHandler: vi.fn(),
    setPermissionCheckHandler: vi.fn(),
    on: vi.fn()
  }
  return {
    BrowserWindow,
    app: { getPath: () => '/tmp/vorn-test' },
    session: { fromPartition: () => profile }
  }
})

vi.mock('../src/main/logger', () => ({ default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() } }))

const { checkSession, closeConnectionWindows, fetchInSession } =
  await import('../src/main/connection-sessions')

const origins = ['https://substack.com']
const request = { url: 'https://substack.com/api/v1/user/profile/self', method: 'GET' }

beforeEach(() => {
  closeConnectionWindows()
  windows.length = 0
})

afterEach(() => {
  vi.useRealTimers()
})

describe('the hidden page a signed-in call runs in', () => {
  it('opens once for calls that arrive while it is still loading, and answers them all', async () => {
    const calls = Promise.all([
      fetchInSession('c1', origins, request),
      fetchInSession('c1', origins, request)
    ])
    await vi.waitFor(() => expect(windows).toHaveLength(1))
    windows[0]!.load()
    await expect(calls).resolves.toHaveLength(2)
    expect(windows).toHaveLength(1)
    expect(windows[0]!.destroyed).toBe(false)
  })

  it('stays open while a call is running, and closes once it has been idle', async () => {
    vi.useFakeTimers()
    const first = fetchInSession('c1', origins, request)
    await vi.waitFor(() => expect(windows).toHaveLength(1))
    windows[0]!.load()
    await first

    await vi.advanceTimersByTimeAsync(59_000)
    let finish: (answer: unknown) => void = () => {}
    windows[0]!.webContents.executeJavaScript.mockImplementationOnce(
      () => new Promise((resolve) => (finish = resolve))
    )
    const slow = fetchInSession('c1', origins, request)
    await vi.advanceTimersByTimeAsync(5_000)
    expect(windows[0]!.destroyed).toBe(false)

    finish({ status: 200, headers: {}, body: '' })
    await slow
    await vi.advanceTimersByTimeAsync(60_000)
    expect(windows[0]!.destroyed).toBe(true)
  })
})

describe('the check that says whether a connection is signed in', () => {
  it('sends the headers the connector declared, beside accept', async () => {
    const check = checkSession('c1', {
      signInUrl: 'https://substack.com/sign-in',
      origins,
      check: { url: request.url, identity: ['name'], headers: { 'X-CSRF-Protection': '1' } }
    })
    await vi.waitFor(() => expect(windows).toHaveLength(1))
    windows[0]!.load()
    await expect(check).resolves.toEqual({ signedIn: true, identity: null })
    const script = windows[0]!.webContents.executeJavaScript.mock.calls[0]![0] as string
    expect(script).toContain('"X-CSRF-Protection":"1"')
    expect(script).toContain('"accept":"application/json"')
  })
})
