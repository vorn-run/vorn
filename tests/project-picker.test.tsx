// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

vi.mock('react-dom', async () => {
  const actual = await vi.importActual<typeof import('react-dom')>('react-dom')
  return { ...actual, createPortal: (node: React.ReactNode) => node }
})

vi.mock('framer-motion', () => ({
  motion: {
    div: ({ children, ...props }: React.PropsWithChildren<Record<string, unknown>>) => (
      <div {...props}>{children}</div>
    )
  },
  AnimatePresence: ({ children }: React.PropsWithChildren) => <>{children}</>
}))

import { ProjectPicker } from '../src/renderer/components/ProjectPicker'
import type { ProjectConfig } from '../src/shared/types'

const projects: ProjectConfig[] = [
  { name: 'Vorn', path: '/r/vorn', preferredAgents: [], icon: 'Rocket', iconColor: '#fff' },
  { name: 'Other', path: '/r/other', preferredAgents: [], icon: 'Folder', iconColor: '#fff' }
]

describe('ProjectPicker — From Context entry', () => {
  it('renders the trigger label as the current project name by default', () => {
    render(
      <ProjectPicker currentProject="Vorn" projects={projects} onChange={vi.fn()} variant="form" />
    )
    expect(screen.getByText('Vorn')).toBeInTheDocument()
  })

  it("renders the trigger label as 'From Context' when isFromContext is true", () => {
    render(
      <ProjectPicker
        currentProject=""
        projects={projects}
        onChange={vi.fn()}
        variant="form"
        isFromContext
      />
    )
    // Two "From Context" labels can appear (trigger + dropdown), expect at least one
    expect(screen.getAllByText('From Context').length).toBeGreaterThan(0)
  })

  it('renders the placeholder when currentProject is empty and not from-context', () => {
    render(
      <ProjectPicker currentProject="" projects={projects} onChange={vi.fn()} variant="form" />
    )
    expect(screen.getByText('Select project...')).toBeInTheDocument()
  })

  it("shows a 'From Context' option in the dropdown when allowFromContext is true", () => {
    render(
      <ProjectPicker
        currentProject="Vorn"
        projects={projects}
        onChange={vi.fn()}
        variant="form"
        allowFromContext
        onSelectFromContext={vi.fn()}
      />
    )
    fireEvent.click(screen.getByRole('button', { name: /Vorn/ }))
    // After opening, the dropdown also surfaces 'From Context' as an entry
    expect(screen.getByText('From Context')).toBeInTheDocument()
  })

  it('invokes onSelectFromContext when the dropdown entry is clicked', () => {
    const onSelectFromContext = vi.fn()
    render(
      <ProjectPicker
        currentProject="Vorn"
        projects={projects}
        onChange={vi.fn()}
        variant="form"
        allowFromContext
        onSelectFromContext={onSelectFromContext}
      />
    )
    fireEvent.click(screen.getByRole('button', { name: /Vorn/ }))
    fireEvent.click(screen.getByText('From Context'))
    expect(onSelectFromContext).toHaveBeenCalled()
  })

  it("includes a 'None' entry when allowNone is set", () => {
    render(
      <ProjectPicker
        currentProject=""
        projects={projects}
        onChange={vi.fn()}
        variant="form"
        allowNone
      />
    )
    fireEvent.click(screen.getByRole('button', { name: /Select project/ }))
    expect(screen.getByText('None')).toBeInTheDocument()
  })

  it('selects a project from the dropdown', () => {
    const onChange = vi.fn()
    render(
      <ProjectPicker currentProject="Vorn" projects={projects} onChange={onChange} variant="form" />
    )
    fireEvent.click(screen.getByRole('button', { name: /Vorn/ }))
    fireEvent.click(screen.getByText('Other'))
    expect(onChange).toHaveBeenCalledWith('Other')
  })
})
