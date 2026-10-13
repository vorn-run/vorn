import { useState, useCallback, useMemo } from 'react'
import { useAppStore } from '../../stores'
import { Tooltip } from '../Tooltip'
import { SessionGroupItem } from './SessionGroupItem'
import { SessionItem } from './SessionItem'
import { SidebarNavItem } from './SidebarNavItem'
import { SidebarSectionHeader } from './SidebarSectionHeader'
import { ProjectsSectionToolbar } from './ProjectsSectionToolbar'
import { useWorkspaceSessionGroups } from '../../hooks/useWorkspaceSessionGroups'
import { useClaimedTerminalIds } from '../../hooks/usePanelTerminals'
import { getDisplayName } from '../../lib/terminal-display'
import { Group, Layers } from 'lucide-react'
import type { SidebarSessionInfo } from './types'

/**
 * Sessions arranged by the buckets a person made, rather than by where their
 * code lives. Groups are drawn here and nowhere else: seeing them beside the
 * project tree as well was two organisations of the same sessions at once.
 */
export function GroupSessionsSection({
  isCollapsed,
  workspaceProjectNames,
  workspaceTerminalCount
}: {
  isCollapsed: boolean
  workspaceProjectNames: Set<string>
  workspaceTerminalCount: number
}) {
  const terminals = useAppStore((s) => s.terminals)
  const claimed = useClaimedTerminalIds()
  const activeProject = useAppStore((s) => s.activeProject)
  const activeGroupId = useAppStore((s) => s.activeGroupId)
  const setActiveProject = useAppStore((s) => s.setActiveProject)
  const setActiveGroup = useAppStore((s) => s.setActiveGroup)
  const setFocusedTerminal = useAppStore((s) => s.setFocusedTerminal)
  const addSessionGroup = useAppStore((s) => s.addSessionGroup)
  const moveSessionToGroup = useAppStore((s) => s.moveSessionToGroup)
  const activeWorkspace = useAppStore((s) => s.activeWorkspace)
  const groups = useWorkspaceSessionGroups()

  const [sectionCollapsed, setSectionCollapsed] = useState(false)
  const [collapsedGroups, setCollapsedGroups] = useState<Set<string>>(new Set())
  const [renamingId, setRenamingId] = useState<string | null>(null)
  const [dropTarget, setDropTarget] = useState<string | null>(null)

  const groupIds = useMemo(() => new Set(groups.map((g) => g.id)), [groups])

  const { grouped, ungrouped } = useMemo(() => {
    const all: (SidebarSessionInfo & { groupId?: string; lastActivity: number })[] = []
    for (const [id, t] of terminals) {
      if (claimed.has(id)) continue
      if (!workspaceProjectNames.has(t.session.projectName)) continue
      all.push({
        id,
        name: getDisplayName(t.session),
        status: t.status,
        agentType: t.session.agentType,
        branch: t.session.branch,
        isWorktree: t.session.isWorktree,
        worktreePath: t.session.worktreePath,
        groupId: t.session.groupId,
        lastActivity: t.lastOutputTimestamp
      })
    }
    all.sort((a, b) => b.lastActivity - a.lastActivity)
    const byGroup = new Map<string, SidebarSessionInfo[]>()
    const loose: SidebarSessionInfo[] = []
    for (const s of all) {
      // A group that has been deleted leaves its sessions loose, not lost.
      if (s.groupId && groupIds.has(s.groupId)) {
        byGroup.set(s.groupId, [...(byGroup.get(s.groupId) ?? []), s])
      } else {
        loose.push(s)
      }
    }
    return { grouped: byGroup, ungrouped: loose }
  }, [terminals, claimed, workspaceProjectNames, groupIds])

  const createGroup = useCallback(() => {
    const id = crypto.randomUUID()
    const order = groups.reduce((max, g) => Math.max(max, g.order), -1) + 1
    addSessionGroup({ id, name: 'New group', order, workspaceId: activeWorkspace })
    setRenamingId(id)
  }, [groups, addSessionGroup, activeWorkspace])

  return (
    <>
      <SidebarSectionHeader
        title="Groups"
        isCollapsed={isCollapsed}
        sectionCollapsed={sectionCollapsed}
        onToggle={() => setSectionCollapsed(!sectionCollapsed)}
        actions={
          <>
            <ProjectsSectionToolbar />
            <Tooltip label="New group" position="bottom">
              <button
                onClick={createGroup}
                aria-label="New group"
                className="p-0.5 rounded text-gray-600 hover:text-white hover:bg-white/[0.08] transition-colors"
              >
                <Group size={13} strokeWidth={1.5} />
              </button>
            </Tooltip>
          </>
        }
      />

      {!sectionCollapsed && (
        <SidebarNavItem
          isActive={activeProject === null && activeGroupId === null}
          isCollapsed={isCollapsed}
          icon={<Layers size={isCollapsed ? 22 : 14} strokeWidth={1.5} />}
          label="All Sessions"
          badge={workspaceTerminalCount}
          onClick={() => {
            setActiveProject(null)
            setActiveGroup(null)
            setFocusedTerminal(null)
          }}
        />
      )}

      {!isCollapsed &&
        !sectionCollapsed &&
        groups.map((group) => {
          const members = grouped.get(group.id) ?? []
          const isExpanded = !collapsedGroups.has(group.id)
          return (
            <div
              key={group.id}
              onDragOver={(e) => {
                if (!e.dataTransfer.types.includes('application/vorn-session')) return
                e.preventDefault()
                e.dataTransfer.dropEffect = 'move'
                setDropTarget(group.id)
              }}
              onDragLeave={() => setDropTarget(null)}
              onDrop={(e) => {
                e.preventDefault()
                const id = e.dataTransfer.getData('application/vorn-session')
                if (id) moveSessionToGroup(id, group.id)
                setDropTarget(null)
              }}
              className={dropTarget === group.id ? 'rounded-md bg-white/[0.04]' : undefined}
            >
              <SessionGroupItem
                group={group}
                sessionCount={members.length}
                hasWaiting={members.some((m) => m.status === 'waiting')}
                isExpanded={isExpanded}
                onToggleExpanded={() =>
                  setCollapsedGroups((prev) => {
                    const next = new Set(prev)
                    if (next.has(group.id)) next.delete(group.id)
                    else next.add(group.id)
                    return next
                  })
                }
                startRenaming={renamingId === group.id}
                onRenameSettled={() => setRenamingId(null)}
              />
              {isExpanded && (
                <div className="ml-2">
                  {members.length > 0 ? (
                    members.map((m) => <SessionItem key={m.id} session={m} showBranch={true} />)
                  ) : (
                    <p className="text-[11px] text-gray-600 px-2 py-1">Drop a session here</p>
                  )}
                </div>
              )}
            </div>
          )
        })}

      {/* Ungrouped sessions, drawn exactly as the flat list draws them. */}
      {!isCollapsed && !sectionCollapsed && (
        <div className="space-y-0.5 mt-1">
          {ungrouped.map((s) => (
            <SessionItem key={s.id} session={s} showBranch={true} />
          ))}
          {ungrouped.length === 0 && grouped.size === 0 && (
            <p className="text-[11px] text-gray-600 px-2 py-1">No active sessions</p>
          )}
        </div>
      )}
    </>
  )
}
