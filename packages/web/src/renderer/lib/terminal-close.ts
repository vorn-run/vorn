import { useAppStore } from '../stores'
import { destroyTerminal } from './terminal-registry'

const pendingTerminalCloses = new Set<string>()

export function consumePendingTerminalClose(id: string): boolean {
  const pending = pendingTerminalCloses.has(id)
  if (pending) pendingTerminalCloses.delete(id)
  return pending
}

/**
 * Close a session's terminals panel, and the shells it is holding.
 *
 * The shells go with it. They were created by the panel and are claimed by it,
 * and that claim is the only thing keeping them off every other surface — so
 * dropping the panel alone would leave live ptys scattered across the grid as
 * top-level cards nobody asked for, and the next click on the same control
 * would start yet another shell beside them.
 *
 * Each kill is independent, so one failure cannot strand the rest.
 */
export async function closeTerminalsPanel(sessionId: string): Promise<void> {
  const state = useAppStore.getState()
  const held = state.terminalsPanes.get(sessionId)?.terminals ?? []
  for (const heldId of held) {
    pendingTerminalCloses.add(heldId)
    destroyTerminal(heldId)
  }
  // Closes the panel too: its last shell leaving is what shuts it, and
  // `removeTerminal` releases each claim on the way through.
  for (const heldId of held) state.removeTerminal(heldId)
  state.closeTerminalsPane(sessionId)

  await Promise.allSettled(
    held.map(async (target) => {
      try {
        await window.api.killTerminal(target)
      } catch (err) {
        console.warn(`[terminal-close] killTerminal failed for ${target}:`, err)
      }
    })
  )
}

export async function closeTerminalSession(id: string): Promise<void> {
  const state = useAppStore.getState()

  // Shells held in this session's panel go with it. Read before the store is
  // touched — `removeTerminal` drops the panel, and after that there is nothing
  // left to say which ptys these were, so they would keep running unreachable.
  //
  // Each kill is independent: a chain that aborted partway would leave some of
  // them alive with no record of them anywhere.
  const held = state.terminalsPanes.get(id)?.terminals ?? []
  for (const heldId of held) {
    pendingTerminalCloses.add(heldId)
    destroyTerminal(heldId)
  }

  pendingTerminalCloses.add(id)
  if (state.focusedTerminalId === id) state.setFocusedTerminal(null)
  if (state.selectedTerminalId === id) state.setSelectedTerminal(null)
  if (state.renamingTerminalId === id) state.setRenamingTerminalId(null)
  destroyTerminal(id)
  state.removeTerminal(id)

  await Promise.allSettled(
    [...held, id].map(async (target) => {
      try {
        await window.api.killTerminal(target)
      } catch (err) {
        console.warn(`[terminal-close] killTerminal failed for ${target}:`, err)
      }
    })
  )
}
