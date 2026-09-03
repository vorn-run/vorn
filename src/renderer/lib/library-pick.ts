import type { TriggerConfig } from '../../shared/types'
import type { AddableNodeType } from '../components/workflow-editor/WorkflowCanvas'

/** A step type, a parallel branch, a connector action, or a trigger. */
export type LibraryPick =
  | { kind: 'type'; type: AddableNodeType }
  | { kind: 'parallel' }
  | { kind: 'connectorAction'; connectionId: string; action: string; actionLabel: string }
  | { kind: 'triggerType'; triggerType: TriggerConfig['triggerType'] }
  | { kind: 'connectorTrigger'; connectionId: string; event: string }
  /** An action from a connector nobody has installed yet; it lands unbound. */
  | { kind: 'catalogAction'; connectorId: string; action: string; actionLabel: string }
  /** An HTTP request with one of the saved profiles already chosen. */
  | { kind: 'httpProfile'; profileConnectionId: string; profileName: string }

/** The step a pick becomes: its node type and whatever it already knows about itself. */
export function stepForPick(pick: LibraryPick): {
  type: AddableNodeType
  config?: Record<string, unknown>
} {
  switch (pick.kind) {
    case 'connectorAction':
      return {
        type: 'connectorAction',
        config: {
          connectionId: pick.connectionId,
          action: pick.action,
          actionLabel: pick.actionLabel
        }
      }
    case 'catalogAction':
      return {
        type: 'connectorAction',
        config: {
          connectionId: '',
          connectorId: pick.connectorId,
          action: pick.action,
          actionLabel: pick.actionLabel
        }
      }
    case 'httpProfile':
      return { type: 'httpRequest', config: { profileConnectionId: pick.profileConnectionId } }
    case 'type':
      return { type: pick.type }
    default:
      throw new Error(`${pick.kind} is not a step`)
  }
}
