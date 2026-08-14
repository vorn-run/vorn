import { useAppStore } from '../stores'
import { useShallow } from 'zustand/react/shallow'
import { useIsMobile } from './useIsMobile'

/**
 * Whether the grid is the thing currently drawing cells.
 *
 * A popped-out card is a grid cell, so it only has somewhere to be drawn while
 * the grid is what's on screen. The other layouts each show one session and
 * bypass the grid entirely: the tab strip, focused mode, mobile. There a card
 * would be nowhere at all — and the controls that bring it back travel with it,
 * so it could not be recovered from the card either.
 *
 * So outside the grid, a session's cards fall back into its pane column. The
 * item does not move in the store; only where it is drawn changes, and it is
 * drawn in exactly one place under either answer — which is what this is for.
 * Get it wrong in the true direction and a browser card mounts twice, two guests
 * on one url.
 */
export function useGridDrawsCards(): boolean {
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
