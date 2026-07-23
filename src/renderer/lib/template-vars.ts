import {
  TaskConfig,
  TriggerConfig,
  WorkflowExecutionContext,
  WorkflowNode,
  WorkflowEdge,
  CallConnectorActionConfig,
  LaunchAgentConfig,
  ConnectorActionDef
} from '../../shared/types'
import { schemaProperties, schemaTypeHint } from '../../shared/json-schema-utils'

// --- Slug Utilities ---

export function slugify(label: string): string {
  return (
    label
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, '_')
      .replace(/^_|_$/g, '')
      .replace(/_+/g, '_') || 'step'
  )
}

export function ensureUniqueSlug(slug: string, existing: Set<string>): string {
  if (!existing.has(slug)) return slug
  let i = 2
  while (existing.has(`${slug}_${i}`)) i++
  return `${slug}_${i}`
}

// --- Step Output Types ---

/**
 * Per-step output map. Top-level keys are always strings (output/status/error
 * plus any connector-declared fields), but values can be arbitrary nested
 * objects/arrays that the template resolver walks into via dotted paths.
 */
export type StepOutputs = Record<string, Record<string, unknown>>

export const DEFAULT_OUTPUT_KEYS = [
  { key: 'output', label: 'Output', description: 'Primary output (stdout / agent logs)' },
  { key: 'status', label: 'Status', description: 'success or error' },
  { key: 'error', label: 'Error', description: 'Error message if failed' }
] as const

// --- Variable Group for Autocomplete UI ---

export interface StepVariableGroup {
  nodeId: string
  label: string
  slug: string
  nodeType: string
  disabled?: boolean
  keys: { key: string; label: string; description: string }[]
}

// --- Template Variable Types ---

export interface TemplateVariable {
  key: string
  label: string
  category: 'task' | 'trigger' | 'connectorItem' | 'context'
}

export const TEMPLATE_VARIABLES: TemplateVariable[] = [
  { key: '{{task.title}}', label: 'Title', category: 'task' },
  { key: '{{task.description}}', label: 'Description', category: 'task' },
  { key: '{{task.id}}', label: 'ID', category: 'task' },
  { key: '{{task.status}}', label: 'Status', category: 'task' },
  { key: '{{task.branch}}', label: 'Branch', category: 'task' },
  { key: '{{task.projectName}}', label: 'Project', category: 'task' },
  { key: '{{trigger.fromStatus}}', label: 'Previous Status', category: 'trigger' },
  { key: '{{trigger.toStatus}}', label: 'New Status', category: 'trigger' },
  { key: '{{connectorItem.externalId}}', label: 'External ID', category: 'connectorItem' },
  { key: '{{connectorItem.title}}', label: 'Item Title', category: 'connectorItem' },
  { key: '{{connectorItem.externalUrl}}', label: 'Item URL', category: 'connectorItem' },
  { key: '{{connectorItem.body}}', label: 'Item Body', category: 'connectorItem' },
  { key: '{{connectorItem.connectorId}}', label: 'Connector', category: 'connectorItem' },
  { key: '{{context.cwd}}', label: 'Working Directory', category: 'context' },
  { key: '{{context.projectPath}}', label: 'Project Path', category: 'context' },
  { key: '{{context.projectName}}', label: 'Project Name', category: 'context' },
  { key: '{{context.branch}}', label: 'Branch', category: 'context' },
  { key: '{{context.worktreePath}}', label: 'Worktree Path', category: 'context' }
]

/**
 * Sentinels written into LaunchAgent / Script string fields when the user
 * picks "From Context" in the editor. The runtime template resolver expands
 * these against `WorkflowExecutionContext.task` (card) or `.source` (terminal).
 */
export const CONTEXT_REF = {
  cwd: '{{context.cwd}}',
  projectPath: '{{context.projectPath}}',
  projectName: '{{context.projectName}}',
  branch: '{{context.branch}}',
  worktreePath: '{{context.worktreePath}}'
} as const

export function isContextRef(value: string | undefined): boolean {
  return typeof value === 'string' && /^\s*\{\{\s*context\.[\w]+\s*\}\}\s*$/.test(value)
}

/**
 * Variables available in autocomplete given the current trigger flags. Used
 * by both LaunchAgentConfigForm and ScriptConfigForm.
 */
export function getAvailableContextVars(opts: {
  triggerType: TriggerConfig['triggerType'] | undefined
  isContextualTrigger: boolean
}): TemplateVariable[] {
  const isTaskTrigger =
    opts.triggerType === 'taskCreated' || opts.triggerType === 'taskStatusChanged'
  return TEMPLATE_VARIABLES.filter((v) => {
    if (isTaskTrigger && v.category === 'task') return true
    if (isTaskTrigger && v.category === 'trigger' && opts.triggerType === 'taskStatusChanged') {
      return true
    }
    if (opts.isContextualTrigger && v.category === 'context') return true
    return false
  })
}

/** Whether `value` contains a `{{context.<field>}}` reference anywhere. */
export function containsContextRef(value: string | undefined, field?: string): boolean {
  if (typeof value !== 'string') return false
  const pattern = field
    ? new RegExp(`\\{\\{\\s*context\\.${field}\\s*\\}\\}`)
    : /\{\{\s*context\.[\w]+\s*\}\}/
  return pattern.test(value)
}

// --- DAG Ancestor Computation ---

export function getAncestorNodes(
  nodes: WorkflowNode[],
  edges: WorkflowEdge[],
  currentNodeId: string
): WorkflowNode[] {
  const nodeMap = new Map(nodes.map((n) => [n.id, n]))
  const predecessorsMap = new Map<string, string[]>()
  for (const edge of edges) {
    const preds = predecessorsMap.get(edge.target) || []
    preds.push(edge.source)
    predecessorsMap.set(edge.target, preds)
  }

  const ancestors: WorkflowNode[] = []
  const visited = new Set<string>()
  const queue = [currentNodeId]
  visited.add(currentNodeId)

  while (queue.length > 0) {
    const id = queue.shift()!
    const preds = predecessorsMap.get(id) || []
    for (const predId of preds) {
      if (visited.has(predId)) continue
      visited.add(predId)
      queue.push(predId)
      const node = nodeMap.get(predId)
      if (node && node.type !== 'trigger') {
        ancestors.push(node)
      }
    }
  }

  return ancestors
}

/** Synchronous lookup of a connector action by (connectionId, actionType).
 *  The WorkflowEditor prefetches actions for every connection referenced
 *  in the workflow so this can be called inside a `useMemo`. */
export type ConnectorActionLookup = (
  connectionId: string,
  actionType: string
) => ConnectorActionDef | undefined

function schemaTopLevelKeys(
  schema: Record<string, unknown> | undefined
): { key: string; label: string; description: string }[] {
  return Object.entries(schemaProperties(schema)).map(([key, raw]) => {
    const description = (raw as { description?: string }).description
    return {
      key,
      label: key,
      description: description || schemaTypeHint(raw) || ''
    }
  })
}

export function buildStepGroups(
  ancestorNodes: WorkflowNode[],
  lookupAction?: ConnectorActionLookup
): StepVariableGroup[] {
  return ancestorNodes
    .filter((n) => n.slug)
    .map((n) => {
      const defaultKeys: { key: string; label: string; description: string }[] =
        DEFAULT_OUTPUT_KEYS.map((k) => ({ ...k }))
      let keys = defaultKeys
      if (n.type === 'callConnectorAction' && lookupAction) {
        const cfg = n.config as CallConnectorActionConfig
        if (cfg.connectionId && cfg.action) {
          const action = lookupAction(cfg.connectionId, cfg.action)
          const schemaKeys = schemaTopLevelKeys(action?.outputSchema)
          if (schemaKeys.length > 0) {
            // Schema-derived keys first — those are what users will usually
            // reach for. The three defaults stay at the bottom as fallbacks.
            keys = [...schemaKeys, ...defaultKeys]
          }
        }
      } else if (n.type === 'launchAgent') {
        // A launchAgent node with a declared outputSchema surfaces its typed
        // fields the same way — `{{steps.<slug>.<field>}}` — populated at run
        // time from the agent's parsed structuredOutput.
        const cfg = n.config as LaunchAgentConfig
        const schemaKeys = schemaTopLevelKeys(cfg.outputSchema)
        if (schemaKeys.length > 0) {
          keys = [...schemaKeys, ...defaultKeys]
        }
      }
      return {
        nodeId: n.id,
        label: n.label,
        slug: n.slug!,
        nodeType: n.type,
        keys
      }
    })
}

// --- Template Variable Resolution ---

const MAX_OUTPUT_LENGTH = 50_000

/** Walk a dotted path into a nested value. Stops at undefined/null. */
function walkPath(root: unknown, path: string[]): unknown {
  let current: unknown = root
  for (const segment of path) {
    if (current == null) return undefined
    if (typeof current !== 'object') return undefined
    current = (current as Record<string, unknown>)[segment]
  }
  return current
}

/** Turn a resolved value into the string that gets substituted into the
 *  template. Scalars stringify directly; objects/arrays get JSON-serialized
 *  so downstream prompts / args still receive something meaningful. */
function stringifyResolved(val: unknown): string {
  if (val == null) return ''
  if (typeof val === 'string') {
    return val.length > MAX_OUTPUT_LENGTH ? val.slice(-MAX_OUTPUT_LENGTH) : val
  }
  if (typeof val === 'number' || typeof val === 'boolean') return String(val)
  try {
    const serialized = JSON.stringify(val)
    return serialized.length > MAX_OUTPUT_LENGTH ? serialized.slice(-MAX_OUTPUT_LENGTH) : serialized
  } catch {
    return String(val)
  }
}

export function resolveTemplateVars(
  template: string,
  context?: WorkflowExecutionContext,
  stepOutputs?: StepOutputs
): string {
  if (!template) return template
  if (!context && !stepOutputs) return template

  // `{{ ns.k1.k2.k3... }}` — identifier-first, then any number of dotted
  // segments. The resolver walks those segments into whichever namespace
  // matches (steps / task / trigger / connectorItem).
  return template.replace(/\{\{\s*([a-zA-Z_][\w.]*)\s*\}\}/g, (match, path: string) => {
    const segments = path.split('.')
    const ns = segments[0]
    const rest = segments.slice(1)
    if (rest.length === 0) return match

    if (ns === 'steps' && stepOutputs) {
      const [stepName, ...keyPath] = rest
      const stepData = stepOutputs[stepName]
      if (!stepData) return ''
      if (keyPath.length === 0) return stringifyResolved(stepData)
      return stringifyResolved(walkPath(stepData, keyPath))
    }
    if (ns === 'task' && context?.task) {
      if (rest.length === 1) {
        const val = context.task[rest[0] as keyof TaskConfig]
        return val != null ? String(val) : ''
      }
      return stringifyResolved(walkPath(context.task, rest))
    }
    if (ns === 'trigger' && context?.trigger) {
      return stringifyResolved(walkPath(context.trigger, rest))
    }
    if (ns === 'connectorItem' && context?.connectorItem) {
      return stringifyResolved(walkPath(context.connectorItem, rest))
    }
    if (ns === 'context' && rest.length === 1 && context) {
      const resolved = resolveContextField(rest[0], context)
      return resolved != null ? String(resolved) : ''
    }

    return match
  })
}

/**
 * Resolve a `{{context.*}}` field. The workflow runtime synthesizes a
 * `context.source` (TerminalSession-shaped) for card launches by looking up
 * the task's project, so all path-like fields read from `source`. `task`
 * remains the primary source for branch / projectName / worktree.
 *
 * Returns `undefined` (rather than throwing) when neither `task` nor `source`
 * is present so callers can detect missing context and route through
 * SourcePromptDialog.
 */
export function resolveContextField(
  field: string,
  context: WorkflowExecutionContext
): string | boolean | undefined {
  const task = context.task
  const source = context.source
  switch (field) {
    case 'cwd':
      return task?.worktreePath ?? source?.worktreePath ?? source?.projectPath
    case 'projectPath':
      return source?.projectPath
    case 'projectName':
      return task?.projectName ?? source?.projectName
    case 'branch':
      return task?.branch ?? source?.branch
    case 'worktreePath':
      return task?.worktreePath ?? source?.worktreePath
    case 'useWorktree':
      if (task) return task.useWorktree ?? !!task.worktreePath
      if (source) return source.isWorktree ?? !!source.worktreePath
      return undefined
    default:
      return undefined
  }
}
