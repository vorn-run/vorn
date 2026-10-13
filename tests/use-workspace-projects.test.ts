// @vitest-environment jsdom
import { describe, it, expect } from 'vitest'
import { renderHook, act } from '@testing-library/react'
import { useAppStore } from '../packages/web/src/renderer/stores'
import { useWorkspaceProjects } from '../packages/web/src/renderer/hooks/useWorkspaceProjects'
import type { AppConfig, ProjectConfig } from '../packages/shared/src/types'

function makeProject(name: string, workspaceId?: string): ProjectConfig {
  return { name, path: `/projects/${name}`, preferredAgents: [], workspaceId }
}

function seedStore(projects: ProjectConfig[], activeWorkspace: string) {
  const config: AppConfig = {
    version: 1,
    projects,
    tasks: [],
    workspaces: [],
    workflows: [],
    defaults: { shell: '/bin/zsh', fontSize: 14, theme: 'dark', defaultAgent: 'claude' }
  }
  useAppStore.setState({ config, activeWorkspace })
}

describe('useWorkspaceProjects', () => {
  it('returns only projects matching the active workspace', () => {
    seedStore(
      [makeProject('alpha', 'work'), makeProject('beta', 'personal'), makeProject('gamma', 'work')],
      'work'
    )
    const { result } = renderHook(() => useWorkspaceProjects())
    expect(result.current.map((p) => p.name)).toEqual(['alpha', 'gamma'])
  })

  it('treats missing workspaceId as "personal"', () => {
    seedStore(
      [makeProject('noWs'), makeProject('explicit', 'personal'), makeProject('other', 'work')],
      'personal'
    )
    const { result } = renderHook(() => useWorkspaceProjects())
    expect(result.current.map((p) => p.name)).toEqual(['noWs', 'explicit'])
  })

  it('returns empty array when config is null', () => {
    useAppStore.setState({ config: null, activeWorkspace: 'personal' })
    const { result } = renderHook(() => useWorkspaceProjects())
    expect(result.current).toEqual([])
  })

  it('updates when active workspace changes', () => {
    seedStore([makeProject('a', 'ws1'), makeProject('b', 'ws2')], 'ws1')
    const { result } = renderHook(() => useWorkspaceProjects())
    expect(result.current.map((p) => p.name)).toEqual(['a'])

    act(() => useAppStore.setState({ activeWorkspace: 'ws2' }))
    expect(result.current.map((p) => p.name)).toEqual(['b'])
  })
})
