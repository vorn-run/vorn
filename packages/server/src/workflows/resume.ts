import { listRunningRuns, listRunsWithWaitingGates } from '../database'
import { configManager } from '../config-manager'
import log from '../logger'
import { reconcileRunningExecutions, rescheduleWaitingGateTimers } from './engine'

/**
 * Pick up the runs the last server left behind.
 *
 * Both halves used to happen on every window open, because the runs lived in
 * the window: a gate's timeout timer was re-armed, and a run still marked
 * `running` was reconciled against its session events and closed out if its
 * agent had already exited. Neither survives a process, and the process they
 * belong to is this one now -- so without this a restart leaves a gate that can
 * never time out and a finished run that says it is still going, for ever.
 */
export async function resumeRunsAfterStart(): Promise<void> {
  const workflows = configManager.loadConfig().workflows ?? []
  if (workflows.length === 0) return

  try {
    const waiting = listRunsWithWaitingGates()
    if (waiting.length > 0) {
      log.info({ count: waiting.length }, '[workflow] re-arming gates left waiting')
      rescheduleWaitingGateTimers(waiting, workflows)
    }

    const running = listRunningRuns()
    if (running.length > 0) {
      log.info({ count: running.length }, '[workflow] reconciling runs left running')
      await reconcileRunningExecutions(running, workflows)
    }
  } catch (err) {
    log.warn({ err }, '[workflow] could not pick up the previous run state')
  }
}
