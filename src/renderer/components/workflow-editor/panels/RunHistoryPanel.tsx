import { useState } from 'react'
import { X } from 'lucide-react'
import {
  workflowRunId,
  WorkflowExecution,
  WorkflowNode,
  TaskConfig,
  AiAgentType
} from '../../../../shared/types'
import { RunEntry } from '../RunEntry'
import { LogReplayModal } from '../../LogReplayModal'

interface Props {
  executions: WorkflowExecution[]
  nodes: WorkflowNode[]
  tasks?: TaskConfig[]
  onClose: () => void
  onClickTask?: (taskId: string) => void
  onResumeSession?: (
    agentSessionId: string,
    agentType: AiAgentType,
    projectName: string,
    projectPath: string,
    branch?: string,
    useWorktree?: boolean
  ) => void
}

export function RunHistoryPanel({
  executions,
  nodes,
  tasks,
  onClose,
  onClickTask,
  onResumeSession
}: Props) {
  const [fullOutputLogs, setFullOutputLogs] = useState<string | null>(null)

  return (
    <>
      <div className="w-[340px] border-l border-white/[0.08] bg-surface-node flex flex-col h-full overflow-hidden titlebar-no-drag">
        <div className="flex items-center justify-between px-4 py-3 border-b border-white/[0.08]">
          <span className="text-[13px] font-medium text-white">Run History</span>
          <button
            onClick={onClose}
            className="text-gray-500 hover:text-white p-1 rounded-md transition-colors"
          >
            <X size={14} />
          </button>
        </div>

        <div className="flex-1 overflow-y-auto p-3 space-y-2">
          {executions.length === 0 ? (
            <p className="text-[12px] text-gray-600 text-center py-8">No runs yet</p>
          ) : (
            executions.map((exec) => (
              <RunEntry
                key={workflowRunId(exec)}
                execution={exec}
                nodes={nodes}
                tasks={tasks}
                onViewFullOutput={setFullOutputLogs}
                onClickTask={onClickTask}
                onResumeSession={onResumeSession}
              />
            ))
          )}
        </div>
      </div>

      {fullOutputLogs !== null && (
        <LogReplayModal logs={fullOutputLogs} onClose={() => setFullOutputLogs(null)} />
      )}
    </>
  )
}
