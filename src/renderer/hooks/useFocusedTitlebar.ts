import { useAppStore } from '../stores'
import { useIsMobile } from './useIsMobile'
import { isMac, isWeb } from '../lib/platform'

/**
 * Who is drawing the window's titlebar right now.
 *
 * On macOS the app drops its own bar whenever the focus stage fills the window,
 * which promotes that stage's card header to the window's drag region — and to
 * the rest of a titlebar's job with it: the inset the traffic lights sit in, and
 * the controls the bar it replaced was carrying. Both were missing, so the
 * lights landed on the session name and there was no way to reach tasks or
 * workflows without collapsing the session first.
 *
 * It lives here rather than in each header so `App` and the headers cannot
 * disagree about which of them owns the bar — the same reason `TabView` reads
 * the pane column's own answer instead of re-deriving it.
 */
export interface FocusedTitlebar {
  /** The app hides its own bar; the focused header stands in for it. */
  ownsTitlebar: boolean
  /** Leave room for the traffic lights — nothing else on screen is holding them. */
  needsTrafficLightPad: boolean
  /** Carry the sidebar toggle and the view pills, because the sidebar is not. */
  showsAppNav: boolean
}

export function useFocusedTitlebar(): FocusedTitlebar {
  const focusedId = useAppStore((s) => s.focusedTerminalId)
  const previewId = useAppStore((s) => s.previewTerminalId)
  const isSidebarOpen = useAppStore((s) => s.isSidebarOpen)
  const isMobile = useIsMobile()

  // Windows and Linux keep their own titlebar in this state — it holds their
  // window controls — so nothing is handed over there.
  const ownsTitlebar = isMac && !isMobile && (!!focusedId || !!previewId)

  return {
    ownsTitlebar,
    // An open sidebar already holds both: SidebarHeader insets for the lights
    // and lists the three views. Repeating them here would draw each twice.
    // The web app has no traffic lights to clear, but it does lose the same
    // controls, hence the split.
    needsTrafficLightPad: ownsTitlebar && !isWeb && !isSidebarOpen,
    showsAppNav: ownsTitlebar && !isSidebarOpen
  }
}
