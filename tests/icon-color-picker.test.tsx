// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import { IconColorPicker } from '../src/renderer/components/IconColorPicker'
import { PROJECT_ICON_OPTIONS, ICON_COLOR_PALETTE } from '../src/renderer/lib/project-icons'

/** Shared by the project dialog and a group's Change icon, so it holds no state. */
describe('the icon and colour picker', () => {
  const draw = (over: Partial<React.ComponentProps<typeof IconColorPicker>> = {}) =>
    render(
      <IconColorPicker
        icon="Folder"
        color="#6b7280"
        onIconChange={vi.fn()}
        onColorChange={vi.fn()}
        {...over}
      />
    )

  it('offers every icon and every colour', () => {
    draw()
    for (const opt of PROJECT_ICON_OPTIONS) {
      expect(screen.getByRole('button', { name: opt.label })).toBeInTheDocument()
    }
    for (const swatch of ICON_COLOR_PALETTE) {
      expect(screen.getByRole('button', { name: `Color ${swatch}` })).toBeInTheDocument()
    }
  })

  it('reports which icon is chosen', () => {
    draw({ icon: 'Terminal' })
    expect(screen.getByRole('button', { name: 'Terminal' })).toHaveAttribute('aria-pressed', 'true')
    expect(screen.getByRole('button', { name: 'Folder' })).toHaveAttribute('aria-pressed', 'false')
  })

  it('reports which colour is chosen', () => {
    draw({ color: ICON_COLOR_PALETTE[1] })
    expect(screen.getByRole('button', { name: `Color ${ICON_COLOR_PALETTE[1]}` })).toHaveAttribute(
      'aria-pressed',
      'true'
    )
  })

  it('hands back the icon that was picked', () => {
    const onIconChange = vi.fn()
    draw({ onIconChange })
    fireEvent.click(screen.getByRole('button', { name: 'Launch' }))
    expect(onIconChange).toHaveBeenCalledWith('Rocket')
  })

  it('hands back the colour that was picked', () => {
    const onColorChange = vi.fn()
    draw({ onColorChange })
    fireEvent.click(screen.getByRole('button', { name: `Color ${ICON_COLOR_PALETTE[2]}` }))
    expect(onColorChange).toHaveBeenCalledWith(ICON_COLOR_PALETTE[2])
  })

  it('shows a preview, and can be asked not to', () => {
    const { unmount } = draw()
    expect(screen.getByText('Preview')).toBeInTheDocument()
    unmount()
    draw({ showPreview: false })
    expect(screen.queryByText('Preview')).not.toBeInTheDocument()
  })

  it('falls back to a folder for an icon it does not know', () => {
    draw({ icon: 'not-a-real-icon' })
    expect(screen.getByText('Preview')).toBeInTheDocument()
  })
})
