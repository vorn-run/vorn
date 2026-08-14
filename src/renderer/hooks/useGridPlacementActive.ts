import { useAppStore } from '../stores'
import { useShallow } from 'zustand/react/shallow'
import { useIsMobile } from './useIsMobile'

/**
 * Whether the grid is the thing currently drawing sessions.
 *
 * Placement state — minimized, promoted — describes where something sits *in
 * the grid*, so it means nothing in the layouts that bypass the grid entirely:
 * the tab strip shows one session, focused mode shows one, mobile shows one.
 * The dock already follows this rule (`includeMinimized={layoutMode === 'grid'}`),
 * and a promoted pane follows the same one — outside the grid it goes back to
 * rendering inside its owner's card, because otherwise it would be nowhere at
 * all, with the control to bring it back living on the pane that vanished.
 *
 * Nothing is reset when the layout changes: switch back to the grid and the
 * pane is promoted again, exactly as a minimized session is still minimized.
 */
export function useGridPlacementActive(): boolean {
  const { layoutMode, focusedId, previewId } = useAppStore(
    useShallow((s) => ({
      layoutMode: s.config?.defaults?.layoutMode ?? 'grid',
      focusedId: s.focusedTerminalId,
      previewId: s.previewTerminalId
    }))
  )
  const isMobile = useIsMobile()

  return layoutMode !== 'tabs' && !focusedId && !previewId && !isMobile
}
