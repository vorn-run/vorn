import { useState, useEffect, useMemo } from 'react'
import { motion, AnimatePresence } from 'framer-motion'
import { ChevronRight, Settings2, GitBranch, EyeOff, Zap } from 'lucide-react'
import {
  LaunchAgentConfig,
  TriggerConfig,
  WorkflowNode,
  getProjectRemoteHostId
} from '../../../../shared/types'
import { useAppStore } from '../../../stores'
import {
  StepVariableGroup,
  CONTEXT_REF,
  getAvailableContextVars,
  isContextRef
} from '../../../lib/template-vars'
import { getWorktreeMode } from '../../../lib/workflow-helpers'
import { useAgentInstallStatus } from '../../../hooks/useAgentInstallStatus'
import { VariableAutocomplete } from './VariableAutocomplete'
import { ProjectPicker } from '../../ProjectPicker'
import { AgentPicker } from '../../AgentPicker'
import { SelectPicker } from '../../SelectPicker'
import { Tooltip } from '../../Tooltip'
import { RichMarkdownEditor } from '../../rich-editor/RichMarkdownEditor'

interface Props {
  config: LaunchAgentConfig
  onChange: (config: LaunchAgentConfig) => void
  triggerType?: TriggerConfig['triggerType']
  isContextualTrigger?: boolean
  stepGroups?: StepVariableGroup[]
  currentNodeId?: string
  allNodes?: WorkflowNode[]
}

const EMPTY_PROJECTS: import('../../../../shared/types').ProjectConfig[] = []
const EMPTY_TASKS: import('../../../../shared/types').TaskConfig[] = []

const PROMPT_SOURCES = [
  { value: 'inline', label: 'Inline' },
  { value: 'task', label: 'Task' },
  { value: 'queue', label: 'Queue' }
]

export function LaunchAgentConfigForm({
  config,
  onChange,
  triggerType,
  isContextualTrigger = false,
  stepGroups = [],
  currentNodeId,
  allNodes
}: Props) {
  const [advancedOpen, setAdvancedOpen] = useState(!!config.args?.length)
  const projects = useAppStore((s) => s.config?.projects ?? EMPTY_PROJECTS)
  const tasks = useAppStore((s) => s.config?.tasks ?? EMPTY_TASKS)
  const { status: installStatus } = useAgentInstallStatus()
  const projectTasks = tasks.filter(
    (t) => t.projectName === config.projectName && t.status === 'todo'
  )

  const [existingWorktrees, setExistingWorktrees] = useState<
    { path: string; branch: string; isMain: boolean; name: string }[]
  >([])
  const [isGitRepo, setIsGitRepo] = useState(true)

  const selectedProject = projects.find((p) => p.name === config.projectName)
  const isRemote = !!selectedProject && !!getProjectRemoteHostId(selectedProject)

  useEffect(() => {
    if (!config.projectPath || isRemote) {
      // eslint-disable-next-line react-hooks/set-state-in-effect
      setExistingWorktrees([])
      setIsGitRepo(true)
      return
    }
    let cancelled = false
    window.api
      .isGitRepo(config.projectPath)
      .then((v) => {
        if (!cancelled) setIsGitRepo(v)
      })
      .catch(() => {
        if (!cancelled) setIsGitRepo(false)
      })
    window.api
      .listWorktrees(config.projectPath)
      .then((wts) => {
        if (!cancelled) setExistingWorktrees(wts.filter((w) => !w.isMain))
      })
      .catch(() => {
        if (!cancelled) setExistingWorktrees([])
      })
    return () => {
      cancelled = true
    }
  }, [config.projectPath, isRemote])

  const priorWorktreeSteps = useMemo(
    () =>
      (allNodes ?? []).filter((n) => {
        if (n.id === currentNodeId) return false
        if (n.type !== 'launchAgent') return false
        const mode = getWorktreeMode(n.config as LaunchAgentConfig)
        return mode === 'new' || mode === 'fromStep'
      }),
    [allNodes, currentNodeId]
  )

  const projectIsFromContext = isContextRef(config.projectName) || isContextRef(config.projectPath)
  const branchIsFromContext = isContextRef(config.branch)
  const worktreeIsFromContext = config.useWorktree === 'fromContext'
  const worktreeMode = getWorktreeMode(config)
  const promptSource = config.taskId ? 'task' : config.taskFromQueue ? 'queue' : 'inline'
  const isTaskTrigger = triggerType === 'taskCreated' || triggerType === 'taskStatusChanged'
  const hasTemplateVars = stepGroups.length > 0 || isTaskTrigger || isContextualTrigger
  const hasBranch = !isRemote && !!(config.branch && config.branch.trim())
  const isHeadless = !!config.headless
  const canUseFromTask = isTaskTrigger || promptSource === 'task' || promptSource === 'queue'
  const defaultAgentFallback = useAppStore((s) => s.config?.defaults.defaultAgent) ?? 'claude'

  useEffect(() => {
    if (!canUseFromTask && config.agentType === 'fromTask') {
      onChange({ ...config, agentType: defaultAgentFallback })
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [canUseFromTask])

  // Reset any "From Context" fields if the trigger is no longer contextual.
  // The runtime resolver returns empty strings for `{{context.*}}` without a
  // source — silently leaving these in place after toggling off would mean
  // the workflow runs with empty cwd / branch.
  useEffect(() => {
    if (isContextualTrigger) return
    if (!projectIsFromContext && !branchIsFromContext && !worktreeIsFromContext) return
    const updates: Partial<LaunchAgentConfig> = {}
    if (projectIsFromContext) {
      updates.projectName = ''
      updates.projectPath = ''
    }
    if (branchIsFromContext) updates.branch = undefined
    if (worktreeIsFromContext) {
      updates.useWorktree = undefined
      updates.worktreeMode = 'none'
    }
    onChange({ ...config, ...updates })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isContextualTrigger])

  const contextVars = getAvailableContextVars({ triggerType, isContextualTrigger })

  return (
    <div className="space-y-5">
      <div>
        <label className="text-[13px] text-gray-400 font-medium block mb-2">Agent</label>
        <AgentPicker
          currentAgent={config.agentType}
          onChange={(agent) => agent && onChange({ ...config, agentType: agent })}
          installStatus={installStatus}
          variant="form"
          allowFromTask={canUseFromTask}
        />
      </div>

      <div>
        <label className="text-[13px] text-gray-400 font-medium block mb-2">Project</label>
        <ProjectPicker
          currentProject={projectIsFromContext ? '' : config.projectName}
          projects={projects}
          onChange={(name) => {
            const proj = projects.find((p) => p.name === name)
            if (proj) onChange({ ...config, projectName: proj.name, projectPath: proj.path })
          }}
          variant="form"
          allowFromContext={isContextualTrigger}
          isFromContext={projectIsFromContext}
          onSelectFromContext={() =>
            onChange({
              ...config,
              projectName: CONTEXT_REF.projectName,
              projectPath: CONTEXT_REF.projectPath
            })
          }
        />
      </div>

      <div>
        <label className="text-[13px] text-gray-400 font-medium block mb-2">Prompt</label>
        <div className="mb-2">
          <SelectPicker
            value={promptSource}
            options={PROMPT_SOURCES}
            onChange={(v) => {
              if (v === 'inline')
                onChange({ ...config, taskId: undefined, taskFromQueue: undefined })
              else if (v === 'task')
                onChange({
                  ...config,
                  prompt: undefined,
                  taskFromQueue: undefined,
                  taskId: projectTasks[0]?.id
                })
              else
                onChange({ ...config, prompt: undefined, taskId: undefined, taskFromQueue: true })
            }}
            variant="form"
          />
        </div>

        {promptSource === 'inline' && (
          <>
            {hasTemplateVars ? (
              <VariableAutocomplete
                value={config.prompt || ''}
                onChange={(val) => onChange({ ...config, prompt: val || undefined })}
                placeholder="Enter prompt..."
                rows={4}
                stepGroups={stepGroups}
                contextVars={contextVars}
              />
            ) : (
              <RichMarkdownEditor
                value={config.prompt || ''}
                onChange={(val) => onChange({ ...config, prompt: val || undefined })}
                placeholder="Enter prompt..."
                className="min-h-[100px]"
              />
            )}
          </>
        )}

        {promptSource === 'task' && (
          <SelectPicker
            value={config.taskId || ''}
            options={[
              { value: '', label: 'Select task...' },
              ...projectTasks.map((t) => ({ value: t.id, label: t.title }))
            ]}
            onChange={(v) => onChange({ ...config, taskId: v || undefined })}
            placeholder="Select task..."
            variant="form"
          />
        )}

        {promptSource === 'queue' && (
          <p className="text-[11px] text-gray-500 leading-relaxed">
            Auto-picks the next todo task from{' '}
            <span className="text-gray-400">{config.projectName || 'the project'}</span> at runtime.
          </p>
        )}
      </div>

      {!isRemote && isGitRepo && (
        <div className="border border-white/[0.06] rounded-lg p-3 space-y-3">
          <div className="text-[13px] text-gray-400 font-medium flex items-center gap-1.5">
            <GitBranch size={11} />
            Git &amp; Branch
          </div>

          <div>
            <div className="flex items-center gap-1.5 px-3 py-2 bg-white/[0.06] border border-white/[0.1] rounded-md">
              {branchIsFromContext ? (
                <Zap size={12} color="#60a5fa" className="shrink-0" />
              ) : (
                <GitBranch size={12} strokeWidth={2} className="text-gray-500 shrink-0" />
              )}
              {branchIsFromContext ? (
                <span className="flex-1 text-[13px] text-gray-200">From Context</span>
              ) : (
                <input
                  type="text"
                  value={config.branch || ''}
                  onChange={(e) => {
                    const branch = e.target.value || undefined
                    const updates: Partial<LaunchAgentConfig> = { branch }
                    if (!branch) updates.useWorktree = undefined
                    onChange({ ...config, ...updates })
                  }}
                  placeholder="feature/my-branch"
                  className="flex-1 min-w-0 bg-transparent text-[13px] text-white placeholder-gray-600
                             focus:outline-none border-none px-0"
                />
              )}
              {isContextualTrigger && (
                <Tooltip
                  label={branchIsFromContext ? 'Use a specific branch' : 'Use From Context'}
                  position="top"
                >
                  <button
                    type="button"
                    onClick={() =>
                      onChange({
                        ...config,
                        branch: branchIsFromContext ? undefined : CONTEXT_REF.branch
                      })
                    }
                    className={`shrink-0 p-0.5 rounded hover:bg-white/[0.08] transition-colors ${
                      branchIsFromContext ? 'text-blue-400' : 'text-gray-500 hover:text-gray-300'
                    }`}
                  >
                    <Zap size={11} />
                  </button>
                </Tooltip>
              )}
            </div>
            <p className="text-[11px] text-gray-500 mt-1">
              Checks out this branch before launching
            </p>
          </div>

          <div>
            <div className="text-[11px] text-gray-500 mb-1.5">Worktree</div>
            <SelectPicker
              value={worktreeMode}
              options={[
                { value: 'none', label: 'None' },
                {
                  value: 'new',
                  label: 'New worktree',
                  hint: !hasBranch ? 'needs branch' : undefined
                },
                {
                  value: 'fromStep',
                  label: 'From step',
                  hint: priorWorktreeSteps.length === 0 ? 'no steps' : undefined
                },
                { value: 'existing', label: 'Existing worktree' },
                ...(isContextualTrigger ? [{ value: 'fromContext', label: 'From Context' }] : [])
              ]}
              onChange={(v) => {
                const key = v as 'none' | 'new' | 'fromStep' | 'existing' | 'fromContext'
                if (key === 'new' && !hasBranch) return
                if (key === 'fromStep' && priorWorktreeSteps.length === 0) return
                if (key === 'fromContext') {
                  onChange({
                    ...config,
                    useWorktree: 'fromContext',
                    worktreeMode: undefined,
                    worktreeFromStepSlug: undefined,
                    existingWorktreePath: undefined
                  })
                  return
                }
                const updates: Partial<LaunchAgentConfig> = {
                  worktreeMode: key,
                  useWorktree: key === 'new' ? true : undefined,
                  worktreeFromStepSlug: undefined,
                  existingWorktreePath: undefined
                }
                if (key === 'none') {
                  updates.branch = undefined
                  updates.useWorktree = undefined
                }
                onChange({ ...config, ...updates })
              }}
              variant="form"
            />
            <p className="text-[11px] text-gray-500 mt-1.5">
              {worktreeMode === 'none' && 'Agent runs in the project directory'}
              {worktreeMode === 'new' && "Isolated directory — won't affect the main working tree"}
              {worktreeMode === 'fromStep' && 'Reuses the worktree created by a previous step'}
              {worktreeMode === 'existing' && 'Launches into an existing worktree on disk'}
              {worktreeMode === 'fromContext' &&
                "Inherits the launching card or terminal's worktree (won't be auto-cleaned)"}
            </p>
          </div>

          {worktreeMode === 'fromStep' && (
            <SelectPicker
              value={config.worktreeFromStepSlug || ''}
              options={[
                { value: '', label: 'Select step...' },
                ...priorWorktreeSteps.map((step) => ({
                  value: step.slug || step.id,
                  label: step.label || step.slug || step.id
                }))
              ]}
              onChange={(v) => onChange({ ...config, worktreeFromStepSlug: v || undefined })}
              placeholder="Select step..."
              variant="form"
            />
          )}

          {worktreeMode === 'existing' && (
            <SelectPicker
              value={config.existingWorktreePath || ''}
              options={[
                { value: '', label: 'Select worktree...' },
                ...existingWorktrees.map((wt) => ({
                  value: wt.path,
                  label: wt.name,
                  hint: wt.branch
                }))
              ]}
              onChange={(v) => onChange({ ...config, existingWorktreePath: v || undefined })}
              placeholder="Select worktree..."
              variant="form"
            />
          )}
        </div>
      )}

      <div className="border border-white/[0.06] rounded-lg p-3 space-y-3">
        <div className="text-[13px] text-gray-400 font-medium">Execution</div>

        <button
          role="switch"
          aria-checked={isHeadless}
          onClick={() => onChange({ ...config, headless: isHeadless ? false : true })}
          disabled={isRemote}
          className={`flex items-center gap-3 w-full px-3 py-2.5 rounded-lg border transition-all ${
            isRemote
              ? 'border-white/[0.04] bg-white/[0.01] opacity-50 cursor-not-allowed'
              : isHeadless
                ? 'border-white/[0.1] bg-white/[0.04]'
                : 'border-white/[0.04] bg-white/[0.02] hover:border-white/[0.1]'
          }`}
        >
          <div
            className={`w-7 h-[16px] rounded-full transition-colors relative shrink-0 ${
              isHeadless ? 'bg-gray-400' : 'bg-white/[0.1]'
            }`}
          >
            <div
              className={`absolute top-[2px] w-[12px] h-[12px] rounded-full bg-white transition-transform ${
                isHeadless ? 'translate-x-[13px]' : 'translate-x-[2px]'
              }`}
            />
          </div>
          <div className="text-left min-w-0">
            <div className="flex items-center gap-1.5">
              <EyeOff size={12} className={isHeadless ? 'text-gray-300' : 'text-gray-500'} />
              <span className={`text-[12px] ${isHeadless ? 'text-gray-200' : 'text-gray-400'}`}>
                Headless
              </span>
            </div>
            <p className="text-[11px] text-gray-500 mt-0.5">
              {isHeadless
                ? 'Runs in background. Waits for completion before next step.'
                : 'Opens a terminal tab. Step completes immediately.'}
            </p>
          </div>
        </button>

        {!isHeadless && (
          <div>
            <label className="text-[13px] text-gray-400 font-medium block mb-2">Tab Name</label>
            <input
              type="text"
              value={config.displayName || ''}
              onChange={(e) => onChange({ ...config, displayName: e.target.value || undefined })}
              placeholder={config.projectName || 'Uses project name'}
              className="w-full px-3 py-2 text-[13px] bg-white/[0.06] border border-white/[0.1] rounded-md
                         text-white placeholder:text-gray-600 focus:outline-none focus:border-white/[0.2]"
            />
          </div>
        )}
      </div>

      <div>
        <button
          onClick={() => setAdvancedOpen(!advancedOpen)}
          className="flex items-center gap-1.5 text-[11px] text-gray-500 hover:text-gray-400
                     transition-colors uppercase tracking-wider font-medium w-full"
        >
          <ChevronRight
            size={12}
            className={`transition-transform duration-200 ${advancedOpen ? 'rotate-90' : ''}`}
          />
          <Settings2 size={11} />
          Advanced
        </button>

        <AnimatePresence initial={false}>
          {advancedOpen && (
            <motion.div
              initial={{ height: 0, opacity: 0 }}
              animate={{ height: 'auto', opacity: 1 }}
              exit={{ height: 0, opacity: 0 }}
              transition={{ duration: 0.2, ease: 'easeInOut' }}
              className="overflow-hidden"
            >
              <div className="space-y-4 pt-3">
                <div>
                  <label className="text-[13px] text-gray-400 font-medium block mb-2">
                    Extra Arguments
                  </label>
                  <input
                    type="text"
                    value={(config.args || []).join(' ')}
                    onChange={(e) => {
                      const args = e.target.value.trim()
                        ? e.target.value.trim().split(/\s+/)
                        : undefined
                      onChange({ ...config, args })
                    }}
                    placeholder="e.g. --dangerously-skip-permissions"
                    className="w-full px-3 py-2 text-[13px] bg-white/[0.06] border border-white/[0.1] rounded-md
                               text-white placeholder:text-gray-600 focus:outline-none focus:border-white/[0.2] font-mono"
                  />
                </div>
              </div>
            </motion.div>
          )}
        </AnimatePresence>
      </div>
    </div>
  )
}
