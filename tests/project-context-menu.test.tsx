// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import { ProjectContextMenu } from '../src/renderer/components/project-sidebar/ProjectContextMenu'
import type { ProjectConfig } from '../src/shared/types'

vi.mock('../src/renderer/components/Toast', () => ({
  toast: { success: vi.fn(), error: vi.fn() }
}))

const project: ProjectConfig = {
  name: 'test-project',
  path: '/tmp/test',
  preferredAgents: []
}

describe('ProjectContextMenu', () => {
  it('renders Edit and Delete buttons', () => {
    render(
      <ProjectContextMenu project={project} onEdit={vi.fn()} onDelete={vi.fn()} onClose={vi.fn()} />
    )
    expect(screen.getByText('Edit Project')).toBeInTheDocument()
    expect(screen.getByText('Delete Project')).toBeInTheDocument()
  })

  it('calls onEdit + onClose on Edit click', () => {
    const onEdit = vi.fn()
    const onClose = vi.fn()
    render(
      <ProjectContextMenu project={project} onEdit={onEdit} onDelete={vi.fn()} onClose={onClose} />
    )
    fireEvent.click(screen.getByText('Edit Project'))
    expect(onEdit).toHaveBeenCalledTimes(1)
    expect(onClose).toHaveBeenCalledTimes(1)
  })

  it('shows "Confirm delete?" on first Delete click without firing onDelete', () => {
    const onDelete = vi.fn()
    render(
      <ProjectContextMenu
        project={project}
        onEdit={vi.fn()}
        onDelete={onDelete}
        onClose={vi.fn()}
      />
    )
    fireEvent.click(screen.getByText('Delete Project'))
    expect(screen.getByText('Confirm delete?')).toBeInTheDocument()
    expect(onDelete).not.toHaveBeenCalled()
  })

  it('calls onDelete + onClose on confirmation click', () => {
    const onDelete = vi.fn()
    const onClose = vi.fn()
    render(
      <ProjectContextMenu
        project={project}
        onEdit={vi.fn()}
        onDelete={onDelete}
        onClose={onClose}
      />
    )
    fireEvent.click(screen.getByText('Delete Project'))
    fireEvent.click(screen.getByText('Confirm delete?'))
    expect(onDelete).toHaveBeenCalledTimes(1)
    expect(onClose).toHaveBeenCalledTimes(1)
  })

  it('calls onClose on click outside', () => {
    const onClose = vi.fn()
    render(
      <div>
        <div data-testid="outside">outside</div>
        <ProjectContextMenu
          project={project}
          onEdit={vi.fn()}
          onDelete={vi.fn()}
          onClose={onClose}
        />
      </div>
    )
    fireEvent.pointerDown(screen.getByTestId('outside'))
    expect(onClose).toHaveBeenCalledTimes(1)
  })
})
