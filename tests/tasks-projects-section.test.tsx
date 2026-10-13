// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { ProjectConfig } from '../packages/web/src/shared/types'

const mockStore = {
  activeProject: null as string | null,
  setActiveProject: vi.fn()
}

vi.mock('../packages/web/src/renderer/stores', () => ({
  useAppStore: (selector?: (state: unknown) => unknown) => {
    return selector ? selector(mockStore) : mockStore
  }
}))

const { TasksProjectsSection } =
  await import('../packages/web/src/renderer/components/project-sidebar/TasksProjectsSection')

function makeProject(name: string): ProjectConfig {
  return {
    name,
    path: `/tmp/${name}`,
    icon: 'Folder',
    iconColor: '#888'
  } as ProjectConfig
}

beforeEach(() => {
  mockStore.activeProject = null
  mockStore.setActiveProject.mockReset()
})

describe('TasksProjectsSection', () => {
  it('renders the Projects header and All Projects entry', () => {
    render(<TasksProjectsSection isCollapsed={false} workspaceProjects={[]} />)
    expect(screen.getByText('Projects')).toBeInTheDocument()
    expect(screen.getByText('All Projects')).toBeInTheDocument()
  })

  it('shows empty-state message when no projects', () => {
    render(<TasksProjectsSection isCollapsed={false} workspaceProjects={[]} />)
    expect(screen.getByText('No projects')).toBeInTheDocument()
  })

  it('renders one row per project', () => {
    render(
      <TasksProjectsSection
        isCollapsed={false}
        workspaceProjects={[makeProject('alpha'), makeProject('beta')]}
      />
    )
    expect(screen.getByText('alpha')).toBeInTheDocument()
    expect(screen.getByText('beta')).toBeInTheDocument()
  })

  it('clicking All Projects clears the active project', () => {
    mockStore.activeProject = 'alpha'
    render(<TasksProjectsSection isCollapsed={false} workspaceProjects={[makeProject('alpha')]} />)
    fireEvent.click(screen.getByText('All Projects'))
    expect(mockStore.setActiveProject).toHaveBeenCalledWith(null)
  })

  it('clicking a project sets it as active', () => {
    render(<TasksProjectsSection isCollapsed={false} workspaceProjects={[makeProject('alpha')]} />)
    fireEvent.click(screen.getByText('alpha'))
    expect(mockStore.setActiveProject).toHaveBeenCalledWith('alpha')
  })

  it('collapsing the section hides project rows', () => {
    render(<TasksProjectsSection isCollapsed={false} workspaceProjects={[makeProject('alpha')]} />)
    expect(screen.getByText('alpha')).toBeInTheDocument()
    fireEvent.click(screen.getByText('Projects'))
    expect(screen.queryByText('alpha')).not.toBeInTheDocument()
    expect(screen.queryByText('All Projects')).not.toBeInTheDocument()
  })

  it('hides the header text when sidebar is collapsed', () => {
    render(<TasksProjectsSection isCollapsed={true} workspaceProjects={[makeProject('alpha')]} />)
    expect(screen.queryByText('Projects')).not.toBeInTheDocument()
    expect(screen.queryByText('alpha')).not.toBeInTheDocument()
  })
})
