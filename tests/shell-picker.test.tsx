// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render, screen, fireEvent, waitFor, cleanup } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import { ShellPicker } from '../src/renderer/components/settings/ShellPicker'
import type { InstalledShell } from '../src/shared/types'

/**
 * Choosing a shell used to be a free-text path, which only helps someone who
 * already knows what is installed and where. On Windows the choice also decides
 * whether blocks carry an exit status at all, so the picker has to say so.
 */

const SHELLS: InstalledShell[] = [
  {
    family: 'powershell',
    name: 'PowerShell 7',
    path: 'C:\\Program Files\\PowerShell\\7\\pwsh.exe',
    version: '7.4.1',
    blocks: { level: 'partial', limitation: 'Blocks appear once each command finishes' }
  },
  {
    family: 'cmd',
    name: 'Command Prompt',
    path: 'C:\\Windows\\system32\\cmd.exe',
    version: null,
    blocks: { level: 'limited', limitation: 'No exit status or command name on blocks' }
  }
]

function stubApi(shells: InstalledShell[] | Error): void {
  Object.defineProperty(window, 'api', {
    value: {
      listInstalledShells: () =>
        shells instanceof Error ? Promise.reject(shells) : Promise.resolve(shells)
    },
    writable: true,
    configurable: true
  })
}

beforeEach(() => stubApi(SHELLS))
afterEach(() => cleanup())

describe('ShellPicker', () => {
  it('names the selected shell rather than only its path', async () => {
    render(<ShellPicker value={SHELLS[0].path} onChange={vi.fn()} />)
    expect(await screen.findByText('PowerShell 7')).toBeInTheDocument()
  })

  it('lists what is installed, with versions', async () => {
    render(<ShellPicker value={SHELLS[0].path} onChange={vi.fn()} />)
    await screen.findByText('PowerShell 7')
    fireEvent.click(screen.getByRole('button', { name: /PowerShell 7/ }))
    expect(await screen.findByRole('option', { name: /Command Prompt/ })).toBeInTheDocument()
    expect(screen.getByText('7.4.1')).toBeInTheDocument()
  })

  it('says what a shell cannot report, so the choice is informed', async () => {
    render(<ShellPicker value={SHELLS[0].path} onChange={vi.fn()} />)
    await screen.findByText('PowerShell 7')
    fireEvent.click(screen.getByRole('button', { name: /PowerShell 7/ }))
    expect(await screen.findByText('No exit status or command name on blocks')).toBeInTheDocument()
  })

  it('reports the choice by path, which is what gets launched', async () => {
    const onChange = vi.fn()
    render(<ShellPicker value={SHELLS[0].path} onChange={onChange} />)
    await screen.findByText('PowerShell 7')
    fireEvent.click(screen.getByRole('button', { name: /PowerShell 7/ }))
    fireEvent.click(await screen.findByRole('option', { name: /Command Prompt/ }))
    expect(onChange).toHaveBeenCalledWith('C:\\Windows\\system32\\cmd.exe')
  })

  it('marks the current shell', async () => {
    render(<ShellPicker value={SHELLS[1].path} onChange={vi.fn()} />)
    await screen.findByText('Command Prompt')
    fireEvent.click(screen.getByRole('button', { name: /Command Prompt/ }))
    const options = await screen.findAllByRole('option')
    expect(options.find((o) => o.getAttribute('aria-selected') === 'true')).toHaveTextContent(
      'Command Prompt'
    )
  })

  it('still accepts a typed path, for a shell we did not detect', async () => {
    const onChange = vi.fn()
    render(<ShellPicker value={SHELLS[0].path} onChange={onChange} />)
    await screen.findByText('PowerShell 7')
    fireEvent.click(screen.getByRole('button', { name: /PowerShell 7/ }))
    fireEvent.click(await screen.findByText('Enter a path…'))
    const input = screen.getByPlaceholderText('Path to a shell')
    fireEvent.change(input, { target: { value: '/opt/weird/shell' } })
    fireEvent.keyDown(input, { key: 'Enter' })
    expect(onChange).toHaveBeenCalledWith('/opt/weird/shell')
  })

  it('stays usable when detection fails', async () => {
    // Detection is a convenience. Losing it must not cost the ability to set a
    // shell at all.
    stubApi(new Error('no bridge'))
    render(<ShellPicker value="/bin/zsh" onChange={vi.fn()} />)
    fireEvent.click(screen.getByRole('button'))
    await waitFor(() => expect(screen.getByText(/No shells detected/)).toBeInTheDocument())
    expect(screen.getByText('Enter a path…')).toBeInTheDocument()
  })
})
