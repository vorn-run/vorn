import { useState, useEffect, useCallback, useRef } from 'react'
import { motion, AnimatePresence } from 'framer-motion'
import { useAppStore } from '../stores'
import { ONBOARDING_STEPS } from '../lib/onboarding-data'
import { AGENT_LIST, AGENT_DEFINITIONS } from '../lib/agent-definitions'
import { AgentIcon } from './AgentIcon'
import { useAgentInstallStatus } from '../hooks/useAgentInstallStatus'
import { SHORTCUTS } from '../lib/keyboard-shortcuts'
import { AGENT_MCP_SETUPS } from '../lib/mcp-data'
import {
  Bot,
  FolderGit2,
  KanbanSquare,
  Zap,
  Plug,
  ArrowRight,
  ArrowLeft,
  Check,
  X,
  RefreshCw,
  Copy,
  CheckCircle,
  FolderPlus,
  GitBranch,
  ListTodo,
  Clock,
  Sparkles,
  Rocket,
  LayoutDashboard,
  Search,
  ChevronDown,
  ChevronRight,
  LayoutGrid,
  PanelRight,
  Bell,
  Monitor,
  Settings,
  Palette,
  Columns3,
  FileDiff,
  FileCode,
  GitCommitHorizontal
} from 'lucide-react'

const STEP_ICONS: Record<string, React.ComponentType<{ size?: number; className?: string }>> = {
  Bot,
  FolderGit2,
  KanbanSquare,
  Zap,
  Plug,
  Rocket,
  LayoutDashboard,
  Search,
  Sparkles
}

const EMPTY_PROJECTS: { name: string }[] = []

const ANIM_DELAY = 200

/* Pre-computed since both SHORTCUTS and the IDs are static */
const ESSENTIAL_SHORTCUTS = [
  'command-palette',
  'new-session',
  'view-options',
  'toggle-sidebar',
  'shortcuts-panel',
  'filter-all'
]
  .map((id) => SHORTCUTS.find((s) => s.id === id))
  .filter(Boolean) as (typeof SHORTCUTS)[number][]

const EXPLORE_ITEMS: {
  icon: React.ComponentType<{ size?: number; className?: string }>
  label: string
  category: 'notifications' | 'ssh' | 'appearance' | 'agents'
}[] = [
  { icon: Bell, label: 'Enable notifications & sounds', category: 'notifications' },
  { icon: Monitor, label: 'Set up remote hosts (SSH)', category: 'ssh' },
  { icon: Palette, label: 'Customize appearance', category: 'appearance' },
  { icon: Settings, label: 'Configure agent commands', category: 'agents' }
]

const LAUNCHER_PICKERS = [
  { icon: Bot, label: 'Agent' },
  { icon: FolderGit2, label: 'Project' },
  { icon: GitBranch, label: 'Branch' }
] as const

function CopyBtn({ text }: { text: string }) {
  const [copied, setCopied] = useState(false)
  const timerRef = useRef<ReturnType<typeof setTimeout>>(undefined)

  const copy = () => {
    navigator.clipboard.writeText(text)
    setCopied(true)
    clearTimeout(timerRef.current)
    timerRef.current = setTimeout(() => setCopied(false), 1500)
  }

  useEffect(() => () => clearTimeout(timerRef.current), [])

  return (
    <button
      onClick={copy}
      className="p-1.5 rounded-md text-gray-500 hover:text-white bg-white/[0.04] hover:bg-white/[0.08]
                 transition-all shrink-0"
      title="Copy"
    >
      {copied ? <CheckCircle size={12} className="text-emerald-400" /> : <Copy size={12} />}
    </button>
  )
}

function Kbd({ children }: { children: React.ReactNode }) {
  return (
    <kbd className="text-[11px] text-gray-400 bg-white/[0.06] border border-white/[0.08] px-1.5 py-0.5 rounded font-mono">
      {children}
    </kbd>
  )
}

function AccentButton({
  icon: Icon,
  label,
  onClick
}: {
  icon: React.ComponentType<{ size?: number }>
  label: string
  onClick: () => void
}) {
  return (
    <button
      onClick={onClick}
      className="flex items-center gap-2 px-3.5 py-1.5 text-[12px] font-medium rounded-lg
                 text-bronzo bg-bronzo/[0.04] border border-white/[0.08] hover:border-white/[0.14] transition-all"
    >
      <Icon size={13} />
      {label}
    </button>
  )
}

/** Close onboarding then invoke a callback after exit animation */
function closeAndOpen(fn: () => void) {
  useAppStore.getState().setOnboardingOpen(false)
  setTimeout(fn, ANIM_DELAY)
}

/* ═══════════════════════════════════════════════════════════════════════
   Step 1 — Agents + MCP
   ═══════════════════════════════════════════════════════════════════════ */
function StepAgents() {
  const { status, loading, refresh } = useAgentInstallStatus()
  const installed = AGENT_LIST.filter((a) => status[a.type])
  const notInstalled = AGENT_LIST.filter((a) => !status[a.type])
  const [mcpOpen, setMcpOpen] = useState(false)
  const mcpCommands = AGENT_MCP_SETUPS.filter((m) => status[m.agentType])
  const mcpToShow = mcpCommands.length > 0 ? mcpCommands : AGENT_MCP_SETUPS.slice(0, 2)

  return (
    <div className="space-y-3">
      <p className="text-[13px] text-gray-400 leading-relaxed">
        Vorn works with multiple AI coding agents. We detected which ones you have installed.
      </p>

      {installed.length > 0 && (
        <div className="space-y-1.5">
          {installed.map((a) => {
            const def = AGENT_DEFINITIONS[a.type]
            return (
              <div
                key={a.type}
                className="flex items-center gap-3 px-3 py-2 rounded-lg border border-white/[0.06] bg-surface-sunken"
              >
                <div
                  className="w-7 h-7 rounded-md flex items-center justify-center shrink-0"
                  style={{ background: def.bgColor }}
                >
                  <AgentIcon agentType={a.type} size={14} />
                </div>
                <span className="text-sm text-gray-200 flex-1">{a.displayName}</span>
                <span className="flex items-center gap-1 text-[11px] text-emerald-400">
                  <Check size={10} /> Ready
                </span>
              </div>
            )
          })}
        </div>
      )}

      {notInstalled.length > 0 && (
        <div className="space-y-1.5">
          <span className="text-[11px] text-gray-600 uppercase tracking-wider font-medium">
            Not detected
          </span>
          {notInstalled.map((a) => (
            <div
              key={a.type}
              className="flex items-center gap-3 px-3 py-2 rounded-lg border border-white/[0.04] bg-surface-sunken"
            >
              <div className="w-7 h-7 rounded-md flex items-center justify-center shrink-0 bg-white/[0.03] opacity-40">
                <AgentIcon agentType={a.type} size={14} />
              </div>
              <span className="text-sm text-gray-500 flex-1 min-w-0">{a.displayName}</span>
              <code className="text-[10px] text-gray-600 font-mono overflow-x-auto whitespace-nowrap max-w-[180px]">
                {AGENT_DEFINITIONS[a.type].installCommand}
              </code>
            </div>
          ))}
        </div>
      )}

      <button
        onClick={refresh}
        disabled={loading}
        className="flex items-center gap-1.5 text-[12px] text-gray-500 hover:text-gray-300
                   transition-colors disabled:opacity-40"
      >
        <RefreshCw size={11} className={loading ? 'animate-spin' : ''} />
        {loading ? 'Scanning...' : 'Re-detect agents'}
      </button>

      <div className="rounded-lg border border-white/[0.06] overflow-hidden bg-surface-sunken">
        <button
          onClick={() => setMcpOpen(!mcpOpen)}
          className="w-full flex items-center gap-2 px-3 py-2.5 text-left hover:bg-white/[0.02] transition-colors"
        >
          <Plug size={13} className="text-bronzo" />
          <span className="text-[12px] font-medium text-gray-200 flex-1">
            Install skills &amp; MCP server
          </span>
          <span className="text-[10px] font-medium px-1.5 py-0.5 rounded text-bronzo bg-bronzo/[0.08]">
            Recommended
          </span>
          {mcpOpen ? (
            <ChevronDown size={12} className="text-gray-600" />
          ) : (
            <ChevronRight size={12} className="text-gray-600" />
          )}
        </button>
        {mcpOpen && (
          <div className="px-3 pb-3 space-y-2.5 border-t border-white/[0.04]">
            <p className="text-[12px] text-gray-400 pt-2 leading-relaxed">
              Agents get full context about your projects and tasks — they can launch other agents,
              pick up tasks, read terminal output, and trigger workflows autonomously.
            </p>

            <div className="flex items-center gap-2 flex-wrap">
              {['Launch sessions', 'Manage tasks', 'Read terminals', 'Trigger workflows'].map(
                (cap) => (
                  <span
                    key={cap}
                    className="flex items-center gap-1 text-[10px] text-gray-400 px-2 py-0.5 rounded bg-white/[0.03]"
                  >
                    <Check size={8} className="text-emerald-400/50" />
                    {cap}
                  </span>
                )
              )}
            </div>

            <div className="space-y-2.5">
              {mcpToShow.map(({ agentType, commands, inAgent, note }) => {
                const def = AGENT_DEFINITIONS[agentType]
                return (
                  <div key={agentType} className="space-y-1">
                    <div className="flex items-center gap-2">
                      <div
                        className="w-5 h-5 rounded flex items-center justify-center shrink-0"
                        style={{ background: def.bgColor }}
                      >
                        <AgentIcon agentType={agentType} size={10} />
                      </div>
                      <span className="text-[10px] text-gray-600 uppercase tracking-wider font-medium">
                        {inAgent ? `Run in ${def.displayName}` : 'Run in your terminal'}
                      </span>
                    </div>
                    {commands.map((command) => (
                      <div key={command} className="flex items-center gap-2 pl-7">
                        <code className="flex-1 text-[11px] font-mono text-gray-400 overflow-x-auto whitespace-nowrap">
                          {command}
                        </code>
                        <CopyBtn text={command} />
                      </div>
                    ))}
                    {note && <p className="pl-7 text-[10px] text-gray-600">{note}</p>}
                  </div>
                )
              })}
            </div>
          </div>
        )}
      </div>
    </div>
  )
}

/* ═══════════════════════════════════════════════════════════════════════
   Step 2 — Projects & Tasks
   ═══════════════════════════════════════════════════════════════════════ */
function StepProjects() {
  const projects = useAppStore((s) => s.config?.projects ?? EMPTY_PROJECTS)
  const setAddProjectDialogOpen = useAppStore((s) => s.setAddProjectDialogOpen)
  const setMainViewMode = useAppStore((s) => s.setMainViewMode)

  return (
    <div className="space-y-3">
      <p className="text-[13px] text-gray-400 leading-relaxed">
        A <span className="text-gray-200">project</span> is a Git repo.{' '}
        <span className="text-gray-200">Worktrees</span> let multiple agents work on different
        branches simultaneously without conflicts.
      </p>

      <div className="rounded-lg border border-white/[0.06] p-3.5 space-y-2.5 bg-surface-sunken">
        <div className="flex items-start gap-3">
          <FolderGit2 size={15} className="text-cyan-400 mt-0.5 shrink-0" />
          <div className="text-[12px] text-gray-200 font-medium">my-app/</div>
        </div>
        <div className="ml-4 pl-4 border-l border-white/[0.06] space-y-1.5">
          <div className="flex items-center gap-2">
            <GitBranch size={11} className="text-gray-500" />
            <span className="text-[11px] text-gray-400">main</span>
            <span className="text-[9px] text-gray-600 px-1.5 py-0.5 rounded bg-white/[0.04]">
              default
            </span>
          </div>
          <div className="flex items-center gap-2">
            <GitBranch size={11} className="text-violet-400" />
            <span className="text-[11px] text-gray-400">feat/auth</span>
            <span className="text-[9px] text-violet-400/60 px-1.5 py-0.5 rounded bg-violet-400/[0.06]">
              worktree
            </span>
          </div>
          <div className="flex items-center gap-2">
            <GitBranch size={11} className="text-amber-400" />
            <span className="text-[11px] text-gray-400">fix/api-bug</span>
            <span className="text-[9px] text-amber-400/60 px-1.5 py-0.5 rounded bg-amber-400/[0.06]">
              worktree
            </span>
          </div>
        </div>
      </div>

      <div className="grid grid-cols-2 gap-1.5">
        {[
          { icon: KanbanSquare, text: 'Kanban board' },
          { icon: GitBranch, text: 'Auto branch & worktree' },
          { icon: Bot, text: 'Launch agents from tasks' },
          { icon: ListTodo, text: 'Full task lifecycle' }
        ].map(({ icon: Icon, text }) => (
          <div
            key={text}
            className="flex items-center gap-2 px-2.5 py-1.5 rounded-md border border-white/[0.04] text-[11px] text-gray-400 bg-surface-sunken"
          >
            <Icon size={11} className="text-gray-500 shrink-0" />
            {text}
          </div>
        ))}
      </div>

      <div className="flex items-center gap-3">
        {projects.length > 0 ? (
          <span className="text-[12px] text-gray-500">
            {projects.length} project{projects.length > 1 ? 's' : ''} configured
          </span>
        ) : (
          <AccentButton
            icon={FolderPlus}
            label="Add project"
            onClick={() => closeAndOpen(() => setAddProjectDialogOpen(true))}
          />
        )}
        <button
          onClick={() => closeAndOpen(() => setMainViewMode('tasks'))}
          className="flex items-center gap-1.5 text-[12px] text-gray-500 hover:text-gray-300 transition-colors"
        >
          <KanbanSquare size={11} />
          Open Tasks
        </button>
      </div>
    </div>
  )
}

/* ═══════════════════════════════════════════════════════════════════════
   Step 3 — Launch Sessions
   ═══════════════════════════════════════════════════════════════════════ */
function StepSessions() {
  const setNewAgentDialogOpen = useAppStore((s) => s.setNewAgentDialogOpen)
  const newSessionShortcut = SHORTCUTS.find((s) => s.id === 'new-session')

  return (
    <div className="space-y-4">
      <p className="text-[13px] text-gray-400 leading-relaxed">
        Press <Kbd>{newSessionShortcut?.display ?? '⌘N'}</Kbd> to open the session launcher. Pick an
        agent, project, and branch — then type an optional prompt to direct the agent.
      </p>

      <div className="rounded-lg border border-white/[0.06] p-4 space-y-3 bg-surface-sunken">
        <div className="flex items-center gap-2">
          {LAUNCHER_PICKERS.map(({ icon: Icon, label }) => (
            <div
              key={label}
              className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-md bg-white/[0.04] border border-white/[0.06]"
            >
              <Icon size={12} className="text-gray-400" />
              <span className="text-[11px] text-gray-300">{label}</span>
              <ChevronDown size={10} className="text-gray-600" />
            </div>
          ))}
        </div>

        <div className="w-full px-3 py-2 rounded-md bg-black/30 border border-white/[0.06]">
          <span className="text-[11px] text-gray-600 italic">
            Type a prompt to direct the agent...
          </span>
        </div>

        <div className="flex items-center justify-between">
          <span className="text-[10px] text-gray-600">
            Sessions can also run on remote hosts via SSH
          </span>
          <div className="flex items-center gap-1 px-3 py-1 rounded-md bg-white/[0.06] text-[11px] text-gray-400">
            <Rocket size={10} />
            Launch
          </div>
        </div>
      </div>

      <AccentButton
        icon={Rocket}
        label="Try it now"
        onClick={() => closeAndOpen(() => setNewAgentDialogOpen(true))}
      />
    </div>
  )
}

/* ═══════════════════════════════════════════════════════════════════════
   Step 4 — Your Workspace
   ═══════════════════════════════════════════════════════════════════════ */
function StepWorkspace() {
  const viewOptionsShortcut = SHORTCUTS.find((s) => s.id === 'view-options')

  return (
    <div className="space-y-4">
      <p className="text-[13px] text-gray-400 leading-relaxed">
        View sessions in a multi-agent <span className="text-gray-200">Grid</span> or focused{' '}
        <span className="text-gray-200">Tabs</span> layout. Press{' '}
        <Kbd>{viewOptionsShortcut?.display ?? '⌘J'}</Kbd> to toggle.
      </p>

      <div className="grid grid-cols-2 gap-3">
        <div className="rounded-lg border border-white/[0.06] p-3 space-y-2 bg-surface-sunken">
          <div className="flex items-center gap-1.5 mb-1">
            <LayoutGrid size={11} className="text-cyan-400" />
            <span className="text-[11px] font-medium text-gray-300">Grid</span>
          </div>
          <div className="grid grid-cols-2 gap-1.5">
            {[1, 2, 3, 4].map((i) => (
              <div key={i} className="h-8 rounded bg-white/[0.04] border border-white/[0.06]" />
            ))}
          </div>
          <span className="text-[10px] text-gray-600">Side-by-side comparison</span>
        </div>

        <div className="rounded-lg border border-white/[0.06] p-3 space-y-2 bg-surface-sunken">
          <div className="flex items-center gap-1.5 mb-1">
            <Columns3 size={11} className="text-violet-400" />
            <span className="text-[11px] font-medium text-gray-300">Tabs</span>
          </div>
          <div className="space-y-1.5">
            <div className="flex gap-1">
              {['Agent 1', 'Agent 2', 'Agent 3'].map((t, i) => (
                <span
                  key={t}
                  className="text-[9px] px-1.5 py-0.5 rounded"
                  style={{
                    background: i === 0 ? 'rgba(201,151,42,0.08)' : 'rgba(255,255,255,0.03)',
                    color: i === 0 ? 'var(--color-bronzo)' : '#555'
                  }}
                >
                  {t}
                </span>
              ))}
            </div>
            <div className="h-12 rounded bg-white/[0.04] border border-white/[0.06]" />
          </div>
          <span className="text-[10px] text-gray-600">Maximum terminal space</span>
        </div>
      </div>

      <div className="rounded-lg border border-white/[0.06] p-3 bg-surface-sunken">
        <div className="flex items-center gap-2 mb-2">
          <PanelRight size={12} className="text-gray-400" />
          <span className="text-[11px] font-medium text-gray-300">Review Panel</span>
          <span className="text-[10px] text-gray-600">— click the diff badge on any card</span>
        </div>
        <div className="flex items-center gap-3 text-[11px] text-gray-500">
          <span className="flex items-center gap-1">
            <FileCode size={10} /> File tree
          </span>
          <span className="flex items-center gap-1">
            <FileDiff size={10} /> Syntax diffs
          </span>
          <span className="flex items-center gap-1">
            <GitCommitHorizontal size={10} /> Commit & push
          </span>
        </div>
      </div>

      <p className="text-[11px] text-gray-600">
        Headless agents appear as compact pills above the grid for background tasks.
      </p>
    </div>
  )
}

/* ═══════════════════════════════════════════════════════════════════════
   Step 5 — Workflows & Schedules
   ═══════════════════════════════════════════════════════════════════════ */
function StepWorkflows() {
  return (
    <div className="space-y-4">
      <p className="text-[13px] text-gray-400 leading-relaxed">
        Build multi-step workflows that launch agents, run scripts, and branch based on conditions.
        Trigger them manually, on a schedule, or when a task changes status.
      </p>

      <div className="rounded-lg border border-white/[0.06] p-4 space-y-2.5 bg-surface-sunken">
        <div className="text-[11px] text-gray-600 uppercase tracking-wider font-medium mb-2">
          Trigger types
        </div>
        {[
          { icon: Zap, label: 'Manual', desc: 'Run on demand from sidebar' },
          { icon: Clock, label: 'Scheduled', desc: 'Cron or one-time schedule' },
          { icon: KanbanSquare, label: 'Task event', desc: 'When a task status changes' }
        ].map(({ icon: Icon, label, desc }) => (
          <div key={label} className="flex items-center gap-3">
            <div className="w-6 h-6 rounded flex items-center justify-center bg-white/[0.04]">
              <Icon size={12} className="text-gray-400" />
            </div>
            <div>
              <div className="text-[12px] text-gray-300">{label}</div>
              <div className="text-[11px] text-gray-600">{desc}</div>
            </div>
          </div>
        ))}
      </div>

      <div className="flex items-center gap-2 flex-wrap">
        {['Launch Agent', 'Run Script', 'Condition', 'Parallel'].map((n) => (
          <span
            key={n}
            className="text-[11px] px-2.5 py-1 rounded-md border border-white/[0.06] text-gray-400 bg-surface-sunken"
          >
            {n}
          </span>
        ))}
      </div>

      <p className="text-[11px] text-gray-600">
        Create workflows from the sidebar or command palette.
      </p>
    </div>
  )
}

/* ═══════════════════════════════════════════════════════════════════════
   Step 6 — Command Palette & Shortcuts
   ═══════════════════════════════════════════════════════════════════════ */
function StepShortcuts() {
  const setCommandPaletteOpen = useAppStore((s) => s.setCommandPaletteOpen)

  return (
    <div className="space-y-4">
      <p className="text-[13px] text-gray-400 leading-relaxed">
        Press <Kbd>{ESSENTIAL_SHORTCUTS[0]?.display ?? '⌘K'}</Kbd> to open the command palette — it
        searches sessions, projects, workflows, and every action in one place.
      </p>

      <div className="rounded-lg border border-white/[0.06] p-3 grid grid-cols-2 gap-x-4 gap-y-2 bg-surface-sunken">
        {ESSENTIAL_SHORTCUTS.map((s) => (
          <div key={s.id} className="flex items-center justify-between gap-2">
            <span className="text-[12px] text-gray-400">{s.description}</span>
            <Kbd>{s.display}</Kbd>
          </div>
        ))}
      </div>

      <AccentButton
        icon={Search}
        label="Try Command Palette"
        onClick={() => closeAndOpen(() => setCommandPaletteOpen(true))}
      />

      <p className="text-[11px] text-gray-600">
        View all shortcuts anytime with{' '}
        <Kbd>{ESSENTIAL_SHORTCUTS.find((s) => s.id === 'shortcuts-panel')?.display ?? '⌘/'}</Kbd>
      </p>
    </div>
  )
}

/* ═══════════════════════════════════════════════════════════════════════
   Step 7 — You're Ready
   ═══════════════════════════════════════════════════════════════════════ */
function StepReady() {
  const setSettingsOpen = useAppStore((s) => s.setSettingsOpen)
  const setSettingsCategory = useAppStore((s) => s.setSettingsCategory)

  const openSettings = (category: 'notifications' | 'ssh' | 'appearance' | 'agents') => {
    closeAndOpen(() => {
      setSettingsCategory(category)
      setSettingsOpen(true)
    })
  }

  return (
    <div className="space-y-4">
      <p className="text-[13px] text-gray-400 leading-relaxed">
        You are all set. Here are a few more things to explore when you are ready.
      </p>

      <div className="space-y-1.5">
        {EXPLORE_ITEMS.map(({ icon: Icon, label, category }) => (
          <button
            key={category}
            onClick={() => openSettings(category)}
            className="w-full flex items-center gap-3 px-3 py-2.5 rounded-lg border border-white/[0.04]
                       hover:border-white/[0.08] hover:bg-white/[0.02] transition-all text-left group bg-surface-sunken"
          >
            <div className="w-7 h-7 rounded-md flex items-center justify-center bg-white/[0.04] group-hover:bg-white/[0.06] transition-colors">
              <Icon size={14} className="text-gray-400" />
            </div>
            <span className="text-[13px] text-gray-300 flex-1">{label}</span>
            <ArrowRight
              size={12}
              className="text-gray-600 group-hover:text-gray-400 transition-colors"
            />
          </button>
        ))}
      </div>

      <p className="text-[11px] text-gray-600">
        You can reopen this guide anytime from the sidebar or command palette.
      </p>
    </div>
  )
}

const StepFallback: React.FC = () => null

const STEP_CONTENT: Record<string, React.FC> = {
  agents: StepAgents,
  projects: StepProjects,
  sessions: StepSessions,
  workspace: StepWorkspace,
  workflows: StepWorkflows,
  shortcuts: StepShortcuts,
  ready: StepReady
}

/* ═══════════════════════════════════════════════════════════════════════
   Main modal
   ═══════════════════════════════════════════════════════════════════════ */
export function OnboardingModal() {
  const [currentStep, setCurrentStep] = useState(0)

  const step = ONBOARDING_STEPS[currentStep]
  const StepIcon = STEP_ICONS[step.icon]
  const StepBody = STEP_CONTENT[step.id] ?? StepFallback
  const isLast = currentStep === ONBOARDING_STEPS.length - 1

  const close = useCallback(() => {
    useAppStore.getState().setOnboardingOpen(false)
    const config = useAppStore.getState().config
    if (config) {
      const updated = {
        ...config,
        defaults: { ...config.defaults, hasSeenOnboarding: 2 }
      }
      useAppStore.getState().setConfig(updated)
      window.api.saveConfig(updated)
    }
  }, [])

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') close()
      if (e.key === 'ArrowRight')
        setCurrentStep((s) => Math.min(ONBOARDING_STEPS.length - 1, s + 1))
      if (e.key === 'ArrowLeft') setCurrentStep((s) => Math.max(0, s - 1))
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [close])

  return (
    <>
      <motion.div
        className="fixed inset-0 bg-black/60 z-[60]"
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        exit={{ opacity: 0 }}
        onClick={close}
      />

      <motion.div
        className="fixed top-1/2 left-1/2 z-[60] w-[520px] max-h-[85vh]
                   border border-white/[0.08] rounded-2xl shadow-2xl overflow-hidden flex flex-col bg-surface-overlay"
        initial={{ opacity: 0, scale: 0.96, x: '-50%', y: '-50%' }}
        animate={{ opacity: 1, scale: 1, x: '-50%', y: '-50%' }}
        exit={{ opacity: 0, scale: 0.96, x: '-50%', y: '-50%' }}
        transition={{ duration: 0.18, ease: [0.4, 0, 0.2, 1] }}
      >
        <div className="px-6 pt-5 pb-4 border-b border-white/[0.06]">
          <button
            onClick={close}
            className="absolute top-3.5 right-3.5 p-1 text-gray-600 hover:text-gray-300 transition-colors"
            aria-label="Close welcome guide"
          >
            <X size={16} />
          </button>

          <div className="flex items-center gap-1.5 mb-4">
            {ONBOARDING_STEPS.map((s, i) => (
              <button
                key={s.id}
                onClick={() => setCurrentStep(i)}
                className="relative h-1 rounded-full transition-all duration-300"
                style={{
                  width: i === currentStep ? 32 : 16,
                  background: i <= currentStep ? 'var(--color-bronzo)' : 'rgba(255,255,255,0.08)'
                }}
                aria-label={`Go to step ${i + 1}: ${s.title}`}
              />
            ))}
            <span className="ml-auto text-[11px] text-gray-600">
              {currentStep + 1}/{ONBOARDING_STEPS.length}
            </span>
          </div>

          <AnimatePresence mode="wait">
            <motion.div
              key={step.id}
              initial={{ opacity: 0, x: 12 }}
              animate={{ opacity: 1, x: 0 }}
              exit={{ opacity: 0, x: -12 }}
              transition={{ duration: 0.15 }}
              className="flex items-center gap-3"
            >
              <div className="w-10 h-10 rounded-xl flex items-center justify-center shrink-0 bg-bronzo/[0.06]">
                {StepIcon && <StepIcon size={20} className="text-bronzo" />}
              </div>
              <div>
                <h2 className="text-base font-semibold text-white">{step.title}</h2>
                <p className="text-[12px] text-gray-500">{step.subtitle}</p>
              </div>
            </motion.div>
          </AnimatePresence>
        </div>

        <div className="flex-1 overflow-y-auto px-6 py-5">
          <AnimatePresence mode="wait">
            <motion.div
              key={step.id}
              initial={{ opacity: 0, y: 8 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: -8 }}
              transition={{ duration: 0.15 }}
            >
              <StepBody />
            </motion.div>
          </AnimatePresence>
        </div>

        <div className="px-6 py-3.5 border-t border-white/[0.06] flex items-center gap-3">
          <button
            onClick={close}
            className="text-[12px] text-gray-600 hover:text-gray-400 transition-colors"
          >
            Skip guide
          </button>

          <div className="flex-1" />

          {currentStep > 0 && (
            <button
              onClick={() => setCurrentStep((s) => s - 1)}
              className="flex items-center gap-1.5 px-3 py-1.5 text-[13px] text-gray-400
                         hover:text-white rounded-lg hover:bg-white/[0.04] transition-all"
            >
              <ArrowLeft size={13} />
              Back
            </button>
          )}

          <button
            onClick={isLast ? close : () => setCurrentStep((s) => s + 1)}
            className="flex items-center gap-1.5 px-4 py-1.5 text-[13px] font-medium text-black
                       rounded-lg bg-bronzo hover:bg-bronzo-dark transition-colors"
          >
            {isLast ? 'Get Started' : 'Next'}
            <ArrowRight size={13} />
          </button>
        </div>
      </motion.div>
    </>
  )
}
