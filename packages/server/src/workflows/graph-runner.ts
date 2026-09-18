import type { WorkflowEdge, WorkflowExecution, WorkflowNode } from '@vornrun/shared/types'
import type { StepOutputs } from '@vornrun/shared/template-vars'
import {
  buildGraph,
  collectSkippedBranch,
  skipEntryPoints,
  stopsRunOnError,
  updateNodeState
} from '@vornrun/shared/workflow-graph'
import log from '../logger'

/**
 * One graph, run in waves: every step whose predecessors have settled runs,
 * together, then the next wave. The main run drives the workflow through this,
 * and a loop drives its body through it once per pass — so a condition inside
 * a loop branches exactly as it does outside one.
 */
export interface WaveRun {
  /** The steps this run schedules. */
  nodes: readonly WorkflowNode[]
  /** Edges among those steps, plus any from `roots`. */
  edges: readonly WorkflowEdge[]
  /** Ids treated as already complete: the loop, when running its body. */
  roots?: readonly string[]
  execution: WorkflowExecution
  signal: AbortSignal
  /** Past this many waves the graph is spinning, and the run says so. */
  maxWaves: number
  staggerMs?: number
  stepOutputs: () => StepOutputs
  runNode: (node: WorkflowNode, stepOutputs: StepOutputs) => Promise<void>
  persist: () => void
}

/** Returns the ids it skipped, which decide whether their failures count against the run. */
export async function runWaves(run: WaveRun): Promise<Set<string>> {
  const { execution, edges } = run
  const { successors, predecessors } = buildGraph(edges)
  const roots = new Set(run.roots ?? [])

  // Rebuilt at the start of every wave so external mutations (a sibling gate
  // approved by another re-entry, say) are picked up.
  const completed = new Set<string>()
  const skipped = new Set<string>()
  const running = new Set<string>()
  const stateOf = (id: string) => execution.nodeStates.find((s) => s.nodeId === id)
  const settled = (id: string) => completed.has(id) || skipped.has(id)

  function rebuild(): void {
    completed.clear()
    skipped.clear()
    for (const id of roots) completed.add(id)
    for (const ns of execution.nodeStates) {
      if (ns.status === 'success' || ns.status === 'error') completed.add(ns.nodeId)
      else if (ns.status === 'skipped') skipped.add(ns.nodeId)
    }
  }

  function markSkippedBranch(start: string): void {
    for (const id of collectSkippedBranch(start, successors, predecessors, settled)) skipped.add(id)
  }

  function ready(): WorkflowNode[] {
    return run.nodes.filter((node) => {
      if (node.type === 'trigger' || roots.has(node.id)) return false
      if (settled(node.id) || running.has(node.id)) return false
      if (stateOf(node.id)?.status === 'waiting') return false
      const preds = predecessors.get(node.id) ?? []
      return preds.every(settled) && preds.some((p) => completed.has(p))
    })
  }

  function stampSkipped(
    updates: Parameters<typeof updateNodeState>[2],
    onlyPending: boolean
  ): void {
    for (const id of skipped) {
      if (onlyPending && stateOf(id)?.status !== 'pending') continue
      updateNodeState(execution, id, {
        status: 'skipped',
        completedAt: new Date().toISOString(),
        ...updates
      })
    }
    run.persist()
  }

  let wave = 0
  while (!run.signal.aborted) {
    rebuild()
    const batch = ready()
    if (batch.length === 0) break

    wave++
    if (wave > run.maxWaves) {
      throw new Error(`Stopped after ${run.maxWaves} waves: steps kept becoming ready again`)
    }
    log.info(
      `[workflow] wave ${wave}: executing ${batch.length} node(s) in parallel: ${batch.map((n) => n.label).join(', ')}`
    )
    if (wave > 1 && run.staggerMs) await new Promise((r) => setTimeout(r, run.staggerMs))

    const outputs = run.stepOutputs()
    const stateless: string[] = []

    await Promise.all(
      batch.map(async (node) => {
        running.add(node.id)
        try {
          await run.runNode(node, outputs)
        } catch (err) {
          log.error({ err, node: node.label }, '[workflow] a step failed')
          updateNodeState(execution, node.id, {
            status: 'error',
            completedAt: new Date().toISOString(),
            error: err instanceof Error ? err.message : String(err)
          })
          run.persist()
        }
        running.delete(node.id)

        const state = stateOf(node.id)
        // A step the run holds no state for would be ready again on every wave.
        if (!state) {
          const error = `Step "${node.label}" has no state in this run; its workflow changed under it`
          execution.nodeStates.push({
            nodeId: node.id,
            status: 'error',
            completedAt: new Date().toISOString(),
            error
          })
          stateless.push(error)
          return
        }
        if (state.status === 'waiting') return
        completed.add(node.id)

        // A failed step stops what it feeds unless it opted out. Skipping that
        // branch is what halts it: nothing downstream becomes ready. Steps a
        // live path still reaches are left alone by the join guard.
        if (state.status === 'error' && stopsRunOnError(node)) {
          for (const entry of skipEntryPoints(node.id, [...edges], predecessors, settled)) {
            markSkippedBranch(entry)
          }
          stampSkipped({ error: `Skipped: "${node.label}" failed` }, true)
          return
        }

        if (node.type !== 'condition') return
        // A condition that failed answered nothing, so neither branch is the one it chose.
        const skipBranch =
          state.status === 'error' ? undefined : state.output === 'true' ? 'false' : 'true'
        let skippedAny = false
        for (const edge of edges) {
          if (edge.source !== node.id || !edge.conditionBranch) continue
          if (skipBranch && edge.conditionBranch !== skipBranch) continue
          markSkippedBranch(edge.target)
          skippedAny = true
        }
        if (skippedAny) stampSkipped({ skipReason: 'branch' }, false)
      })
    )
    if (stateless.length > 0) throw new Error(stateless[0])
  }
  return skipped
}
