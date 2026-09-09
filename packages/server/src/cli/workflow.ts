import type { WorkflowDefinition } from '@vornrun/shared/types'
import type { ClientContext } from './deps'
import { EXIT_FAILURE, EXIT_OK, EXIT_UNREACHABLE, EXIT_USAGE } from './exit'
import { asJson, paintStatus, shortId, table, timeAgo } from './output'

export const WORKFLOW_USAGE = `Usage
  vorn workflow list [--json]
  vorn workflow run <name or id> [--input key=value] [--json]
  vorn workflow stop <run>
  vorn workflow runs [--workflow <name or id>] [--limit <n>] [--json]

A run happens in the server, so it keeps going when nothing is watching. The
command answers with the run rather than waiting for it to finish.
`

function usage(ctx: ClientContext, message: string): number {
  ctx.writeErr(`vorn: ${message}\n\n${WORKFLOW_USAGE}`)
  return EXIT_USAGE
}

function failed(ctx: ClientContext, what: string, err: unknown): number {
  ctx.writeErr(`vorn: ${what}: ${err instanceof Error ? err.message : String(err)}\n`)
  return EXIT_FAILURE
}

/** What starts a workflow, read off its trigger node. */
function triggerKind(workflow: WorkflowDefinition): string {
  const trigger = workflow.nodes.find((node) => node.type === 'trigger')
  if (!trigger) return 'none'
  const kind = (trigger.config as { triggerType?: string }).triggerType
  return kind ?? 'manual'
}

/**
 * A workflow by id, by a prefix of one, or by name -- and only when that names
 * exactly one of them.
 *
 * A prefix that matches several is refused rather than resolved to the first:
 * seeded and imported workflows carry ids like `import:foo`, so shared prefixes
 * are ordinary here, and acting on the wrong workflow is not a small mistake.
 */
function findWorkflow(workflows: WorkflowDefinition[], given: string): WorkflowDefinition {
  const exact = workflows.find((w) => w.id === given)
  if (exact) return exact

  const byPrefix = workflows.filter((w) => w.id.startsWith(given))
  if (byPrefix.length === 1) return byPrefix[0]
  if (byPrefix.length > 1) {
    throw new Error(
      `"${given}" matches ${byPrefix.length} workflows: ${byPrefix.map((w) => w.name).join(', ')}`
    )
  }

  // Trimmed on both sides: a trailing space is invisible in the list, so two
  // rows that read the same must be ambiguous rather than quietly distinct.
  const wanted = given.trim().toLowerCase()
  const byName = workflows.filter((w) => w.name.trim().toLowerCase() === wanted)
  if (byName.length === 1) return byName[0]
  if (byName.length === 0) throw new Error(`no workflow matches "${given}"`)
  throw new Error(`"${given}" matches ${byName.length} workflows; use an id`)
}

async function listWorkflows(ctx: ClientContext): Promise<number> {
  try {
    const workflows = await ctx.rpc.call('workflow:list')
    if (ctx.args.json) {
      ctx.write(asJson(workflows))
      return EXIT_OK
    }
    if (workflows.length === 0) {
      ctx.writeErr('No workflows.\n')
      return EXIT_OK
    }
    // How it went and when are two columns, not one: colour belongs on the
    // status word alone, and it cannot be picked out of "success 2h ago".
    const statusColumn = 4
    ctx.write(
      table(
        ['ID', 'NAME', 'TRIGGER', 'ENABLED', 'LAST RUN', 'WHEN'],
        workflows.map((w) => [
          shortId(w.id),
          w.name,
          triggerKind(w),
          w.enabled ? 'yes' : 'no',
          w.lastRunAt ? (w.lastRunStatus ?? 'ran') : '-',
          w.lastRunAt ? timeAgo(w.lastRunAt) : '-'
        ]),
        (cell, column) => (column === statusColumn ? paintStatus(cell, ctx.plain) : cell)
      )
    )
    return EXIT_OK
  } catch (err) {
    return failed(ctx, 'could not list workflows', err)
  }
}

async function startRun(ctx: ClientContext, given: string | undefined): Promise<number> {
  if (!given) return usage(ctx, 'workflow run needs a workflow')
  try {
    const workflows = await ctx.rpc.call('workflow:list')
    const workflow = findWorkflow(workflows, given)
    const execution = await ctx.rpc.call('workflow:run', {
      workflowId: workflow.id,
      ...(ctx.args.inputs && { context: { inputs: ctx.args.inputs } })
    })
    if (!execution) {
      ctx.writeErr(`vorn: "${workflow.name}" did not start. Check the server log.\n`)
      return EXIT_FAILURE
    }

    if (ctx.args.json) {
      ctx.write(asJson(execution))
      return EXIT_OK
    }
    ctx.write(
      [
        `run      ${execution.runId}`,
        `workflow ${workflow.name}`,
        `steps    ${execution.nodeStates.filter((ns) => ns.status === 'pending').length} to run`,
        ''
      ].join('\n')
    )
    ctx.writeErr(`Follow it with: vorn workflow runs --workflow ${shortId(workflow.id)}\n`)
    return EXIT_OK
  } catch (err) {
    return failed(ctx, 'could not start the workflow', err)
  }
}

/** A run by id or by any prefix that names one, across every workflow. */
async function resolveRunId(ctx: ClientContext, given: string): Promise<string> {
  const runs = await ctx.rpc.call('workflowRun:listAll', {})
  const exact = runs.find((run) => run.runId === given)
  if (exact) return exact.runId

  const matches = runs.filter((run) => run.runId.startsWith(given))
  if (matches.length === 1) return matches[0].runId
  if (matches.length === 0) throw new Error(`no run matches "${given}"`)
  throw new Error(
    `"${given}" matches ${matches.length} runs: ${matches.map((r) => shortId(r.runId)).join(', ')}`
  )
}

async function stopRun(ctx: ClientContext, given: string | undefined): Promise<number> {
  if (!given) return usage(ctx, 'workflow stop needs a run')
  try {
    const runId = await resolveRunId(ctx, given)
    await ctx.rpc.call('workflow:stopRun', { runId })
    ctx.writeErr(`Stopped ${shortId(runId)}.\n`)
    return EXIT_OK
  } catch (err) {
    return failed(ctx, 'could not stop the run', err)
  }
}

async function listRuns(ctx: ClientContext): Promise<number> {
  try {
    const workflows = await ctx.rpc.call('workflow:list')
    const names = new Map(workflows.map((w) => [w.id, w.name]))

    const runs = ctx.args.workflow
      ? await ctx.rpc.call('workflowRun:list', {
          workflowId: findWorkflow(workflows, ctx.args.workflow).id,
          limit: ctx.args.limit
        })
      : await ctx.rpc.call('workflowRun:listAll', { limit: ctx.args.limit })

    if (ctx.args.json) {
      ctx.write(asJson(runs))
      return EXIT_OK
    }
    if (runs.length === 0) {
      ctx.writeErr('No runs.\n')
      return EXIT_OK
    }

    const statusColumn = 2
    ctx.write(
      table(
        ['RUN', 'WORKFLOW', 'STATUS', 'STARTED'],
        runs.map((run) => [
          shortId(run.runId),
          names.get(run.workflowId) ?? shortId(run.workflowId),
          run.status,
          timeAgo(run.startedAt)
        ]),
        (cell, column) => (column === statusColumn ? paintStatus(cell, ctx.plain) : cell)
      )
    )
    return EXIT_OK
  } catch (err) {
    return failed(ctx, 'could not list runs', err)
  }
}

export async function runWorkflowCommand(ctx: ClientContext): Promise<number> {
  const [, verb, ...rest] = ctx.args.positionals

  if (ctx.args.help) {
    ctx.write(WORKFLOW_USAGE)
    return EXIT_OK
  }
  if (!verb) {
    ctx.writeErr(WORKFLOW_USAGE)
    return EXIT_USAGE
  }
  if (!['list', 'run', 'runs', 'stop'].includes(verb)) {
    return usage(ctx, `unknown workflow command "${verb}"`)
  }
  if (!(await ctx.server())) return EXIT_UNREACHABLE

  switch (verb) {
    case 'list':
      return listWorkflows(ctx)
    case 'run':
      return startRun(ctx, rest[0])
    case 'stop':
      return stopRun(ctx, rest[0])
    default:
      return listRuns(ctx)
  }
}
