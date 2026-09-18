import crypto from 'node:crypto'
import { isDeepStrictEqual } from 'node:util'
import {
  isSignInWait,
  loopBodyOwners,
  loopStructureError,
  MAX_GATE_ROUNDS,
  nodesBetween
} from '@vornrun/shared/workflow-graph'
import { z } from 'zod'
import type { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import { V } from '../validation'
import {
  edgeSchema,
  NODE_TYPES,
  nodeConfigIssues,
  triggerConfigSchema,
  workflowInputDefSchema,
  workflowInputsSchema,
  type NodeType
} from './node-config-schemas'
import type {
  ApprovalConfig,
  WorkflowDefinition,
  WorkflowExecution,
  WorkflowNode,
  WorkflowEdge,
  TriggerConfig,
  LaunchAgentConfig,
  WorkflowInputDef
} from '@vornrun/shared/types'
import type { GateDecision, ScheduleLogEntry } from '@vornrun/shared/types'
import {
  dbListProjects,
  dbListWorkflows,
  dbInsertWorkflow,
  dbUpdateWorkflow,
  dbDeleteWorkflow,
  listWorkflowRuns,
  listWorkflowRunsByTask,
  listAllWorkflowRuns,
  listRunsWithWaitingGates,
  dbSignalChange
} from '../data-access'
import { rpcCall } from '@vornrun/server/rpc-client'
import {
  toPortable,
  fromPortable,
  importedWorkflowIdFor,
  unresolvedRequirements,
  residualAbsolutePaths,
  slugify,
  PORTABLE_FORMAT_VERSION,
  type PortableConnection,
  type PortableWorkflow
} from '@vornrun/shared/workflow-portability'

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

export { edgeSchema, triggerConfigSchema, workflowInputDefSchema, workflowInputsSchema }

/**
 * A node's shape without its config checked: what update_workflow accepts, so a
 * stored workflow whose config predates today's rules can still be edited. It
 * checks configs itself, and tells an untouched old config from a new mistake.
 */
export const nodeShapeSchema = z.object({
  id: V.id.describe('Unique within the workflow; edges and bodyNodeIds refer to it'),
  // Full node palette — parity with the editor.
  type: z
    .enum(NODE_TYPES)
    .describe('Node type; describe_workflow_nodes gives the config each one takes'),
  label: V.shortText.describe('Name shown on the canvas and in run history'),
  // Referenced by typed step vars as `{{steps.<slug>.<field>}}`. Set one on any
  // node whose output a later node consumes.
  slug: V.shortText
    .optional()
    .describe('Name later steps read this one by, as {{steps.<slug>.<field>}}'),
  config: z
    .record(z.string(), z.unknown())
    .describe("The type's own settings; describe_workflow_nodes has the schema for each type"),
  position: z
    .object({ x: z.number(), y: z.number() })
    .describe('Where the node sits on the canvas'),
  // Omitted means stop. Declared here because the object strips what it does
  // not name: without it a workflow authored over MCP could never say a step
  // is survivable, and the field would be dropped without complaint.
  onError: z
    .enum(['stop', 'continue'])
    .optional()
    .describe('stop (default) ends the run when this step fails; continue carries on past it')
})

/** Each config problem on a node, as "field: why", for a tool result. */
export function describeConfigIssues(node: { type: NodeType; config: unknown }): string[] {
  return nodeConfigIssues(node.type, node.config).map(
    (issue) => `${['config', ...issue.path.map(String)].join('.')}: ${issue.message}`
  )
}

/**
 * Config problems on the nodes an update sends, split by whether the caller
 * made them.
 *
 * A config identical to the one already stored under that node was accepted
 * by an older build, or written by the editor, and refusing it would make the
 * whole workflow uneditable over MCP until someone fixed a node they never
 * touched. Those become warnings; a config the caller changed is held to the
 * current rules.
 */
export function checkNodeConfigs(
  nodes: { id: string; type: NodeType; label?: string; config: unknown }[],
  stored: readonly WorkflowNode[]
): { errors: string[]; warnings: string[] } {
  const errors: string[] = []
  const warnings: string[] = []
  for (const node of nodes) {
    const issues = describeConfigIssues(node)
    if (issues.length === 0) continue
    const before = stored.find((n) => n.id === node.id && n.type === node.type)
    const untouched = before !== undefined && isDeepStrictEqual(before.config, node.config)
    const line = `node "${node.label || node.id}" ${issues.join(', ')}`
    if (untouched) warnings.push(line)
    else errors.push(line)
  }
  return { errors, warnings }
}

// A node whose config does not match its type would be stored and then fail,
// or worse run on a misread value, on a run someone is waiting for. Checking it
// against its type's schema puts the error on the call that introduced it. The
// config itself passes through unchanged, so nothing the caller wrote is lost.
export const nodeSchema = nodeShapeSchema.superRefine((node, ctx) => {
  for (const issue of nodeConfigIssues(node.type, node.config)) {
    ctx.addIssue({ code: 'custom', path: ['config', ...issue.path], message: issue.message })
  }
})

/**
 * Loop bodies must name steps that exist in the same workflow.
 *
 * Checked across the whole graph rather than per node, since a node cannot see
 * its siblings during its own parse.
 */
export function validateLoopBodies(
  nodes: { id: string; type: string; label?: string; config: Record<string, unknown> }[]
): string[] {
  const ids = new Set(nodes.map((n) => n.id))
  const errors: string[] = []

  for (const node of nodes) {
    if (node.type !== 'loop') continue
    const body = (node.config.bodyNodeIds as string[] | undefined) ?? []

    for (const id of body) {
      if (!ids.has(id)) {
        errors.push(`loop "${node.label || node.id}" references unknown body step "${id}"`)
      }
    }
    if (body.includes(node.id)) {
      errors.push(`loop "${node.label || node.id}" lists itself as a body step`)
    }
  }

  return errors
}

type LooseNode = { id: string; type: string; label?: string; config: Record<string, unknown> }

/**
 * Whether each loop's body is a shape the engine will run, by the same check
 * the engine makes before a first pass. The engine would fail such a loop on
 * every run, so storing it only defers the error to someone waiting on one.
 */
export function validateLoopStructures(
  nodes: LooseNode[],
  edges: { id?: string; source: string; target: string; conditionBranch?: string }[]
): string[] {
  const graph = nodes as unknown as WorkflowNode[]
  const wires = edges as unknown as WorkflowEdge[]
  return graph
    .filter((node) => node.type === 'loop')
    .flatMap((loop) => {
      const error = loopStructureError(graph, wires, loop)
      return error ? [`loop "${loop.label || loop.id}": ${error}`] : []
    })
}

/** Everything about a graph that no single node can check on its own. */
export function validateGraph(
  nodes: LooseNode[],
  edges: { id?: string; source: string; target: string; conditionBranch?: string }[]
): string[] {
  return [
    ...validateLoopBodies(nodes),
    ...validateLoopStructures(nodes, edges),
    ...validateGateFeedback(nodes, edges)
  ]
}

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

    // A declared toggle always carries an answer. defaultInputValue seeds
    // `false` when nothing was authored and areInputsValid counts `false` as a
    // real answer, so omitting it here would expand `{{inputs.x}}` to '' on an
    // MCP run and 'false' on a UI run — and dedupe them as different runs.
    if (def.type === 'boolean') {
      if (raw === undefined) {
        values[def.key] = false
      } else if (typeof raw === 'boolean') {
        values[def.key] = raw
      } else if (raw === 'true' || raw === 'false') {
        values[def.key] = raw === 'true'
      } else {
        errors.push(`input "${def.key}" must be a boolean, got ${JSON.stringify(raw)}`)
      }
      continue
    }

    // Whitespace-only is "no value", matching areInputsValid's String(v).trim().
    if (raw === undefined || (typeof raw === 'string' && raw.trim() === '')) {
      if (def.required) errors.push(`missing required input "${def.key}" (${def.label})`)
      continue
    }

    switch (def.type) {
      case 'number': {
        // parseNumberInput rejects anything non-finite because NaN and Infinity
        // do not survive JSON — they corrupt the persisted run, the template
        // expansion and the dedupe fingerprint alike. A boolean is not a number
        // either, however willingly Number() turns it into one.
        if (typeof raw === 'boolean') {
          errors.push(`input "${def.key}" must be a number, got ${JSON.stringify(raw)}`)
          break
        }
        const n = typeof raw === 'number' ? raw : Number(String(raw).trim())
        if (!Number.isFinite(n)) {
          errors.push(`input "${def.key}" must be a finite number, got ${JSON.stringify(raw)}`)
        } else {
          values[def.key] = n
        }
        break
      }
      case 'select': {
        const allowed = (def.options ?? []).map((o) => o.value)
        if (allowed.length === 0) {
          // The run dialog can only offer what the select declares, so a select
          // with no options can produce no value at all. Accepting an arbitrary
          // string here would let a tool caller past a constraint the UI
          // enforces absolutely.
          errors.push(`input "${def.key}" is a select but declares no options`)
        } else if (!allowed.includes(String(raw))) {
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

/** The approval node a gate belongs to, so its question and its name come from one lookup. */
export function approvalNode(
  workflow: Pick<WorkflowDefinition, 'nodes'> | undefined,
  nodeId: string
): WorkflowNode | undefined {
  return workflow?.nodes.find((n) => n.id === nodeId && n.type === 'approval')
}

/** What an approval node asks, so a caller answering a gate can read the question first. */
export function askedBy(node: WorkflowNode | undefined): string | undefined {
  return (node?.config as ApprovalConfig | undefined)?.message?.trim() || undefined
}

/** What the gate on a node asks, for a caller holding the definition rather than the node. */
export function gateMessage(
  workflow: Pick<WorkflowDefinition, 'nodes'> | undefined,
  nodeId: string
): string | undefined {
  return askedBy(approvalNode(workflow, nodeId))
}

/** A waiting node says it is waiting and nothing else; the gate's own question lives in the definition. */
export function annotateWaitingGates<T extends WorkflowExecution>(
  runs: T[],
  workflows: Pick<WorkflowDefinition, 'id' | 'nodes'>[]
): T[] {
  return runs.map((run) => {
    if (!run.nodeStates.some((n) => n.status === 'waiting')) return run
    const workflow = workflows.find((w) => w.id === run.workflowId)
    return {
      ...run,
      nodeStates: run.nodeStates.map((state) => {
        if (state.status !== 'waiting') return state
        if (isSignInWait(state)) {
          return {
            ...state,
            asks: 'Sign in to its connection in the Vorn app, and this step runs again'
          }
        }
        const asks = state.message ?? gateMessage(workflow, state.nodeId)
        return asks ? { ...state, asks } : state
      })
    }
  })
}

const ANSWERED: Record<GateDecision, string> = {
  approve: 'Approved',
  reject: 'Rejected',
  changes: 'Sent back'
}

/** A gate that takes changes names a step above it to redo from, and a round count the engine allows. */
export function validateGateFeedback(
  nodes: { id: string; type: string; label?: string; config: Record<string, unknown> }[],
  edges: { source: string; target: string }[]
): string[] {
  const errors: string[] = []
  const owners = loopBodyOwners(nodes as unknown as WorkflowNode[])
  for (const node of nodes) {
    if (node.type !== 'approval' || node.config?.feedback === undefined) continue
    const feedback = node.config.feedback as { from?: unknown; maxRounds?: unknown } | null
    const name = node.label || node.id
    const from = typeof feedback?.from === 'string' ? feedback.from : ''
    const source = nodes.find((n) => n.id === from)
    if (!source) errors.push(`gate "${name}" redoes from unknown step "${from}"`)
    else if (source.type === 'trigger') errors.push(`gate "${name}" cannot redo from its trigger`)
    // A loop drives its own body, so re-running one member on its own would run
    // it outside any pass: no {{loop.*}}, and none of the steps around it.
    else if (owners.has(from)) {
      const loop = nodes.find((n) => n.id === owners.get(from))
      errors.push(
        `gate "${name}" redoes from "${source.label || from}", which is inside loop "${loop?.label || owners.get(from)}"; redo from the loop, or a step before it`
      )
    } else if (nodesBetween(from, node.id, edges).size === 0) {
      errors.push(`gate "${name}" redoes from "${source.label || from}", which does not lead to it`)
    }
    const rounds = Number(feedback?.maxRounds)
    if (!Number.isInteger(rounds) || rounds < 2 || rounds > MAX_GATE_ROUNDS) {
      errors.push(`gate "${name}" needs maxRounds between 2 and ${MAX_GATE_ROUNDS}`)
    }
  }
  return errors
}

/** A run to answer a gate on: recent history first, then every parked run, since a gate can wait past the cap. */
async function runById(
  runId: string
): Promise<(WorkflowExecution & { workflowName?: string }) | undefined> {
  const recent = (await listAllWorkflowRuns(undefined, 500)).find((r) => r.runId === runId)
  if (recent) return recent
  return (await listRunsWithWaitingGates()).find((r) => r.runId === runId)
}

// Which gate a decision answers: one waiting node needs no naming, several would make picking a guess.
export function resolveGateTarget(
  run: Pick<WorkflowExecution, 'nodeStates'>,
  nodeId?: string,
  decision: GateDecision = 'approve'
): { nodeId: string } | { error: string } {
  const parked = run.nodeStates.filter((n) => n.status === 'waiting')
  // Rejecting a sign-in wait ends the run; approving one would skip the step it waits to run.
  const answerable = parked.filter((n) => decision === 'reject' || !isSignInWait(n))
  const waiting = answerable.map((n) => n.nodeId)
  const signIns = parked.filter((n) => !answerable.includes(n)).map((n) => n.nodeId)
  if (nodeId) {
    if (signIns.includes(nodeId)) {
      return {
        error: `node "${nodeId}" is waiting for a sign-in in the Vorn app, not for an approval`
      }
    }
    if (waiting.includes(nodeId)) return { nodeId }
    return {
      error: waiting.length
        ? `node "${nodeId}" is not waiting. Waiting: ${waiting.join(', ')}`
        : `node "${nodeId}" is not waiting, and neither is any other node in this run`
    }
  }
  if (waiting.length === 1) return { nodeId: waiting[0] }
  if (waiting.length === 0) {
    return {
      error: signIns.length
        ? 'this run is waiting for a sign-in in the Vorn app, not for an approval'
        : 'no node in this run is waiting on a gate'
    }
  }
  return { error: `${waiting.length} nodes are waiting — pass node_id: ${waiting.join(', ')}` }
}

/** Connections for naming and rebinding requirements; absent ones cost detail, not the export. */
async function listPortableConnections(): Promise<PortableConnection[]> {
  try {
    return await rpcCall<PortableConnection[]>('connection:list', { connectorId: undefined })
  } catch {
    return []
  }
}

export function registerWorkflowTools(server: McpServer): void {
  server.tool(
    'list_workflows',
    'List all workflows, optionally filtered by workspace',
    {
      workspace_id: V.id.optional().describe('Filter by workspace ID')
    },
    async (args) => {
      let workflows = await dbListWorkflows()
      if (args.workspace_id) {
        workflows = workflows.filter((w) => (w.workspaceId ?? 'personal') === args.workspace_id)
      }
      return { content: [{ type: 'text', text: JSON.stringify(workflows, null, 2) }] }
    }
  )

  server.tool(
    'create_workflow',
    'Create a workflow: a graph of steps started by one trigger. Call describe_workflow_nodes ' +
      'first — it gives the config schema, outputs and rules of every node type, the template ' +
      'namespaces, and a complete example to copy. Pass nodes and edges (every node type is ' +
      'supported), or the convenience format (trigger + actions, a straight line of agents).\n\n' +
      'Node types: trigger (exactly one, what starts the run), launchAgent (an AI agent; headless ' +
      'with an outputSchema gives typed fields), script (bash/python/node/powershell), condition ' +
      '(branches: wire both outcomes with edge conditionBranch "true" and "false"), approval (a ' +
      'human gate; can show a table to edit and send work back), createTaskFromItem, ' +
      'callConnectorAction (an action of a connected service; list_connector_actions gives its ' +
      'args), httpRequest, and loop.\n\n' +
      'Give a node a slug and later steps read its output as {{steps.<slug>.<field>}}.\n\n' +
      'Loops: config.bodyNodeIds lists the steps the loop runs each pass. Wire an edge from the ' +
      'loop to each body entry step, edges between body steps, and from the last body steps to ' +
      'the step after the loop; nothing outside the loop may point into its body, and a ' +
      'condition inside it keeps both branches inside. mode "repeat" runs up to maxIterations ' +
      '(1-10) passes, stopping early when until holds; mode "forEach" runs once per item of ' +
      'items (e.g. {{steps.gate.items}}), and body steps read {{loop.item.<field>}}. A loop body ' +
      'cannot hold an approval, a trigger or another loop.\n\n' +
      'Each node config is checked against its type, and the graph for loop and gate wiring; ' +
      'a refusal names the node and field.',
    {
      name: V.title.describe('Workflow name'),
      trigger: triggerConfigSchema
        .optional()
        .describe('Trigger config (convenience mode). Defaults to manual.'),
      actions: z
        .array(launchAgentConfigSchema)
        .optional()
        .describe('Agents to run one after another (convenience mode). Auto-generates the graph.'),
      nodes: z
        .array(nodeSchema)
        .optional()
        .describe('Graph nodes; pass with edges. describe_workflow_nodes has each config schema'),
      edges: z
        .array(edgeSchema)
        .optional()
        .describe('Graph edges; pass with nodes. A step runs once every edge into it has run'),
      icon: V.shortText.optional().describe('Lucide icon name (default: zap)'),
      icon_color: V.hexColor.optional().describe('Hex color (default: #6366f1)'),
      enabled: z
        .boolean()
        .optional()
        .describe(
          'Whether its trigger fires on its own (default: true). Manual runs work either way'
        ),
      stagger_delay_ms: z.number().optional().describe('Delay in ms between actions')
    },
    async (args) => {
      let nodes: WorkflowNode[]
      let edges: WorkflowEdge[]

      if (args.nodes && args.edges) {
        nodes = args.nodes as unknown as WorkflowNode[]
        edges = args.edges as unknown as WorkflowEdge[]
        const graphErrors = validateGraph(nodes as unknown as LooseNode[], edges)
        if (graphErrors.length > 0) {
          return {
            content: [{ type: 'text', text: `Error: ${graphErrors.join('; ')}` }],
            isError: true
          }
        }
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

      await dbInsertWorkflow(workflow)
      dbSignalChange()

      return { content: [{ type: 'text', text: JSON.stringify(workflow, null, 2) }] }
    }
  )

  server.tool(
    'update_workflow',
    'Change a workflow. Only the fields you pass change; nodes and edges each replace the ' +
      'whole list, so read the workflow first (list_workflows), edit it, and send the full list ' +
      'back. Checked like create_workflow: each node config against its type ' +
      '(describe_workflow_nodes), and the graph for loop and gate wiring. A node whose config you ' +
      'left exactly as stored is let through with a warning even if it breaks a newer rule, so an ' +
      'older workflow stays editable; fix what the warning names when you can.',
    {
      workflow_id: V.id.optional().describe('Workflow ID (from list_workflows)'),
      id: V.id.optional().describe('Deprecated alias for workflow_id'),
      name: V.title.optional().describe('New name'),
      // The shape only: configs are checked in the handler, where an untouched
      // stored config can be told apart from a new mistake.
      nodes: z
        .array(nodeShapeSchema)
        .optional()
        .describe('Every node of the workflow, replacing the stored list'),
      edges: z
        .array(edgeSchema)
        .optional()
        .describe('Every edge of the workflow, replacing the stored list'),
      icon: V.shortText.optional().describe('Lucide icon name'),
      icon_color: V.hexColor.optional().describe('Hex color'),
      enabled: z
        .boolean()
        .optional()
        .describe('Whether its trigger fires on its own. Manual runs work either way'),
      stagger_delay_ms: z.number().optional().describe('Delay in ms between actions')
    },
    async (args) => {
      const resolved = resolveWorkflowId(args)
      if ('error' in resolved) {
        return { content: [{ type: 'text', text: `Error: ${resolved.error}` }], isError: true }
      }
      const workflows = await dbListWorkflows()
      const workflow = workflows.find((w) => w.id === resolved.id)
      if (!workflow) {
        return {
          content: [{ type: 'text', text: `Error: workflow "${resolved.id}" not found` }],
          isError: true
        }
      }

      const warnings: string[] = []
      if (args.nodes !== undefined) {
        const { errors, warnings: legacy } = checkNodeConfigs(args.nodes, workflow.nodes)
        if (errors.length > 0) {
          return {
            content: [{ type: 'text', text: `Error: ${errors.join('; ')}` }],
            isError: true
          }
        }
        warnings.push(...legacy)
      }
      if (args.nodes !== undefined || args.edges !== undefined) {
        // Either list changes the graph the other is read against, so both
        // are checked together as they will be stored.
        const nodes = (args.nodes ?? workflow.nodes) as unknown as LooseNode[]
        const edges = args.edges ?? workflow.edges
        const graphErrors = validateGraph(nodes, edges)
        if (graphErrors.length > 0) {
          return {
            content: [{ type: 'text', text: `Error: ${graphErrors.join('; ')}` }],
            isError: true
          }
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

      await dbUpdateWorkflow(resolved.id, updates)
      dbSignalChange()

      const saved = JSON.stringify({ ...workflow, ...updates }, null, 2)
      return {
        content: [
          {
            type: 'text',
            text:
              warnings.length > 0
                ? `Warning: saved, but these configs were stored before and break a current rule, so they were kept as they are:\n  - ${warnings.join('\n  - ')}\n\n${saved}`
                : saved
          }
        ]
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
      const workflows = await dbListWorkflows()
      const workflow = workflows.find((w) => w.id === resolved.id)
      if (!workflow) {
        return {
          content: [{ type: 'text', text: `Error: workflow "${resolved.id}" not found` }],
          isError: true
        }
      }

      await dbDeleteWorkflow(resolved.id)
      dbSignalChange()

      return { content: [{ type: 'text', text: `Deleted workflow: ${workflow.name}` }] }
    }
  )

  server.tool(
    'list_workflow_runs',
    'List workflow execution history. Filter by workflow_id or task_id; with neither, lists the runs parked on an approval gate, each waiting node saying what it asks.',
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
      // The definitions cost a round trip, so they are read only for a run that is parked on a gate.
      const withGates = async <T extends WorkflowExecution>(runs: T[]): Promise<T[]> =>
        runs.some((r) => r.nodeStates.some((n) => n.status === 'waiting'))
          ? annotateWaitingGates(runs, await dbListWorkflows())
          : runs

      if (args.task_id) {
        const runs = await withGates(await listWorkflowRunsByTask(args.task_id, args.limit ?? 20))
        return { content: [{ type: 'text', text: JSON.stringify(runs, null, 2) }] }
      }
      if (args.workflow_id) {
        const runs = await withGates(await listWorkflowRuns(args.workflow_id, args.limit ?? 20))
        return { content: [{ type: 'text', text: JSON.stringify(runs, null, 2) }] }
      }
      // Neither filter: the runs someone has to answer, which no other tool can find.
      const parked = (await listRunsWithWaitingGates()).slice(0, args.limit ?? 20)
      const waiting = await withGates(parked)
      return { content: [{ type: 'text', text: JSON.stringify(waiting, null, 2) }] }
    }
  )

  server.tool(
    'stop_workflow_run',
    'Stop a workflow run that is still going, including one parked on an approval gate. Kills the agents it started, marks its unfinished nodes, and closes the run as cancelled. Requires the Vorn app to be running.',
    {
      run_id: V.id.describe('Run ID (from list_workflow_runs)')
    },
    async (args) => {
      // Scanning recent runs beats broadcasting a typo that nothing answers:
      // the stop is fire-and-forget, so an id that matches nothing would
      // otherwise report success and do nothing at all.
      const run = await runById(args.run_id)
      if (!run) {
        return {
          content: [
            {
              type: 'text',
              text: `Error: no run "${args.run_id}" in the last 500. Check list_workflow_runs.`
            }
          ],
          isError: true
        }
      }
      if (run.status !== 'running') {
        return {
          content: [
            {
              type: 'text',
              text: `Run ${args.run_id} already finished (${run.status}) — nothing to stop.`
            }
          ]
        }
      }

      try {
        await rpcCall('workflow:stopRun', { runId: args.run_id })
      } catch (err) {
        return {
          content: [{ type: 'text', text: `Error: ${err instanceof Error ? err.message : err}` }],
          isError: true
        }
      }

      const live = run.nodeStates.filter(
        (n) => n.status === 'running' || n.status === 'waiting'
      ).length
      return {
        content: [
          {
            type: 'text',
            text: `Asked to stop run ${args.run_id}${run.workflowName ? ` of "${run.workflowName}"` : ''} — ${live} node(s) were still live.\n\nThe run is stopped by the instance holding it, so confirm with list_workflow_runs.`
          }
        ]
      }
    }
  )

  server.tool(
    'resolve_gate',
    'Approve, reject, or send back the approval gate a workflow run is parked on, the way the Vorn app does. Requires the Vorn app to be running: the decision is broadcast, and the instance holding the run is what resumes it. Read what is being approved first — list_workflow_runs names the waiting node and what it asks.',
    {
      run_id: V.id.describe('Run ID (from list_workflow_runs)'),
      decision: z
        .enum(['approve', 'reject', 'changes'])
        .describe(
          'approve lets the run go on; reject ends it; changes sends the work back to the step the gate redoes from, with your comment, and the gate asks again'
        ),
      comment: z
        .string()
        .max(20000)
        .optional()
        .describe(
          'Required for changes: what to change. Optional on approve and reject; kept with the run.'
        ),
      edited: z
        .string()
        .max(200000)
        .optional()
        .describe(
          "Your rewrite of the gate's editable text, when it has one. Kept beside the original, and read by later steps as {{steps.<gate>.text}}. Ignored on reject."
        ),
      node_id: V.id.optional().describe('The waiting node, when a run has more than one gate open')
    },
    async (args) => {
      // The decision is broadcast, so an id nothing matches would report success and answer no gate at all.
      const run = await runById(args.run_id)
      if (!run) {
        return {
          content: [
            {
              type: 'text',
              text: `Error: no run "${args.run_id}" in the recent history, and none parked on a gate. Check list_workflow_runs.`
            }
          ],
          isError: true
        }
      }
      if (run.status !== 'running') {
        return {
          content: [
            {
              type: 'text',
              text: `Run ${args.run_id} already finished (${run.status}) — no gate to answer.`
            }
          ]
        }
      }

      const target = resolveGateTarget(run, args.node_id, args.decision)
      if ('error' in target) {
        return {
          content: [{ type: 'text', text: `Error: ${target.error}` }],
          isError: true
        }
      }

      if (args.decision === 'changes' && !args.comment?.trim()) {
        return {
          content: [
            { type: 'text', text: 'Error: changes needs a comment saying what to change.' }
          ],
          isError: true
        }
      }

      const workflow = (await dbListWorkflows()).find((w) => w.id === run.workflowId)
      const gateNode = approvalNode(workflow, target.nodeId)
      const asked =
        run.nodeStates.find((n) => n.nodeId === target.nodeId)?.message ?? askedBy(gateNode)

      try {
        const answer = await rpcCall<{ accepted: boolean; reason?: string }>(
          'workflow:resolveGate',
          {
            runId: args.run_id,
            nodeId: target.nodeId,
            decision: args.decision,
            ...(args.comment?.trim() && { comment: args.comment.trim() }),
            ...(args.edited?.trim() && { edited: args.edited.trim() })
          }
        )
        if (answer?.accepted === false) {
          return {
            content: [
              {
                type: 'text',
                text: answer.reason
                  ? `Error: ${answer.reason}`
                  : args.decision === 'changes'
                    ? 'Error: this gate takes no changes now: it has no step to redo from, or its rounds are used up. Approve or reject it instead.'
                    : 'Error: the gate did not take that answer. Check list_workflow_runs.'
              }
            ],
            isError: true
          }
        }
      } catch (err) {
        return {
          content: [{ type: 'text', text: `Error: ${err instanceof Error ? err.message : err}` }],
          isError: true
        }
      }

      const gate = gateNode?.label ?? target.nodeId
      return {
        content: [
          {
            type: 'text',
            text: `${ANSWERED[args.decision]} "${gate}" on run ${args.run_id}${run.workflowName ? ` of "${run.workflowName}"` : ''}.${asked ? `\n\nWhat it asked: ${asked}` : ''}\n\nThe decision went out; the instance holding the run acts on it, so a desktop has to be open. Confirm with list_workflow_runs.`
          }
        ]
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
    "Run a workflow now, as if triggered manually. Supply values for any parameters the workflow declares (see the trigger node's inputs); declared defaults fill in anything omitted. Requires the Vorn app to be running. Returns as soon as the run is queued — poll list_workflow_runs for the outcome. Runs of one workflow go side by side; only a run repeating one started in the last ten seconds with the same inputs is refused as a duplicate.",
    {
      workflow_id: V.id.describe('Workflow ID (from list_workflows)'),
      inputs: z
        .record(z.string(), z.union([z.string(), z.number(), z.boolean()]))
        .optional()
        .describe('Values for the declared parameters, keyed by input key ({{inputs.<key>}})')
    },
    async (args) => {
      const workflow = (await dbListWorkflows()).find((w) => w.id === args.workflow_id)
      if (!workflow) {
        return {
          content: [{ type: 'text', text: `Error: workflow "${args.workflow_id}" not found` }],
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
        // Always an object once the workflow declares inputs, even when every
        // value resolved empty. resolveTemplateVars
        // (src/renderer/lib/template-vars.ts) skips the whole `inputs`
        // namespace when context.inputs is absent, which leaves a literal
        // `{{inputs.topic}}` in the agent's prompt; an empty object resolves it
        // to an empty string instead. Wrong-but-blank beats raw template text.
        inputs = values
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

  server.tool(
    'export_workflow',
    'Export a workflow as a portable file you can commit beside the code it drives. Absolute paths become {{project.path}} and the local remote-host binding is dropped, so it runs on another machine after import. Connections are dropped too and recorded as requirements the importing machine rebinds by connector and name.',
    {
      workflow_id: V.id.optional().describe('Workflow ID (from list_workflows)'),
      id: V.id.optional().describe('Deprecated alias for workflow_id')
    },
    async (args) => {
      const resolved = resolveWorkflowId(args)
      if ('error' in resolved) {
        return { content: [{ type: 'text', text: `Error: ${resolved.error}` }], isError: true }
      }

      const workflow = (await dbListWorkflows()).find((w) => w.id === resolved.id)
      if (!workflow) {
        return {
          content: [{ type: 'text', text: `Error: workflow "${resolved.id}" not found` }],
          isError: true
        }
      }

      // The project the workflow's own steps point at is what its paths are
      // relative to; without it there is nothing to rewrite against.
      const projects = await dbListProjects()
      const projectName = workflow.nodes
        .map((n) => (n.config as Record<string, unknown>).projectName)
        .find((name): name is string => typeof name === 'string' && name.length > 0)
      const project = projects.find((p) => p.name === projectName)

      if (!project) {
        return {
          content: [
            {
              type: 'text',
              text: `Error: no project named "${projectName ?? '(none)'}" is registered, so this workflow's paths cannot be made relative to anything.`
            }
          ],
          isError: true
        }
      }

      const portable = toPortable(workflow, project.path, await listPortableConnections())
      const residual = residualAbsolutePaths(portable)
      const unnamed = (portable.requires ?? []).filter(
        (requirement) => requirement.kind === 'connection' && requirement.connectorId === ''
      )

      return {
        content: [
          {
            type: 'text',
            text:
              JSON.stringify(portable, null, 2) +
              (residual.length > 0
                ? `\n\nWarning: these still hold a machine-specific path and will not travel: ${residual.join(', ')}`
                : '') +
              (unnamed.length > 0
                ? `\n\nWarning: ${unnamed.length} step(s) point at a connection this install could not name, so an import cannot rebind them automatically.`
                : '')
          }
        ]
      }
    }
  )

  server.tool(
    'import_workflow',
    "Import a workflow exported by export_workflow, resolving {{project.path}} and {{project.name}} against a registered project. Recorded connection requirements are rebound when this machine has one unambiguous match, and reported as still to connect otherwise. The id is derived from the bundle and the workflow's slug, so importing the same file again updates it in place instead of creating a duplicate.",
    {
      workflow: z.string().max(500_000).describe('The exported workflow JSON'),
      project_name: V.name.describe('Registered project to resolve paths against'),
      bundle: V.name.optional().describe('Namespace for the derived id (default: the project name)')
    },
    async (args) => {
      let parsed: PortableWorkflow
      try {
        parsed = JSON.parse(args.workflow)
      } catch (err) {
        return {
          content: [
            {
              type: 'text',
              text: `Error: workflow is not valid JSON — ${String(err).slice(0, 200)}`
            }
          ],
          isError: true
        }
      }

      if (parsed?.version !== PORTABLE_FORMAT_VERSION) {
        return {
          content: [
            {
              type: 'text',
              text: `Error: unsupported format version ${parsed?.version}; this build reads version ${PORTABLE_FORMAT_VERSION}`
            }
          ],
          isError: true
        }
      }
      if (!Array.isArray(parsed.nodes) || !Array.isArray(parsed.edges) || !parsed.name) {
        return {
          content: [{ type: 'text', text: 'Error: workflow is missing name, nodes or edges' }],
          isError: true
        }
      }

      const project = (await dbListProjects()).find((p) => p.name === args.project_name)
      if (!project) {
        return {
          content: [
            {
              type: 'text',
              text: `Error: no project named "${args.project_name}". Create it first so its path is known.`
            }
          ],
          isError: true
        }
      }

      const loopErrors = validateGraph(parsed.nodes as unknown as LooseNode[], parsed.edges)
      if (loopErrors.length > 0) {
        return {
          content: [{ type: 'text', text: `Error: ${loopErrors.join('; ')}` }],
          isError: true
        }
      }

      const bundle = args.bundle ?? slugify(project.name)
      const portable = { ...parsed, slug: parsed.slug ?? slugify(parsed.name) }
      const connections = await listPortableConnections()
      const resolved = fromPortable(
        portable,
        bundle,
        {
          name: project.name,
          path: project.path
        },
        connections
      )
      const unresolved = unresolvedRequirements(portable, connections)

      const known = await dbListWorkflows()
      const id = importedWorkflowIdFor(bundle, portable.slug, portable.name, known)
      const existing = known.find((w) => w.id === id)
      // A file describes a workflow; whether it runs is this machine's answer,
      // and one that was already running keeps running.
      const definition = { ...resolved, id, enabled: existing ? existing.enabled : false }

      if (existing) {
        await dbUpdateWorkflow(definition.id, definition)
      } else {
        await dbInsertWorkflow(definition)
      }
      dbSignalChange()

      const pending = unresolved
        .map((requirement) =>
          requirement.kind === 'httpProfile'
            ? `${requirement.nodeId} needs an HTTP profile${requirement.name ? ` like "${requirement.name}"` : ''}`
            : `${requirement.nodeId} needs a ${requirement.connectorId || 'connector'} connection${requirement.name ? ` like "${requirement.name}"` : ''}`
        )
        .join('; ')

      return {
        content: [
          {
            type: 'text',
            text:
              `${existing ? 'Updated' : 'Imported'} "${definition.name}" as ${definition.id}, resolved against ${project.path}` +
              (existing || definition.enabled ? '' : '. It is disabled; enable it when ready') +
              (pending ? `\n\nStill to connect: ${pending}` : '')
          }
        ]
      }
    }
  )
}
