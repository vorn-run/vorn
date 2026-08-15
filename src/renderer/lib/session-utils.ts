import {
  supportsExactSessionResume,
  getProjectRemoteHostId,
  type TerminalSession,
  type RecentSession,
  type CreateTerminalPayload,
  type ProjectConfig,
  type AiAgentType
} from '../../shared/types'
import { useAppStore } from '../stores'
import { toast } from '../components/Toast'

function normalizeComparablePath(p: string): string {
  const normalized = p.replace(/\\/g, '/').replace(/\/+$/, '')
  if (!normalized) return '/'
  return normalized.toLowerCase()
}

export function getDisplayPathBasename(p: string): string | undefined {
  const normalized = p.replace(/\\/g, '/').replace(/\/+$/, '')
  if (!normalized || normalized === '/') return undefined
  const parts = normalized.split('/').filter(Boolean)
  return parts.length > 0 ? parts[parts.length - 1] : undefined
}

function getManagedWorktreePrefix(projectPath: string): string | null {
  const normalized = normalizeComparablePath(projectPath)
  const parts = normalized.split('/').filter(Boolean)
  if (parts.length === 0) return null

  const projectName = parts[parts.length - 1]
  if (!projectName) return null

  const prefix = normalized.startsWith('/') ? '/' : ''
  const parent = parts.slice(0, -1).join('/')
  const parentPath = parent ? `${prefix}${parent}` : prefix || '.'
  return `${parentPath}/.vorn-worktrees/${projectName}`
}

function isManagedWorktreePath(candidatePath: string, projectPath: string): boolean {
  const prefix = getManagedWorktreePrefix(projectPath)
  if (!prefix) return false
  return normalizeComparablePath(candidatePath).startsWith(`${prefix}/`)
}

function pluralizeActivityLabel(label: string, count: number): string {
  if (count === 1) return label
  if (label.endsWith('y')) return `${label.slice(0, -1)}ies`
  return `${label}s`
}

export function formatRecentSessionActivity(
  session: Pick<RecentSession, 'activityCount' | 'activityLabel'>
): string {
  return `${session.activityCount} ${pluralizeActivityLabel(session.activityLabel, session.activityCount)}`
}

export function formatSessionCount(count: number): string {
  return count > 0 ? `${count} session${count > 1 ? 's' : ''}` : ''
}

export function countSessionsByWorktree(
  terminals: Iterable<{ session: Pick<TerminalSession, 'worktreePath'> }>
): Map<string, number> {
  const counts = new Map<string, number>()
  for (const t of terminals) {
    const wtPath = t.session.worktreePath
    if (wtPath) counts.set(wtPath, (counts.get(wtPath) ?? 0) + 1)
  }
  return counts
}

function isDefined<T>(value: T | undefined): value is T {
  return value !== undefined
}

/**
 * Resolve the agent session ID to pass as --resume when restoring a session.
 * Falls back through progressively looser matching strategies.
 *
 * @param claimed - session IDs already assigned to other terminals in this
 *   restore batch; prevents multiple terminals from resuming the same session.
 */
export async function resolveResumeSessionId(
  s: TerminalSession,
  claimed: Set<string> = new Set()
): Promise<string | undefined> {
  if (!supportsExactSessionResume(s.agentType)) return undefined
  // Only use agentSessionId for resume — it's the real agent session ID passed
  // via --session-id on fresh launch. hookSessionId is a Vorn-internal UUID
  // for hook event routing; the agent CLI doesn't know about it. For agents
  // without pinning support, fall through to the history-based scan.
  const resumeId = s.agentSessionId
  if (resumeId && !claimed.has(resumeId)) return resumeId

  const isAvailable = (r: RecentSession): boolean =>
    r.agentType === s.agentType && r.canResumeExact && !claimed.has(r.sessionId)

  // Fallback for agents without hookSessionId: progressively looser matching
  const targetPaths = [s.worktreePath, s.projectPath].filter(isDefined).map(normalizeComparablePath)
  const findPreferredPathMatch = (sessions: RecentSession[]): RecentSession | undefined => {
    for (const targetPath of targetPaths) {
      const match = sessions.find(
        (session) =>
          isAvailable(session) && normalizeComparablePath(session.projectPath) === targetPath
      )
      if (match) return match
    }
    return undefined
  }

  // Scoped fetch first — the global unscoped list has a default limit of 20,
  // so a project's sessions may not appear in the global results.
  try {
    const scopedRecent = await window.api.getRecentSessions(s.projectPath)
    const scopedExact = findPreferredPathMatch(scopedRecent)
    if (scopedExact) return scopedExact.sessionId

    const scopedMatch = scopedRecent.find(isAvailable)
    if (scopedMatch) return scopedMatch.sessionId
  } catch {
    // Scoped fetch failed — fall through to unscoped lookup
  }

  // Unscoped fallback for looser matching (path normalization mismatches)
  const allRecent = await window.api.getRecentSessions()

  // Exact project path match
  const exact = findPreferredPathMatch(allRecent)
  if (exact) return exact.sessionId

  return undefined
}

/**
 * Find the best matching project name for a recent session's path.
 * Normalizes trailing slashes for robust matching.
 */
export function resolveProjectName(
  session: RecentSession,
  projects: { name: string; path: string }[] | undefined
): string {
  const normalized = normalizeComparablePath(session.projectPath)
  const displayBasename = getDisplayPathBasename(session.projectPath)
  if (!projects) return displayBasename || 'untitled'

  const project = projects.find((p) => {
    const projectPath = normalizeComparablePath(p.path)
    return projectPath === normalized || isManagedWorktreePath(session.projectPath, p.path)
  })
  return project?.name || displayBasename || 'untitled'
}

export function buildRestorePayload(
  s: TerminalSession,
  resumeSessionId?: string
): CreateTerminalPayload {
  if (s.agentType === 'shell') {
    throw new Error(
      'buildRestorePayload: shell sessions restore via createShellTerminal, not createTerminal'
    )
  }
  return {
    agentType: s.agentType,
    projectName: s.projectName,
    projectPath: s.projectPath,
    displayName: s.displayName,
    branch: s.isWorktree ? s.branch : undefined,
    existingWorktreePath: s.isWorktree ? s.worktreePath : undefined,
    worktreeName: s.worktreeName,
    useWorktree: (s.isWorktree && !s.worktreePath) || undefined,
    remoteHostId: s.remoteHostId,
    resumeSessionId
  }
}

/**
 * Create a terminal session in a project. Handles agent defaults, remote host
 * resolution, and the worktree branch lookup so callers (grid context menu,
 * command palette, project sidebar) share one code path.
 */
export async function createSessionFromProject(
  p: ProjectConfig,
  opts: {
    agentType?: AiAgentType
    branch?: string
    existingWorktreePath?: string
    useWorktree?: boolean
  } = {}
): Promise<void> {
  const state = useAppStore.getState()
  const agentType = opts.agentType ?? state.config?.defaults.defaultAgent ?? 'claude'
  const remoteHostId = getProjectRemoteHostId(p)
  let branch = opts.branch
  if (opts.useWorktree && !branch) {
    const branchResult = await window.api.listBranches(p.path)
    branch = branchResult.current || 'main'
  }
  const session = await window.api.createTerminal({
    agentType,
    projectName: p.name,
    projectPath: p.path,
    branch,
    existingWorktreePath: opts.existingWorktreePath,
    useWorktree: opts.useWorktree,
    remoteHostId
  })
  state.addTerminal(session)
}

/**
 * Start a shell inside a session's terminals panel.
 *
 * Same creation as `createShellInProject` — a shell terminal is a full session
 * either way — differing only in where the new id is put. It must not become
 * the active tab: a terminal claimed by a panel is deliberately absent from the
 * tab strip, so activating one would select a tab that is not there.
 */
export async function addTerminalToPanel(sessionId: string, cwd: string): Promise<void> {
  try {
    const session = await window.api.createShellTerminal(cwd)
    const state = useAppStore.getState()
    const owner = state.terminals.get(sessionId)?.session
    const enriched: TerminalSession = owner
      ? {
          ...session,
          projectName: owner.projectName,
          projectPath: owner.projectPath,
          worktreePath: owner.worktreePath,
          worktreeName: owner.worktreeName,
          branch: owner.branch,
          isWorktree: owner.isWorktree
        }
      : session
    state.addTerminal(enriched)
    state.openTerminalsPane(sessionId, enriched.id)
  } catch (err) {
    console.error('[addTerminalToPanel] failed:', err)
    toast.error(`Failed to start terminal: ${err instanceof Error ? err.message : String(err)}`)
  }
}

export async function createShellInProject(
  cwd?: string,
  context?: {
    project?: ProjectConfig
    worktreePath?: string
    worktreeName?: string
    branch?: string
  }
): Promise<void> {
  try {
    const session = await window.api.createShellTerminal(cwd)
    const project = context?.project ?? resolveProjectForPath(cwd)
    const enriched: TerminalSession = project
      ? {
          ...session,
          projectName: project.name,
          projectPath: project.path,
          worktreePath: context?.worktreePath,
          worktreeName: context?.worktreeName,
          branch: context?.branch,
          isWorktree: Boolean(context?.worktreePath && context.worktreePath !== project.path)
        }
      : session
    const state = useAppStore.getState()
    state.addTerminal(enriched)
    state.setActiveTabId(enriched.id)
  } catch (err) {
    console.error('[createShellInProject] failed:', err)
    toast.error(`Failed to start terminal: ${err instanceof Error ? err.message : String(err)}`)
  }
}

/**
 * Start an agent session from a shell card, seeded with the typed prompt.
 *
 * The agent lands in whatever directory the shell is currently in, so a `cd`
 * before the prompt is honoured.
 */
export async function launchAgentFromShell(
  shellSession: TerminalSession,
  agentType: AiAgentType,
  prompt: string
): Promise<void> {
  const cwd = shellSession.shellCwd ?? shellSession.worktreePath ?? shellSession.projectPath
  try {
    // `branch`, `useWorktree` and `existingWorktreePath` are deliberately
    // omitted. createPty checks out `payload.branch` against the project when
    // no worktree is supplied, which would move the user's checkout under
    // them. The shell is already in the directory we want.
    const session = await window.api.createTerminal({
      agentType,
      projectName: shellSession.projectName,
      projectPath: cwd,
      initialPrompt: prompt,
      remoteHostId: shellSession.remoteHostId,
      worktreeName: shellSession.worktreeName
    })
    const state = useAppStore.getState()
    state.addTerminal(session)
    state.setActiveTabId(session.id)
  } catch (err) {
    console.error('[launchAgentFromShell] failed:', err)
    toast.error(`Failed to start agent: ${err instanceof Error ? err.message : String(err)}`)
  }
}

function resolveProjectForPath(p?: string): ProjectConfig | undefined {
  if (!p) return undefined
  const projects = useAppStore.getState().config?.projects ?? []
  const normalized = normalizeComparablePath(p)
  const match = projects.find((proj) => {
    const projPath = normalizeComparablePath(proj.path)
    return (
      projPath === normalized ||
      normalized.startsWith(`${projPath}/`) ||
      isManagedWorktreePath(p, proj.path)
    )
  })
  return match
}

/**
 * Resolve the currently active project from the store.
 * Falls back to the first project in the active workspace.
 */
export function resolveActiveProject() {
  const state = useAppStore.getState()
  const projects = state.config?.projects
  if (!projects || projects.length === 0) return undefined

  const activeProjectName = state.activeProject
  if (activeProjectName) {
    const match = projects.find((p) => p.name === activeProjectName)
    if (match) return match
  }
  const ws = state.activeWorkspace
  return projects.find((p) => (p.workspaceId ?? 'personal') === ws)
}
