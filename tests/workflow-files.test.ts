import { describe, it, expect } from 'vitest'
import {
  definitionFromFile,
  describeRequirement,
  fileFromWorkflow,
  placeImportedWorkflow,
  projectForWorkflow,
  workflowFileName,
  MAX_WORKFLOW_FILE_BYTES,
  WORKFLOW_FILE_SUFFIX
} from '../src/renderer/lib/workflow-files'
import type { SourceConnection, WorkflowDefinition } from '../packages/shared/src/types'

const PROJECT = { name: 'Novum', path: '/Users/someone/dev/novum' }

function workflow(overrides: Partial<WorkflowDefinition> = {}): WorkflowDefinition {
  return {
    id: 'abc-123',
    name: 'Nightly digest',
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
        id: 'run-1',
        type: 'script',
        label: 'Build',
        config: {
          scriptType: 'bash',
          projectName: 'Novum',
          projectPath: PROJECT.path,
          scriptContent: 'make'
        },
        position: { x: 0, y: 110 }
      }
    ],
    edges: [{ id: 'e1', source: 'trigger-1', target: 'run-1' }],
    ...overrides
  }
}

function connection(overrides: Partial<SourceConnection> = {}): SourceConnection {
  return {
    id: 'conn-1',
    name: 'workspace-eng',
    connectorId: 'github',
    filters: {},
    syncIntervalMinutes: 5,
    statusMapping: {},
    createdAt: '2026-09-01T00:00:00Z',
    ...overrides
  }
}

describe('naming the file', () => {
  it('names it for the workflow so a folder of them reads', () => {
    expect(workflowFileName(workflow())).toBe(`nightly-digest${WORKFLOW_FILE_SUFFIX}`)
  })
})

describe('projectForWorkflow', () => {
  it('finds the project the workflow points at', () => {
    expect(projectForWorkflow(workflow(), [PROJECT])).toEqual(PROJECT)
  })

  it('answers nothing when no registered project matches', () => {
    expect(projectForWorkflow(workflow(), [{ name: 'Other', path: '/o' }])).toBeUndefined()
  })

  it('falls back to the project in view when no step names one', () => {
    // A script step carries a cwd without a projectName; without a project to
    // be relative to, that path would travel as this machine's.
    const nameless = workflow({
      nodes: [
        {
          id: 'run-1',
          type: 'script',
          label: 'Build',
          config: { scriptType: 'bash', scriptContent: 'make', cwd: `${PROJECT.path}/sub` },
          position: { x: 0, y: 0 }
        }
      ]
    })
    expect(projectForWorkflow(nameless, [PROJECT], 'Novum')).toEqual(PROJECT)
  })

  it('prefers what the workflow names over what is in view', () => {
    const other = { name: 'Other', path: '/o' }
    expect(projectForWorkflow(workflow(), [PROJECT, other], 'Other')).toEqual(PROJECT)
  })
})

describe('fileFromWorkflow', () => {
  it('writes readable JSON that ends in a newline', () => {
    const file = fileFromWorkflow(workflow(), PROJECT.path, [])
    expect(file.contents.endsWith('\n')).toBe(true)
    expect(JSON.parse(file.contents).name).toBe('Nightly digest')
  })

  it('reports nothing machine-specific for a workflow inside its project', () => {
    expect(fileFromWorkflow(workflow(), PROJECT.path, []).residual).toEqual([])
  })

  it('keeps paths intact when no project could be resolved', () => {
    // An empty root must not rewrite every absolute path as if it sat inside it.
    const file = fileFromWorkflow(workflow(), '', [])
    const script = JSON.parse(file.contents).nodes[1].config
    expect(script.projectPath).toBe(PROJECT.path)
    expect(file.residual).toContain('run-1.projectPath')
  })
})

describe('definitionFromFile', () => {
  function exported(): string {
    return fileFromWorkflow(workflow(), PROJECT.path, []).contents
  }

  it('resolves a file against the importing project', () => {
    const result = definitionFromFile(exported(), { name: 'Other', path: '/o/code' }, 'novum', [])
    expect(result.ok).toBe(true)
    if (!result.ok) return
    const script = result.definition.nodes[1].config as Record<string, unknown>
    expect(script.projectPath).toBe('/o/code')
    expect(script.projectName).toBe('Other')
  })

  it('derives the same id every time, so a re-import updates in place', () => {
    const a = definitionFromFile(exported(), PROJECT, 'novum', [])
    const b = definitionFromFile(exported(), { name: 'X', path: '/x' }, 'novum', [])
    expect(a.ok && b.ok && a.definition.id === b.definition.id).toBe(true)
  })

  it('refuses a file that is not JSON', () => {
    const result = definitionFromFile('not json at all', PROJECT, 'novum', [])
    expect(result).toEqual({ ok: false, error: 'That file is not valid JSON' })
  })

  it('refuses a version this build does not read', () => {
    const bumped = JSON.stringify({ ...JSON.parse(exported()), version: 99 })
    const result = definitionFromFile(bumped, PROJECT, 'novum', [])
    expect(result.ok).toBe(false)
    if (result.ok) return
    expect(result.error).toContain('version 99')
  })

  it('refuses a file missing the graph', () => {
    const gutted = JSON.stringify({ version: 1, name: 'X', slug: 'x' })
    const result = definitionFromFile(gutted, PROJECT, 'novum', [])
    expect(result.ok).toBe(false)
    if (result.ok) return
    expect(result.error).toContain('missing a name, nodes or edges')
  })

  it('reports what a connector step still needs here', () => {
    const wf = workflow()
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
      position: { x: 0, y: 220 }
    })
    const contents = fileFromWorkflow(wf, PROJECT.path, [connection()]).contents

    const result = definitionFromFile(contents, PROJECT, 'novum', [])
    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(result.unresolved).toHaveLength(1)
    expect(describeRequirement(result.unresolved[0])).toBe(
      'a github connection like "workspace-eng"'
    )
  })

  it('binds the step and says nothing is pending when this machine has the connector', () => {
    const wf = workflow()
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
      position: { x: 0, y: 220 }
    })
    const contents = fileFromWorkflow(wf, PROJECT.path, [connection()]).contents

    const here = [connection({ id: 'mine', name: 'my-workspace' })]
    const result = definitionFromFile(contents, PROJECT, 'novum', here)
    expect(result.ok).toBe(true)
    if (!result.ok) return
    const action = result.definition.nodes.find((n) => n.id === 'act-1')!.config as Record<
      string,
      unknown
    >
    expect(action.connectionId).toBe('mine')
    expect(result.unresolved).toEqual([])
  })
})

describe('refusing a file before it becomes a workflow', () => {
  function exported(): string {
    return fileFromWorkflow(workflow(), PROJECT.path, []).contents
  }

  it('refuses more text than a workflow could be', () => {
    const huge = `${' '.repeat(MAX_WORKFLOW_FILE_BYTES)}${exported()}`
    expect(definitionFromFile(huge, PROJECT, 'novum', [])).toEqual({
      ok: false,
      error: 'That file is too large to be a workflow'
    })
  })

  it('refuses a step it cannot read, not merely a non-array', () => {
    const gutted = JSON.stringify({ ...JSON.parse(exported()), nodes: ['oops'], edges: [] })
    const result = definitionFromFile(gutted, PROJECT, 'novum', [])
    expect(result.ok).toBe(false)
    if (result.ok) return
    expect(result.error).toContain('step this build cannot read')
  })

  it('refuses an edge that names a step the file does not carry', () => {
    const parsed = JSON.parse(exported())
    parsed.edges = [{ id: 'e9', source: 'trigger-1', target: 'ghost' }]
    const result = definitionFromFile(JSON.stringify(parsed), PROJECT, 'novum', [])
    expect(result.ok).toBe(false)
    if (result.ok) return
    expect(result.error).toContain('connects a step it does not carry')
  })

  it('refuses an edge with no endpoints at all', () => {
    const parsed = JSON.parse(exported())
    parsed.edges = [{ id: 'e9' }]
    const result = definitionFromFile(JSON.stringify(parsed), PROJECT, 'novum', [])
    expect(result.ok).toBe(false)
    if (result.ok) return
    expect(result.error).toContain('connection this build cannot read')
  })

  it('still accepts the file it wrote', () => {
    expect(definitionFromFile(exported(), PROJECT, 'novum', []).ok).toBe(true)
  })
})

describe('an import lands switched off', () => {
  function exported(): string {
    return fileFromWorkflow(workflow(), PROJECT.path, []).contents
  }

  it('arrives disabled, whatever the file was exported from', () => {
    const result = definitionFromFile(exported(), PROJECT, 'novum', [])
    expect(result.ok && result.definition.enabled).toBe(false)
  })

  it('leaves a workflow that was already running alone', () => {
    const result = definitionFromFile(exported(), PROJECT, 'novum', [])
    if (!result.ok) throw new Error('expected a definition')
    const already = workflow({ id: result.definition.id, enabled: true, workspaceId: 'team' })

    expect(placeImportedWorkflow(result.definition, already, 'personal').enabled).toBe(true)
  })

  it('leaves a workflow that was switched off switched off', () => {
    const result = definitionFromFile(exported(), PROJECT, 'novum', [])
    if (!result.ok) throw new Error('expected a definition')
    const already = workflow({ id: result.definition.id, enabled: false })

    expect(placeImportedWorkflow(result.definition, already, 'personal').enabled).toBe(false)
  })

  it('steps aside from a workflow of another name holding its id', () => {
    const held = [{ id: 'import:novum:nightly-digest', name: 'Something else' }]
    const result = definitionFromFile(exported(), PROJECT, 'novum', [], held)
    expect(result.ok && result.definition.id).toBe('import:novum:nightly-digest-2')
  })

  it('updates in place when the name matches', () => {
    const held = [{ id: 'import:novum:nightly-digest', name: 'Nightly digest' }]
    const result = definitionFromFile(exported(), PROJECT, 'novum', [], held)
    expect(result.ok && result.definition.id).toBe('import:novum:nightly-digest')
  })
})

describe('describeRequirement', () => {
  it('names an http profile without pretending to know its connector', () => {
    expect(describeRequirement({ kind: 'httpProfile', nodeId: 'n', name: 'reporting API' })).toBe(
      'an HTTP profile like "reporting API"'
    )
    expect(describeRequirement({ kind: 'httpProfile', nodeId: 'n', name: '' })).toBe(
      'an HTTP profile'
    )
  })

  it('falls back to a plain connection when the file named neither', () => {
    expect(
      describeRequirement({ kind: 'connection', nodeId: 'n', connectorId: '', name: '' })
    ).toBe('a connector connection')
  })
})

describe('placeImportedWorkflow', () => {
  it('puts a new workflow in the workspace being viewed', () => {
    expect(placeImportedWorkflow(workflow(), undefined, 'team').workspaceId).toBe('team')
  })

  it('leaves a re-imported workflow in the workspace it already lived in', () => {
    const existing = workflow({ workspaceId: 'personal' })
    expect(placeImportedWorkflow(workflow(), existing, 'team').workspaceId).toBe('personal')
  })
})
