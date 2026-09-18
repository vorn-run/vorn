import { MAX_GATE_ROUNDS, MAX_LOOP_ITERATIONS } from '@vornrun/shared/workflow-graph'
import { z } from 'zod'
import type { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import { configSchemaByType, edgeSchema, NODE_TYPES, type NodeType } from './node-config-schemas'

/**
 * The reference an agent needs to write a workflow without guessing.
 *
 * create_workflow takes each node's config as an open object, because the
 * shape depends on the node's type and a tool's argument schema cannot say
 * that. Without this, an agent learned a config's fields by having calls
 * refused one field at a time. Everything here is derived from, or sits next
 * to, the schemas those calls are checked against, so the two cannot disagree.
 */

type Output = { field: string; description: string }

interface NodeTypeDoc {
  summary: string
  outputs: Output[]
  rules: string[]
}

/** The fields every step has, whatever its type, once it has run. */
const COMMON_OUTPUTS: Output[] = [
  { field: 'status', description: 'success or error' },
  { field: 'error', description: 'Why it failed, or empty' }
]

const DOCS: Record<NodeType, NodeTypeDoc> = {
  trigger: {
    summary:
      'What starts a run: by hand, on a schedule, on a task event, when a session is restored, by polling a connection, or on a webhook. config.triggerType picks which.',
    outputs: [],
    rules: [
      'A workflow has exactly one trigger, and no edge points into it.',
      'A manual trigger may declare inputs; each is read anywhere as {{inputs.<key>}}, and execute_workflow takes their values.',
      'What the trigger carries is read as {{task.*}}, {{trigger.*}} or {{connectorItem.*}}, depending on its type.'
    ]
  },
  launchAgent: {
    summary:
      'Starts an AI coding agent in a project with a prompt. Headless, it runs in the background and the run waits for its answer; otherwise it opens a terminal tab.',
    outputs: [
      { field: 'output', description: "The agent's final answer (headless)" },
      {
        field: '<outputSchema property>',
        description:
          'Each top-level property of config.outputSchema, parsed from the answer (headless only)'
      },
      { field: 'worktreePath', description: 'The directory the agent worked in' },
      ...COMMON_OUTPUTS
    ],
    rules: [
      'outputSchema requires headless: true, and an answer that does not match it fails the step.',
      'Read typed fields in a condition, e.g. variable {{steps.review.approved}} equals true, rather than matching words in output.',
      'agentType fromTask needs a task in scope: a task trigger, taskId or taskFromQueue. model needs a concrete agentType.',
      'worktreeMode fromStep reuses the worktree of the earlier agent step named by worktreeFromStepSlug.'
    ]
  },
  script: {
    summary: 'Runs a bash, python, node or powershell script.',
    outputs: [
      { field: 'output', description: 'What the script printed to stdout' },
      ...COMMON_OUTPUTS
    ],
    rules: [
      "Templates expand into the script text before it runs. Keep a value as data rather than code, e.g. inside a quoted heredoc: cat <<'EOF' ... EOF.",
      'A non-zero exit fails the step.',
      'secretsFrom names a connection whose secrets become environment variables, so no secret is written into the workflow.'
    ]
  },
  condition: {
    summary:
      'Compares a value, usually a step output, and sends the run down its true or its false branch.',
    outputs: [{ field: 'output', description: '"true" or "false"' }, ...COMMON_OUTPUTS],
    rules: [
      'Give it two outgoing edges, one with conditionBranch "true" and one with "false". Steps on the branch not taken are skipped.',
      'Comparisons are on text: equals true matches the text "true".',
      'Inside a loop body, both branches must stay inside the body.'
    ]
  },
  approval: {
    summary:
      'Pauses the run until a person approves or rejects, in the app or with resolve_gate. It can show a review page, let the reviewer rewrite some text, and send the work back with a comment.',
    outputs: [
      { field: 'text', description: 'The editable text as the reviewer left it' },
      {
        field: 'items',
        description:
          'When the editable text is a JSON list of records: the rows the reviewer kept, as a list, ready for a forEach loop'
      },
      { field: 'feedback', description: "The reviewer's latest comment" },
      { field: 'feedbackAll', description: 'Every comment, one line per round' },
      { field: 'round', description: 'Which time the gate asked, from 1' },
      ...COMMON_OUTPUTS
    ],
    rules: [
      'A gate cannot sit inside a loop body.',
      'When edit expands to a JSON list of records the reviewer sees a table; a rewrite that is no longer valid JSON is refused.',
      `feedback.from is the id of an earlier step that leads to this gate, not the trigger and not a step inside a loop. Sending back re-runs every step from it to the gate. maxRounds is 2 to ${MAX_GATE_ROUNDS}.`,
      'A reject ends the run.'
    ]
  },
  createTaskFromItem: {
    summary:
      'Creates a task from the connector item that started the run, or updates the one it created before.',
    outputs: [
      { field: 'output', description: 'Imported or Updated, with the item id and title' },
      ...COMMON_OUTPUTS
    ],
    rules: [
      'Needs {{connectorItem.*}}: put it after a connectorPoll trigger.',
      'Re-syncing an item updates its title and description but never its status.'
    ]
  },
  callConnectorAction: {
    summary:
      'Runs one action of a connected service (a comment, an issue, a merge...) with arguments built from templates.',
    outputs: [
      { field: 'output', description: 'The action result as text' },
      {
        field: "<the action's output fields>",
        description:
          'Each field the action declares in its outputSchema; list_connector_actions gives them'
      },
      ...COMMON_OUTPUTS
    ],
    rules: [
      'connectionId comes from list_connections; action and its args from list_connector_actions.',
      'A step waits, rather than fails, when its connection needs signing in again.'
    ]
  },
  httpRequest: {
    summary: 'Sends one HTTP request, optionally through an http connection that adds auth.',
    outputs: [
      { field: 'body', description: 'Response body, JSON-parsed when possible' },
      { field: 'headers', description: 'Response headers, by lower-case name' },
      {
        field: 'output',
        description: 'HTTP <code>, e.g. "HTTP 404": the way to read the response status'
      },
      {
        field: 'status',
        description:
          "success or error: the step's own status, which replaces the response code under this name"
      },
      { field: 'error', description: 'Why it failed, or empty' }
    ],
    rules: [
      'Put credentials in an http connection (profileConnectionId), not in headers.',
      'Any answer succeeds, a 4xx or 5xx included: to act on the code, test {{steps.<slug>.output}} (e.g. contains "HTTP 2"). Only a network error or the 30 second timeout fails the step. Redirects are not followed.'
    ]
  },
  loop: {
    summary: `Runs a group of steps (its body) several times: repeat up to ${MAX_LOOP_ITERATIONS} passes, stopping early when until holds, or forEach once per item of a list.`,
    outputs: [
      { field: 'output', description: 'How many passes ran' },
      { field: 'passes', description: 'How many passes ran, as a number' },
      {
        field: 'count',
        description: 'How many passes were planned: maxIterations, or the number of items'
      },
      {
        field: 'results',
        description:
          'Per pass: {index, item?, status, steps: {<slug>: {output, status, error, ...}}}; each output cut to loopResultOutputChars'
      },
      {
        field: 'outputs',
        description:
          'Per pass, the output of the last body step that ran (on the branch taken), cut to loopResultOutputChars'
      },
      ...COMMON_OUTPUTS
    ],
    rules: [
      'bodyNodeIds lists every step in the body. Wire the loop to each body entry step, body steps to each other, and the last body steps to the step after the loop.',
      'Nothing outside the loop may point into its body; only the loop starts its steps.',
      'A body cannot hold an approval gate, a trigger or another loop, and cannot contain a cycle.',
      `repeat needs maxIterations, 1 to ${MAX_LOOP_ITERATIONS}. forEach needs items and has no cap.`,
      'Body steps read {{loop.item}} / {{loop.item.<field>}} (forEach), {{loop.index}} from 0, {{loop.number}} from 1 and {{loop.count}}.',
      'A body step that fails ends the loop there, unless that step has onError: continue.',
      'Inside the body, {{steps.<slug>.*}} of a body step is this pass. After the loop, read the passes through {{steps.<loop>.results}} or .outputs.'
    ]
  }
}

/**
 * A complete workflow that uses the parts agents find hardest to wire: a gate
 * that shows a table, a forEach loop over the rows kept, a condition inside the
 * body, and a step after the loop reading every pass. Kept valid by a test that
 * runs it through the same checks create_workflow does.
 */
const EXAMPLE = {
  nodes: [
    {
      id: 'start',
      type: 'trigger',
      label: 'Start',
      config: {
        triggerType: 'manual',
        inputs: [{ key: 'pullRequest', label: 'Pull request', type: 'text', required: true }]
      },
      position: { x: 0, y: 0 }
    },
    {
      id: 'review',
      type: 'callConnectorAction',
      label: 'Read review findings',
      slug: 'review',
      config: {
        nodeType: 'callConnectorAction',
        connectionId: '<connection id from list_connections>',
        action: 'listReviewFindings',
        args: { pullRequest: '{{inputs.pullRequest}}' }
      },
      position: { x: 0, y: 140 }
    },
    {
      id: 'gate',
      type: 'approval',
      label: 'Pick findings to post',
      slug: 'gate',
      config: {
        message: 'Keep the findings worth a comment, and delete the rest.',
        edit: '{{steps.review.findings}}'
      },
      position: { x: 0, y: 280 }
    },
    {
      id: 'loop',
      type: 'loop',
      label: 'For each finding',
      slug: 'loop',
      config: {
        nodeType: 'loop',
        mode: 'forEach',
        items: '{{steps.gate.items}}',
        bodyNodeIds: ['severe', 'comment', 'skip']
      },
      position: { x: 0, y: 420 }
    },
    {
      id: 'severe',
      type: 'condition',
      label: 'High severity?',
      slug: 'severe',
      config: { variable: '{{loop.item.severity}}', operator: 'equals', value: 'high' },
      position: { x: 0, y: 560 }
    },
    {
      id: 'comment',
      type: 'callConnectorAction',
      label: 'Comment on the line',
      slug: 'comment',
      config: {
        nodeType: 'callConnectorAction',
        connectionId: '<connection id from list_connections>',
        action: 'commentOnPullRequest',
        args: {
          pullRequest: '{{inputs.pullRequest}}',
          path: '{{loop.item.path}}',
          line: '{{loop.item.line}}',
          body: '{{loop.item.body}}'
        }
      },
      position: { x: -160, y: 700 }
    },
    {
      id: 'skip',
      type: 'script',
      label: 'Note the skip',
      slug: 'skip',
      config: {
        scriptType: 'bash',
        scriptContent: "cat <<'EOF'\nNot posted: {{loop.item.path}}\nEOF\n"
      },
      position: { x: 160, y: 700 }
    },
    {
      id: 'summary',
      type: 'script',
      label: 'Summarise',
      slug: 'summary',
      config: {
        scriptType: 'bash',
        scriptContent: "cat <<'EOF'\nWhat each finding came to:\n{{steps.loop.outputs}}\nEOF\n"
      },
      position: { x: 0, y: 840 }
    }
  ],
  edges: [
    { id: 'e-start-review', source: 'start', target: 'review' },
    { id: 'e-review-gate', source: 'review', target: 'gate' },
    { id: 'e-gate-loop', source: 'gate', target: 'loop' },
    { id: 'e-loop-severe', source: 'loop', target: 'severe' },
    { id: 'e-severe-comment', source: 'severe', target: 'comment', conditionBranch: 'true' },
    { id: 'e-severe-skip', source: 'severe', target: 'skip', conditionBranch: 'false' },
    { id: 'e-comment-summary', source: 'comment', target: 'summary' },
    { id: 'e-skip-summary', source: 'skip', target: 'summary' }
  ]
}

function jsonSchema(schema: z.ZodType): unknown {
  return z.toJSONSchema(schema, { io: 'input', unrepresentable: 'any' })
}

/** Built once: none of it changes while the server runs. */
export const WORKFLOW_NODES_REFERENCE = {
  nodeTypes: Object.fromEntries(
    NODE_TYPES.map((type) => [
      type,
      {
        summary: DOCS[type].summary,
        configSchema: jsonSchema(configSchemaByType[type]),
        outputs: DOCS[type].outputs,
        rules: DOCS[type].rules
      }
    ])
  ) as Record<
    NodeType,
    { summary: string; configSchema: unknown; outputs: Output[]; rules: string[] }
  >,
  edgeSchema: jsonSchema(edgeSchema),
  limits: {
    repeatMaxIterations: MAX_LOOP_ITERATIONS,
    forEachMaxItems: 'uncapped',
    maxGateRounds: MAX_GATE_ROUNDS,
    templateTextChars: 50_000,
    loopResultOutputChars: 8000
  },
  templates: {
    syntax:
      '{{namespace.path.to.field}} anywhere a config takes text. An unknown namespace is left as written; a known one with nothing there becomes empty. Text longer than templateTextChars keeps its end.',
    namespaces: {
      steps:
        '{{steps.<slug>.<field>}}: the output of an earlier step, by its slug; the fields are listed under each node type',
      task: '{{task.title}}, .description, .id, .status, .branch, .projectName: the task a task trigger fired for',
      trigger:
        '{{trigger.*}}: what the trigger carried: fromStatus/toStatus, restore, or a webhook body, headers and query',
      connectorItem:
        '{{connectorItem.title}}, .body, .externalId, .externalUrl: the item a connectorPoll trigger fired for',
      inputs: '{{inputs.<key>}}: the values a manual run was started with',
      context:
        '{{context.cwd}}, .projectPath, .projectName, .branch, .worktreePath: where a contextual manual run was started from',
      loop: '{{loop.item}}, {{loop.item.<field>}}, {{loop.index}} (from 0), {{loop.number}} (from 1), {{loop.count}}: only inside a loop body'
    },
    examples: [
      '{{steps.review.approved}}',
      '{{steps.gate.items}}',
      '{{loop.item.path}}',
      '{{steps.loop.outputs}}',
      '{{inputs.pullRequest}}',
      '{{task.title}}'
    ]
  },
  example: EXAMPLE
}

export function registerDescribeNodesTool(server: McpServer): void {
  server.tool(
    'describe_workflow_nodes',
    'The reference for writing a workflow: for each node type its config schema, the outputs later steps read, and the rules create_workflow checks; the edge schema; limits; the template namespaces; and a complete example workflow to adapt. Call it before create_workflow or update_workflow.',
    {
      types: z
        .array(z.enum(NODE_TYPES))
        .optional()
        .describe('Only these node types. Omit for all nine')
    },
    async (args) => {
      const reference = args.types?.length
        ? {
            ...WORKFLOW_NODES_REFERENCE,
            nodeTypes: Object.fromEntries(
              args.types.map((type) => [type, WORKFLOW_NODES_REFERENCE.nodeTypes[type]])
            )
          }
        : WORKFLOW_NODES_REFERENCE
      return { content: [{ type: 'text', text: JSON.stringify(reference, null, 2) }] }
    }
  )
}
