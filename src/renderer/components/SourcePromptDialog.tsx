import { useEffect, useMemo, useState } from 'react'
import { motion, AnimatePresence } from 'framer-motion'
import { GitBranch, Zap } from 'lucide-react'
import { useAppStore } from '../stores'
import { ProjectPicker } from './ProjectPicker'
import { executeWorkflow } from '../lib/workflow-execution'
import { containsContextRef, isContextRef } from '../lib/template-vars'
import type {
  LaunchAgentConfig,
  ScriptConfig,
  TerminalSession,
  WorkflowDefinition
} from '../../shared/types'

/**
 * Walk the workflow's nodes and decide which fields the user actually needs
 * to fill in. A contextual workflow that only references `{{context.cwd}}`
 * shouldn't bother prompting for branch — and vice versa.
 */
function detectRequiredFields(workflow: WorkflowDefinition): {
  needsProject: boolean
  needsBranch: boolean
  needsWorktree: boolean
} {
  const projectFields = ['cwd', 'projectPath', 'projectName']
  let needsProject = false
  let needsBranch = false
  let needsWorktree = false

  for (const node of workflow.nodes) {
    if (node.type === 'launchAgent') {
      const cfg = node.config as LaunchAgentConfig
      if (
        isContextRef(cfg.projectName) ||
        isContextRef(cfg.projectPath) ||
        projectFields.some((f) => containsContextRef(cfg.prompt, f))
      ) {
        needsProject = true
      }
      if (isContextRef(cfg.branch) || containsContextRef(cfg.prompt, 'branch')) {
        needsBranch = true
      }
      if (cfg.useWorktree === 'fromContext') {
        needsWorktree = true
        // worktree implies branch + project so the cwd resolves correctly
        needsProject = true
        needsBranch = true
      }
    } else if (node.type === 'script') {
      const cfg = node.config as ScriptConfig
      if (
        isContextRef(cfg.cwd) ||
        isContextRef(cfg.projectName) ||
        isContextRef(cfg.projectPath) ||
        projectFields.some((f) => containsContextRef(cfg.scriptContent, f))
      ) {
        needsProject = true
      }
      if (containsContextRef(cfg.scriptContent, 'branch')) {
        needsBranch = true
      }
    }
  }

  return { needsProject, needsBranch, needsWorktree }
}

export function SourcePromptDialog() {
  const pendingId = useAppStore((s) => s.pendingContextualWorkflowId)
  const setPendingId = useAppStore((s) => s.setPendingContextualWorkflowId)
  const config = useAppStore((s) => s.config)
  const workflow = useAppStore((s) =>
    pendingId ? (s.config?.workflows ?? []).find((w) => w.id === pendingId) : undefined
  )

  const [projectName, setProjectName] = useState('')
  const [projectPath, setProjectPath] = useState('')
  const [branch, setBranch] = useState('')
  const [useWorktree, setUseWorktree] = useState(false)

  const required = useMemo(
    () =>
      workflow
        ? detectRequiredFields(workflow)
        : { needsProject: true, needsBranch: false, needsWorktree: false },
    [workflow]
  )

  // Reset state and seed sensible defaults each time the dialog opens for a
  // new workflow. Depending on `config` here would clobber user input
  // mid-edit on any unrelated config change.
  useEffect(() => {
    if (!pendingId) return
    const first = (useAppStore.getState().config?.projects ?? [])[0]
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setProjectName(first?.name ?? '')
    setProjectPath(first?.path ?? '')
    setBranch('')
    setUseWorktree(false)
  }, [pendingId])

  if (!pendingId || !workflow) return null

  const close = () => setPendingId(null)

  const submit = () => {
    if (required.needsProject && (!projectName || !projectPath)) return
    // Synthesize a TerminalSession-shaped source for the resolver. Fields
    // the dialog doesn't capture (id, status, etc.) are placeholders.
    const source: TerminalSession = {
      id: `prompt:${pendingId}:${Date.now()}`,
      agentType: 'shell',
      projectName,
      projectPath,
      status: 'idle',
      createdAt: Date.now(),
      pid: 0,
      branch: branch || undefined,
      isWorktree: useWorktree
    }
    void executeWorkflow(workflow, { source }, { source: 'manual' })
    close()
  }

  return (
    <AnimatePresence>
      <motion.div
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        exit={{ opacity: 0 }}
        className="fixed inset-0 z-50 bg-black/40 backdrop-blur-sm"
        onClick={close}
      />
      <motion.div
        className="fixed top-1/2 left-1/2 z-50 w-[460px] border border-white/[0.08] rounded-xl shadow-2xl
                   overflow-hidden"
        style={{ background: '#1e1e22' }}
        initial={{ opacity: 0, scale: 0.95, x: '-50%', y: '-50%' }}
        animate={{ opacity: 1, scale: 1, x: '-50%', y: '-50%' }}
        exit={{ opacity: 0, scale: 0.95, x: '-50%', y: '-50%' }}
        transition={{ type: 'spring', stiffness: 400, damping: 30 }}
      >
        <div className="px-6 py-4 border-b border-white/[0.06]">
          <div className="flex items-center gap-2 text-white text-[15px] font-medium">
            <Zap size={14} className="text-blue-400" />
            Run "{workflow.name}"
          </div>
          <p className="text-[12px] text-gray-500 mt-1">
            This contextual workflow needs a source. Pick the folder it should run against.
          </p>
        </div>

        <div className="p-6 space-y-4">
          {required.needsProject && (
            <div>
              <label className="text-[11px] font-medium text-gray-500 uppercase tracking-wider mb-2 block">
                Project
              </label>
              <ProjectPicker
                currentProject={projectName}
                projects={config?.projects ?? []}
                onChange={(name) => {
                  const proj = (config?.projects ?? []).find((p) => p.name === name)
                  if (proj) {
                    setProjectName(proj.name)
                    setProjectPath(proj.path)
                  }
                }}
                variant="form"
              />
            </div>
          )}

          {required.needsBranch && (
            <div>
              <label className="text-[11px] font-medium text-gray-500 uppercase tracking-wider mb-2 block">
                Branch
              </label>
              <div className="flex items-center gap-1.5 px-3 py-2 bg-white/[0.06] border border-white/[0.1] rounded-md">
                <GitBranch size={12} className="text-gray-500 shrink-0" />
                <input
                  type="text"
                  value={branch}
                  onChange={(e) => setBranch(e.target.value)}
                  placeholder="leave blank to use the project's current branch"
                  className="flex-1 min-w-0 bg-transparent text-[13px] text-white placeholder-gray-600
                             focus:outline-none border-none px-0"
                />
              </div>
            </div>
          )}

          {required.needsWorktree && (
            <label className="flex items-center gap-2 cursor-pointer">
              <input
                type="checkbox"
                checked={useWorktree}
                onChange={(e) => setUseWorktree(e.target.checked)}
                className="accent-white/80"
              />
              <span className="text-[13px] text-gray-300">Run in a new worktree</span>
            </label>
          )}
        </div>

        <div className="px-6 py-3 border-t border-white/[0.06] flex justify-end gap-2">
          <button
            onClick={close}
            className="px-3 py-1.5 text-[13px] text-gray-300 hover:text-white rounded-md
                       hover:bg-white/[0.06] transition-colors"
          >
            Cancel
          </button>
          <button
            onClick={submit}
            disabled={required.needsProject && !projectName}
            className="px-4 py-1.5 text-[13px] text-white bg-white/[0.1] hover:bg-white/[0.15]
                       rounded-md transition-colors disabled:opacity-40 disabled:pointer-events-none"
          >
            Run
          </button>
        </div>
      </motion.div>
    </AnimatePresence>
  )
}
