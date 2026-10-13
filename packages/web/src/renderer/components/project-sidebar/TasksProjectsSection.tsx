import { useState } from 'react'
import { useAppStore } from '../../stores'
import { Layers } from 'lucide-react'
import { ProjectIcon } from './ProjectIcon'
import { SidebarNavItem } from './SidebarNavItem'
import { SidebarSectionHeader } from './SidebarSectionHeader'
import type { ProjectConfig } from '../../../shared/types'

export function TasksProjectsSection({
  isCollapsed,
  workspaceProjects
}: {
  isCollapsed: boolean
  workspaceProjects: ProjectConfig[]
}) {
  const activeProject = useAppStore((s) => s.activeProject)
  const setActiveProject = useAppStore((s) => s.setActiveProject)

  const [sectionCollapsed, setSectionCollapsed] = useState(false)

  const iconSize = isCollapsed ? 22 : 14

  return (
    <>
      <SidebarSectionHeader
        title="Projects"
        isCollapsed={isCollapsed}
        sectionCollapsed={sectionCollapsed}
        onToggle={() => setSectionCollapsed(!sectionCollapsed)}
      />

      {!sectionCollapsed && (
        <SidebarNavItem
          isActive={activeProject === null}
          isCollapsed={isCollapsed}
          icon={<Layers size={iconSize} strokeWidth={1.5} />}
          label="All Projects"
          onClick={() => setActiveProject(null)}
        />
      )}

      {!isCollapsed && !sectionCollapsed && workspaceProjects.length === 0 && (
        <p className="text-[13px] text-gray-600 px-2.5 py-1">No projects</p>
      )}

      {!sectionCollapsed &&
        workspaceProjects.map((project) => (
          <SidebarNavItem
            key={project.name}
            isActive={activeProject === project.name}
            isCollapsed={isCollapsed}
            icon={<ProjectIcon icon={project.icon} color={project.iconColor} size={iconSize} />}
            label={project.name}
            onClick={() => setActiveProject(project.name)}
          />
        ))}
    </>
  )
}
