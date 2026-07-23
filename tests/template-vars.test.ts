import { describe, it, expect } from 'vitest'
import {
  slugify,
  ensureUniqueSlug,
  getAncestorNodes,
  buildStepGroups,
  resolveTemplateVars,
  resolveContextField,
  isContextRef,
  containsContextRef,
  getAvailableContextVars,
  CONTEXT_REF
} from '../src/renderer/lib/template-vars'
import type {
  TerminalSession,
  WorkflowNode,
  WorkflowEdge,
  WorkflowExecutionContext
} from '../src/shared/types'
import type { StepOutputs } from '../src/renderer/lib/template-vars'

function makeNode(id: string, type: string, slug?: string): WorkflowNode {
  return {
    id,
    type: type as WorkflowNode['type'],
    label: id,
    slug,
    config: { triggerType: 'manual' },
    position: { x: 0, y: 0 }
  }
}

function makeEdge(source: string, target: string): WorkflowEdge {
  return { id: `${source}->${target}`, source, target }
}

describe('slugify', () => {
  it('lowercases and replaces spaces with underscores', () => {
    expect(slugify('Launch Agent')).toBe('launch_agent')
  })

  it('strips special characters', () => {
    expect(slugify('Hello World!')).toBe('hello_world')
  })

  it('removes leading and trailing underscores', () => {
    expect(slugify('  hello  ')).toBe('hello')
  })

  it('collapses consecutive underscores', () => {
    expect(slugify('a---b___c')).toBe('a_b_c')
  })

  it('returns "step" for empty string', () => {
    expect(slugify('')).toBe('step')
  })

  it('returns "step" for string with only special chars', () => {
    expect(slugify('!!!')).toBe('step')
  })
})

describe('ensureUniqueSlug', () => {
  it('returns slug if not in set', () => {
    expect(ensureUniqueSlug('foo', new Set(['bar']))).toBe('foo')
  })

  it('appends _2 on first collision', () => {
    expect(ensureUniqueSlug('foo', new Set(['foo']))).toBe('foo_2')
  })

  it('appends _3 when _2 also exists', () => {
    expect(ensureUniqueSlug('foo', new Set(['foo', 'foo_2']))).toBe('foo_3')
  })
})

describe('getAncestorNodes', () => {
  it('returns empty for node with no predecessors', () => {
    const nodes = [makeNode('a', 'launchAgent')]
    expect(getAncestorNodes(nodes, [], 'a')).toEqual([])
  })

  it('returns predecessors in BFS order (excluding triggers)', () => {
    const nodes = [
      makeNode('t', 'trigger'),
      makeNode('a', 'launchAgent'),
      makeNode('b', 'launchAgent')
    ]
    const edges = [makeEdge('t', 'a'), makeEdge('a', 'b')]
    const result = getAncestorNodes(nodes, edges, 'b')
    expect(result).toHaveLength(1)
    expect(result[0].id).toBe('a')
  })

  it('excludes trigger nodes from results', () => {
    const nodes = [makeNode('t', 'trigger'), makeNode('a', 'launchAgent')]
    const edges = [makeEdge('t', 'a')]
    const result = getAncestorNodes(nodes, edges, 'a')
    expect(result).toEqual([])
  })

  it('handles diamond DAG without duplicates', () => {
    const nodes = [
      makeNode('t', 'trigger'),
      makeNode('a', 'launchAgent'),
      makeNode('b', 'launchAgent'),
      makeNode('c', 'launchAgent')
    ]
    const edges = [makeEdge('t', 'a'), makeEdge('t', 'b'), makeEdge('a', 'c'), makeEdge('b', 'c')]
    const result = getAncestorNodes(nodes, edges, 'c')
    expect(result).toHaveLength(2)
    const ids = result.map((n) => n.id).sort()
    expect(ids).toEqual(['a', 'b'])
  })
})

describe('buildStepGroups', () => {
  it('filters nodes without slug', () => {
    const nodes = [makeNode('a', 'launchAgent', 'step_a'), makeNode('b', 'launchAgent')]
    const groups = buildStepGroups(nodes)
    expect(groups).toHaveLength(1)
    expect(groups[0].slug).toBe('step_a')
  })

  it('maps to StepVariableGroup with default output keys', () => {
    const nodes = [makeNode('a', 'script', 'my_script')]
    const groups = buildStepGroups(nodes)
    expect(groups[0].keys).toHaveLength(3)
    expect(groups[0].keys.map((k) => k.key)).toEqual(['output', 'status', 'error'])
  })

  it('prepends schema-derived keys for callConnectorAction nodes when a lookup is provided', () => {
    const node: WorkflowNode = {
      id: 'n1',
      type: 'callConnectorAction',
      label: 'Create Issue',
      slug: 'create_issue',
      config: {
        nodeType: 'callConnectorAction',
        connectionId: 'c1',
        action: 'createIssue',
        args: {}
      },
      position: { x: 0, y: 0 }
    }
    const lookup = (cid: string, atype: string) =>
      cid === 'c1' && atype === 'createIssue'
        ? {
            type: 'createIssue',
            label: 'Create Issue',
            configFields: [],
            outputSchema: {
              type: 'object',
              properties: {
                html_url: { type: 'string', description: 'Issue URL' },
                number: { type: 'number' }
              }
            }
          }
        : undefined
    const groups = buildStepGroups([node], lookup)
    const keys = groups[0].keys.map((k) => k.key)
    expect(keys.slice(0, 2)).toEqual(['html_url', 'number'])
    expect(keys.slice(-3)).toEqual(['output', 'status', 'error'])
  })

  it('prepends outputSchema keys for launchAgent nodes', () => {
    const node: WorkflowNode = {
      id: 'n1',
      type: 'launchAgent',
      label: 'Review',
      slug: 'review',
      config: {
        agentType: 'claude',
        projectName: 'p',
        projectPath: '/p',
        headless: true,
        outputSchema: {
          type: 'object',
          properties: {
            verdict: { type: 'string', description: 'APPROVE or REQUEST_CHANGES' },
            tests_passed: { type: 'boolean' }
          },
          required: ['verdict']
        }
      },
      position: { x: 0, y: 0 }
    }
    const groups = buildStepGroups([node])
    const keys = groups[0].keys.map((k) => k.key)
    expect(keys.slice(0, 2)).toEqual(['verdict', 'tests_passed'])
    expect(keys.slice(-3)).toEqual(['output', 'status', 'error'])
  })

  it('keeps only default keys for a launchAgent node without a schema', () => {
    const node = makeNode('a', 'launchAgent', 'plain')
    node.config = {
      agentType: 'claude',
      projectName: 'p',
      projectPath: '/p'
    } as WorkflowNode['config']
    const groups = buildStepGroups([node])
    expect(groups[0].keys.map((k) => k.key)).toEqual(['output', 'status', 'error'])
  })
})

describe('resolveTemplateVars', () => {
  const context: WorkflowExecutionContext = {
    task: {
      id: 'abc123',
      projectName: 'MyProject',
      title: 'Fix bug',
      description: 'Fix the login bug',
      status: 'in_progress',
      order: 0,
      createdAt: '',
      updatedAt: ''
    },
    trigger: { type: 'taskStatusChanged', fromStatus: 'todo', toStatus: 'in_progress' }
  }

  it('resolves task variables', () => {
    expect(resolveTemplateVars('Title: {{task.title}}', context)).toBe('Title: Fix bug')
  })

  it('resolves trigger variables', () => {
    expect(resolveTemplateVars('From: {{trigger.fromStatus}}', context)).toBe('From: todo')
  })

  it('resolves step outputs', () => {
    const outputs: StepOutputs = { build: { output: 'success', status: 'ok' } }
    expect(resolveTemplateVars('Result: {{steps.build.output}}', context, outputs)).toBe(
      'Result: success'
    )
  })

  it('returns empty for missing step output', () => {
    const outputs: StepOutputs = {}
    expect(resolveTemplateVars('{{steps.unknown.output}}', context, outputs)).toBe('')
  })

  it('returns match verbatim for unknown namespace', () => {
    expect(resolveTemplateVars('{{unknown.key}}', context)).toBe('{{unknown.key}}')
  })

  it('returns empty string for empty template', () => {
    expect(resolveTemplateVars('', context)).toBe('')
  })

  it('returns template unchanged when no context provided', () => {
    expect(resolveTemplateVars('{{task.title}}')).toBe('{{task.title}}')
  })

  it('truncates step output longer than 50k chars', () => {
    const longOutput = 'x'.repeat(60_000)
    const outputs: StepOutputs = { build: { output: longOutput } }
    const result = resolveTemplateVars('{{steps.build.output}}', context, outputs)
    expect(result.length).toBe(50_000)
  })

  it('walks nested paths into a step output object', () => {
    const outputs: StepOutputs = {
      create_issue: { issue: { id: 7, html_url: 'https://gh/x/1' } }
    }
    expect(
      resolveTemplateVars('Url: {{steps.create_issue.issue.html_url}}', context, outputs)
    ).toBe('Url: https://gh/x/1')
  })

  it('JSON-stringifies object/array leaves', () => {
    const outputs: StepOutputs = {
      list_dir: { entries: [{ name: 'a' }, { name: 'b' }] }
    }
    expect(resolveTemplateVars('{{steps.list_dir.entries}}', context, outputs)).toBe(
      '[{"name":"a"},{"name":"b"}]'
    )
  })

  it('returns empty when a nested path segment is missing', () => {
    const outputs: StepOutputs = { x: { a: { b: 1 } } }
    expect(resolveTemplateVars('{{steps.x.a.missing}}', context, outputs)).toBe('')
  })
})

describe('isContextRef', () => {
  it('matches every CONTEXT_REF sentinel', () => {
    for (const ref of Object.values(CONTEXT_REF)) {
      expect(isContextRef(ref)).toBe(true)
    }
  })
  it('tolerates surrounding whitespace and inner spaces', () => {
    expect(isContextRef('  {{ context.cwd }}  ')).toBe(true)
  })
  it('rejects non-context templates and plain strings', () => {
    expect(isContextRef('{{task.title}}')).toBe(false)
    expect(isContextRef('plain folder')).toBe(false)
    expect(isContextRef(undefined)).toBe(false)
    expect(isContextRef('')).toBe(false)
  })
  it('rejects strings that contain but are not equal to a context ref', () => {
    expect(isContextRef('Run in {{context.cwd}}')).toBe(false)
  })
})

describe('containsContextRef', () => {
  it('matches an embedded context ref anywhere in the value', () => {
    expect(containsContextRef('Run in {{context.cwd}} now')).toBe(true)
  })
  it('honors the field filter', () => {
    expect(containsContextRef('cd {{context.cwd}}', 'cwd')).toBe(true)
    expect(containsContextRef('cd {{context.cwd}}', 'branch')).toBe(false)
  })
  it('returns false for non-string and empty input', () => {
    expect(containsContextRef(undefined)).toBe(false)
    expect(containsContextRef('')).toBe(false)
  })
})

describe('resolveContextField', () => {
  const task = {
    id: 't1',
    projectName: 'Vorn',
    title: 'card',
    description: '',
    status: 'in_progress' as const,
    order: 0,
    branch: 'feature/x',
    useWorktree: true,
    worktreePath: '/wt/feature-x',
    createdAt: '',
    updatedAt: ''
  }
  const source: TerminalSession = {
    id: 's1',
    agentType: 'shell',
    projectName: 'Other',
    projectPath: '/repo/other',
    status: 'idle',
    createdAt: 0,
    pid: 0,
    branch: 'main',
    isWorktree: false
  }

  it('reads cwd from task worktree first', () => {
    expect(resolveContextField('cwd', { task })).toBe('/wt/feature-x')
  })
  it('reads cwd from source projectPath when task has no worktree', () => {
    const ctx: WorkflowExecutionContext = { source }
    expect(resolveContextField('cwd', ctx)).toBe('/repo/other')
  })
  it('prefers source.worktreePath over source.projectPath for cwd', () => {
    const wtSource = { ...source, worktreePath: '/wt/other-feat', isWorktree: true }
    expect(resolveContextField('cwd', { source: wtSource })).toBe('/wt/other-feat')
  })
  it('reads projectName from task before source', () => {
    expect(resolveContextField('projectName', { task, source })).toBe('Vorn')
  })
  it('reads projectName from source when task has none', () => {
    expect(resolveContextField('projectName', { source })).toBe('Other')
  })
  it('reads branch with task taking precedence', () => {
    expect(resolveContextField('branch', { task, source })).toBe('feature/x')
    expect(resolveContextField('branch', { source })).toBe('main')
  })
  it('reads useWorktree as boolean from either side', () => {
    expect(resolveContextField('useWorktree', { task })).toBe(true)
    const sourceWt = { ...source, isWorktree: true }
    expect(resolveContextField('useWorktree', { source: sourceWt })).toBe(true)
    expect(resolveContextField('useWorktree', { source })).toBe(false)
  })
  it('returns undefined for unknown fields', () => {
    expect(resolveContextField('bogus', { task })).toBeUndefined()
  })
  it('returns undefined when context has neither task nor source', () => {
    expect(resolveContextField('cwd', {})).toBeUndefined()
  })
})

describe('resolveTemplateVars — context namespace', () => {
  const task = {
    id: 't1',
    projectName: 'Vorn',
    title: '',
    description: '',
    status: 'in_progress' as const,
    order: 0,
    branch: 'feat/a',
    worktreePath: '/wt/a',
    createdAt: '',
    updatedAt: ''
  }
  it('expands {{context.cwd}} from task', () => {
    expect(resolveTemplateVars('cd {{context.cwd}}', { task })).toBe('cd /wt/a')
  })
  it('expands {{context.branch}}', () => {
    expect(resolveTemplateVars('on {{context.branch}}', { task })).toBe('on feat/a')
  })
  it('returns empty for {{context.*}} when no source available', () => {
    expect(resolveTemplateVars('cd {{context.cwd}}', {})).toBe('cd ')
  })
})

describe('getAvailableContextVars', () => {
  it('returns context vars only when trigger is contextual', () => {
    const vars = getAvailableContextVars({ triggerType: 'manual', isContextualTrigger: true })
    expect(vars.every((v) => v.category === 'context')).toBe(true)
    expect(vars.length).toBeGreaterThan(0)
  })
  it('returns task vars for taskCreated trigger', () => {
    const vars = getAvailableContextVars({ triggerType: 'taskCreated', isContextualTrigger: false })
    expect(vars.some((v) => v.category === 'task')).toBe(true)
  })
  it('returns task + trigger + context vars for a contextual taskStatusChanged', () => {
    const vars = getAvailableContextVars({
      triggerType: 'taskStatusChanged',
      isContextualTrigger: true
    })
    const cats = new Set(vars.map((v) => v.category))
    expect(cats.has('task')).toBe(true)
    expect(cats.has('trigger')).toBe(true)
    expect(cats.has('context')).toBe(true)
  })
  it('returns empty list for plain manual non-contextual trigger', () => {
    const vars = getAvailableContextVars({ triggerType: 'manual', isContextualTrigger: false })
    expect(vars).toEqual([])
  })
})
