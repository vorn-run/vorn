import { gateMaxRounds } from '@vornrun/shared/workflow-graph'
import type { ApprovalConfig, NodeExecutionState } from '../../shared/types'

/** "round 2 of 3" on a gate that takes changes; nothing on one that does not. */
export function roundLabel(
  config: ApprovalConfig | undefined,
  state: Pick<NodeExecutionState, 'round'>
): string | null {
  if (!config?.feedback?.from) return null
  return `round ${state.round ?? 1} of ${gateMaxRounds(config.feedback)}`
}
