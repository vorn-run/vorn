// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { CoreStatus, SessionHolders, VorndStatus } from '../src/shared/types'

let status: CoreStatus | Error | undefined
let daemon: VorndStatus | null
let holders: SessionHolders | null
const endSessionHolder = vi.fn()

const api: Record<string, unknown> = {
  getCoreStatus: () => (status instanceof Error ? Promise.reject(status) : Promise.resolve(status)),
  getVorndStatus: () => Promise.resolve(daemon),
  getSessionHolders: () => Promise.resolve(holders),
  endSessionHolder: (...a: unknown[]) => endSessionHolder(...a)
}
Object.defineProperty(window, 'api', { value: api, writable: true })

const { NativeCoreStatus } = await import('../src/renderer/components/settings/NativeCoreStatus')

function core(over: Partial<CoreStatus> = {}): CoreStatus {
  return { loaded: true, version: '0.2.0', error: null, missing: [], ...over }
}

beforeEach(() => {
  vi.clearAllMocks()
  status = core()
  daemon = { state: 'off' }
  holders = null
  endSessionHolder.mockResolvedValue({ ok: true })
  api.getVorndStatus = () => Promise.resolve(daemon)
  api.getSessionHolders = () => Promise.resolve(holders)
})

describe('NativeCoreStatus', () => {
  it('shows the native core it runs on', async () => {
    render(<NativeCoreStatus />)
    expect(await screen.findByText('Native core 0.2.0')).toBeInTheDocument()
  })

  it('says nothing about the core when the server cannot report on it', async () => {
    status = new Error('older server')
    render(<NativeCoreStatus />)
    await Promise.resolve()
    expect(screen.queryByText(/Native core/)).not.toBeInTheDocument()
  })

  describe('vornd', () => {
    it('has no switch of its own: every terminal runs in it', async () => {
      render(<NativeCoreStatus />)
      await screen.findByText('Native core 0.2.0')
      expect(screen.queryByRole('switch', { name: 'Native daemon' })).not.toBeInTheDocument()
    })

    it('says why terminals cannot run when it is not in use', async () => {
      daemon = { state: 'failed', detail: 'vornd is not in this build' }
      render(<NativeCoreStatus />)
      expect(
        await screen.findByText(
          'Terminals cannot run, because vornd, the native daemon, is not in use: vornd is not in this build.'
        )
      ).toBeInTheDocument()
    })

    it('says nothing about it while it is up, and asks for its session holders only then', async () => {
      const getHolders = vi.fn(() => Promise.resolve(holders))
      api.getSessionHolders = getHolders
      const { unmount } = render(<NativeCoreStatus />)
      await screen.findByText('Native core 0.2.0')
      expect(screen.queryByText(/Terminals cannot run/)).not.toBeInTheDocument()
      expect(getHolders).not.toHaveBeenCalled()
      unmount()

      daemon = { state: 'on', port: 47001 }
      render(<NativeCoreStatus />)
      await screen.findByText('Native core 0.2.0')
      expect(getHolders).toHaveBeenCalled()
    })

    describe('with older session holders', () => {
      const older = {
        pid: 4242,
        instance: '1a2b',
        build: '0.7.5',
        proto: 1,
        sessions: 2,
        compatible: true
      }
      beforeEach(() => {
        daemon = { state: 'on', port: 47001 }
      })

      it('says how many sessions are still on one and that it exits after them', async () => {
        holders = {
          current: { ...older, pid: 1, instance: '99', build: '0.8.0' },
          older: [older],
          error: null
        }
        render(<NativeCoreStatus />)
        expect(
          await screen.findByText(
            '2 sessions started before Vorn was updated are still on the older session holder (0.7.5). It exits after the last one ends.'
          )
        ).toBeInTheDocument()
      })

      it('says when this version cannot talk to one', async () => {
        holders = {
          current: null,
          older: [{ ...older, sessions: null, compatible: false }],
          error: null
        }
        render(<NativeCoreStatus />)
        expect(await screen.findByText(/this version cannot talk to/)).toBeInTheDocument()
      })

      it('ends one only after asking', async () => {
        holders = { current: null, older: [{ ...older, sessions: 1 }], error: null }
        const confirm = vi
          .spyOn(window, 'confirm')
          .mockReturnValueOnce(false)
          .mockReturnValueOnce(true)
        render(<NativeCoreStatus />)
        fireEvent.click(await screen.findByRole('button', { name: 'End them' }))
        expect(confirm).toHaveBeenCalledWith(
          'End 1 session on the older session holder? Their processes stop.'
        )
        expect(endSessionHolder).not.toHaveBeenCalled()
        fireEvent.click(screen.getByRole('button', { name: 'End them' }))
        expect(endSessionHolder).toHaveBeenCalledWith('1a2b')
        confirm.mockRestore()
      })

      it('says why when it could not end one', async () => {
        holders = { current: null, older: [older], error: null }
        endSessionHolder.mockResolvedValue({
          ok: false,
          detail: 'that session holder is no longer running'
        })
        const confirm = vi.spyOn(window, 'confirm').mockReturnValue(true)
        render(<NativeCoreStatus />)
        fireEvent.click(await screen.findByRole('button', { name: 'End them' }))
        expect(
          await screen.findByText('Could not end them: that session holder is no longer running.')
        ).toBeInTheDocument()
        confirm.mockRestore()
      })

      it('hides one with no sessions left, and says when none is running', async () => {
        holders = {
          current: null,
          older: [{ ...older, sessions: 0 }],
          error: 'sessiond did not start'
        }
        render(<NativeCoreStatus />)
        expect(
          await screen.findByText('The session holder is not running: sessiond did not start.')
        ).toBeInTheDocument()
        expect(screen.queryByRole('button', { name: 'End them' })).not.toBeInTheDocument()
      })
    })
  })
})
