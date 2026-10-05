// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { AppConfig, CoreStatus, SessionHolders, VorndStatus } from '../src/shared/types'

const mockStore = {
  config: null as AppConfig | null,
  setConfig: vi.fn()
}

vi.mock('../src/renderer/stores', () => ({
  useAppStore: (selector?: (state: unknown) => unknown) =>
    selector ? selector(mockStore) : mockStore
}))

const saveConfig = vi.fn()
let status: CoreStatus | Error | undefined
let daemon: VorndStatus | null
let holders: SessionHolders | null
const endSessionHolder = vi.fn()

const api: Record<string, unknown> = {
  saveConfig: (...a: unknown[]) => saveConfig(...a),
  getCoreStatus: () => (status instanceof Error ? Promise.reject(status) : Promise.resolve(status)),
  getVorndStatus: () => Promise.resolve(daemon),
  getSessionHolders: () => Promise.resolve(holders),
  endSessionHolder: (...a: unknown[]) => endSessionHolder(...a)
}
Object.defineProperty(window, 'api', { value: api, writable: true })

const { ExperimentalSettings } =
  await import('../src/renderer/components/settings/ExperimentalSettings')

function config(experimental?: AppConfig['defaults']['experimental']): AppConfig {
  return { defaults: { experimental } } as unknown as AppConfig
}

function core(over: Partial<CoreStatus> = {}): CoreStatus {
  return { loaded: true, version: '0.2.0', error: null, missing: [], ...over }
}

beforeEach(() => {
  vi.clearAllMocks()
  mockStore.config = config()
  status = core()
  daemon = { state: 'off' }
  holders = null
  endSessionHolder.mockResolvedValue({ ok: true })
  api.getVorndStatus = () => Promise.resolve(daemon)
})

describe('ExperimentalSettings', () => {
  it('renders nothing before the config has loaded', () => {
    mockStore.config = null
    const { container } = render(<ExperimentalSettings />)
    expect(container).toBeEmptyDOMElement()
  })

  it('shows the native core it runs on, and nothing else about it when it is whole', async () => {
    render(<ExperimentalSettings />)
    await screen.findByText('Native core 0.2.0')
    expect(screen.queryByText(/did not load|was made without/)).not.toBeInTheDocument()
  })

  it('says what is missing when the core did not load', async () => {
    status = core({ loaded: false, version: null, error: 'vorn_core.node not found' })
    render(<ExperimentalSettings />)
    expect(
      await screen.findByText(
        'The native core did not load, so terminals have no screen model, agent status or terminal output for agents: vorn_core.node not found'
      )
    ).toBeInTheDocument()
    expect(screen.queryByText(/Native core \d/)).not.toBeInTheDocument()
  })

  it('names what a core was built without', async () => {
    status = core({ missing: ['the screen model', 'agent status'] })
    render(<ExperimentalSettings />)
    expect(
      await screen.findByText(
        'This build of the native core was made without the screen model, agent status.'
      )
    ).toBeInTheDocument()
  })

  it('says nothing about the core when the server cannot report on it', async () => {
    status = new Error('older server')
    render(<ExperimentalSettings />)
    await screen.findByRole('switch', { name: 'Native daemon' })
    expect(screen.queryByText(/Native core/)).not.toBeInTheDocument()
  })

  describe('the native daemon switch', () => {
    const daemonSwitch = (): HTMLElement => screen.getByRole('switch', { name: 'Native daemon' })

    const findDaemonSwitch = (): Promise<HTMLElement> =>
      screen.findByRole('switch', { name: 'Native daemon' })

    it('is off by default and saves under defaults.experimental', async () => {
      render(<ExperimentalSettings />)
      await findDaemonSwitch()
      await screen.findByText('Native core 0.2.0')
      expect(daemonSwitch()).toHaveAttribute('aria-checked', 'false')
      fireEvent.click(daemonSwitch())
      expect(saveConfig).toHaveBeenCalledWith(config({ vornd: true }))
    })

    it('stays usable whatever the native core says', async () => {
      status = core({ loaded: false, version: null, error: 'vorn_core.node not found' })
      render(<ExperimentalSettings />)
      await screen.findByText(/did not load/)
      await findDaemonSwitch()
      expect(daemonSwitch()).not.toBeDisabled()
    })

    it('says the switch applies from the next start', async () => {
      mockStore.config = config({ vornd: true })
      render(<ExperimentalSettings />)
      expect(
        await screen.findByText('Vorn connects through vornd the next time it starts.')
      ).toBeInTheDocument()
    })

    it('says nothing more while vornd is in use as asked', async () => {
      mockStore.config = config({ vornd: true })
      daemon = { state: 'on', port: 47001 }
      render(<ExperimentalSettings />)
      await screen.findByText('Native core 0.2.0')
      expect(screen.queryByText(/next time it starts/)).not.toBeInTheDocument()
    })

    it('says why the app went to the server directly', async () => {
      mockStore.config = config({ vornd: true })
      daemon = { state: 'failed', detail: 'vornd is not in this build' }
      render(<ExperimentalSettings />)
      expect(
        await screen.findByText(
          'Vorn is connected to the server directly: vornd is not in this build.'
        )
      ).toBeInTheDocument()
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
        mockStore.config = config({ vornd: true })
        daemon = { state: 'on', port: 47001 }
      })

      it('says how many sessions are still on one and that it exits after them', async () => {
        holders = {
          current: { ...older, pid: 1, instance: '99', build: '0.8.0' },
          older: [older],
          error: null
        }
        render(<ExperimentalSettings />)
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
        render(<ExperimentalSettings />)
        expect(await screen.findByText(/this version cannot talk to/)).toBeInTheDocument()
      })

      it('ends one only after asking', async () => {
        holders = { current: null, older: [{ ...older, sessions: 1 }], error: null }
        const confirm = vi
          .spyOn(window, 'confirm')
          .mockReturnValueOnce(false)
          .mockReturnValueOnce(true)
        render(<ExperimentalSettings />)
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
        render(<ExperimentalSettings />)
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
        render(<ExperimentalSettings />)
        expect(
          await screen.findByText('The session holder is not running: sessiond did not start.')
        ).toBeInTheDocument()
        expect(screen.queryByRole('button', { name: 'End them' })).not.toBeInTheDocument()
      })
    })

    it('is not shown where the app cannot run vornd', async () => {
      daemon = null
      render(<ExperimentalSettings />)
      await screen.findByText('Native core 0.2.0')
      expect(screen.queryByRole('switch', { name: 'Native daemon' })).not.toBeInTheDocument()
    })
  })

  describe('the native store switch', () => {
    const findStoreSwitch = (): Promise<HTMLElement> =>
      screen.findByRole('switch', { name: 'Native store' })

    it('is off by default and saves under defaults.experimental', async () => {
      render(<ExperimentalSettings />)
      const toggle = await findStoreSwitch()
      expect(toggle).toHaveAttribute('aria-checked', 'false')
      fireEvent.click(toggle)
      expect(saveConfig).toHaveBeenCalledWith(config({ nativeStore: true }))
    })

    it('is shown where vornd cannot run', async () => {
      daemon = null
      render(<ExperimentalSettings />)
      expect(await findStoreSwitch()).toBeInTheDocument()
    })

    it('says the switch applies from the next start, both ways', async () => {
      mockStore.config = config({ nativeStore: true })
      status = core({ store: { native: false, error: null } })
      const { unmount } = render(<ExperimentalSettings />)
      expect(
        await screen.findByText('Vorn uses the native store the next time it starts.')
      ).toBeInTheDocument()
      unmount()

      mockStore.config = config({ nativeStore: false })
      status = core({ store: { native: true, error: null } })
      render(<ExperimentalSettings />)
      expect(
        await screen.findByText('Vorn goes back to its built-in store the next time it starts.')
      ).toBeInTheDocument()
    })

    it('says why the native store did not open', async () => {
      mockStore.config = config({ nativeStore: true })
      status = core({ store: { native: false, error: 'the native core is not loaded' } })
      render(<ExperimentalSettings />)
      expect(
        await screen.findByText(
          'Vorn kept its built-in store, because the native store did not open: the native core is not loaded.'
        )
      ).toBeInTheDocument()
    })

    it('says nothing more while the native store is in use as asked', async () => {
      mockStore.config = config({ nativeStore: true })
      status = core({ store: { native: true, error: null } })
      render(<ExperimentalSettings />)
      await screen.findByText('Native core 0.2.0')
      expect(screen.queryByText(/store the next time/)).not.toBeInTheDocument()
    })
  })
})
