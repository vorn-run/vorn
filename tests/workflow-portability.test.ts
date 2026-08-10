import { describe, it, expect } from 'vitest'
import {
  toPortable,
  fromPortable,
  portabilityBlockers,
  residualAbsolutePaths,
  importedWorkflowId,
  parseImportedWorkflowId,
  slugify,
  PROJECT_PATH_TOKEN
} from '../packages/mcp/src/workflow-portability'
import type { WorkflowDefinition } from '../packages/shared/src/types'

const PROJECT = '/Users/someone/dev/novum'

function workflow(overrides: Partial<WorkflowDefinition> = {}): WorkflowDefinition {
  return {
    id: 'abc-123',
    name: 'Novum AI — weekly edition',
    icon: 'Newspaper',
    iconColor: '#c9a227',
    enabled: true,
    nodes: [
      {
        id: 'trigger-1',
        type: 'trigger',
        label: 'Manual',
        config: { triggerType: 'manual' },
        position: { x: 0, y: 0 }
      },
      {
        id: 'fetch-1',
        type: 'script',
        label: 'Pull feeds',
        config: {
          scriptType: 'bash',
          projectName: 'Novum',
          projectPath: PROJECT,
          scriptContent: 'python3 scripts/fetch_feeds.py'
        },
        position: { x: 0, y: 110 }
      },
      {
        id: 'write-1',
        type: 'launchAgent',
        label: 'Write',
        config: {
          agentType: 'claude',
          projectName: 'Novum',
          projectPath: PROJECT,
          existingWorktreePath: `${PROJECT}/wt/draft`,
          remoteHostId: 'host-uuid-local',
          headless: true
        },
        position: { x: 0, y: 220 }
      }
    ],
    edges: [
      { id: 'e1', source: 'trigger-1', target: 'fetch-1' },
      { id: 'e2', source: 'fetch-1', target: 'write-1' }
    ],
    ...overrides
  }
}

describe('toPortable', () => {
  it('replaces the project path everywhere it appears', () => {
    const p = toPortable(workflow(), PROJECT)
    const script = p.nodes.find((n) => n.id === 'fetch-1')!.config as Record<string, unknown>
    expect(script.projectPath).toBe(PROJECT_PATH_TOKEN)
  })

  it('keeps the tail of a path inside the project', () => {
    // A worktree must not collapse onto the project root.
    const p = toPortable(workflow(), PROJECT)
    const agent = p.nodes.find((n) => n.id === 'write-1')!.config as Record<string, unknown>
    expect(agent.existingWorktreePath).toBe(`${PROJECT_PATH_TOKEN}/wt/draft`)
  })

  it('drops remoteHostId, which names a host only this install knows', () => {
    const p = toPortable(workflow(), PROJECT)
    const agent = p.nodes.find((n) => n.id === 'write-1')!.config as Record<string, unknown>
    expect(agent).not.toHaveProperty('remoteHostId')
  })

  it('leaves nothing machine-specific behind', () => {
    expect(residualAbsolutePaths(toPortable(workflow(), PROJECT))).toEqual([])
  })

  it('does not mutate the workflow it was given', () => {
    const wf = workflow()
    toPortable(wf, PROJECT)
    expect((wf.nodes[1].config as Record<string, unknown>).projectPath).toBe(PROJECT)
  })
})

describe('round trip', () => {
  it('resolves back to a runnable workflow for another project', () => {
    const p = toPortable(workflow(), PROJECT)
    const imported = fromPortable(p, 'novum', { name: 'Novum2', path: '/Users/other/code/novum2' })

    const script = imported.nodes.find((n) => n.id === 'fetch-1')!.config as Record<string, unknown>
    expect(script.projectPath).toBe('/Users/other/code/novum2')
    expect(script.projectName).toBe('Novum2')

    const agent = imported.nodes.find((n) => n.id === 'write-1')!.config as Record<string, unknown>
    expect(agent.existingWorktreePath).toBe('/Users/other/code/novum2/wt/draft')
  })

  it('gives the same id on every machine, so re-import updates in place', () => {
    const a = fromPortable(toPortable(workflow(), PROJECT), 'novum', {
      name: 'A',
      path: '/one'
    })
    const b = fromPortable(toPortable(workflow(), '/somewhere/else'), 'novum', {
      name: 'B',
      path: '/two'
    })
    expect(a.id).toBe(b.id)
    expect(a.id).toBe('import:novum:novum-ai-weekly-edition')
  })

  it('preserves the graph', () => {
    const imported = fromPortable(toPortable(workflow(), PROJECT), 'novum', {
      name: 'X',
      path: '/x'
    })
    expect(imported.nodes).toHaveLength(3)
    expect(imported.edges).toHaveLength(2)
  })
})

describe('portabilityBlockers', () => {
  it('passes an ordinary workflow', () => {
    expect(portabilityBlockers(workflow())).toEqual([])
  })

  it('refuses a connector-poll trigger rather than exporting a broken file', () => {
    const wf = workflow()
    wf.nodes[0].config = {
      triggerType: 'connectorPoll',
      connectionId: 'local-uuid',
      event: 'issue.created',
      cron: '*/5 * * * *'
    }
    expect(portabilityBlockers(wf)[0]).toContain('connector connection')
  })

  it('refuses a connector action step', () => {
    const wf = workflow()
    wf.nodes.push({
      id: 'act-1',
      type: 'callConnectorAction',
      label: 'Create issue',
      config: {
        nodeType: 'callConnectorAction',
        connectionId: 'local-uuid',
        action: 'x',
        args: {}
      },
      position: { x: 0, y: 330 }
    })
    expect(portabilityBlockers(wf).some((b) => b.includes('Create issue'))).toBe(true)
  })
})

describe('ids and slugs', () => {
  it('round-trips an id', () => {
    expect(parseImportedWorkflowId(importedWorkflowId('novum', 'weekly'))).toEqual({
      bundle: 'novum',
      slug: 'weekly'
    })
  })

  it('ignores an id it did not mint', () => {
    expect(parseImportedWorkflowId('connector:abc:issue')).toBeNull()
    expect(parseImportedWorkflowId('system:default-task-workflow')).toBeNull()
  })

  it('slugifies punctuation and unicode dashes out of a name', () => {
    expect(slugify('Novum AI — weekly edition')).toBe('novum-ai-weekly-edition')
  })

  it('never produces an empty slug', () => {
    expect(slugify('———')).toBe('workflow')
  })
})

describe('Windows paths', () => {
  // Vorn ships a Windows installer, so a workflow can be authored with
  // backslash paths. A POSIX-only rewriter would leave them untouched and the
  // export would claim to be portable while naming one laptop's directory.
  const WIN = 'C:\\Users\\someone\\dev\\novum'

  function winWorkflow(): WorkflowDefinition {
    const wf = workflow()
    wf.nodes[1].config = {
      scriptType: 'bash',
      projectName: 'Novum',
      projectPath: WIN,
      scriptContent: 'x'
    } as WorkflowDefinition['nodes'][number]['config']
    wf.nodes[2].config = {
      agentType: 'claude',
      projectName: 'Novum',
      projectPath: WIN,
      existingWorktreePath: `${WIN}\\wt\\draft`,
      headless: true
    } as WorkflowDefinition['nodes'][number]['config']
    return wf
  }

  it('rewrites a backslash project path', () => {
    const p = toPortable(winWorkflow(), WIN)
    const script = p.nodes.find((n) => n.id === 'fetch-1')!.config as Record<string, unknown>
    expect(script.projectPath).toBe(PROJECT_PATH_TOKEN)
  })

  it('rewrites a nested backslash path and keeps its tail', () => {
    const p = toPortable(winWorkflow(), WIN)
    const agent = p.nodes.find((n) => n.id === 'write-1')!.config as Record<string, unknown>
    expect(agent.existingWorktreePath).toBe(`${PROJECT_PATH_TOKEN}/wt/draft`)
  })

  it('reports nothing machine-specific left behind', () => {
    expect(residualAbsolutePaths(toPortable(winWorkflow(), WIN))).toEqual([])
  })

  it('flags a leftover drive-letter path, which a POSIX-only check called portable', () => {
    const p = toPortable(workflow(), '/wrong/project')
    const stray = { ...p, nodes: [...p.nodes] }
    stray.nodes[1] = {
      ...stray.nodes[1],
      config: { projectPath: 'C:\\Users\\someone\\elsewhere' } as never
    }
    expect(residualAbsolutePaths(stray)).toContain('fetch-1.projectPath')
  })

  it('flags a leftover UNC share', () => {
    const p = toPortable(workflow(), '/wrong/project')
    const stray = { ...p, nodes: [...p.nodes] }
    stray.nodes[1] = {
      ...stray.nodes[1],
      config: { projectPath: '\\\\server\\share\\project' } as never
    }
    expect(residualAbsolutePaths(stray)).toContain('fetch-1.projectPath')
  })

  it('does not double the separator when the project path ends in one', () => {
    const p = toPortable(winWorkflow(), WIN)
    const imported = fromPortable(p, 'novum', { name: 'N', path: 'D:\\code\\novum\\' })
    const script = imported.nodes.find((n) => n.id === 'fetch-1')!.config as Record<string, unknown>
    expect(script.projectPath).toBe('D:\\code\\novum')
  })

  it('does not double a trailing POSIX separator either', () => {
    const p = toPortable(workflow(), PROJECT)
    const imported = fromPortable(p, 'novum', { name: 'N', path: '/Users/other/novum/' })
    const script = imported.nodes.find((n) => n.id === 'fetch-1')!.config as Record<string, unknown>
    expect(script.projectPath).toBe('/Users/other/novum')
  })
})

describe('import refuses what export refuses', () => {
  // Export rejects a connector-bound workflow because its connectionId is
  // local. Without the same check on the way in, a hand-written file walks
  // past that contract and lands a workflow that fails at run time against a
  // connection this machine never had.
  it('detects a connector trigger in a resolved definition', () => {
    const portable = toPortable(workflow(), PROJECT)
    portable.nodes[0].config = {
      triggerType: 'connectorPoll',
      connectionId: 'from-another-machine',
      event: 'issue.created',
      cron: '*/5 * * * *'
    } as never
    const resolved = fromPortable(portable, 'novum', { name: 'N', path: '/tmp/n' })
    expect(portabilityBlockers(resolved)).not.toEqual([])
  })

  it('detects a connector action step in a resolved definition', () => {
    const portable = toPortable(workflow(), PROJECT)
    portable.nodes.push({
      id: 'act',
      type: 'callConnectorAction',
      label: 'Create issue',
      config: {
        nodeType: 'callConnectorAction',
        connectionId: 'x',
        action: 'a',
        args: {}
      } as never,
      position: { x: 0, y: 0 }
    })
    const resolved = fromPortable(portable, 'novum', { name: 'N', path: '/tmp/n' })
    expect(portabilityBlockers(resolved).some((b) => b.includes('Create issue'))).toBe(true)
  })

  it('passes an ordinary workflow through both directions', () => {
    const resolved = fromPortable(toPortable(workflow(), PROJECT), 'novum', {
      name: 'N',
      path: '/tmp/n'
    })
    expect(portabilityBlockers(resolved)).toEqual([])
  })
})
