/**
 * Killing a session has to hand its device back.
 *
 * This is the one device path with no visible symptom when it breaks. The
 * session disappears from the UI either way; what is left behind is a live
 * `idb_companion` holding a unix socket and a claim that belongs to no session,
 * so the simulator stays locked out of every other session until the app quits
 * — and the only clue is a "held by <dead session>" message pointing at a
 * session that no longer exists.
 *
 * It also has to fire for a session that never opened a pane, because an agent
 * can claim a device headlessly. Wiring the release to pane teardown instead
 * would look correct in the UI and leak in exactly that case.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest'

const handlers = new Map<string, (...args: unknown[]) => unknown>()

vi.mock('electron', () => ({
  app: { on: vi.fn(), getPath: () => '/tmp' },
  dialog: {},
  shell: {},
  BrowserWindow: { getAllWindows: () => [] },
  ipcMain: {
    handle: (channel: string, fn: (...args: unknown[]) => unknown) => handlers.set(channel, fn),
    on: vi.fn()
  }
}))

const { releaseForSession } = vi.hoisted(() => ({ releaseForSession: vi.fn() }))
vi.mock('../src/main/device-registry', () => ({ releaseForSession }))
vi.mock('../src/main/browser-registry', () => ({}))
vi.mock('../src/main/credential-handlers', () => ({
  registerCredentialHandlers: vi.fn(),
  enrichPayloadWithCredentials: async (p: unknown) => p
}))
vi.mock('../src/main/logger', () => ({ default: { error: vi.fn(), info: vi.fn() } }))

import { registerIpcHandlers, setBridge } from '../src/main/ipc-handlers'
import { IPC } from '../src/shared/types'

const request = vi.fn().mockResolvedValue(undefined)

beforeEach(() => {
  handlers.clear()
  releaseForSession.mockClear()
  request.mockClear()
  setBridge({ request, handle: vi.fn() } as unknown as Parameters<typeof setBridge>[0])
  registerIpcHandlers()
})

describe('session teardown releases the device', () => {
  it('releases the claim when the session is killed', async () => {
    await handlers.get(IPC.TERMINAL_KILL)?.({}, 'sess-a')
    expect(releaseForSession).toHaveBeenCalledWith('sess-a')
  })

  it('still kills the terminal', async () => {
    // The release must not become a precondition for teardown: a session that
    // cannot be closed is worse than a leaked claim.
    await handlers.get(IPC.TERMINAL_KILL)?.({}, 'sess-a')
    expect(request).toHaveBeenCalledWith(IPC.TERMINAL_KILL, 'sess-a')
  })

  it('releases the session that was killed, not some other one', async () => {
    await handlers.get(IPC.TERMINAL_KILL)?.({}, 'sess-b')
    expect(releaseForSession).toHaveBeenCalledTimes(1)
    expect(releaseForSession).toHaveBeenCalledWith('sess-b')
  })
})
