import { SidebarToggleButton } from './SidebarToggleButton'
import { MainViewPills } from './MainViewPills'

/**
 * The way back to the sidebar and to the other two views, for whichever bar is
 * standing in for the titlebar.
 *
 * Three bars can be that bar — the app toolbar, the merged tab strip, and a
 * focused session's own header — and each of them was spelling this pair out
 * for itself. The focused header spelled out neither, which is how expanding a
 * session with the sidebar closed left no route to tasks or workflows at all.
 *
 * Shown only while the sidebar is hidden; open, it carries both itself.
 */
export function AppNavCluster() {
  return (
    <div className="shrink-0 flex items-center gap-1 titlebar-no-drag">
      <SidebarToggleButton />
      <div className="w-px h-4 bg-white/[0.06] mx-0.5" />
      <MainViewPills />
    </div>
  )
}
