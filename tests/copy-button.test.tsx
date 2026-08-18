// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render, screen, waitFor, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

import { CopyButton } from '../src/renderer/components/settings/network/shared'

/**
 * Copying an address out of the Remote Access panel.
 *
 * Driven with `fireEvent` rather than `userEvent`: `userEvent.setup()` installs its
 * own `navigator.clipboard` stub, which would quietly replace the one each case
 * here is trying to test.
 *
 * The panel exists to hand someone a plain-HTTP address on a LAN, which is exactly
 * the insecure context where `navigator.clipboard` is not merely permission-denied
 * but absent. So the fallback is not a rare edge here — for anyone without
 * Tailscale it is the only path that runs.
 */

const URL_TEXT = 'http://192.168.0.4:61601/app/'

beforeEach(() => {
  vi.restoreAllMocks()
})

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('copying an address', () => {
  it('uses the clipboard when there is one', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined)
    vi.stubGlobal('navigator', { clipboard: { writeText } })
    render(<CopyButton text={URL_TEXT} />)

    fireEvent.click(screen.getByRole('button'))

    expect(writeText).toHaveBeenCalledWith(URL_TEXT)
    await waitFor(() => expect(screen.getByRole('button')).toHaveTextContent('Copied'))
  })

  it('prompts instead when the clipboard API is absent', async () => {
    // The bug this pins: reaching for `.writeText` on an undefined clipboard throws
    // synchronously, before any promise exists, so a rejection handler never runs
    // and the fallback was unreachable. The click threw instead of copying.
    vi.stubGlobal('navigator', {})
    const prompt = vi.fn()
    vi.stubGlobal('prompt', prompt)
    render(<CopyButton text={URL_TEXT} />)

    fireEvent.click(screen.getByRole('button'))

    expect(prompt).toHaveBeenCalledWith('Copy this URL:', URL_TEXT)
  })

  it('prompts when the clipboard exists but refuses', async () => {
    // Permission denied, which is the case the original code did handle.
    vi.stubGlobal('navigator', {
      clipboard: { writeText: vi.fn().mockRejectedValue(new Error('denied')) }
    })
    const prompt = vi.fn()
    vi.stubGlobal('prompt', prompt)
    render(<CopyButton text={URL_TEXT} />)

    fireEvent.click(screen.getByRole('button'))

    await waitFor(() => expect(prompt).toHaveBeenCalledWith('Copy this URL:', URL_TEXT))
  })

  it('prompts when the clipboard throws rather than rejecting', async () => {
    vi.stubGlobal('navigator', {
      clipboard: {
        writeText: () => {
          throw new Error('blocked')
        }
      }
    })
    const prompt = vi.fn()
    vi.stubGlobal('prompt', prompt)
    render(<CopyButton text={URL_TEXT} />)

    fireEvent.click(screen.getByRole('button'))

    expect(prompt).toHaveBeenCalledWith('Copy this URL:', URL_TEXT)
  })

  it('does not claim success when nothing was copied', async () => {
    // "Copied" against a clipboard that never took the text would send someone off
    // to paste an address they do not have.
    vi.stubGlobal('navigator', {})
    vi.stubGlobal('prompt', vi.fn())
    render(<CopyButton text={URL_TEXT} />)

    fireEvent.click(screen.getByRole('button'))

    expect(screen.getByRole('button')).not.toHaveTextContent('Copied')
  })
})
