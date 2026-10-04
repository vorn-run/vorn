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
  return { loaded: true, version: '0.2.0', error: null, forced: null, missing: [], ...over }
}

const screenSwitch = (): HTMLElement => screen.getAllByRole('switch')[0]!

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

  it('turns a switch on and saves it under defaults.experimental', async () => {
    render(<ExperimentalSettings />)
    await screen.findByText('Native core 0.2.0')
    expect(screenSwitch()).toHaveAttribute('aria-checked', 'false')
    fireEvent.click(screenSwitch())
    const saved = config({ nativeScreen: true })
    expect(saveConfig).toHaveBeenCalledWith(saved)
    expect(mockStore.setConfig).toHaveBeenCalledWith(saved)
  })

  it('shows a switch that is on, and keeps the other flags when turning it off', async () => {
    mockStore.config = config({ nativeScreen: true, nativeAnalysis: true } as never)
    render(<ExperimentalSettings />)
    await screen.findByText('Native core 0.2.0')
    expect(screenSwitch()).toHaveAttribute('aria-checked', 'true')
    fireEvent.click(screenSwitch())
    expect(saveConfig).toHaveBeenCalledWith(
      config({ nativeScreen: false, nativeAnalysis: true } as never)
    )
  })

  it.each([
    ['VORN_CORE=js', core({ loaded: null, version: null, forced: 'js' }), /VORN_CORE=js is set/],
    [
      'an unrecognized VORN_CORE',
      core({
        loaded: null,
        version: null,
        forced: 'js',
        error: 'VORN_CORE=rust is not recognized'
      }),
      /VORN_CORE=rust is not recognized by the server, so every native core switch is off/
    ],
    ['VORN_CORE=native', core({ forced: 'native' }), /every native core switch is on/],
    [
      'a binary that will not load',
      core({ loaded: false, version: null, error: 'vorn_core.node not found' }),
      /not available in this build.*vorn_core\.node not found/
    ],
    [
      'a binary that will not load under VORN_CORE=native',
      core({ loaded: false, version: null, error: null, forced: 'native' }),
      /not available in this build, so the native core switches stay on JavaScript\.$/
    ]
  ])('locks the switches and says why for %s', async (_, next, note) => {
    status = next
    render(<ExperimentalSettings />)
    expect(await screen.findByText(note)).toBeInTheDocument()
    expect(screenSwitch()).toBeDisabled()
    fireEvent.click(screenSwitch())
    expect(saveConfig).not.toHaveBeenCalled()
    // Device video does not run on the core, so nothing about it locks it.
    expect(screen.getByRole('switch', { name: 'Device video' })).not.toBeDisabled()
  })

  it('disables only the switches the binary was built without', async () => {
    status = core({ missing: ['nativeScreen'] })
    render(<ExperimentalSettings />)
    expect(
      await screen.findByText(/does not include Native screen model, so that stays on JavaScript/)
    ).toBeInTheDocument()
    expect(screenSwitch()).toBeDisabled()
    expect(screen.getByRole('switch', { name: 'Native git' })).not.toBeDisabled()
  })

  it('names both missing pieces when the binary has neither', async () => {
    status = core({ missing: ['nativeScreen', 'nativeGit'] })
    render(<ExperimentalSettings />)
    expect(
      await screen.findByText(/does not include Native screen model, Native git, so those stay/)
    ).toBeInTheDocument()
    expect(screen.getByRole('switch', { name: 'Native git' })).toBeDisabled()
  })

  it('says a switch stays on JavaScript under VORN_CORE=native when the binary lacks it', async () => {
    status = core({ forced: 'native', missing: ['nativeScreen'] })
    render(<ExperimentalSettings />)
    expect(
      await screen.findByText(/does not include Native screen model.*even with VORN_CORE=native/)
    ).toBeInTheDocument()
    expect(screen.queryByText(/every native core switch is on/)).not.toBeInTheDocument()
  })

  it('turns device video on without the core', async () => {
    status = core({ loaded: false, version: null, error: 'vorn_core.node not found' })
    render(<ExperimentalSettings />)
    await screen.findByText(/not available in this build/)
    fireEvent.click(screen.getByRole('switch', { name: 'Device video' }))
    expect(saveConfig).toHaveBeenCalledWith(config({ deviceVideo: true }))
  })

  it('keeps the switches locked until the status arrives', async () => {
    render(<ExperimentalSettings />)
    expect(screenSwitch()).toBeDisabled()
    await screen.findByText('Native core 0.2.0')
    expect(screenSwitch()).not.toBeDisabled()
  })

  it('names each switch for a screen reader', () => {
    render(<ExperimentalSettings />)
    expect(screen.getByRole('switch', { name: 'Native screen model' })).toBeInTheDocument()
    expect(screen.getByRole('switch', { name: 'Native git' })).toBeInTheDocument()
  })

  it('locks the switches when the server cannot report on the core', async () => {
    status = new Error('older server')
    render(<ExperimentalSettings />)
    expect(await screen.findByText(/can't report on the native core/)).toBeInTheDocument()
    expect(screenSwitch()).toBeDisabled()
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
      await screen.findByText(/not available in this build/)
      await findDaemonSwitch()
      expect(screenSwitch()).toBeDisabled()
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
})
