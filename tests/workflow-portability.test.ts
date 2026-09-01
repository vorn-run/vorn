import { describe, it, expect } from 'vitest'
import {
  toPortable,
  fromPortable,
  resolveRequirement,
  unresolvedRequirements,
  residualAbsolutePaths,
  importedWorkflowId,
  parseImportedWorkflowId,
  slugify,
  PROJECT_PATH_TOKEN,
  type PortableConnection
} from '../packages/shared/src/workflow-portability'
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

function connection(overrides: Partial<PortableConnection> = {}): PortableConnection {
  return { id: 'conn-1', name: 'workspace-eng', connectorId: 'github', ...overrides }
}

/** A workflow whose trigger polls a connector and whose step calls one. */
function connectorWorkflow(): WorkflowDefinition {
  const wf = workflow()
  wf.nodes[0].config = {
    triggerType: 'connectorPoll',
    connectionId: 'conn-1',
    event: 'issueCreated',
    cron: '*/5 * * * *'
  } as WorkflowDefinition['nodes'][number]['config']
  wf.nodes.push({
    id: 'act-1',
    type: 'callConnectorAction',
    label: 'Create issue',
    config: {
      nodeType: 'callConnectorAction',
      connectionId: 'conn-1',
      action: 'createIssue',
      args: {}
    } as WorkflowDefinition['nodes'][number]['config'],
    position: { x: 0, y: 330 }
  })
  return wf
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

  it('says nothing about requirements when nothing is bound', () => {
    expect(toPortable(workflow(), PROJECT).requires).toBeUndefined()
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

describe('connections travel as requirements', () => {
  it('drops the local id and records what it stood for', () => {
    const p = toPortable(connectorWorkflow(), PROJECT, [connection()])

    expect((p.nodes[0].config as Record<string, unknown>).connectionId).toBe('')
    expect(p.requires).toEqual([
      {
        kind: 'connection',
        nodeId: 'trigger-1',
        connectorId: 'github',
        name: 'workspace-eng',
        event: 'issueCreated'
      },
      { kind: 'connection', nodeId: 'act-1', connectorId: 'github', name: 'workspace-eng' }
    ])
  })

  it('names a packaged connector by its manifest id, not mcp', () => {
    const packaged = connection({ connectorId: 'mcp', filters: { sdkConnectorId: 'packdemo' } })
    const p = toPortable(connectorWorkflow(), PROJECT, [packaged])
    expect((p.requires ?? [])[0]).toMatchObject({ connectorId: 'packdemo' })
  })

  it('unbinds an http profile even when the connection is unknown here', () => {
    const wf = workflow()
    wf.nodes.push({
      id: 'http-1',
      type: 'httpRequest',
      label: 'Report',
      config: {
        nodeType: 'httpRequest',
        method: 'POST',
        url: '/report',
        headers: {},
        body: '',
        profileConnectionId: 'gone'
      } as WorkflowDefinition['nodes'][number]['config'],
      position: { x: 0, y: 440 }
    })

    const p = toPortable(wf, PROJECT)
    const http = p.nodes.find((n) => n.id === 'http-1')!.config as Record<string, unknown>
    expect(http).not.toHaveProperty('profileConnectionId')
    expect(p.requires).toEqual([{ kind: 'httpProfile', nodeId: 'http-1', name: '' }])
  })

  it('never lets a local id reach the file', () => {
    const p = toPortable(connectorWorkflow(), PROJECT, [connection()])
    expect(JSON.stringify(p)).not.toContain('conn-1')
  })

  it('rebinds on import when this machine has one match', () => {
    const p = toPortable(connectorWorkflow(), PROJECT, [connection()])
    const here = [connection({ id: 'other-uuid', name: 'their-workspace' })]

    const imported = fromPortable(p, 'novum', { name: 'N', path: '/n' }, here)
    const trigger = imported.nodes[0].config as Record<string, unknown>
    const action = imported.nodes.find((n) => n.id === 'act-1')!.config as Record<string, unknown>
    expect(trigger.connectionId).toBe('other-uuid')
    expect(action.connectionId).toBe('other-uuid')
  })

  it('prefers the connection the file named when several could match', () => {
    const p = toPortable(connectorWorkflow(), PROJECT, [connection()])
    const here = [
      connection({ id: 'a', name: 'other-team' }),
      connection({ id: 'b', name: 'workspace-eng' })
    ]
    const imported = fromPortable(p, 'novum', { name: 'N', path: '/n' }, here)
    expect((imported.nodes[0].config as Record<string, unknown>).connectionId).toBe('b')
  })

  it('leaves the step unbound rather than guessing between two accounts', () => {
    const p = toPortable(connectorWorkflow(), PROJECT, [connection()])
    const here = [connection({ id: 'a', name: 'one' }), connection({ id: 'b', name: 'two' })]

    const imported = fromPortable(p, 'novum', { name: 'N', path: '/n' }, here)
    expect((imported.nodes[0].config as Record<string, unknown>).connectionId).toBe('')
    expect(unresolvedRequirements(p, here)).toHaveLength(2)
  })

  it('leaves the step unbound when nothing here is that connector', () => {
    const p = toPortable(connectorWorkflow(), PROJECT, [connection()])
    const imported = fromPortable(p, 'novum', { name: 'N', path: '/n' }, [
      connection({ id: 'z', connectorId: 'slack' })
    ])
    expect((imported.nodes[0].config as Record<string, unknown>).connectionId).toBe('')
  })

  it('matches an http profile by its connector, whatever it is called', () => {
    const requirement = { kind: 'httpProfile', nodeId: 'http-1', name: 'reporting API' } as const
    const profiles = [connection({ id: 'p1', name: 'anything', connectorId: 'http' })]
    expect(resolveRequirement(requirement, profiles)).toBe('p1')
  })

  it('cannot rebind a requirement the exporter could not name', () => {
    const p = toPortable(connectorWorkflow(), PROJECT)
    expect((p.requires ?? [])[0]).toMatchObject({ connectorId: '', name: '' })
    expect(resolveRequirement((p.requires ?? [])[0], [connection()])).toBeUndefined()
  })

  it('ignores a requirement for a node the file no longer carries', () => {
    const p = toPortable(connectorWorkflow(), PROJECT, [connection()])
    const trimmed = { ...p, nodes: p.nodes.filter((n) => n.id !== 'act-1') }
    expect(unresolvedRequirements(trimmed, []).map((r) => r.nodeId)).toEqual(['trigger-1'])
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

  describe('a webhook hook secret', () => {
    const hooked = () =>
      workflow({
        nodes: [
          {
            id: 'trigger-1',
            type: 'trigger',
            label: 'Webhook',
            config: { triggerType: 'webhook', method: 'POST', token: 'live-secret-token' },
            position: { x: 0, y: 0 }
          }
        ],
        edges: []
      })

    it('never travels in the file', () => {
      const p = toPortable(hooked(), PROJECT)
      const config = p.nodes[0].config as Record<string, unknown>
      expect(config.token).toBe('')
      expect(JSON.stringify(p)).not.toContain('live-secret-token')
    })

    it('is minted fresh for the machine importing it', () => {
      const p = toPortable(hooked(), PROJECT)
      const project = { name: 'N', path: '/Users/other/novum' }
      const first = fromPortable(p, 'novum', project, [], () => 'token-a')
      const second = fromPortable(p, 'novum', project, [], () => 'token-b')

      expect((first.nodes[0].config as Record<string, unknown>).token).toBe('token-a')
      expect((second.nodes[0].config as Record<string, unknown>).token).toBe('token-b')
    })

    it('leaves other triggers alone', () => {
      const imported = fromPortable(toPortable(workflow(), PROJECT), 'novum', {
        name: 'N',
        path: '/Users/other/novum'
      })
      expect((imported.nodes[0].config as Record<string, unknown>).token).toBeUndefined()
    })
  })
})
