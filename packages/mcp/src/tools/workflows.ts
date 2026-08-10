import crypto from 'node:crypto'
import { z } from 'zod'
import type { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import { V } from '../validation'
import type {
  WorkflowDefinition,
  WorkflowNode,
  WorkflowEdge,
  TriggerConfig,
  LaunchAgentConfig,
  WorkflowInputDef
} from '@vornrun/shared/types'
import type { ScheduleLogEntry } from '@vornrun/shared/types'
import {
  dbListWorkflows,
  dbInsertWorkflow,
  dbUpdateWorkflow,
  dbDeleteWorkflow,
  listWorkflowRuns,
  listWorkflowRunsByTask,
  dbSignalChange
} from '@vornrun/server/database'
import { rpcCall } from '../ws-client'

const launchAgentConfigSchema = z
  .object({
    agentType: z.enum(['claude', 'copilot', 'codex', 'opencode', 'gemini']),
    projectName: V.name,
    projectPath: V.absolutePath,
    args: z.array(V.shortText).optional(),
    displayName: V.shortText.optional(),
    branch: V.shortText.optional(),
    // boolean | 'fromContext' — `'fromContext'` only valid when the workflow's
    // manual trigger is contextual (validated at runtime, not here).
    useWorktree: z.union([z.boolean(), z.literal('fromContext')]).optional(),
    remoteHostId: V.id.optional(),
    prompt: V.prompt.optional(),
    promptDelayMs: z.number().optional(),
    taskId: V.id.optional(),
    taskFromQueue: z.boolean().optional(),
    // Runs the agent in the background and waits for it to finish (required for
    // typed output). Opens a terminal tab when false/omitted.
    headless: z.boolean().optional(),
    // JSON Schema the agent's final answer must satisfy (headless only). When set,
    // the engine parses a matching object from the run and exposes its fields as
    // `{{steps.<slug>.<field>}}` for downstream condition nodes.
    outputSchema: z.record(z.string(), z.unknown()).optional()
  })
  .refine((c) => !c.outputSchema || c.headless === true, {
    // The engine only parses typed output for headless runs; reject a config that
    // declares a schema it would silently ignore instead of accepting a lie.
    message: 'outputSchema requires headless: true',
    path: ['outputSchema']
  })

// Parameters the run dialog prompts for, declared on the manual trigger so they
// travel with the definition. Without this here, a workflow authored over MCP
// could never declare the `{{inputs.*}}` it reads.
const workflowInputDefSchema = z.object({
  key: z
    .string()
    .regex(
      /^[A-Za-z_][A-Za-z0-9_]*$/,
      'key must be a valid identifier — it becomes {{inputs.<key>}}'
    )
    .max(100),
  label: V.shortText,
  type: z.enum(['text', 'textarea', 'number', 'select', 'boolean', 'project', 'branch']),
  required: z.boolean().optional(),
  defaultValue: V.shortText.optional(),
  options: z.array(z.object({ value: V.shortText, label: V.shortText })).optional(),
  placeholder: V.shortText.optional(),
  description: V.shortText.optional()
})

const triggerConfigSchema = z.union([
  z.object({
    triggerType: z.literal('manual'),
    contextual: z.boolean().optional(),
    inputs: z.array(workflowInputDefSchema).optional()
  }),
  z.object({ triggerType: z.literal('once'), runAt: V.shortText }),
  z.object({
    triggerType: z.literal('recurring'),
    cron: V.shortText,
    timezone: V.shortText.optional()
  }),
  z.object({ triggerType: z.literal('taskCreated'), projectFilter: V.name.optional() }),
  z.object({
    triggerType: z.literal('taskStatusChanged'),
    projectFilter: V.name.optional(),
    fromStatus: z.enum(['todo', 'in_progress', 'in_review', 'done', 'cancelled']).optional(),
    toStatus: z.enum(['todo', 'in_progress', 'in_review', 'done', 'cancelled']).optional()
  })
])

const nodeSchema = z.object({
  id: V.id,
  // Full node palette — parity with the editor. `config` is a passthrough so
  // each type carries its own shape (ConditionConfig, ApprovalConfig, etc.).
  type: z.enum([
    'trigger',
    'launchAgent',
    'script',
    'condition',
    'approval',
    'createTaskFromItem',
    'callConnectorAction'
  ]),
  label: V.shortText,
  // Referenced by typed step vars as `{{steps.<slug>.<field>}}`. Set one on any
  // node whose output a later node consumes.
  slug: V.shortText.optional(),
  config: z.record(z.string(), z.unknown()),
  position: z.object({ x: z.number(), y: z.number() })
})

const edgeSchema = z.object({
  id: V.id,
  source: V.id,
  target: V.id,
  // Which branch of a `condition` node this edge represents. Omit for normal
  // edges; required to wire both outcomes of a condition.
  conditionBranch: z.enum(['true', 'false']).optional()
})

/**
 * Build a workflow graph from a flat action list + trigger config (convenience format).
 */
function buildGraphFromFlat(
  trigger: TriggerConfig,
  actions: LaunchAgentConfig[]
): { nodes: WorkflowNode[]; edges: WorkflowEdge[] } {
  const nodes: WorkflowNode[] = []
  const edges: WorkflowEdge[] = []

  const triggerNode: WorkflowNode = {
    id: crypto.randomUUID(),
    type: 'trigger',
    label:
      trigger.triggerType === 'manual'
        ? 'Manual Trigger'
        : trigger.triggerType === 'once'
          ? 'Schedule (Once)'
          : trigger.triggerType === 'recurring'
            ? 'Schedule (Recurring)'
            : trigger.triggerType === 'taskCreated'
              ? 'When Task Created'
              : trigger.triggerType === 'taskStatusChanged'
                ? 'When Task Status Changes'
                : 'Trigger',
    config: trigger,
    position: { x: 0, y: 0 }
  }
  nodes.push(triggerNode)

  let prevId = triggerNode.id
  const NODE_GAP = 140

  for (let i = 0; i < actions.length; i++) {
    const action = actions[i]
    const nodeId = crypto.randomUUID()
    nodes.push({
      id: nodeId,
      type: 'launchAgent',
      label: `Launch ${action.agentType}`,
      config: action,
      position: { x: 0, y: (i + 1) * NODE_GAP }
    })
    edges.push({
      id: crypto.randomUUID(),
      source: prevId,
      target: nodeId
    })
    prevId = nodeId
  }

  return { nodes, edges }
}

/**
 * Match supplied values against the parameters a workflow declares.
 *
 * The run dialog does this for a human — applies defaults, enforces required,
 * limits a select to its options. An agent calling the tool gets no dialog, so
 * without this a typo'd key would sail through and surface as an empty
 * `{{inputs.*}}` somewhere deep in the run, which is a miserable thing to debug.
 */
export function resolveWorkflowInputs(
  defs: WorkflowInputDef[],
  supplied: Record<string, string | number | boolean>
): { values: Record<string, unknown>; errors: string[] } {
  const errors: string[] = []
  const known = new Set(defs.map((d) => d.key))

  for (const key of Object.keys(supplied)) {
    if (!known.has(key)) {
      errors.push(
        `unknown input "${key}" — this workflow declares: ${Array.from(known).join(', ') || '(none)'}`
      )
    }
  }

  const values: Record<string, unknown> = {}
  for (const def of defs) {
    const provided = Object.prototype.hasOwnProperty.call(supplied, def.key)
    const raw = provided ? supplied[def.key] : def.defaultValue

    if (raw === undefined || raw === '') {
      if (def.required) errors.push(`missing required input "${def.key}" (${def.label})`)
      continue
    }

    switch (def.type) {
      case 'number': {
        const n = typeof raw === 'number' ? raw : Number(raw)
        if (Number.isNaN(n)) {
          errors.push(`input "${def.key}" must be a number, got ${JSON.stringify(raw)}`)
        } else {
          values[def.key] = n
        }
        break
      }
      case 'boolean': {
        if (typeof raw === 'boolean') values[def.key] = raw
        else if (raw === 'true' || raw === 'false') values[def.key] = raw === 'true'
        else errors.push(`input "${def.key}" must be a boolean, got ${JSON.stringify(raw)}`)
        break
      }
      case 'select': {
        const allowed = (def.options ?? []).map((o) => o.value)
        if (allowed.length > 0 && !allowed.includes(String(raw))) {
          errors.push(`input "${def.key}" must be one of: ${allowed.join(', ')}`)
        } else {
          values[def.key] = String(raw)
        }
        break
      }
      default:
        values[def.key] = String(raw)
    }
  }

  return { values, errors }
}

/**
 * The workflow tools grew two names for one argument: the read tools take
 * `workflow_id`, while update/delete took `id`. Nothing signals which a given
 * tool wants, so reaching for the wrong one costs a failed call and a re-read
 * of the schema.
 *
 * `workflow_id` is canonical now — it is what the majority already used.
 * `id` keeps working, because breaking every existing caller to win
 * consistency would be a poor trade.
 */
export function resolveWorkflowId(args: {
  workflow_id?: string
  id?: string
}): { id: string } | { error: string } {
  const id = args.workflow_id ?? args.id
  if (!id) return { error: 'provide workflow_id' }
  if (args.workflow_id && args.id && args.workflow_id !== args.id) {
    return { error: 'workflow_id and id disagree — pass only workflow_id' }
  }
  return { id }
}

export function registerWorkflowTools(server: McpServer): void {
  server.tool(
    'list_workflows',
    'List all workflows, optionally filtered by workspace',
    {
      workspace_id: V.id.optional().describe('Filter by workspace ID')
    },
    async (args) => {
      let workflows = dbListWorkflows()
      if (args.workspace_id) {
        workflows = workflows.filter((w) => (w.workspaceId ?? 'personal') === args.workspace_id)
      }
      return { content: [{ type: 'text', text: JSON.stringify(workflows, null, 2) }] }
    }
  )

  server.tool(
    'create_workflow',
    'Create a new workflow. Accepts either full nodes/edges (advanced mode — every node ' +
      'type is supported: trigger, launchAgent, script, condition, approval, ' +
      'createTaskFromItem, callConnectorAction; wire condition outcomes with edge ' +
      'conditionBranch "true"/"false") or a convenience flat format (trigger + actions ' +
      'array). Give a node a slug to reference its output downstream as {{steps.<slug>.<field>}}. ' +
      'A headless launchAgent with an outputSchema returns typed fields for condition nodes.',
    {
      name: V.title.describe('Workflow name'),
      trigger: triggerConfigSchema
        .optional()
        .describe('Trigger config (convenience mode). Defaults to manual.'),
      actions: z
        .array(launchAgentConfigSchema)
        .optional()
        .describe('Actions to execute (convenience mode). Auto-generates graph.'),
      nodes: z.array(nodeSchema).optional().describe('Full graph nodes (advanced mode)'),
      edges: z.array(edgeSchema).optional().describe('Full graph edges (advanced mode)'),
      icon: V.shortText.optional().describe('Lucide icon name (default: zap)'),
      icon_color: V.hexColor.optional().describe('Hex color (default: #6366f1)'),
      enabled: z.boolean().optional().describe('Whether workflow is enabled (default: true)'),
      stagger_delay_ms: z.number().optional().describe('Delay in ms between actions')
    },
    async (args) => {
      let nodes: WorkflowNode[]
      let edges: WorkflowEdge[]

      if (args.nodes && args.edges) {
        nodes = args.nodes as unknown as WorkflowNode[]
        edges = args.edges as unknown as WorkflowEdge[]
      } else {
        const trigger = (args.trigger as TriggerConfig) ?? { triggerType: 'manual' as const }
        const actions = (args.actions as LaunchAgentConfig[]) ?? []
        const graph = buildGraphFromFlat(trigger, actions)
        nodes = graph.nodes
        edges = graph.edges
      }

      const workflow: WorkflowDefinition = {
        id: crypto.randomUUID(),
        name: args.name,
        icon: args.icon ?? 'Zap',
        iconColor: args.icon_color ?? '#6366f1',
        nodes,
        edges,
        enabled: args.enabled ?? true,
        ...(args.stagger_delay_ms && { staggerDelayMs: args.stagger_delay_ms })
      }

      dbInsertWorkflow(workflow)
      dbSignalChange()

      return { content: [{ type: 'text', text: JSON.stringify(workflow, null, 2) }] }
    }
  )

  server.tool(
    'update_workflow',
    "Update a workflow's properties",
    {
      workflow_id: V.id.optional().describe('Workflow ID (from list_workflows)'),
      id: V.id.optional().describe('Deprecated alias for workflow_id'),
      name: V.title.optional(),
      nodes: z.array(nodeSchema).optional(),
      edges: z.array(edgeSchema).optional(),
      icon: V.shortText.optional(),
      icon_color: V.hexColor.optional(),
      enabled: z.boolean().optional(),
      stagger_delay_ms: z.number().optional()
    },
    async (args) => {
      const resolved = resolveWorkflowId(args)
      if ('error' in resolved) {
        return { content: [{ type: 'text', text: `Error: ${resolved.error}` }], isError: true }
      }
      const workflows = dbListWorkflows()
      const workflow = workflows.find((w) => w.id === resolved.id)
      if (!workflow) {
        return {
          content: [{ type: 'text', text: `Error: workflow "${resolved.id}" not found` }],
          isError: true
        }
      }

      const updates: Partial<WorkflowDefinition> = {}
      if (args.name !== undefined) updates.name = args.name
      if (args.nodes !== undefined) updates.nodes = args.nodes as unknown as WorkflowNode[]
      if (args.edges !== undefined) updates.edges = args.edges as unknown as WorkflowEdge[]
      if (args.icon !== undefined) updates.icon = args.icon
      if (args.icon_color !== undefined) updates.iconColor = args.icon_color
      if (args.enabled !== undefined) updates.enabled = args.enabled
      if (args.stagger_delay_ms !== undefined) updates.staggerDelayMs = args.stagger_delay_ms

      dbUpdateWorkflow(resolved.id, updates)
      dbSignalChange()

      return {
        content: [{ type: 'text', text: JSON.stringify({ ...workflow, ...updates }, null, 2) }]
      }
    }
  )

  server.tool(
    'delete_workflow',
    'Delete a workflow',
    {
      workflow_id: V.id.optional().describe('Workflow ID (from list_workflows)'),
      id: V.id.optional().describe('Deprecated alias for workflow_id')
    },
    async (args) => {
      const resolved = resolveWorkflowId(args)
      if ('error' in resolved) {
        return { content: [{ type: 'text', text: `Error: ${resolved.error}` }], isError: true }
      }
      const workflows = dbListWorkflows()
      const workflow = workflows.find((w) => w.id === resolved.id)
      if (!workflow) {
        return {
          content: [{ type: 'text', text: `Error: workflow "${resolved.id}" not found` }],
          isError: true
        }
      }

      dbDeleteWorkflow(resolved.id)
      dbSignalChange()

      return { content: [{ type: 'text', text: `Deleted workflow: ${workflow.name}` }] }
    }
  )

  server.tool(
    'list_workflow_runs',
    'List workflow execution history. Filter by workflow_id or task_id.',
    {
      workflow_id: V.id.optional().describe('Filter by workflow ID'),
      task_id: V.id.optional().describe('Filter by task ID (runs triggered by this task)'),
      limit: z.number().int().min(1).max(100).optional().describe('Max results (default: 20)')
    },
    async (args) => {
      if (args.workflow_id && args.task_id) {
        return {
          content: [{ type: 'text', text: 'Error: provide workflow_id or task_id, not both' }],
          isError: true
        }
      }
      if (args.task_id) {
        const runs = listWorkflowRunsByTask(args.task_id, args.limit ?? 20)
        return { content: [{ type: 'text', text: JSON.stringify(runs, null, 2) }] }
      }
      if (args.workflow_id) {
        const runs = listWorkflowRuns(args.workflow_id, args.limit ?? 20)
        return { content: [{ type: 'text', text: JSON.stringify(runs, null, 2) }] }
      }
      return {
        content: [{ type: 'text', text: 'Error: provide either workflow_id or task_id' }],
        isError: true
      }
    }
  )

  server.tool(
    'get_workflow_schedule',
    'Get scheduler info for a workflow: execution log or next scheduled run. Requires the Vorn app to be running.',
    {
      workflow_id: V.id
        .optional()
        .describe('Workflow ID (required for next_run, optional for log)'),
      info: z.enum(['log', 'next_run']).optional().describe('What to retrieve (default: log)')
    },
    async (args) => {
      const info = args.info ?? 'log'
      try {
        if (info === 'next_run') {
          if (!args.workflow_id) {
            return {
              content: [{ type: 'text', text: 'Error: workflow_id is required for next_run' }],
              isError: true
            }
          }
          const nextRun = await rpcCall<string | null>('scheduler:getNextRun', args.workflow_id)
          return {
            content: [
              {
                type: 'text',
                text: nextRun
                  ? JSON.stringify({ nextRun }, null, 2)
                  : 'No scheduled run (workflow may be manual or disabled)'
              }
            ]
          }
        } else {
          const log = await rpcCall<ScheduleLogEntry[]>('scheduler:getLog', args.workflow_id)
          return { content: [{ type: 'text', text: JSON.stringify(log, null, 2) }] }
        }
      } catch (err) {
        return {
          content: [{ type: 'text', text: `Error: ${err instanceof Error ? err.message : err}` }],
          isError: true
        }
      }
    }
  )

  server.tool(
    'execute_workflow',
    "Run a workflow now, as if triggered manually. Supply values for any parameters the workflow declares (see the trigger node's inputs); declared defaults fill in anything omitted. Requires the Vorn app to be running. Returns as soon as the run is queued — poll list_workflow_runs for the outcome.",
    {
      workflow_id: V.id.describe('Workflow ID (from list_workflows)'),
      inputs: z
        .record(z.string(), z.union([z.string(), z.number(), z.boolean()]))
        .optional()
        .describe('Values for the declared parameters, keyed by input key ({{inputs.<key>}})')
    },
    async (args) => {
      const workflow = dbListWorkflows().find((w) => w.id === args.workflow_id)
      if (!workflow) {
        return {
          content: [{ type: 'text', text: `Error: no workflow with id ${args.workflow_id}` }],
          isError: true
        }
      }

      const trigger = workflow.nodes.find((n) => n.type === 'trigger')?.config as
        | TriggerConfig
        | undefined
      const defs = trigger?.triggerType === 'manual' ? (trigger.inputs ?? []) : []
      const supplied = args.inputs ?? {}

      let inputs: Record<string, unknown> | undefined
      if (defs.length > 0) {
        const { values, errors } = resolveWorkflowInputs(defs, supplied)
        if (errors.length > 0) {
          return {
            content: [
              { type: 'text', text: `Error: invalid inputs\n  - ${errors.join('\n  - ')}` }
            ],
            isError: true
          }
        }
        inputs = Object.keys(values).length > 0 ? values : undefined
      } else if (Object.keys(supplied).length > 0) {
        // Nothing declared to validate against. Pass the values through rather
        // than rejecting them: a non-manual trigger cannot declare inputs, but
        // its nodes may still read `{{inputs.*}}`.
        inputs = supplied
      }

      try {
        await rpcCall('workflow:runManual', { workflowId: args.workflow_id, inputs })
      } catch (err) {
        return {
          content: [{ type: 'text', text: `Error: ${err instanceof Error ? err.message : err}` }],
          isError: true
        }
      }

      // A disabled workflow still runs when triggered by hand — the flag only
      // stops the scheduler. Say so, so the result is not a surprise.
      const disabled =
        workflow.enabled === false ? ' (workflow is disabled; manual runs still execute)' : ''
      const shown = inputs ? `\ninputs: ${JSON.stringify(inputs, null, 2)}` : '\nno inputs'
      return {
        content: [
          {
            type: 'text',
            text: `Queued "${workflow.name}"${disabled}${shown}\n\nRun history: list_workflow_runs with workflow_id ${args.workflow_id}`
          }
        ]
      }
    }
  )
}
