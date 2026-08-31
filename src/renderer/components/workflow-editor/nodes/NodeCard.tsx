import {
  WorkflowNode,
  TriggerConfig,
  LaunchAgentConfig,
  ScriptConfig,
  ConditionConfig,
  ApprovalConfig,
  CreateTaskFromItemConfig,
  CallConnectorActionConfig,
  NodeExecutionStatus
} from '../../../../shared/types'
import { TriggerNode } from './TriggerNode'
import { LaunchAgentNode } from './LaunchAgentNode'
import { ScriptNode } from './ScriptNode'
import { ConditionNode } from './ConditionNode'
import { ApprovalNode } from './ApprovalNode'
import { CreateTaskFromItemNode } from './CreateTaskFromItemNode'
import { CallConnectorActionNode } from './CallConnectorActionNode'

/**
 * The card for a single step, dispatched by type.
 *
 * Loops are not here on purpose: on the canvas a loop is a composite that
 * draws its own body, and a loop can never be another loop's body step, so a
 * card-shaped loop has no call site left.
 */
export function NodeCard({
  node,
  selected,
  onClick,
  executionStatus
}: {
  node: WorkflowNode
  selected: boolean
  onClick: () => void
  executionStatus?: NodeExecutionStatus
}) {
  if (node.type === 'trigger') {
    return (
      <TriggerNode
        label={node.label}
        config={node.config as TriggerConfig}
        selected={selected}
        onClick={onClick}
      />
    )
  }

  if (node.type === 'script') {
    return (
      <ScriptNode
        label={node.label}
        config={node.config as ScriptConfig}
        selected={selected}
        onClick={onClick}
        executionStatus={executionStatus}
      />
    )
  }

  if (node.type === 'condition') {
    return (
      <ConditionNode
        label={node.label}
        config={node.config as ConditionConfig}
        selected={selected}
        onClick={onClick}
        executionStatus={executionStatus}
      />
    )
  }

  if (node.type === 'approval') {
    return (
      <ApprovalNode
        label={node.label}
        config={node.config as ApprovalConfig}
        selected={selected}
        onClick={onClick}
        executionStatus={executionStatus}
      />
    )
  }

  if (node.type === 'createTaskFromItem') {
    return (
      <CreateTaskFromItemNode
        label={node.label}
        config={node.config as CreateTaskFromItemConfig}
        selected={selected}
        onClick={onClick}
        executionStatus={executionStatus}
      />
    )
  }

  if (node.type === 'callConnectorAction') {
    return (
      <CallConnectorActionNode
        label={node.label}
        config={node.config as CallConnectorActionConfig}
        selected={selected}
        onClick={onClick}
        executionStatus={executionStatus}
      />
    )
  }

  return (
    <LaunchAgentNode
      label={node.label}
      config={node.config as LaunchAgentConfig}
      selected={selected}
      onClick={onClick}
      executionStatus={executionStatus}
    />
  )
}
