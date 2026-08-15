import { useAppStore } from '../stores'
import { destroyTerminal } from './terminal-registry'

const pendingTerminalCloses = new Set<string>()

export function consumePendingTerminalClose(id: string): boolean {
  const pending = pendingTerminalCloses.has(id)
  if (pending) pendingTerminalCloses.delete(id)
  return pending
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
