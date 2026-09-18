import { MAX_GATE_ROUNDS, MAX_LOOP_ITERATIONS } from '@vornrun/shared/workflow-graph'
import { z } from 'zod'
import { V } from '../validation'

/**
 * One config schema per workflow node type, mirroring the shapes in
 * packages/shared/src/types.ts.
 *
 * Every object is `.loose()`: a key this build does not know passes through
 * untouched. Workflows written by the editor carry fields MCP has never had a
 * reason to name, and a newer app may add more, so refusing unknown keys would
 * make stored workflows impossible to save again. What these schemas do refuse
 * is a known key holding the wrong kind of value, because that is a workflow
 * the engine would misread or fail on at run time.
 *
 * The descriptions are read by agents, through describe_workflow_nodes, so
 * they say what a field does to the run rather than restating its type.
 */

/** Longest template text a config field accepts: scripts, bodies, review pages. */
const longText = z.string().max(200_000, 'Text must be 200000 characters or less')

const taskStatus = z.enum(['todo', 'in_progress', 'in_review', 'done', 'cancelled'])

// Parameters the run dialog prompts for, declared on the manual trigger so they
// travel with the definition. Without this here, a workflow authored over MCP
// could never declare the `{{inputs.*}}` it reads.
export const workflowInputDefSchema = z
  .object({
    key: z
      .string()
      .regex(
        /^[A-Za-z_][A-Za-z0-9_]*$/,
        'key must be a valid identifier — it becomes {{inputs.<key>}}'
      )
      .max(100)
      .describe('Identifier the value is read by, as {{inputs.<key>}}'),
    label: V.shortText.describe('What the run dialog calls this field'),
    type: z
      .enum(['text', 'textarea', 'number', 'select', 'boolean', 'project', 'branch'])
      .describe('Which control the run dialog shows, and how a supplied value is checked'),
    required: z.boolean().optional().describe('A run cannot start without a value'),
    defaultValue: V.shortText
      .optional()
      .describe(
        'Used when the run supplies nothing. A number must be finite, a boolean "true" or "false", a select one of its options'
      ),
    options: z
      .array(
        z
          .object({
            value: V.shortText.describe('What {{inputs.<key>}} expands to'),
            label: V.shortText.describe('What the dialog shows')
          })
          .loose()
      )
      .optional()
      .describe('Choices for a select; a select without any is refused'),
    placeholder: V.shortText.optional().describe('Hint shown in an empty field'),
    description: V.shortText.optional().describe('Help text shown under the field')
  })
  .loose()
  // Reject a declaration that can never be satisfied at the moment it is
  // authored, rather than letting it become a run-time error later. A workflow
  // that cannot be run correctly should not be storable.
  .superRefine((def, ctx) => {
    const options = def.options ?? []

    if (def.type === 'select' && options.length === 0) {
      ctx.addIssue({
        code: 'custom',
        path: ['options'],
        message: `select input "${def.key}" declares no options, so the run dialog could offer nothing`
      })
    }

    if (def.defaultValue === undefined) return

    // Finite, not merely numeric: "Infinity" and "1e999" parse but do not
    // survive JSON, and resolveWorkflowInputs rejects them at run time. Letting
    // one be authored would only defer the same failure to a worse moment.
    if (def.type === 'number' && !Number.isFinite(Number(def.defaultValue))) {
      ctx.addIssue({
        code: 'custom',
        path: ['defaultValue'],
        message: `default "${def.defaultValue}" for number input "${def.key}" is not a finite number`
      })
    }

    if (def.type === 'boolean' && !['true', 'false'].includes(def.defaultValue)) {
      ctx.addIssue({
        code: 'custom',
        path: ['defaultValue'],
        message: `default "${def.defaultValue}" for boolean input "${def.key}" must be "true" or "false"`
      })
    }

    if (
      def.type === 'select' &&
      options.length > 0 &&
      !options.some((o) => o.value === def.defaultValue)
    ) {
      ctx.addIssue({
        code: 'custom',
        path: ['defaultValue'],
        message: `default "${def.defaultValue}" for select input "${def.key}" is not one of its options`
      })
    }
  })

// Two inputs sharing a key cannot both survive under one `{{inputs.<key>}}`.
// The editor already flags this as an error the author has to resolve, so MCP
// authoring must not be the way an ambiguous workflow gets persisted.
export const workflowInputsSchema = z.array(workflowInputDefSchema).superRefine((inputs, ctx) => {
  const seen = new Set<string>()
  inputs.forEach((def, index) => {
    if (seen.has(def.key)) {
      ctx.addIssue({
        code: 'custom',
        path: [index, 'key'],
        message: `duplicate input key "${def.key}" — only one value can survive under {{inputs.${def.key}}}`
      })
    }
    seen.add(def.key)
  })
})

const projectFilter = V.name
  .optional()
  .describe('Only fire for this project, by name. Omit to fire for every project')

// Keyed on triggerType so a wrong field is reported against the trigger the
// author meant, instead of as a failure to match any of eight shapes.
export const triggerConfigSchema = z.discriminatedUnion('triggerType', [
  z
    .object({
      triggerType: z.literal('manual').describe('Runs when someone starts it, or execute_workflow'),
      contextual: z
        .boolean()
        .optional()
        .describe(
          'Offered from a task card or terminal right-click, inheriting its folder, branch and worktree as {{context.*}}'
        ),
      inputs: workflowInputsSchema
        .optional()
        .describe('Parameters asked for before each run, read as {{inputs.<key>}}')
    })
    .loose(),
  z
    .object({
      triggerType: z.literal('once').describe('Runs once at a set time'),
      runAt: V.shortText.describe('When to run, as an ISO 8601 date-time')
    })
    .loose(),
  z
    .object({
      triggerType: z.literal('recurring').describe('Runs on a cron schedule'),
      cron: V.shortText.describe('Five-field cron expression, e.g. "0 9 * * 1-5"'),
      timezone: V.shortText
        .optional()
        .describe('IANA zone the cron is read in, e.g. "Europe/Madrid". Defaults to the machine')
    })
    .loose(),
  z
    .object({
      triggerType: z
        .literal('taskCreated')
        .describe('Runs when a task is created; the task is {{task.*}}'),
      projectFilter
    })
    .loose(),
  z
    .object({
      triggerType: z
        .literal('taskStatusChanged')
        .describe(
          'Runs when a task moves between statuses; the task is {{task.*}}, the move {{trigger.fromStatus}} and {{trigger.toStatus}}'
        ),
      projectFilter,
      fromStatus: taskStatus.optional().describe('Only when it leaves this status'),
      toStatus: taskStatus.optional().describe('Only when it enters this status')
    })
    .loose(),
  z
    .object({
      triggerType: z
        .literal('sessionRestored')
        .describe(
          'Runs when a session comes back after a quit, crash or reboot; {{trigger.restore}} says cold or warm'
        ),
      projectFilter,
      restore: z
        .enum(['cold', 'any'])
        .optional()
        .describe(
          'cold (default): only when a process starts again. any: also when a pane reattaches to one still running'
        ),
      concurrency: z
        .enum(['perProject', 'unbounded'])
        .optional()
        .describe(
          'perProject (default) runs one at a time per project, so restored sessions do not race for one port'
        )
    })
    .loose(),
  z
    .object({
      triggerType: z
        .literal('connectorPoll')
        .describe(
          'Polls a connection on a cron and runs once per new item; the item is {{connectorItem.*}}'
        ),
      connectionId: V.id.describe('Connection to poll, from list_connections'),
      event: V.shortText.describe(
        "Event type from the connector's manifest, e.g. issueCreated (list_connector_actions shows them)"
      ),
      cron: V.shortText.describe('How often to poll, as a five-field cron expression'),
      timezone: V.shortText.optional().describe('IANA zone the cron is read in')
    })
    .loose(),
  z
    .object({
      triggerType: z
        .literal('webhook')
        .describe(
          'Runs when the local webhook route receives a request; its body, headers and query are {{trigger.*}}'
        ),
      method: z.enum(['POST', 'GET']).describe('HTTP method the route answers'),
      token: V.shortText.describe(
        'Secret path segment; a request without it is refused. Use a long random string'
      )
    })
    .loose()
])

const launchAgentNodeConfigSchema = z
  .object({
    agentType: z
      .enum(['claude', 'copilot', 'codex', 'opencode', 'gemini', 'fromTask'])
      .describe(
        "Which agent CLI runs. fromTask uses the task's assigned agent, so it needs a task in scope (a task trigger, taskId or taskFromQueue)"
      ),
    model: V.shortText
      .optional()
      .describe('Model passed to the CLI as written, e.g. "opus". Needs a concrete agentType'),
    projectName: z
      .string()
      .max(200)
      .describe('Registered project the agent works in, by name (list_projects)'),
    projectPath: z
      .string()
      .max(1000)
      .describe("That project's absolute path; templates such as {{context.projectPath}} work"),
    prompt: V.prompt
      .optional()
      .describe('What the agent is asked to do. Templates expand before it is sent'),
    headless: z
      .boolean()
      .optional()
      .describe(
        'Run in the background and wait for it to finish, so later steps can read its output. Without it the agent opens in a terminal tab and the run does not wait on its answer'
      ),
    outputSchema: z
      .record(z.string(), z.unknown())
      .optional()
      .describe(
        'JSON Schema its final answer must match (headless only). Each top-level property becomes {{steps.<slug>.<field>}}; an answer that does not match fails the step'
      ),
    timeoutMs: z
      .number()
      .int()
      .positive()
      .optional()
      .describe(
        'How long a headless run may take before it is killed and the step fails, in milliseconds. Defaults to the app setting'
      ),
    worktreeMode: z
      .enum(['none', 'new', 'fromStep', 'existing'])
      .optional()
      .describe(
        'Where it works: none (the project folder), new (a fresh git worktree), fromStep (the worktree an earlier step made), existing (existingWorktreePath)'
      ),
    worktreeFromStepSlug: V.shortText
      .optional()
      .describe(
        'For worktreeMode fromStep: slug of the earlier agent step whose worktree to reuse'
      ),
    existingWorktreePath: z
      .string()
      .max(1000)
      .optional()
      .describe('For worktreeMode existing: absolute path of the worktree to work in'),
    useWorktree: z
      .union([z.boolean(), z.literal('fromContext')])
      .optional()
      .describe(
        'Older switch for a new worktree; prefer worktreeMode. fromContext inherits it from a contextual manual trigger'
      ),
    branch: V.shortText.optional().describe('Branch to check out or create for the worktree'),
    args: z
      .array(V.shortText)
      .optional()
      .describe('Extra command-line arguments for the agent CLI'),
    displayName: V.shortText.optional().describe('Name shown on the session tab'),
    remoteHostId: V.id.optional().describe('Run on this remote host instead of this machine'),
    promptDelayMs: z
      .number()
      .nonnegative()
      .optional()
      .describe('Wait this long after the agent starts before sending the prompt, in milliseconds'),
    taskId: V.id.optional().describe('Task the session is attached to'),
    taskFromQueue: z
      .boolean()
      .optional()
      .describe("Take the project's next todo task and attach the session to it")
  })
  .loose()
  .refine((c) => !c.outputSchema || c.headless === true, {
    // The engine only parses typed output for headless runs; reject a config that
    // declares a schema it would silently ignore instead of accepting a lie.
    message: 'outputSchema requires headless: true',
    path: ['outputSchema']
  })

const scriptConfigSchema = z
  .object({
    scriptType: z
      .enum(['bash', 'powershell', 'python', 'node'])
      .describe('Interpreter the script runs under'),
    scriptContent: longText.describe(
      'The script. Templates expand before it runs, so quote them as data. Its stdout is {{steps.<slug>.output}}; a non-zero exit fails the step'
    ),
    cwd: z.string().max(1000).optional().describe('Directory it runs in. Defaults to projectPath'),
    projectName: z.string().max(200).optional().describe('Project the script belongs to, by name'),
    projectPath: z
      .string()
      .max(1000)
      .optional()
      .describe("That project's absolute path, used as the working directory when cwd is unset"),
    args: z.array(V.shortText).optional().describe('Arguments passed to the script'),
    secretsFrom: V.id
      .optional()
      .describe(
        "Connection whose stored secrets become this script's environment variables at run time, so no secret is written into the workflow"
      ),
    runId: V.id.optional().describe('Set by the engine; leave it out')
  })
  .loose()

const conditionConfigSchema = z
  .object({
    variable: longText.describe(
      'The value to test, usually a template such as {{steps.review.approved}}'
    ),
    operator: z
      .enum(['equals', 'notEquals', 'contains', 'notContains', 'isEmpty', 'isNotEmpty'])
      .describe('How variable is compared with value. Comparisons are on text'),
    value: longText
      .optional()
      .describe('What variable is compared with; ignored by isEmpty and isNotEmpty')
  })
  .loose()

const approvalConfigSchema = z
  .object({
    message: longText
      .optional()
      .describe('The question the reviewer is asked. Templates expand when the gate opens'),
    timeoutMs: z
      .number()
      .int()
      .positive()
      .optional()
      .describe('Reject automatically after this long, in milliseconds. Omit to wait for ever'),
    view: longText
      .optional()
      .describe(
        'A review page shown beside the question: HTML, or the path of a .html file, with templates filled in when the gate opens'
      ),
    edit: longText
      .optional()
      .describe(
        'Text the reviewer may rewrite, as a template. Later steps read the result as {{steps.<gate>.text}}. When it expands to a JSON list of records the reviewer sees a table, and the rows they keep are {{steps.<gate>.items}}'
      ),
    feedback: z
      .object({
        from: V.id.describe(
          'Id of an earlier step, outside any loop body, that leads to this gate. Send back re-runs everything from it to the gate'
        ),
        maxRounds: z
          .number()
          .int()
          .min(2)
          .max(MAX_GATE_ROUNDS)
          .describe(`How many times the gate may ask, the first included (2 to ${MAX_GATE_ROUNDS})`)
      })
      .loose()
      .optional()
      .describe(
        'Lets the reviewer send the work back with a comment instead of only approving or rejecting; the comment is {{steps.<gate>.feedback}} to the steps that redo it'
      )
  })
  .loose()

const createTaskFromItemConfigSchema = z
  .object({
    nodeType: z.literal('createTaskFromItem').optional().describe('Always createTaskFromItem'),
    project: z
      .string()
      .max(200)
      .describe(
        "Project the task lands in, by name, or fromConnection to use the connection's execution project"
      ),
    initialStatus: taskStatus.describe(
      'Status of a newly created task. Re-syncing an existing one never changes its status'
    )
  })
  .loose()

const callConnectorActionConfigSchema = z
  .object({
    nodeType: z.literal('callConnectorAction').optional().describe('Always callConnectorAction'),
    connectionId: z
      .string()
      .max(100)
      .describe('Connection the action runs against, from list_connections'),
    action: V.shortText.describe(
      'Action type from the connector, e.g. commentOnPullRequest; list_connector_actions lists them with their args'
    ),
    actionLabel: V.shortText.optional().describe("The action's display name, for the canvas"),
    connectorId: V.shortText
      .optional()
      .describe('Connector the action belongs to, for a step placed before a connection exists'),
    args: z
      .record(z.string(), longText)
      .optional()
      .describe(
        "The action's arguments by name, every value a string template; a list or object a template names arrives as JSON text. list_connector_actions gives each action's args"
      )
  })
  .loose()

const httpRequestConfigSchema = z
  .object({
    nodeType: z.literal('httpRequest').optional().describe('Always httpRequest'),
    method: z.enum(['GET', 'POST', 'PUT', 'PATCH', 'DELETE']).describe('HTTP method'),
    url: z
      .string()
      .max(2048)
      .describe(
        "Absolute URL, or a path resolved against the profile's base URL. Templates expand"
      ),
    headers: z
      .record(z.string(), longText)
      .optional()
      .describe('Request headers. Names are literal; values are templates'),
    body: longText.optional().describe('Request body as a template; empty sends none'),
    profileConnectionId: V.id
      .optional()
      .describe(
        'An http connection whose base URL and auth are applied server-side, so no credential sits in the workflow'
      )
  })
  .loose()

const loopConfigSchema = z
  .object({
    nodeType: z.literal('loop').optional().describe('Always loop'),
    mode: z
      .enum(['repeat', 'forEach'])
      .optional()
      .describe(
        `repeat (default) runs the body up to maxIterations times; forEach runs it once for each item of items, with no cap on how many`
      ),
    items: longText
      .optional()
      .describe(
        'For forEach: the list to walk, as a template naming it, e.g. {{steps.gate.items}}. JSON text of a list works too, and so does an object holding exactly one list. Each pass reads its item as {{loop.item}} or {{loop.item.<field>}}'
      ),
    bodyNodeIds: z
      .array(V.id)
      .min(1)
      .describe(
        'Ids of the steps the loop runs each pass. Wire them with edges: loop -> the entry steps, then between body steps, and from the last body steps to the step after the loop'
      ),
    maxIterations: z
      .number()
      .optional()
      .describe(
        `For repeat: how many passes at most, a whole number from 1 to ${MAX_LOOP_ITERATIONS}. Required for repeat, ignored for forEach`
      ),
    until: conditionConfigSchema
      .optional()
      .describe(
        'Checked after each pass; the loop stops early once it holds, e.g. {{steps.review.approved}} equals true'
      )
  })
  .loose()
  // A loop that cannot say what it walks, or how often it repeats, cannot
  // start. Refusing it here puts the error on the call that wrote it instead of
  // on a run someone is waiting for.
  .superRefine((config, ctx) => {
    if (config.mode === 'forEach') {
      if (!config.items?.trim()) {
        ctx.addIssue({
          code: 'custom',
          path: ['items'],
          message: 'a forEach loop needs items: a template naming the list to walk'
        })
      }
      return
    }
    const max = config.maxIterations
    if (typeof max !== 'number' || !Number.isInteger(max) || max < 1 || max > MAX_LOOP_ITERATIONS) {
      ctx.addIssue({
        code: 'custom',
        path: ['maxIterations'],
        message: `a repeat loop needs maxIterations as a whole number from 1 to ${MAX_LOOP_ITERATIONS}`
      })
    }
  })

export const edgeSchema = z.object({
  id: V.id.describe('Unique within the workflow'),
  source: V.id.describe('Id of the node that runs first'),
  target: V.id.describe('Id of the node that runs after it'),
  // Which branch of a `condition` node this edge represents. Omit for normal
  // edges; required to wire both outcomes of a condition.
  conditionBranch: z
    .enum(['true', 'false'])
    .optional()
    .describe(
      'On an edge leaving a condition: which outcome takes it. Wire both. Omit on every other edge'
    )
})

export const NODE_TYPES = [
  'trigger',
  'launchAgent',
  'script',
  'condition',
  'approval',
  'createTaskFromItem',
  'callConnectorAction',
  'httpRequest',
  'loop'
] as const

export type NodeType = (typeof NODE_TYPES)[number]

export const configSchemaByType: Record<NodeType, z.ZodType> = {
  trigger: triggerConfigSchema,
  launchAgent: launchAgentNodeConfigSchema,
  script: scriptConfigSchema,
  condition: conditionConfigSchema,
  approval: approvalConfigSchema,
  createTaskFromItem: createTaskFromItemConfigSchema,
  callConnectorAction: callConnectorActionConfigSchema,
  httpRequest: httpRequestConfigSchema,
  loop: loopConfigSchema
}

/**
 * Why a node's config would be refused, as zod issues whose paths start inside
 * the config. Empty when the config is sound.
 */
export function nodeConfigIssues(type: NodeType, config: unknown): z.ZodError['issues'] {
  const result = configSchemaByType[type].safeParse(config)
  return result.success ? [] : result.error.issues
}
